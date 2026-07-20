//! Developer-only, one-shot compatibility measurements for the macOS input
//! method. This module is deliberately absent from production builds.

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::Duration,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use objc2_app_kit::NSRunningApplication;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::ShortcutState;
use tempfile::Builder as TempFileBuilder;

use crate::platform::{
    CompatibilitySelection, TextTargetController, TextTargetError, TextTargetErrorKind,
    TextTargetToken,
};

pub const COMPATIBILITY_SHORTCUT: &str = "CommandOrControl+Shift+Space";
const CATALOG_ID: &str = "spickInputCompatibilityMacOs";
const CATALOG_VERSION: u32 = 2;
const EVIDENCE_SCHEMA_VERSION: u32 = 1;
const MAX_EVIDENCE_BYTES: u64 = 64 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);
const INPUT_SOURCE_IDENTIFIER: &[u8] = b"app.spick.desktop.input-method\0";
const INPUT_SOURCE_MISSING: u32 = 0;
const INPUT_SOURCE_DISABLED: u32 = 1;
const INPUT_SOURCE_ENABLED: u32 = 2;
const INPUT_SOURCE_SELECTED: u32 = 3;

const ASCII_SINGLE: &str = "Spick test: amber river 27.";
const UNICODE_SINGLE: &str = "Café — हिन्दी — 中文 — مرحبًا — 👋";
const MULTILINE: &str = "First line: café.\nSecond line: 中文 👋";
// This remains an inert shell comment even if the tester accidentally presses
// Return after the harness has exited.
const TERMINAL_INERT: &str = "# Spick compatibility check 27";
const COMMUNICATION_DRAFT: &str = "Spick compatibility test — do not send.";

static HARNESS: OnceLock<HarnessRuntime> = OnceLock::new();

extern "C" {
    fn SpickPeerAuthenticationAllowsUnsafeDevelopment() -> bool;
    fn SpickInspectInputSourceState(expected_identifier: *const libc::c_char) -> u32;
}

#[derive(Clone, Copy)]
struct Fixture {
    id: &'static str,
    text: &'static str,
}

#[derive(Clone, Copy)]
enum ExpectedObservation {
    Confirmed,
    CaptureRefusal(&'static [TextTargetErrorKind]),
}

#[derive(Clone, Copy)]
struct CompatibilityCase {
    id: &'static str,
    target_bundle_identifier: &'static str,
    target_name: &'static str,
    control: &'static str,
    selection: CompatibilitySelection,
    fixture: Fixture,
    expected: ExpectedObservation,
    setup: &'static str,
}

const SECURE_FIELD: &[TextTargetErrorKind] = &[TextTargetErrorKind::SecureField];

const CASES: &[CompatibilityCase] = &[
    CompatibilityCase {
        id: "macos.textEdit.plain.caretAscii",
        target_bundle_identifier: "com.apple.TextEdit",
        target_name: "TextEdit",
        control: "unsaved plain-text document, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "asciiSingleV1", text: ASCII_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Use an unsaved plain-text document and place an empty caret in scratch text.",
    },
    CompatibilityCase {
        id: "macos.textEdit.plain.replaceUnicode",
        target_bundle_identifier: "com.apple.TextEdit",
        target_name: "TextEdit",
        control: "unsaved plain-text document, fixed selection",
        selection: CompatibilitySelection::Range,
        fixture: Fixture { id: "unicodeSingleV1", text: UNICODE_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Use an unsaved plain-text document and select only disposable scratch text.",
    },
    CompatibilityCase {
        id: "macos.textEdit.plain.multiline",
        target_bundle_identifier: "com.apple.TextEdit",
        target_name: "TextEdit",
        control: "unsaved plain-text document, multiline caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "multilineV1", text: MULTILINE },
        expected: ExpectedObservation::Confirmed,
        setup: "Use an unsaved plain-text document and place an empty caret in scratch text.",
    },
    CompatibilityCase {
        id: "macos.textEdit.rich.caretUnicode",
        target_bundle_identifier: "com.apple.TextEdit",
        target_name: "TextEdit",
        control: "unsaved rich-text document, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "unicodeSingleV1", text: UNICODE_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Use an unsaved rich-text document and place an empty caret in scratch text.",
    },
    CompatibilityCase {
        id: "macos.notes.body.caretUnicode",
        target_bundle_identifier: "com.apple.Notes",
        target_name: "Notes",
        control: "new disposable note body, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "unicodeSingleV1", text: UNICODE_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Create a disposable note and place an empty caret in its body.",
    },
    CompatibilityCase {
        id: "macos.chrome.input.caretAscii",
        target_bundle_identifier: "com.google.Chrome",
        target_name: "Google Chrome",
        control: "offline fixture text input, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "asciiSingleV1", text: ASCII_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Open the checked-in browser fixture and focus its Single line field.",
    },
    CompatibilityCase {
        id: "macos.chrome.input.replaceUnicode",
        target_bundle_identifier: "com.google.Chrome",
        target_name: "Google Chrome",
        control: "offline fixture text input, fixed selection",
        selection: CompatibilitySelection::Range,
        fixture: Fixture { id: "unicodeSingleV1", text: UNICODE_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Open the checked-in browser fixture and select disposable text in Single line.",
    },
    CompatibilityCase {
        id: "macos.chrome.textarea.multiline",
        target_bundle_identifier: "com.google.Chrome",
        target_name: "Google Chrome",
        control: "offline fixture textarea, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "multilineV1", text: MULTILINE },
        expected: ExpectedObservation::Confirmed,
        setup: "Open the checked-in browser fixture and focus Long note at an empty caret.",
    },
    CompatibilityCase {
        id: "macos.chrome.contentEditable.replaceUnicode",
        target_bundle_identifier: "com.google.Chrome",
        target_name: "Google Chrome",
        control: "offline fixture contenteditable, fixed selection",
        selection: CompatibilitySelection::Range,
        fixture: Fixture { id: "unicodeSingleV1", text: UNICODE_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Open the checked-in browser fixture and select disposable text in Rich editor.",
    },
    CompatibilityCase {
        id: "macos.chrome.password.reject",
        target_bundle_identifier: "com.google.Chrome",
        target_name: "Google Chrome",
        control: "offline fixture password input",
        selection: CompatibilitySelection::Any,
        fixture: Fixture { id: "asciiSingleV1", text: ASCII_SINGLE },
        expected: ExpectedObservation::CaptureRefusal(SECURE_FIELD),
        setup: "Open the checked-in browser fixture and focus Protected field. Never use a real secret.",
    },
    CompatibilityCase {
        id: "macos.safari.input.caretUnicode",
        target_bundle_identifier: "com.apple.Safari",
        target_name: "Safari",
        control: "offline fixture text input, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "unicodeSingleV1", text: UNICODE_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Open the checked-in browser fixture and focus its Single line field.",
    },
    CompatibilityCase {
        id: "macos.safari.password.reject",
        target_bundle_identifier: "com.apple.Safari",
        target_name: "Safari",
        control: "offline fixture password input",
        selection: CompatibilitySelection::Any,
        fixture: Fixture { id: "asciiSingleV1", text: ASCII_SINGLE },
        expected: ExpectedObservation::CaptureRefusal(SECURE_FIELD),
        setup: "Open the checked-in browser fixture and focus Protected field. Never use a real secret.",
    },
    CompatibilityCase {
        id: "macos.vsCode.monaco.caretUnicode",
        target_bundle_identifier: "com.microsoft.VSCode",
        target_name: "Visual Studio Code",
        control: "untitled plaintext editor, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "unicodeSingleV1", text: UNICODE_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Use a new untitled plaintext editor and place an empty caret in scratch text.",
    },
    CompatibilityCase {
        id: "macos.vsCode.monaco.replaceAscii",
        target_bundle_identifier: "com.microsoft.VSCode",
        target_name: "Visual Studio Code",
        control: "untitled plaintext editor, fixed selection",
        selection: CompatibilitySelection::Range,
        fixture: Fixture { id: "asciiSingleV1", text: ASCII_SINGLE },
        expected: ExpectedObservation::Confirmed,
        setup: "Use a new untitled plaintext editor and select only disposable scratch text.",
    },
    CompatibilityCase {
        id: "macos.terminal.shellPrompt.caretInert",
        target_bundle_identifier: "com.apple.Terminal",
        target_name: "Terminal",
        control: "fresh shell prompt, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "terminalInertV1", text: TERMINAL_INERT },
        expected: ExpectedObservation::Confirmed,
        setup: "Use a fresh prompt. The fixture is a comment with no newline; do not press Return.",
    },
    CompatibilityCase {
        id: "macos.slack.composer.caretDraft",
        target_bundle_identifier: "com.tinyspeck.slackmacgap",
        target_name: "Slack",
        control: "private draft composer, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "communicationDraftV1", text: COMMUNICATION_DRAFT },
        expected: ExpectedObservation::Confirmed,
        setup: "Use a cleared private draft. Do not send the fixture.",
    },
    CompatibilityCase {
        id: "macos.chatGpt.composer.caretDraft",
        target_bundle_identifier: "com.openai.codex",
        target_name: "ChatGPT",
        control: "new-chat composer, empty caret",
        selection: CompatibilitySelection::Caret,
        fixture: Fixture { id: "communicationDraftV1", text: COMMUNICATION_DRAFT },
        expected: ExpectedObservation::Confirmed,
        setup: "Use a cleared new-chat composer. Do not send the fixture.",
    },
];

struct HarnessRuntime {
    run_id: String,
    run_profile: RunProfile,
    case: &'static CompatibilityCase,
    text_targets: TextTargetController,
    state: Mutex<ProbeState>,
    journal: EvidenceJournal,
    build: RuntimeBuildIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum RunProfile {
    Cold,
    Warm,
}

impl RunProfile {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "cold" => Ok(Self::Cold),
            "warm" => Ok(Self::Warm),
            _ => Err("compatibility run profile must be cold or warm".into()),
        }
    }
}

struct RuntimeBuildIdentity {
    source_revision: String,
    source_tree: String,
    signing_mode: String,
    peer_auth_mode: String,
    desktop_cd_hash: String,
    installed_helper_cd_hash: String,
    helper_version: String,
    helper_bundle_version: String,
}

struct CodeIdentity {
    cd_hash: String,
    team_identifier: String,
    ad_hoc: bool,
    hardened_runtime: bool,
}

enum ProbeState {
    Ready,
    Capturing {
        release_pending: bool,
    },
    Captured {
        token: TextTargetToken,
        started_at: Instant,
        started_at_unix_ms: u64,
        capture_ms: u64,
        target_version: Option<String>,
        target_build: Option<String>,
    },
    Finished,
}

/// Parse the diagnostic process command. `Ok(true)` starts Tauri; `Ok(false)`
/// completed an inspection/review command without creating an app.
pub fn prepare_process() -> Result<bool, String> {
    prepare_process_arguments(std::env::args_os().skip(1).collect())
}

fn prepare_process_arguments(arguments: Vec<OsString>) -> Result<bool, String> {
    let arguments = arguments
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "compatibility arguments must be valid UTF-8".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;

    match arguments.as_slice() {
        [command] if command == "--print-input-method-compatibility-mode" => {
            println!("fixed-fixture-v1");
            Ok(false)
        }
        [command] if command == "--list-input-method-compatibility-cases" => {
            for case in CASES {
                println!("{}\t{}\t{}", case.id, case.target_name, case.control);
            }
            Ok(false)
        }
        [command] if command == "--print-input-method-compatibility-input-source-state" => {
            let state =
                unsafe { SpickInspectInputSourceState(INPUT_SOURCE_IDENTIFIER.as_ptr().cast()) };
            let value = match state {
                INPUT_SOURCE_MISSING => "missing",
                INPUT_SOURCE_DISABLED => "disabled",
                INPUT_SOURCE_ENABLED => "enabled",
                INPUT_SOURCE_SELECTED => "selected",
                _ => return Err("macOS reported an invalid Spick Input source state".into()),
            };
            println!("{value}");
            Ok(false)
        }
        [command, case_id] if command == "--describe-input-method-compatibility-case" => {
            let case = find_case(case_id)?;
            println!("Case: {}", case.id);
            println!(
                "Target: {} ({})",
                case.target_name, case.target_bundle_identifier
            );
            println!("Control: {}", case.control);
            println!("Setup: {}", case.setup);
            println!("Fixture [{}]: {}", case.fixture.id, case.fixture.text);
            println!("Expected: {}", expected_observation_code(case.expected));
            println!("Shortcut: {COMPATIBILITY_SHORTCUT}");
            Ok(false)
        }
        [command, case_id, run_id, run_profile]
            if command == "--input-method-compatibility-probe" =>
        {
            validate_uuid_v4(run_id)?;
            let case = find_case(case_id)?;
            let run_profile = RunProfile::parse(run_profile)?;
            let root = validate_evidence_root()?;
            let build = collect_runtime_build_identity()?;
            let journal = EvidenceJournal::create(&root, run_id, case.id)?;
            HARNESS
                .set(HarnessRuntime {
                    run_id: run_id.clone(),
                    run_profile,
                    case,
                    text_targets: TextTargetController::default(),
                    state: Mutex::new(ProbeState::Ready),
                    journal,
                    build,
                })
                .map_err(|_| "a compatibility probe is already active".to_string())?;
            Ok(true)
        }
        [command, run_id, review_id, document, caret, external]
            if command == "--record-input-method-compatibility-review" =>
        {
            validate_uuid_v4(run_id)?;
            validate_uuid_v4(review_id)?;
            record_manual_review(run_id, review_id, document, caret, external)?;
            Ok(false)
        }
        [command, run_id, outcome]
            if command == "--record-input-method-compatibility-lifecycle" =>
        {
            validate_uuid_v4(run_id)?;
            record_runner_lifecycle(run_id, outcome)?;
            Ok(false)
        }
        [command, run_id] if command == "--classify-input-method-compatibility-attempt" => {
            validate_uuid_v4(run_id)?;
            println!("{}", classify_reviewed_attempt(run_id)?);
            Ok(false)
        }
        _ => Err(compatibility_usage()),
    }
}

pub fn is_active() -> bool {
    HARNESS.get().is_some()
}

pub fn ready_message() -> String {
    let Some(runtime) = HARNESS.get() else {
        return "The compatibility harness is not active.".into();
    };
    format!(
        "Compatibility case {} is ready. Focus the prepared {} control, then press and release {}.",
        runtime.case.id, runtime.case.target_name, COMPATIBILITY_SHORTCUT
    )
}

pub fn handle_shortcut<R: Runtime>(app: &AppHandle<R>, event: ShortcutState) {
    let Some(runtime) = HARNESS.get() else {
        return;
    };
    match event {
        ShortcutState::Pressed => runtime.press(app),
        ShortcutState::Released => runtime.release(app),
    }
}

pub fn start_watchdog<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    thread::Builder::new()
        .name("spick-compat-watchdog".into())
        .spawn(move || {
            thread::sleep(PROBE_TIMEOUT);
            if let Some(runtime) = HARNESS.get() {
                runtime.timeout(&app);
            }
        })
        .map(|_| ())
        .map_err(|error| format!("could not start the compatibility watchdog: {error}"))
}

impl HarnessRuntime {
    fn timeout<R: Runtime>(&self, app: &AppHandle<R>) {
        let token = {
            let Ok(mut state) = self.state.lock() else {
                app.exit(75);
                return;
            };
            let token = match *state {
                ProbeState::Captured { token, .. } => Some(token),
                ProbeState::Finished => return,
                ProbeState::Ready | ProbeState::Capturing { .. } => None,
            };
            *state = ProbeState::Finished;
            token
        };
        if let Some(token) = token {
            self.text_targets.discard(token);
        }
        let _ = self
            .journal
            .append("aborted", Some(("probeTimedOut", false, None)))
            .and_then(|_| self.journal.seal());
        eprintln!("Compatibility probe expired without a completed one-shot attempt.");
        app.exit(75);
    }

    fn press<R: Runtime>(&self, app: &AppHandle<R>) {
        let (started_at, started_at_unix_ms) = {
            let Ok(mut state) = self.state.lock() else {
                self.exit_without_insertion(app, "stateUnavailable");
                return;
            };
            if !matches!(*state, ProbeState::Ready) {
                return;
            }
            let started_at = Instant::now();
            let Ok(started_at_unix_ms) = unix_milliseconds() else {
                *state = ProbeState::Finished;
                app.exit(74);
                return;
            };
            *state = ProbeState::Capturing {
                release_pending: false,
            };
            (started_at, started_at_unix_ms)
        };

        if self.journal.append("started", None).is_err() {
            self.exit_without_insertion(app, "evidenceUnavailable");
            return;
        }

        let capture_started = Instant::now();
        let capture = self
            .text_targets
            .capture_for_compatibility(self.case.target_bundle_identifier, self.case.selection);
        let capture_ms = elapsed_milliseconds(capture_started);

        match capture {
            Ok(target) => {
                let (target_version, target_build) = running_target_versions(
                    target.compatibility_target_pid,
                    self.case.target_bundle_identifier,
                );
                if matches!(self.case.expected, ExpectedObservation::CaptureRefusal(_)) {
                    self.text_targets.discard(target.token);
                    self.finish(
                        app,
                        AttemptObservation::unexpected_capture(
                            capture_ms,
                            target_version,
                            target_build,
                        ),
                        started_at,
                        started_at_unix_ms,
                    );
                    return;
                }

                let release_pending = {
                    let Ok(mut state) = self.state.lock() else {
                        self.text_targets.discard(target.token);
                        self.exit_without_insertion(app, "stateUnavailable");
                        return;
                    };
                    let release_pending = match &*state {
                        ProbeState::Capturing {
                            release_pending, ..
                        } => *release_pending,
                        _ => {
                            self.text_targets.discard(target.token);
                            return;
                        }
                    };
                    *state = ProbeState::Captured {
                        token: target.token,
                        started_at,
                        started_at_unix_ms,
                        capture_ms,
                        target_version,
                        target_build,
                    };
                    release_pending
                };
                if release_pending {
                    self.release(app);
                }
            }
            Err(error) => {
                let target_versions = if error.kind == TextTargetErrorKind::SecureField {
                    error
                        .compatibility_target_pid
                        .map(|pid| running_target_versions(pid, self.case.target_bundle_identifier))
                        .unwrap_or((None, None))
                } else {
                    (None, None)
                };
                let observation = AttemptObservation::from_capture_error(
                    &error,
                    self.case,
                    capture_ms,
                    target_versions.0,
                    target_versions.1,
                );
                self.finish(app, observation, started_at, started_at_unix_ms);
            }
        }
    }

    fn release<R: Runtime>(&self, app: &AppHandle<R>) {
        let captured = {
            let Ok(mut state) = self.state.lock() else {
                self.exit_without_insertion(app, "stateUnavailable");
                return;
            };
            match &mut *state {
                ProbeState::Capturing {
                    release_pending, ..
                } => {
                    *release_pending = true;
                    return;
                }
                ProbeState::Captured {
                    token,
                    started_at,
                    started_at_unix_ms,
                    capture_ms,
                    target_version,
                    target_build,
                } => {
                    let captured = (
                        *token,
                        *started_at,
                        *started_at_unix_ms,
                        *capture_ms,
                        target_version.clone(),
                        target_build.clone(),
                    );
                    *state = ProbeState::Finished;
                    captured
                }
                ProbeState::Ready | ProbeState::Finished => return,
            }
        };

        let (token, started_at, started_at_unix_ms, capture_ms, target_version, target_build) =
            captured;
        let commit_started = Instant::now();
        let result = self.text_targets.commit(token, self.case.fixture.text);
        let commit_ms = elapsed_milliseconds(commit_started);
        let observation = AttemptObservation::from_commit_result(
            result,
            self.case,
            capture_ms,
            commit_ms,
            target_version,
            target_build,
        );
        self.finish(app, observation, started_at, started_at_unix_ms);
    }

    fn finish<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        observation: AttemptObservation,
        started_at: Instant,
        started_at_unix_ms: u64,
    ) {
        if let Ok(mut state) = self.state.lock() {
            *state = ProbeState::Finished;
        }
        let completed_at_unix_ms = match unix_milliseconds() {
            Ok(value) => value,
            Err(error) => {
                let _ = self
                    .journal
                    .append("finished", Some(("clockUnavailable", false, None)))
                    .and_then(|_| self.journal.seal());
                eprintln!(
                    "The compatibility attempt ran once, but evidence could not be finalized: {error}. The fixture will not be retried."
                );
                app.exit(74);
                return;
            }
        };
        let attempt = AttemptEvidence::new(
            self,
            observation,
            started_at_unix_ms,
            completed_at_unix_ms,
            elapsed_milliseconds(started_at),
        );
        let write_result = write_attempt(&attempt);

        match write_result {
            Ok(written) => {
                if let Err(error) = self
                    .journal
                    .append(
                        "finished",
                        Some((
                            attempt.observation.verdict.as_str(),
                            true,
                            Some(&written.sha256),
                        )),
                    )
                    .and_then(|_| self.journal.seal())
                {
                    eprintln!(
                        "The compatibility attempt ran once, but its terminal journal could not be sealed: {error}. The fixture will not be retried."
                    );
                    app.exit(74);
                    return;
                }
                eprintln!("Compatibility evidence: {}", written.path.display());
                app.exit(
                    if attempt.observation.verdict == TechnicalVerdict::ExpectedObservation {
                        0
                    } else {
                        3
                    },
                );
            }
            Err(error) => {
                let _ = self
                    .journal
                    .append("finished", Some(("evidenceUnavailable", false, None)))
                    .and_then(|_| self.journal.seal());
                eprintln!(
                    "The compatibility attempt ran once, but evidence could not be finalized: {error}. The fixture will not be retried."
                );
                app.exit(74);
            }
        }
    }

    fn exit_without_insertion<R: Runtime>(&self, app: &AppHandle<R>, code: &'static str) {
        if let Ok(mut state) = self.state.lock() {
            if let ProbeState::Captured { token, .. } = *state {
                self.text_targets.discard(token);
            }
            *state = ProbeState::Finished;
        }
        let _ = self
            .journal
            .append("aborted", Some((code, false, None)))
            .and_then(|_| self.journal.seal());
        eprintln!("Compatibility probe aborted before insertion: {code}");
        app.exit(74);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum TechnicalVerdict {
    ExpectedObservation,
    UnexpectedObservation,
    Indeterminate,
}

impl TechnicalVerdict {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ExpectedObservation => "expectedObservation",
            Self::UnexpectedObservation => "unexpectedObservation",
            Self::Indeterminate => "indeterminate",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttemptObservation {
    phase: String,
    outcome: String,
    error_kind: Option<String>,
    fixture_commit_attempted: bool,
    caret_repositioned: Option<bool>,
    automatic_retry_count: u8,
    verdict: TechnicalVerdict,
    #[serde(skip)]
    capture_ms: u64,
    #[serde(skip)]
    commit_ms: Option<u64>,
    #[serde(skip)]
    target_version: Option<String>,
    #[serde(skip)]
    target_build: Option<String>,
    #[serde(skip)]
    authenticated_helper_cd_hash: Option<String>,
}

impl AttemptObservation {
    fn unexpected_capture(
        capture_ms: u64,
        target_version: Option<String>,
        target_build: Option<String>,
    ) -> Self {
        Self {
            phase: "capture".into(),
            outcome: "capturedWhenRefusalExpected".into(),
            error_kind: None,
            fixture_commit_attempted: false,
            caret_repositioned: None,
            automatic_retry_count: 0,
            verdict: TechnicalVerdict::UnexpectedObservation,
            capture_ms,
            commit_ms: None,
            target_version,
            target_build,
            authenticated_helper_cd_hash: None,
        }
    }

    fn from_capture_error(
        error: &TextTargetError,
        case: &CompatibilityCase,
        capture_ms: u64,
        target_version: Option<String>,
        target_build: Option<String>,
    ) -> Self {
        let expected = match case.expected {
            ExpectedObservation::CaptureRefusal(kinds) => kinds.contains(&error.kind),
            ExpectedObservation::Confirmed => false,
        };
        Self {
            phase: "capture".into(),
            outcome: error_outcome(error.kind).into(),
            error_kind: Some(error_kind_code(error.kind).into()),
            fixture_commit_attempted: false,
            caret_repositioned: None,
            automatic_retry_count: 0,
            verdict: if error.kind == TextTargetErrorKind::Indeterminate {
                TechnicalVerdict::Indeterminate
            } else if expected {
                TechnicalVerdict::ExpectedObservation
            } else {
                TechnicalVerdict::UnexpectedObservation
            },
            capture_ms,
            commit_ms: None,
            target_version,
            target_build,
            authenticated_helper_cd_hash: None,
        }
    }

    fn from_commit_result(
        result: Result<crate::platform::TextInsertionReceipt, TextTargetError>,
        case: &CompatibilityCase,
        capture_ms: u64,
        commit_ms: u64,
        target_version: Option<String>,
        target_build: Option<String>,
    ) -> Self {
        match result {
            Ok(receipt) => {
                let authenticated_helper_cd_hash = receipt.compatibility_peer_cd_hash.clone();
                Self {
                    phase: "commit".into(),
                    outcome: "confirmed".into(),
                    error_kind: None,
                    fixture_commit_attempted: true,
                    caret_repositioned: Some(receipt.caret_repositioned),
                    automatic_retry_count: 0,
                    verdict: if matches!(case.expected, ExpectedObservation::Confirmed) {
                        if receipt.caret_repositioned {
                            TechnicalVerdict::ExpectedObservation
                        } else {
                            TechnicalVerdict::UnexpectedObservation
                        }
                    } else {
                        TechnicalVerdict::UnexpectedObservation
                    },
                    capture_ms,
                    commit_ms: Some(commit_ms),
                    target_version,
                    target_build,
                    authenticated_helper_cd_hash,
                }
            }
            Err(error) => Self {
                phase: "commit".into(),
                outcome: error_outcome(error.kind).into(),
                error_kind: Some(error_kind_code(error.kind).into()),
                fixture_commit_attempted: true,
                caret_repositioned: None,
                automatic_retry_count: 0,
                verdict: if error.kind == TextTargetErrorKind::Indeterminate {
                    TechnicalVerdict::Indeterminate
                } else {
                    TechnicalVerdict::UnexpectedObservation
                },
                capture_ms,
                commit_ms: Some(commit_ms),
                target_version,
                target_build,
                authenticated_helper_cd_hash: None,
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogReference {
    id: String,
    version: u32,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureReference {
    id: String,
    sha256: String,
    utf16_code_units: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildEvidence {
    spick_version: String,
    helper_version: String,
    helper_bundle_version: String,
    source_revision: String,
    source_tree: String,
    protocol_version: u32,
    signing_mode: String,
    peer_auth_mode: String,
    desktop_cd_hash: String,
    installed_helper_cd_hash: String,
    authenticated_helper_cd_hash: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostEvidence {
    mac_os_version: Option<String>,
    mac_os_build: Option<String>,
    architecture: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetEvidence {
    bundle_identifier: String,
    control: String,
    version: Option<String>,
    build: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TimingEvidence {
    capture: u64,
    commit: Option<u64>,
    total: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttemptEvidence {
    record_type: String,
    schema_version: u32,
    catalog: CatalogReference,
    run_id: String,
    run_profile: RunProfile,
    case_id: String,
    expected_observation: String,
    fixture: FixtureReference,
    build: BuildEvidence,
    host: HostEvidence,
    target: TargetEvidence,
    started_at_unix_ms: u64,
    completed_at_unix_ms: u64,
    observation: AttemptObservation,
    timings_ms: TimingEvidence,
}

impl AttemptEvidence {
    fn new(
        runtime: &HarnessRuntime,
        observation: AttemptObservation,
        started_at_unix_ms: u64,
        completed_at_unix_ms: u64,
        total_ms: u64,
    ) -> Self {
        let capture_ms = observation.capture_ms.min(60_000);
        let commit_ms = observation.commit_ms.map(|value| value.min(60_000));
        let target_version = observation.target_version.clone();
        let target_build = observation.target_build.clone();
        let authenticated_helper_cd_hash = observation.authenticated_helper_cd_hash.clone();
        Self {
            record_type: "attempt".into(),
            schema_version: EVIDENCE_SCHEMA_VERSION,
            catalog: CatalogReference {
                id: CATALOG_ID.into(),
                version: CATALOG_VERSION,
                sha256: catalog_digest(),
            },
            run_id: runtime.run_id.clone(),
            run_profile: runtime.run_profile,
            case_id: runtime.case.id.into(),
            expected_observation: expected_observation_code(runtime.case.expected).into(),
            fixture: FixtureReference {
                id: runtime.case.fixture.id.into(),
                sha256: sha256_hex(runtime.case.fixture.text.as_bytes()),
                utf16_code_units: runtime.case.fixture.text.encode_utf16().count(),
            },
            build: BuildEvidence {
                spick_version: env!("CARGO_PKG_VERSION").into(),
                helper_version: runtime.build.helper_version.clone(),
                helper_bundle_version: runtime.build.helper_bundle_version.clone(),
                source_revision: runtime.build.source_revision.clone(),
                source_tree: runtime.build.source_tree.clone(),
                protocol_version: u32::from(crate::platform::INPUT_METHOD_PROTOCOL_VERSION),
                signing_mode: runtime.build.signing_mode.clone(),
                peer_auth_mode: runtime.build.peer_auth_mode.clone(),
                desktop_cd_hash: runtime.build.desktop_cd_hash.clone(),
                installed_helper_cd_hash: runtime.build.installed_helper_cd_hash.clone(),
                authenticated_helper_cd_hash,
            },
            host: HostEvidence {
                mac_os_version: macos_value("-productVersion"),
                mac_os_build: macos_value("-buildVersion"),
                architecture: host_architecture().into(),
            },
            target: TargetEvidence {
                bundle_identifier: runtime.case.target_bundle_identifier.into(),
                control: runtime.case.control.into(),
                version: target_version,
                build: target_build,
            },
            started_at_unix_ms,
            completed_at_unix_ms,
            observation,
            timings_ms: TimingEvidence {
                capture: capture_ms,
                commit: commit_ms,
                total: total_ms.min(60_000),
            },
        }
    }

    fn validate_against_catalog(
        &self,
        expected_run_id: &str,
    ) -> Result<&'static CompatibilityCase, String> {
        validate_uuid_v4(&self.run_id)?;
        if self.record_type != "attempt"
            || self.schema_version != EVIDENCE_SCHEMA_VERSION
            || self.run_id != expected_run_id
            || self.catalog.id != CATALOG_ID
            || self.catalog.version != CATALOG_VERSION
            || self.catalog.sha256 != catalog_digest()
        {
            return Err("attempt evidence does not match this compatibility catalog".into());
        }
        let case = find_case(&self.case_id)?;
        if self.expected_observation != expected_observation_code(case.expected)
            || self.fixture.id != case.fixture.id
            || self.fixture.sha256 != sha256_hex(case.fixture.text.as_bytes())
            || self.fixture.utf16_code_units != case.fixture.text.encode_utf16().count()
            || self.target.bundle_identifier != case.target_bundle_identifier
            || self.target.control != case.control
        {
            return Err("attempt evidence conflicts with its catalog case".into());
        }
        if self.build.spick_version != env!("CARGO_PKG_VERSION")
            || self.build.helper_version != env!("CARGO_PKG_VERSION")
            || !valid_public_version(&self.build.helper_bundle_version)
            || !valid_lower_hex(&self.build.source_revision, &[40])
            || !matches!(self.build.source_tree.as_str(), "clean" | "dirty")
            || self.build.protocol_version
                != u32::from(crate::platform::INPUT_METHOD_PROTOCOL_VERSION)
            || !matches!(
                self.build.signing_mode.as_str(),
                "development" | "unsafeAdhoc"
            )
            || !matches!(self.build.peer_auth_mode.as_str(), "secure" | "unsafeAdhoc")
            || !valid_lower_hex(&self.build.desktop_cd_hash, &[40, 64])
            || !valid_lower_hex(&self.build.installed_helper_cd_hash, &[40, 64])
            || self
                .build
                .authenticated_helper_cd_hash
                .as_deref()
                .is_some_and(|value| {
                    !valid_lower_hex(value, &[40, 64])
                        || value != self.build.installed_helper_cd_hash
                })
            || (self.build.signing_mode == "development" && self.build.peer_auth_mode != "secure")
            || (self.build.signing_mode == "unsafeAdhoc"
                && self.build.peer_auth_mode != "unsafeAdhoc")
        {
            return Err("attempt evidence contains invalid build provenance".into());
        }
        if !matches!(self.host.architecture.as_str(), "arm64" | "x86_64")
            || self
                .host
                .mac_os_version
                .as_deref()
                .is_some_and(|value| !valid_public_version(value))
            || self
                .host
                .mac_os_build
                .as_deref()
                .is_some_and(|value| !valid_public_version(value))
            || self
                .target
                .version
                .as_deref()
                .is_some_and(|value| !valid_public_version(value))
            || self
                .target
                .build
                .as_deref()
                .is_some_and(|value| !valid_public_version(value))
        {
            return Err("attempt evidence contains invalid public version metadata".into());
        }
        if self.started_at_unix_ms == 0
            || self.completed_at_unix_ms < self.started_at_unix_ms
            || self.timings_ms.capture > 60_000
            || self.timings_ms.total > 60_000
            || self.timings_ms.capture > self.timings_ms.total
            || self
                .timings_ms
                .commit
                .is_some_and(|value| value > 60_000 || value > self.timings_ms.total)
            || self.timings_ms.commit.is_some_and(|value| {
                self.timings_ms.capture.saturating_add(value) > self.timings_ms.total
            })
        {
            return Err("attempt evidence contains inconsistent timings".into());
        }

        let error_kind = self
            .observation
            .error_kind
            .as_deref()
            .map(parse_error_kind)
            .transpose()?;
        let structural_result = match self.observation.outcome.as_str() {
            "confirmed" => {
                self.observation.phase == "commit"
                    && error_kind.is_none()
                    && self.observation.fixture_commit_attempted
                    && self.observation.caret_repositioned.is_some()
                    && self.timings_ms.commit.is_some()
            }
            "capturedWhenRefusalExpected" => {
                self.observation.phase == "capture"
                    && error_kind.is_none()
                    && !self.observation.fixture_commit_attempted
                    && self.observation.caret_repositioned.is_none()
                    && self.timings_ms.commit.is_none()
            }
            "refused" | "indeterminate" | "timedOut" | "unavailable" | "failed" => {
                let Some(kind) = error_kind else {
                    return Err("attempt evidence omitted its closed error kind".into());
                };
                self.observation.outcome == error_outcome(kind)
                    && self.observation.caret_repositioned.is_none()
                    && self.observation.fixture_commit_attempted
                        == (self.observation.phase == "commit")
                    && self.timings_ms.commit.is_some() == (self.observation.phase == "commit")
            }
            _ => false,
        };
        if !structural_result
            || !matches!(self.observation.phase.as_str(), "capture" | "commit")
            || self.observation.automatic_retry_count != 0
        {
            return Err("attempt evidence contains an inconsistent observation".into());
        }
        if self.observation.outcome == "confirmed"
            && self.build.authenticated_helper_cd_hash.as_deref()
                != Some(self.build.installed_helper_cd_hash.as_str())
        {
            return Err("confirmed evidence is not bound to the live authenticated helper".into());
        }
        let recomputed = recompute_verdict(case, &self.observation, error_kind);
        if self.observation.verdict != recomputed {
            return Err("attempt evidence contains an inconsistent technical verdict".into());
        }
        Ok(case)
    }
}

fn recompute_verdict(
    case: &CompatibilityCase,
    observation: &AttemptObservation,
    error_kind: Option<TextTargetErrorKind>,
) -> TechnicalVerdict {
    if error_kind == Some(TextTargetErrorKind::Indeterminate) {
        return TechnicalVerdict::Indeterminate;
    }
    match case.expected {
        ExpectedObservation::Confirmed
            if observation.phase == "commit"
                && observation.outcome == "confirmed"
                && observation.caret_repositioned == Some(true) =>
        {
            TechnicalVerdict::ExpectedObservation
        }
        ExpectedObservation::CaptureRefusal(kinds)
            if observation.phase == "capture"
                && error_kind.is_some_and(|kind| kinds.contains(&kind)) =>
        {
            TechnicalVerdict::ExpectedObservation
        }
        ExpectedObservation::Confirmed | ExpectedObservation::CaptureRefusal(_) => {
            TechnicalVerdict::UnexpectedObservation
        }
    }
}

struct EvidenceJournal {
    file: Mutex<File>,
    root: PathBuf,
    sealed: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JournalEvent<'a> {
    record_type: &'static str,
    schema_version: u32,
    run_id: &'a str,
    case_id: &'a str,
    state: &'a str,
    terminal_code: Option<&'a str>,
    evidence_written: Option<bool>,
    attempt_sha256: Option<&'a str>,
    at_unix_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JournalEventRecord {
    record_type: String,
    schema_version: u32,
    run_id: String,
    case_id: String,
    state: String,
    terminal_code: Option<String>,
    evidence_written: Option<bool>,
    attempt_sha256: Option<String>,
    at_unix_ms: u64,
}

impl EvidenceJournal {
    fn create(root: &Path, run_id: &str, case_id: &str) -> Result<Self, String> {
        let path = root.join(format!("journal-{run_id}.jsonl"));
        let mut file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                format!("could not reserve private compatibility evidence: {error}")
            })?;
        write_json_line(
            &mut file,
            &JournalEvent {
                record_type: "journalEvent",
                schema_version: EVIDENCE_SCHEMA_VERSION,
                run_id,
                case_id,
                state: "prepared",
                terminal_code: None,
                evidence_written: None,
                attempt_sha256: None,
                at_unix_ms: unix_milliseconds()?,
            },
        )?;
        file.sync_all()
            .map_err(|error| format!("could not sync compatibility evidence: {error}"))?;
        sync_directory(root)?;
        Ok(Self {
            file: Mutex::new(file),
            root: root.into(),
            sealed: AtomicBool::new(false),
        })
    }

    fn append(
        &self,
        state: &str,
        terminal: Option<(&str, bool, Option<&str>)>,
    ) -> Result<(), String> {
        if self.sealed.load(Ordering::Acquire) {
            return Err("compatibility evidence journal is already sealed".into());
        }
        let runtime = HARNESS
            .get()
            .ok_or_else(|| "compatibility runtime is unavailable".to_string())?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| "compatibility evidence journal is unavailable".to_string())?;
        write_json_line(
            &mut file,
            &JournalEvent {
                record_type: "journalEvent",
                schema_version: EVIDENCE_SCHEMA_VERSION,
                run_id: &runtime.run_id,
                case_id: runtime.case.id,
                state,
                terminal_code: terminal.map(|value| value.0),
                evidence_written: terminal.map(|value| value.1),
                attempt_sha256: terminal.and_then(|value| value.2),
                at_unix_ms: unix_milliseconds()?,
            },
        )?;
        file.sync_all()
            .map_err(|error| format!("could not sync compatibility evidence: {error}"))
    }

    fn seal(&self) -> Result<(), String> {
        let file = self
            .file
            .lock()
            .map_err(|_| "compatibility evidence journal is unavailable".to_string())?;
        file.set_permissions(fs::Permissions::from_mode(0o400))
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("could not seal compatibility evidence: {error}"))?;
        sync_directory(&self.root)?;
        self.sealed.store(true, Ordering::Release);
        Ok(())
    }
}

struct WrittenEvidence {
    path: PathBuf,
    sha256: String,
}

fn write_attempt(attempt: &AttemptEvidence) -> Result<WrittenEvidence, String> {
    let root = validate_evidence_root()?;
    let destination = root.join(format!("attempt-{}.json", attempt.run_id));
    let mut temporary = TempFileBuilder::new()
        .prefix(".attempt-")
        .tempfile_in(&root)
        .map_err(|error| format!("could not create private attempt evidence: {error}"))?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not protect attempt evidence: {error}"))?;
    let mut bytes = serde_json::to_vec_pretty(attempt)
        .map_err(|error| format!("could not encode attempt evidence: {error}"))?;
    bytes.push(b'\n');
    temporary
        .write_all(&bytes)
        .map_err(|error| format!("could not finish attempt evidence: {error}"))?;
    temporary
        .flush()
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("could not sync attempt evidence: {error}"))?;
    let persisted = temporary
        .persist_noclobber(&destination)
        .map_err(|error| format!("could not publish attempt evidence: {}", error.error))?;
    persisted
        .set_permissions(fs::Permissions::from_mode(0o400))
        .and_then(|_| persisted.sync_all())
        .map_err(|error| format!("could not seal attempt evidence: {error}"))?;
    sync_directory(&root)?;
    Ok(WrittenEvidence {
        path: destination,
        sha256: sha256_hex(&bytes),
    })
}

fn record_manual_review(
    run_id: &str,
    review_id: &str,
    document: &str,
    caret: &str,
    external: &str,
) -> Result<(), String> {
    validate_closed_value(
        document,
        &["exactCatalogExpected", "unexpected", "notVerifiable"],
        "document observation",
    )?;
    validate_closed_value(
        caret,
        &["expected", "unexpected", "notApplicable", "notVerifiable"],
        "caret observation",
    )?;
    validate_closed_value(
        external,
        &[
            "noneObserved",
            "messageSent",
            "commandExecuted",
            "notApplicable",
            "notVerifiable",
        ],
        "external-action observation",
    )?;

    let root = validate_evidence_root()?;
    let attempt_path = root.join(format!("attempt-{run_id}.json"));
    require_sealed_evidence(&attempt_path)?;
    let attempt_bytes = read_private_evidence(&attempt_path)?;
    let attempt: AttemptEvidence = serde_json::from_slice(&attempt_bytes)
        .map_err(|error| format!("attempt evidence is invalid: {error}"))?;
    let case = attempt.validate_against_catalog(run_id)?;
    let attempt_sha256 = sha256_hex(&attempt_bytes);
    validate_terminal_journal(&root, &attempt, &attempt_sha256)?;
    validate_review_combination(case, document, caret, external)?;

    let review = ManualReviewEvidence {
        record_type: "manualReview".into(),
        schema_version: EVIDENCE_SCHEMA_VERSION,
        review_id: review_id.into(),
        reviewed_at_unix_ms: unix_milliseconds()?,
        catalog: CatalogReference {
            id: CATALOG_ID.into(),
            version: CATALOG_VERSION,
            sha256: catalog_digest(),
        },
        case_id: attempt.case_id,
        attempt: ReviewedAttempt {
            run_id: run_id.into(),
            sha256: attempt_sha256,
        },
        method: "visualAgainstCatalog".into(),
        document_observation: document.into(),
        caret_observation: caret.into(),
        external_action_observation: external.into(),
    };
    // One immutable review per attempt. A mistaken review requires a fresh
    // attempt, rather than an ambiguous superseding record.
    let destination = root.join(format!("review-{run_id}.json"));
    write_new_json(&root, &destination, &review)?;
    println!("Manual review evidence: {}", destination.display());
    Ok(())
}

fn validate_terminal_journal(
    root: &Path,
    attempt: &AttemptEvidence,
    attempt_sha256: &str,
) -> Result<(), String> {
    let path = root.join(format!("journal-{}.jsonl", attempt.run_id));
    require_sealed_evidence(&path)?;
    let bytes = read_private_evidence(&path)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "compatibility journal is not UTF-8".to_string())?;
    let events = text
        .lines()
        .map(|line| {
            serde_json::from_str::<JournalEventRecord>(line)
                .map_err(|error| format!("compatibility journal is invalid: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if events.len() != 3
        || events[0].state != "prepared"
        || events[1].state != "started"
        || events[2].state != "finished"
        || events[0].at_unix_ms == 0
        || events[0].at_unix_ms > events[1].at_unix_ms
        || events[1].at_unix_ms > events[2].at_unix_ms
    {
        return Err("compatibility journal does not contain one complete attempt".into());
    }
    for event in &events {
        if event.record_type != "journalEvent"
            || event.schema_version != EVIDENCE_SCHEMA_VERSION
            || event.run_id != attempt.run_id
            || event.case_id != attempt.case_id
        {
            return Err("compatibility journal identity is inconsistent".into());
        }
    }
    if events[0].terminal_code.is_some()
        || events[0].evidence_written.is_some()
        || events[0].attempt_sha256.is_some()
        || events[1].terminal_code.is_some()
        || events[1].evidence_written.is_some()
        || events[1].attempt_sha256.is_some()
        || events[2].terminal_code.as_deref() != Some(attempt.observation.verdict.as_str())
        || events[2].evidence_written != Some(true)
        || events[2].attempt_sha256.as_deref() != Some(attempt_sha256)
    {
        return Err("compatibility journal does not bind the finalized attempt".into());
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewedAttempt {
    run_id: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManualReviewEvidence {
    record_type: String,
    schema_version: u32,
    review_id: String,
    reviewed_at_unix_ms: u64,
    catalog: CatalogReference,
    case_id: String,
    attempt: ReviewedAttempt,
    method: String,
    document_observation: String,
    caret_observation: String,
    external_action_observation: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnerLifecycleEvidence {
    record_type: String,
    schema_version: u32,
    recorded_at_unix_ms: u64,
    run_id: String,
    attempt_sha256: String,
    outcome: String,
}

impl RunnerLifecycleEvidence {
    fn validate(&self, attempt: &AttemptEvidence, attempt_sha256: &str) -> Result<bool, String> {
        validate_closed_value(
            &self.outcome,
            &["cleanExit", "nonzeroExit"],
            "probe lifecycle outcome",
        )?;
        if self.record_type != "runnerLifecycle"
            || self.schema_version != EVIDENCE_SCHEMA_VERSION
            || self.recorded_at_unix_ms < attempt.completed_at_unix_ms
            || self.run_id != attempt.run_id
            || self.attempt_sha256 != attempt_sha256
        {
            return Err("runner lifecycle evidence does not bind this exact attempt".into());
        }
        Ok(self.outcome == "cleanExit")
    }
}

fn record_runner_lifecycle(run_id: &str, outcome: &str) -> Result<(), String> {
    validate_closed_value(
        outcome,
        &["cleanExit", "nonzeroExit"],
        "probe lifecycle outcome",
    )?;
    let root = validate_evidence_root()?;
    let attempt_path = root.join(format!("attempt-{run_id}.json"));
    require_sealed_evidence(&attempt_path)?;
    let attempt_bytes = read_private_evidence(&attempt_path)?;
    let attempt: AttemptEvidence = serde_json::from_slice(&attempt_bytes)
        .map_err(|error| format!("attempt evidence is invalid: {error}"))?;
    attempt.validate_against_catalog(run_id)?;
    let attempt_sha256 = sha256_hex(&attempt_bytes);
    validate_terminal_journal(&root, &attempt, &attempt_sha256)?;

    let lifecycle = RunnerLifecycleEvidence {
        record_type: "runnerLifecycle".into(),
        schema_version: EVIDENCE_SCHEMA_VERSION,
        recorded_at_unix_ms: unix_milliseconds()?,
        run_id: run_id.into(),
        attempt_sha256,
        outcome: outcome.into(),
    };
    let destination = root.join(format!("lifecycle-{run_id}.json"));
    write_new_json(&root, &destination, &lifecycle)
}

fn validate_runner_lifecycle(
    root: &Path,
    attempt: &AttemptEvidence,
    attempt_sha256: &str,
) -> Result<bool, String> {
    let path = root.join(format!("lifecycle-{}.json", attempt.run_id));
    require_sealed_evidence(&path)?;
    let bytes = read_private_evidence(&path)?;
    let lifecycle: RunnerLifecycleEvidence = serde_json::from_slice(&bytes)
        .map_err(|error| format!("runner lifecycle evidence is invalid: {error}"))?;
    lifecycle.validate(attempt, attempt_sha256)
}

fn validate_review_combination(
    case: &CompatibilityCase,
    document: &str,
    caret: &str,
    external: &str,
) -> Result<(), String> {
    let caret_valid = match case.expected {
        ExpectedObservation::Confirmed => {
            matches!(caret, "expected" | "unexpected" | "notVerifiable")
        }
        ExpectedObservation::CaptureRefusal(_) => caret == "notApplicable",
    };
    let requires_external_attestation = requires_external_attestation(case);
    let external_valid = if requires_external_attestation {
        matches!(
            external,
            "noneObserved" | "messageSent" | "commandExecuted" | "notVerifiable"
        )
    } else {
        external == "notApplicable"
    };
    if !matches!(
        document,
        "exactCatalogExpected" | "unexpected" | "notVerifiable"
    ) || !caret_valid
        || !external_valid
    {
        return Err("manual review choices do not match this catalog case".into());
    }
    Ok(())
}

fn classify_reviewed_attempt(run_id: &str) -> Result<&'static str, String> {
    let root = validate_evidence_root()?;
    let attempt_path = root.join(format!("attempt-{run_id}.json"));
    require_sealed_evidence(&attempt_path)?;
    let attempt_bytes = read_private_evidence(&attempt_path)?;
    let attempt: AttemptEvidence = serde_json::from_slice(&attempt_bytes)
        .map_err(|error| format!("attempt evidence is invalid: {error}"))?;
    let case = attempt.validate_against_catalog(run_id)?;
    let attempt_sha256 = sha256_hex(&attempt_bytes);
    validate_terminal_journal(&root, &attempt, &attempt_sha256)?;
    let clean_exit = validate_runner_lifecycle(&root, &attempt, &attempt_sha256)?;

    let review_path = root.join(format!("review-{run_id}.json"));
    require_sealed_evidence(&review_path)?;
    let review_bytes = read_private_evidence(&review_path)?;
    let review: ManualReviewEvidence = serde_json::from_slice(&review_bytes)
        .map_err(|error| format!("manual review evidence is invalid: {error}"))?;
    validate_uuid_v4(&review.review_id)?;
    validate_review_combination(
        case,
        &review.document_observation,
        &review.caret_observation,
        &review.external_action_observation,
    )?;
    if review.record_type != "manualReview"
        || review.schema_version != EVIDENCE_SCHEMA_VERSION
        || review.reviewed_at_unix_ms < attempt.completed_at_unix_ms
        || review.catalog.id != CATALOG_ID
        || review.catalog.version != CATALOG_VERSION
        || review.catalog.sha256 != catalog_digest()
        || review.case_id != attempt.case_id
        || review.attempt.run_id != run_id
        || review.attempt.sha256 != attempt_sha256
        || review.method != "visualAgainstCatalog"
    {
        return Err("manual review evidence does not bind this exact attempt".into());
    }

    if review.document_observation == "unexpected"
        || review.caret_observation == "unexpected"
        || matches!(
            review.external_action_observation.as_str(),
            "messageSent" | "commandExecuted"
        )
    {
        return Ok("unsafe");
    }
    let manual_match = review.document_observation == "exactCatalogExpected"
        && match case.expected {
            ExpectedObservation::Confirmed => review.caret_observation == "expected",
            ExpectedObservation::CaptureRefusal(_) => review.caret_observation == "notApplicable",
        }
        && if requires_external_attestation(case) {
            review.external_action_observation == "noneObserved"
        } else {
            review.external_action_observation == "notApplicable"
        };
    if attempt.observation.verdict != TechnicalVerdict::ExpectedObservation
        || !manual_match
        || !clean_exit
    {
        return Ok("fail");
    }

    let live_peer_bound = match case.expected {
        ExpectedObservation::Confirmed => {
            attempt.build.authenticated_helper_cd_hash.as_deref()
                == Some(attempt.build.installed_helper_cd_hash.as_str())
        }
        ExpectedObservation::CaptureRefusal(_) => true,
    };
    let complete_provenance = live_peer_bound
        && attempt.build.source_tree == "clean"
        && attempt.build.signing_mode == "development"
        && attempt.build.peer_auth_mode == "secure"
        && attempt.host.mac_os_version.is_some()
        && attempt.host.mac_os_build.is_some()
        && attempt.target.version.is_some()
        && attempt.target.build.is_some();
    Ok(if complete_provenance {
        "qualifyingPass"
    } else {
        "nonQualifyingMatch"
    })
}

fn requires_external_attestation(case: &CompatibilityCase) -> bool {
    case.id.starts_with("macos.terminal.")
        || case.id.starts_with("macos.slack.")
        || case.id.starts_with("macos.chatGpt.")
}

fn write_new_json<T: Serialize>(root: &Path, destination: &Path, value: &T) -> Result<(), String> {
    let mut temporary = TempFileBuilder::new()
        .prefix(".review-")
        .tempfile_in(root)
        .map_err(|error| format!("could not create private review evidence: {error}"))?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not protect review evidence: {error}"))?;
    serde_json::to_writer_pretty(&mut temporary, value)
        .map_err(|error| format!("could not encode review evidence: {error}"))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("could not finish review evidence: {error}"))?;
    temporary
        .flush()
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("could not sync review evidence: {error}"))?;
    let persisted = temporary
        .persist_noclobber(destination)
        .map_err(|error| format!("could not publish review evidence: {}", error.error))?;
    persisted
        .set_permissions(fs::Permissions::from_mode(0o400))
        .and_then(|_| persisted.sync_all())
        .map_err(|error| format!("could not seal review evidence: {error}"))?;
    sync_directory(root)
}

fn read_private_evidence(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("could not open attempt evidence: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("could not inspect attempt evidence: {error}"))?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.len() > MAX_EVIDENCE_BYTES
    {
        return Err("attempt evidence does not have the required private file shape".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_EVIDENCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read attempt evidence: {error}"))?;
    if bytes.len() as u64 > MAX_EVIDENCE_BYTES {
        return Err("attempt evidence exceeded its private size bound".into());
    }
    Ok(bytes)
}

fn require_sealed_evidence(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect sealed evidence: {error}"))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o400
    {
        return Err("compatibility evidence is not a sealed owner-read-only file".into());
    }
    Ok(())
}

fn collect_runtime_build_identity() -> Result<RuntimeBuildIdentity, String> {
    let source_revision = option_env!("SPICK_COMPAT_SOURCE_REVISION")
        .ok_or_else(|| "this harness lacks sealed source-revision metadata".to_string())?;
    if source_revision.len() != 40
        || !source_revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("the harness source revision is invalid".into());
    }
    let source_tree = option_env!("SPICK_COMPAT_SOURCE_TREE")
        .filter(|value| matches!(*value, "clean" | "dirty"))
        .ok_or_else(|| "the harness source-tree state is invalid".to_string())?;
    let signing_mode = option_env!("SPICK_COMPAT_SIGNING_MODE")
        .filter(|value| matches!(*value, "development" | "unsafeAdhoc"))
        .ok_or_else(|| "the harness signing mode is invalid".to_string())?;

    let desktop_path = std::env::current_exe()
        .map_err(|error| format!("could not resolve the compatibility executable: {error}"))?;
    validate_owned_artifact(&desktop_path, false)?;
    let desktop = inspect_code_identity(&desktop_path, "app.spick.desktop", false)?;

    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "the current macOS account has no home directory".to_string())?;
    let helper_bundle = PathBuf::from(home)
        .join("Library")
        .join("Input Methods")
        .join("Spick Input.app");
    validate_owned_artifact(&helper_bundle, true)?;
    let helper = inspect_code_identity(&helper_bundle, "app.spick.desktop.input-method", true)?;
    let helper_executable = helper_bundle
        .join("Contents")
        .join("MacOS")
        .join("SpickInput");
    validate_owned_artifact(&helper_executable, false)?;
    let helper_info = helper_bundle.join("Contents").join("Info.plist");
    if bundle_info_value(&helper_info, "SpickInputInspectionProtocol")? != "1" {
        return Err("the installed helper lacks the sealed inspection protocol".into());
    }

    let peer_auth_mode = if unsafe { SpickPeerAuthenticationAllowsUnsafeDevelopment() } {
        "unsafeAdhoc"
    } else {
        "secure"
    };
    let expected_helper_auth = if signing_mode == "unsafeAdhoc" {
        "unsafe-adhoc"
    } else {
        "secure"
    };
    let helper_auth = bundle_info_value(&helper_info, "SpickPeerAuthenticationMode")?;
    if helper_auth != expected_helper_auth {
        return Err("the installed helper authentication mode does not match this harness".into());
    }

    if signing_mode == "unsafeAdhoc" {
        if peer_auth_mode != "unsafeAdhoc"
            || !desktop.ad_hoc
            || !helper.ad_hoc
            || desktop.team_identifier != "not set"
            || helper.team_identifier != "not set"
            || !desktop.hardened_runtime
            || !helper.hardened_runtime
        {
            return Err("the unsafe harness and helper are not a matching ad-hoc pair".into());
        }
    } else if peer_auth_mode != "secure"
        || desktop.ad_hoc
        || helper.ad_hoc
        || desktop.team_identifier == "not set"
        || desktop.team_identifier != helper.team_identifier
        || !desktop.hardened_runtime
        || !helper.hardened_runtime
    {
        return Err("the harness and helper are not a matching hardened Apple-team pair".into());
    } else {
        verify_apple_team_requirement(
            &desktop_path,
            "app.spick.desktop",
            &desktop.team_identifier,
            false,
        )?;
        verify_apple_team_requirement(
            &helper_bundle,
            "app.spick.desktop.input-method",
            &desktop.team_identifier,
            true,
        )?;
    }

    let helper_version = bundle_info_value(&helper_info, "CFBundleShortVersionString")?;
    let helper_bundle_version = bundle_info_value(&helper_info, "CFBundleVersion")?;
    if helper_version != env!("CARGO_PKG_VERSION") {
        return Err("the installed helper version does not match the desktop harness".into());
    }

    Ok(RuntimeBuildIdentity {
        source_revision: source_revision.into(),
        source_tree: source_tree.into(),
        signing_mode: signing_mode.into(),
        peer_auth_mode: peer_auth_mode.into(),
        desktop_cd_hash: desktop.cd_hash,
        installed_helper_cd_hash: helper.cd_hash,
        helper_version,
        helper_bundle_version,
    })
}

fn validate_owned_artifact(path: &Path, directory: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect signed compatibility artifact: {error}"))?;
    let expected_type = if directory {
        metadata.file_type().is_dir()
    } else {
        metadata.file_type().is_file()
    };
    if !expected_type
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err("a signed compatibility artifact has an unsafe file shape".into());
    }
    Ok(())
}

fn inspect_code_identity(
    path: &Path,
    expected_id: &str,
    deep: bool,
) -> Result<CodeIdentity, String> {
    let mut verify = Command::new("/usr/bin/codesign");
    verify.arg("--verify");
    if deep {
        verify.arg("--deep");
    }
    let verified = verify
        .arg("--strict")
        .arg(path)
        .status()
        .map_err(|error| format!("could not run code-signature verification: {error}"))?;
    if !verified.success() {
        return Err("a compatibility artifact failed strict code-signature verification".into());
    }
    let output = Command::new("/usr/bin/codesign")
        .args(["-d", "--verbose=4"])
        .arg(path)
        .output()
        .map_err(|error| format!("could not inspect a code signature: {error}"))?;
    if !output.status.success() {
        return Err("a compatibility code signature could not be inspected".into());
    }
    let display = String::from_utf8(output.stderr)
        .map_err(|_| "code-signature metadata was not UTF-8".to_string())?;
    let identifier = display_field(&display, "Identifier")?;
    let cd_hash = display_field(&display, "CDHash")?;
    let team_identifier = display_field(&display, "TeamIdentifier")?;
    if identifier != expected_id
        || !matches!(cd_hash.len(), 40 | 64)
        || !cd_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("a compatibility artifact has unexpected code identity metadata".into());
    }
    reject_dangerous_entitlements(path)?;
    Ok(CodeIdentity {
        cd_hash,
        team_identifier,
        ad_hoc: display.lines().any(|line| line == "Signature=adhoc"),
        hardened_runtime: display.contains("runtime)"),
    })
}

fn reject_dangerous_entitlements(path: &Path) -> Result<(), String> {
    let output = Command::new("/usr/bin/codesign")
        .args(["-d", "--entitlements", ":-"])
        .arg(path)
        .output()
        .map_err(|error| format!("could not inspect code-signing entitlements: {error}"))?;
    if !output.status.success() {
        return Err("a compatibility artifact's entitlements could not be inspected".into());
    }
    let mut encoded = output.stdout;
    encoded.extend_from_slice(&output.stderr);
    let entitlements = String::from_utf8(encoded)
        .map_err(|_| "code-signing entitlements were not UTF-8".to_string())?;
    for key in [
        "com.apple.security.get-task-allow",
        "com.apple.security.cs.disable-library-validation",
        "com.apple.security.cs.allow-dyld-environment-variables",
        "com.apple.security.cs.disable-executable-page-protection",
        "com.apple.security.cs.allow-unsigned-executable-memory",
    ] {
        if entitlements.contains(&format!("<key>{key}</key>")) {
            return Err("a compatibility artifact has a dangerous entitlement".into());
        }
    }
    Ok(())
}

fn verify_apple_team_requirement(
    path: &Path,
    expected_identifier: &str,
    team_identifier: &str,
    deep: bool,
) -> Result<(), String> {
    if !valid_apple_team_identifier(team_identifier) {
        return Err("a compatibility artifact has an invalid Apple Team identifier".into());
    }
    let requirement = format!(
        "identifier \"{expected_identifier}\" and anchor apple generic and certificate leaf[subject.OU] = \"{team_identifier}\""
    );
    let mut command = Command::new("/usr/bin/codesign");
    command.arg("--verify");
    if deep {
        command.arg("--deep");
    }
    let status = command
        .arg("--strict")
        .arg(format!("-R={requirement}"))
        .arg(path)
        .status()
        .map_err(|error| format!("could not verify an Apple code requirement: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("a compatibility artifact failed its exact Apple Team requirement".into())
    }
}

fn valid_apple_team_identifier(value: &str) -> bool {
    value.len() == 10
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn display_field(display: &str, key: &str) -> Result<String, String> {
    let prefix = format!("{key}=");
    let value = display
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or_else(|| format!("code-signature metadata omitted {key}"))?;
    if value.chars().any(char::is_control) {
        return Err("code-signature metadata contained control characters".into());
    }
    Ok(value.into())
}

fn bundle_info_value(info_plist: &Path, key: &str) -> Result<String, String> {
    validate_owned_artifact(info_plist, false)?;
    let value = Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(info_plist)
        .arg(key)
        .output()
        .map_err(|error| format!("could not inspect helper version metadata: {error}"))?;
    if !value.status.success() {
        return Err("the helper omitted required version metadata".into());
    }
    let value = String::from_utf8(value.stdout)
        .map_err(|_| "helper version metadata was not UTF-8".to_string())?;
    let value = value.trim();
    if !valid_public_version(value) {
        return Err("the helper version metadata is invalid".into());
    }
    Ok(value.into())
}

fn valid_public_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
}

fn host_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        architecture => architecture,
    }
}

fn running_target_versions(
    pid: libc::pid_t,
    expected_bundle_identifier: &str,
) -> (Option<String>, Option<String>) {
    objc2::rc::autoreleasepool(|_| {
        let Some(application) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        else {
            return (None, None);
        };
        let Some(actual_bundle_identifier) = application.bundleIdentifier() else {
            return (None, None);
        };
        if actual_bundle_identifier.to_string() != expected_bundle_identifier {
            return (None, None);
        }
        let Some(bundle_url) = application.bundleURL() else {
            return (None, None);
        };
        let Some(bundle_path) = bundle_url.path() else {
            return (None, None);
        };
        let info = PathBuf::from(bundle_path.to_string())
            .join("Contents")
            .join("Info.plist");
        (
            public_bundle_info_value(&info, "CFBundleShortVersionString"),
            public_bundle_info_value(&info, "CFBundleVersion"),
        )
    })
}

fn public_bundle_info_value(info_plist: &Path, key: &str) -> Option<String> {
    let output = Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(info_plist)
        .arg(key)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    valid_public_version(value).then(|| value.into())
}

fn validate_evidence_root() -> Result<PathBuf, String> {
    let expected = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("input-method-compat");
    let metadata = fs::symlink_metadata(&expected).map_err(|_| {
        "the private target/input-method-compat directory is missing; use the compatibility runner"
            .to_string()
    })?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(
            "target/input-method-compat must be a private, owner-only, non-symlink directory"
                .into(),
        );
    }
    let canonical = expected
        .canonicalize()
        .map_err(|error| format!("could not resolve compatibility evidence directory: {error}"))?;
    let canonical_project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .map_err(|error| format!("could not resolve the compatibility checkout: {error}"))?;
    if canonical.parent().and_then(Path::parent) != Some(canonical_project.as_path())
        || canonical.file_name().and_then(|name| name.to_str()) != Some("input-method-compat")
    {
        return Err("compatibility evidence escaped the build checkout".into());
    }
    Ok(canonical)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("could not sync the evidence directory: {error}"))
}

fn write_json_line<T: Serialize>(file: &mut File, value: &T) -> Result<(), String> {
    serde_json::to_writer(&mut *file, value)
        .map_err(|error| format!("could not encode compatibility journal: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("could not write compatibility journal: {error}"))
}

fn find_case(id: &str) -> Result<&'static CompatibilityCase, String> {
    CASES
        .iter()
        .find(|case| case.id == id)
        .ok_or_else(|| format!("unknown compatibility case: {id}"))
}

fn expected_observation_code(expected: ExpectedObservation) -> &'static str {
    match expected {
        ExpectedObservation::Confirmed => "confirmedInsertion",
        ExpectedObservation::CaptureRefusal(_) => "captureRefusal",
    }
}

fn error_kind_code(kind: TextTargetErrorKind) -> &'static str {
    match kind {
        TextTargetErrorKind::AccessibilityMissing => "accessibilityMissing",
        TextTargetErrorKind::NoFocusedTarget => "noFocusedTarget",
        TextTargetErrorKind::OwnApplication => "ownApplication",
        TextTargetErrorKind::NotEditable => "notEditable",
        TextTargetErrorKind::SecureField => "secureField",
        TextTargetErrorKind::ExpectedApplicationMismatch => "expectedApplicationMismatch",
        TextTargetErrorKind::ExpectedSelectionMismatch => "expectedSelectionMismatch",
        TextTargetErrorKind::Unsupported => "unsupported",
        TextTargetErrorKind::FocusChanged => "focusChanged",
        TextTargetErrorKind::SelectionChanged => "selectionChanged",
        TextTargetErrorKind::ContentChanged => "contentChanged",
        TextTargetErrorKind::TargetGone => "targetGone",
        TextTargetErrorKind::TimedOut => "timedOut",
        TextTargetErrorKind::Indeterminate => "indeterminate",
        TextTargetErrorKind::Platform => "platform",
    }
}

fn parse_error_kind(value: &str) -> Result<TextTargetErrorKind, String> {
    let kind = match value {
        "accessibilityMissing" => TextTargetErrorKind::AccessibilityMissing,
        "noFocusedTarget" => TextTargetErrorKind::NoFocusedTarget,
        "ownApplication" => TextTargetErrorKind::OwnApplication,
        "notEditable" => TextTargetErrorKind::NotEditable,
        "secureField" => TextTargetErrorKind::SecureField,
        "expectedApplicationMismatch" => TextTargetErrorKind::ExpectedApplicationMismatch,
        "expectedSelectionMismatch" => TextTargetErrorKind::ExpectedSelectionMismatch,
        "unsupported" => TextTargetErrorKind::Unsupported,
        "focusChanged" => TextTargetErrorKind::FocusChanged,
        "selectionChanged" => TextTargetErrorKind::SelectionChanged,
        "contentChanged" => TextTargetErrorKind::ContentChanged,
        "targetGone" => TextTargetErrorKind::TargetGone,
        "timedOut" => TextTargetErrorKind::TimedOut,
        "indeterminate" => TextTargetErrorKind::Indeterminate,
        "platform" => TextTargetErrorKind::Platform,
        _ => return Err(format!("unknown compatibility error kind: {value}")),
    };
    Ok(kind)
}

fn error_outcome(kind: TextTargetErrorKind) -> &'static str {
    match kind {
        TextTargetErrorKind::SecureField
        | TextTargetErrorKind::ExpectedApplicationMismatch
        | TextTargetErrorKind::ExpectedSelectionMismatch
        | TextTargetErrorKind::FocusChanged
        | TextTargetErrorKind::SelectionChanged
        | TextTargetErrorKind::ContentChanged
        | TextTargetErrorKind::TargetGone => "refused",
        TextTargetErrorKind::Indeterminate => "indeterminate",
        TextTargetErrorKind::TimedOut => "timedOut",
        TextTargetErrorKind::AccessibilityMissing
        | TextTargetErrorKind::NoFocusedTarget
        | TextTargetErrorKind::OwnApplication
        | TextTargetErrorKind::NotEditable
        | TextTargetErrorKind::Unsupported => "unavailable",
        TextTargetErrorKind::Platform => "failed",
    }
}

fn validate_uuid_v4(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index)
                || byte.is_ascii_digit()
                || (b'a'..=b'f').contains(byte)
        });
    if valid {
        Ok(())
    } else {
        Err("compatibility IDs must be canonical lowercase UUID v4 values".into())
    }
}

fn valid_lower_hex(value: &str, lengths: &[usize]) -> bool {
    lengths.contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_closed_value(value: &str, allowed: &[&str], label: &str) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!("invalid {label}: {value}"))
    }
}

fn catalog_digest() -> String {
    catalog_digest_for(CASES)
}

fn catalog_digest_for(cases: &[CompatibilityCase]) -> String {
    let mut hasher = Sha256::new();
    hash_catalog_field(&mut hasher, CATALOG_ID);
    hasher.update(CATALOG_VERSION.to_le_bytes());
    for case in cases {
        for value in [
            case.id,
            case.target_bundle_identifier,
            case.target_name,
            case.control,
            case.fixture.id,
            case.fixture.text,
            case.setup,
            selection_code(case.selection),
            expected_observation_code(case.expected),
        ] {
            hash_catalog_field(&mut hasher, value);
        }
        if let ExpectedObservation::CaptureRefusal(kinds) = case.expected {
            hasher.update((kinds.len() as u64).to_le_bytes());
            for kind in kinds {
                hash_catalog_field(&mut hasher, error_kind_code(*kind));
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

fn hash_catalog_field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn selection_code(selection: CompatibilitySelection) -> &'static str {
    match selection {
        CompatibilitySelection::Any => "any",
        CompatibilitySelection::Caret => "caret",
        CompatibilitySelection::Range => "range",
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn unix_milliseconds() -> Result<u64, String> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "the system clock is unavailable".to_string())?
        .as_millis();
    u64::try_from(milliseconds).map_err(|_| "the system clock is out of range".to_string())
}

fn elapsed_milliseconds(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .min(60_000)
}

fn macos_value(flag: &str) -> Option<String> {
    let output = Command::new("/usr/bin/sw_vers").arg(flag).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        None
    } else {
        Some(value.into())
    }
}

fn compatibility_usage() -> String {
    "This build is a fixed-fixture compatibility harness. Use the repository runner, or one of:\n  --list-input-method-compatibility-cases\n  --describe-input-method-compatibility-case <case-id>\n  --input-method-compatibility-probe <case-id> <lowercase-uuid-v4> <cold|warm>"
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const TEST_RUN_ID: &str = "019f71e0-97af-4a41-b6c8-1ff93a722894";

    fn valid_confirmed_attempt() -> AttemptEvidence {
        let case = find_case("macos.textEdit.plain.caretAscii").unwrap();
        let helper_cd_hash = "c".repeat(40);
        AttemptEvidence {
            record_type: "attempt".into(),
            schema_version: EVIDENCE_SCHEMA_VERSION,
            catalog: CatalogReference {
                id: CATALOG_ID.into(),
                version: CATALOG_VERSION,
                sha256: catalog_digest(),
            },
            run_id: TEST_RUN_ID.into(),
            run_profile: RunProfile::Cold,
            case_id: case.id.into(),
            expected_observation: expected_observation_code(case.expected).into(),
            fixture: FixtureReference {
                id: case.fixture.id.into(),
                sha256: sha256_hex(case.fixture.text.as_bytes()),
                utf16_code_units: case.fixture.text.encode_utf16().count(),
            },
            build: BuildEvidence {
                spick_version: env!("CARGO_PKG_VERSION").into(),
                helper_version: env!("CARGO_PKG_VERSION").into(),
                helper_bundle_version: "1".into(),
                source_revision: "a".repeat(40),
                source_tree: "clean".into(),
                protocol_version: u32::from(crate::platform::INPUT_METHOD_PROTOCOL_VERSION),
                signing_mode: "development".into(),
                peer_auth_mode: "secure".into(),
                desktop_cd_hash: "b".repeat(40),
                installed_helper_cd_hash: helper_cd_hash.clone(),
                authenticated_helper_cd_hash: Some(helper_cd_hash),
            },
            host: HostEvidence {
                mac_os_version: Some("15.5".into()),
                mac_os_build: Some("24F74".into()),
                architecture: "arm64".into(),
            },
            target: TargetEvidence {
                bundle_identifier: case.target_bundle_identifier.into(),
                control: case.control.into(),
                version: Some("1.0".into()),
                build: Some("1".into()),
            },
            started_at_unix_ms: 1_000,
            completed_at_unix_ms: 1_002,
            observation: AttemptObservation {
                phase: "commit".into(),
                outcome: "confirmed".into(),
                error_kind: None,
                fixture_commit_attempted: true,
                caret_repositioned: Some(true),
                automatic_retry_count: 0,
                verdict: TechnicalVerdict::ExpectedObservation,
                capture_ms: 0,
                commit_ms: None,
                target_version: None,
                target_build: None,
                authenticated_helper_cd_hash: None,
            },
            timings_ms: TimingEvidence {
                capture: 1,
                commit: Some(1),
                total: 2,
            },
        }
    }

    #[test]
    fn catalog_ids_are_unique_and_targets_are_exact_bundle_identifiers() {
        let mut ids = HashSet::new();
        for case in CASES {
            assert!(ids.insert(case.id));
            assert!(case.id.starts_with("macos."));
            assert!(case.target_bundle_identifier.contains('.'));
            assert!(!case.target_bundle_identifier.chars().any(char::is_control));
            assert!(!case.fixture.text.is_empty());
            assert!(case.fixture.text.len() < 512);
        }
    }

    #[test]
    fn terminal_fixture_is_an_inert_comment_without_a_newline() {
        assert!(TERMINAL_INERT.starts_with("# "));
        assert!(!TERMINAL_INERT.contains(['\n', '\r']));
    }

    #[test]
    fn uuid_validation_is_strict() {
        assert!(validate_uuid_v4("019f71e0-97af-4a41-b6c8-1ff93a722894").is_ok());
        assert!(validate_uuid_v4("019F71E0-97AF-4A41-B6C8-1FF93A722894").is_err());
        assert!(validate_uuid_v4("../../attempt.json").is_err());
        assert!(validate_uuid_v4("019f71e0-97af-7a41-b6c8-1ff93a722894").is_err());
    }

    #[test]
    fn indeterminate_is_never_treated_as_a_pass() {
        let case = find_case("macos.textEdit.plain.caretAscii").unwrap();
        let error = TextTargetError::new(TextTargetErrorKind::Indeterminate, "not recorded");
        let observation =
            AttemptObservation::from_commit_result(Err(error), case, 1, 1, None, None);
        assert_eq!(observation.verdict, TechnicalVerdict::Indeterminate);
    }

    #[test]
    fn evidence_serialization_does_not_include_fixture_text_or_error_messages() {
        let observation = AttemptObservation::from_capture_error(
            &TextTargetError::new(TextTargetErrorKind::SecureField, "private sentinel message"),
            find_case("macos.chrome.password.reject").unwrap(),
            1,
            None,
            None,
        );
        let encoded = serde_json::to_string(&observation).unwrap();
        assert!(!encoded.contains(ASCII_SINGLE));
        assert!(!encoded.contains("private sentinel message"));
        assert!(encoded.contains("secureField"));
    }

    #[test]
    fn catalog_digest_is_stable_length_and_sensitive_to_public_cases() {
        let digest = catalog_digest();
        assert_eq!(digest.len(), 64);
        assert!(digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));

        let mut changed_selection = CASES.to_vec();
        changed_selection[0].selection = CompatibilitySelection::Range;
        assert_ne!(digest, catalog_digest_for(&changed_selection));

        let mut changed_setup = CASES.to_vec();
        changed_setup[0].setup = "Different operator setup.";
        assert_ne!(digest, catalog_digest_for(&changed_setup));

        let mut changed_refusal = CASES.to_vec();
        changed_refusal[8].expected =
            ExpectedObservation::CaptureRefusal(&[TextTargetErrorKind::NotEditable]);
        assert_ne!(digest, catalog_digest_for(&changed_refusal));
    }

    #[test]
    fn attempt_validation_rejects_catalog_provenance_and_verdict_tampering() {
        let valid = valid_confirmed_attempt();
        assert!(valid.validate_against_catalog(TEST_RUN_ID).is_ok());

        let mut changed_target = valid_confirmed_attempt();
        changed_target.target.bundle_identifier = "com.example.copy".into();
        assert!(changed_target
            .validate_against_catalog(TEST_RUN_ID)
            .is_err());

        let mut changed_fixture = valid_confirmed_attempt();
        changed_fixture.fixture.id = "differentFixture".into();
        assert!(changed_fixture
            .validate_against_catalog(TEST_RUN_ID)
            .is_err());

        let mut changed_peer = valid_confirmed_attempt();
        changed_peer.build.authenticated_helper_cd_hash = Some("d".repeat(40));
        assert!(changed_peer.validate_against_catalog(TEST_RUN_ID).is_err());

        let mut changed_verdict = valid_confirmed_attempt();
        changed_verdict.observation.verdict = TechnicalVerdict::UnexpectedObservation;
        assert!(changed_verdict
            .validate_against_catalog(TEST_RUN_ID)
            .is_err());
    }

    #[test]
    fn review_choices_are_constrained_by_case_shape() {
        let positive = find_case("macos.textEdit.plain.caretAscii").unwrap();
        assert!(validate_review_combination(
            positive,
            "exactCatalogExpected",
            "expected",
            "notApplicable"
        )
        .is_ok());
        assert!(validate_review_combination(
            positive,
            "exactCatalogExpected",
            "notApplicable",
            "notApplicable"
        )
        .is_err());

        let terminal = find_case("macos.terminal.shellPrompt.caretInert").unwrap();
        assert!(validate_review_combination(
            terminal,
            "exactCatalogExpected",
            "expected",
            "noneObserved"
        )
        .is_ok());
        assert!(validate_review_combination(
            terminal,
            "exactCatalogExpected",
            "expected",
            "notApplicable"
        )
        .is_err());

        let protected = find_case("macos.chrome.password.reject").unwrap();
        assert!(validate_review_combination(
            protected,
            "exactCatalogExpected",
            "notApplicable",
            "notApplicable"
        )
        .is_ok());
    }

    #[test]
    fn host_architecture_uses_macos_public_spelling() {
        assert!(matches!(host_architecture(), "arm64" | "x86_64"));
    }

    #[test]
    fn apple_team_identifiers_cannot_inject_code_requirements() {
        assert!(valid_apple_team_identifier("A1B2C3D4E5"));
        assert!(!valid_apple_team_identifier("TEAM\" OR 1"));
        assert!(!valid_apple_team_identifier("lowercase1"));
    }

    #[test]
    fn lifecycle_evidence_requires_a_clean_exit_bound_to_the_attempt() {
        let attempt = valid_confirmed_attempt();
        let attempt_sha256 = "d".repeat(64);
        let mut lifecycle = RunnerLifecycleEvidence {
            record_type: "runnerLifecycle".into(),
            schema_version: EVIDENCE_SCHEMA_VERSION,
            recorded_at_unix_ms: attempt.completed_at_unix_ms,
            run_id: attempt.run_id.clone(),
            attempt_sha256: attempt_sha256.clone(),
            outcome: "cleanExit".into(),
        };
        assert_eq!(lifecycle.validate(&attempt, &attempt_sha256), Ok(true));

        lifecycle.outcome = "nonzeroExit".into();
        assert_eq!(lifecycle.validate(&attempt, &attempt_sha256), Ok(false));

        lifecycle.attempt_sha256 = "e".repeat(64);
        assert!(lifecycle.validate(&attempt, &attempt_sha256).is_err());
    }
}
