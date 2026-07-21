use std::{
    collections::HashMap,
    ffi::c_void,
    ptr::{self, NonNull},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(feature = "macos-input-method-compatibility-harness")]
use std::ffi::CStr;
#[cfg(feature = "macos-input-method-prototype")]
use std::{
    ffi::CString,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, MetadataExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "macos-input-method-prototype")]
use super::input_method_protocol::{
    decode_response, encode_arm_request, encode_disarm_request, encode_insert_request,
    InputMethodResponse, InputMethodResponseStatus, RESPONSE_LENGTH,
};
#[cfg(feature = "macos-input-method-compatibility-harness")]
use super::CompatibilitySelection;
use super::{
    AccessibilityPermissionState, AccessibilityPermissionStatus, CapturedTextTarget,
    TextInsertionReceipt, TextTargetError, TextTargetErrorKind, TextTargetToken,
};
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
use core_graphics::{
    event::{CGEvent, CGEventFlags},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use libc::pid_t;
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
use objc2::{rc::Retained, runtime::ProtocolObject};
#[cfg(feature = "macos-input-method-prototype")]
use objc2_app_kit::NSRunningApplication;
use objc2_app_kit::NSWorkspace;
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardType, NSPasteboardTypeString, NSPasteboardWriting,
};
use objc2_application_services::{
    kAXTrustedCheckOptionPrompt, AXError, AXIsProcessTrusted, AXIsProcessTrustedWithOptions,
    AXObserver, AXUIElement, AXValue, AXValueType,
};
use objc2_core_foundation::{
    kCFBooleanTrue, kCFRunLoopDefaultMode, CFArray, CFBoolean, CFDictionary, CFRange, CFRetained,
    CFRunLoop, CFRunLoopSource, CFString, CFType,
};
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
use objc2_foundation::{NSArray, NSData, NSString};

const AX_FOCUSED_APPLICATION: &str = "AXFocusedApplication";
const AX_FOCUSED_UI_ELEMENT: &str = "AXFocusedUIElement";
const AX_PARENT: &str = "AXParent";
const AX_TITLE: &str = "AXTitle";
const AX_ROLE: &str = "AXRole";
const AX_SUBROLE: &str = "AXSubrole";
const AX_CONTAINS_PROTECTED_CONTENT: &str = "AXContainsProtectedContent";
const AX_ENABLED: &str = "AXEnabled";
const AX_VALUE: &str = "AXValue";
const AX_SELECTED_TEXT: &str = "AXSelectedText";
const AX_SELECTED_TEXT_RANGE: &str = "AXSelectedTextRange";
const AX_SELECTED_TEXT_RANGES: &str = "AXSelectedTextRanges";
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
const AX_STRING_FOR_RANGE: &str = "AXStringForRange";
const AX_SECURE_TEXT_FIELD: &str = "AXSecureTextField";
const AX_TEXT_FIELD_ROLE: &str = "AXTextField";
const AX_TEXT_AREA_ROLE: &str = "AXTextArea";
const AX_WEB_AREA_ROLE: &str = "AXWebArea";
const AX_APPLICATION_DEACTIVATED: &str = "AXApplicationDeactivated";
const AX_FOCUSED_UI_ELEMENT_CHANGED: &str = "AXFocusedUIElementChanged";
const AX_SELECTED_TEXT_CHANGED: &str = "AXSelectedTextChanged";
const AX_VALUE_CHANGED: &str = "AXValueChanged";
const AX_UI_ELEMENT_DESTROYED: &str = "AXUIElementDestroyed";
const AX_MANUAL_ACCESSIBILITY: &str = "AXManualAccessibility";

const OWNER_RESPONSE_TIMEOUT: Duration = Duration::from_millis(2_600);
const CAPTURE_DEADLINE: Duration = Duration::from_millis(700);
const COMMIT_DEADLINE: Duration = Duration::from_millis(1_600);
const APPLICATION_TIMEOUT_SECONDS: f32 = 0.25;
const MAX_PARENT_DEPTH: usize = 64;
const FOCUSED_CONTEXT_RETRY_BUDGET: Duration = Duration::from_millis(600);
const FOCUSED_CONTEXT_RETRY_DELAY: Duration = Duration::from_millis(12);
const RUN_LOOP_POLL: Duration = Duration::from_millis(4);
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
const INSERTION_CONFIRMATION_BUDGET: Duration = Duration::from_millis(850);
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
const PASTEBOARD_SNAPSHOT_MAX_BYTES: usize = 64 * 1024 * 1024;
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
const PASTEBOARD_SNAPSHOT_MAX_ITEMS: usize = 128;
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
const PASTEBOARD_SNAPSHOT_MAX_TYPES: usize = 512;
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
const PASTEBOARD_OWNER_TYPE: &str = "app.spick.desktop.transient-paste-owner";
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
const ANSI_VIRTUAL_KEY_V: u16 = 9;
#[cfg(feature = "macos-input-method-prototype")]
const INPUT_METHOD_SOCKET_NAME: &str = "app.spick.input-method.sock";
#[cfg(feature = "macos-input-method-prototype")]
const INPUT_METHOD_TIMEOUT: Duration = Duration::from_millis(800);
#[cfg(feature = "macos-input-method-prototype")]
const DESKTOP_SIGNING_IDENTIFIER: &[u8] = b"app.spick.desktop\0";
#[cfg(feature = "macos-input-method-prototype")]
const INPUT_METHOD_SIGNING_IDENTIFIER: &[u8] = b"app.spick.desktop.input-method\0";
#[cfg(feature = "macos-input-method-prototype")]
const PEER_TRUST_SECURE: u32 = 0;
#[cfg(feature = "macos-input-method-prototype")]
const PEER_TRUST_UNSAFE_DEVELOPMENT: u32 = 1;

#[cfg(feature = "macos-input-method-prototype")]
extern "C" {
    #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
    fn SpickVerifyPeerSocket(
        descriptor: libc::c_int,
        expected_self_identifier: *const libc::c_char,
        expected_peer_identifier: *const libc::c_char,
    ) -> u32;
    #[cfg(feature = "macos-input-method-compatibility-harness")]
    fn SpickVerifyPeerSocketWithCDHash(
        descriptor: libc::c_int,
        expected_self_identifier: *const libc::c_char,
        expected_peer_identifier: *const libc::c_char,
        peer_cd_hash_hex: *mut libc::c_char,
        peer_cd_hash_hex_capacity: usize,
    ) -> u32;
    fn SpickPeerAuthenticationAllowsUnsafeDevelopment() -> bool;
}

extern "C" {
    fn IsSecureEventInputEnabled() -> libc::c_uchar;
}

#[derive(Default)]
pub(super) struct MacTextTargetController {
    worker: OnceLock<Result<WorkerProxy, String>>,
}

impl MacTextTargetController {
    pub fn permission_status(&self) -> AccessibilityPermissionStatus {
        self.request(TextTargetErrorKind::TimedOut, |reply| {
            Command::PermissionStatus { reply }
        })
        .unwrap_or(AccessibilityPermissionStatus {
            state: AccessibilityPermissionState::Missing,
            can_request: true,
        })
    }

    pub fn request_permission(&self) -> Result<AccessibilityPermissionStatus, TextTargetError> {
        self.request(TextTargetErrorKind::TimedOut, |reply| {
            Command::RequestPermission { reply }
        })?
    }

    pub fn capture(&self) -> Result<CapturedTextTarget, TextTargetError> {
        self.request(TextTargetErrorKind::TimedOut, |reply| Command::Capture {
            deadline: Instant::now() + CAPTURE_DEADLINE,
            expected_bundle_identifier: None,
            expected_selection: None,
            reply,
        })?
    }

    #[cfg(feature = "macos-input-method-compatibility-harness")]
    pub fn capture_for_compatibility(
        &self,
        expected_bundle_identifier: &str,
        expected_selection: CompatibilitySelection,
    ) -> Result<CapturedTextTarget, TextTargetError> {
        self.request(TextTargetErrorKind::TimedOut, |reply| Command::Capture {
            deadline: Instant::now() + CAPTURE_DEADLINE,
            expected_bundle_identifier: Some(expected_bundle_identifier.to_owned()),
            expected_selection: Some(expected_selection),
            reply,
        })?
    }

    pub fn commit(
        &self,
        token: TextTargetToken,
        text: &str,
    ) -> Result<TextInsertionReceipt, TextTargetError> {
        if text.is_empty() {
            return Err(TextTargetError::new(
                TextTargetErrorKind::Platform,
                "Spick received an empty transcript for insertion",
            ));
        }
        self.request(TextTargetErrorKind::Indeterminate, |reply| {
            Command::Commit {
                token: token.platform_value(),
                text: text.to_owned(),
                deadline: Instant::now() + COMMIT_DEADLINE,
                reply,
            }
        })?
    }

    pub fn discard(&self, token: TextTargetToken) {
        let Ok(worker) = self.worker() else {
            return;
        };
        let _ = worker.sender.send(Command::Discard {
            token: token.platform_value(),
        });
    }

    fn request<T: Send + 'static>(
        &self,
        timeout_kind: TextTargetErrorKind,
        command: impl FnOnce(SyncSender<T>) -> Command,
    ) -> Result<T, TextTargetError> {
        let worker = self.worker()?;
        let (reply, response) = mpsc::sync_channel(1);
        worker.sender.send(command(reply)).map_err(|_| {
            TextTargetError::new(
                TextTargetErrorKind::Platform,
                "the macOS text-target worker stopped unexpectedly",
            )
        })?;
        response.recv_timeout(OWNER_RESPONSE_TIMEOUT).map_err(|error| {
            let kind = match error {
                mpsc::RecvTimeoutError::Timeout => timeout_kind,
                mpsc::RecvTimeoutError::Disconnected => TextTargetErrorKind::Platform,
            };
            let message = if kind == TextTargetErrorKind::Indeterminate {
                "macOS did not confirm the text write in time; Spick will not retry automatically"
            } else {
                "macOS did not answer the text-target request in time"
            };
            TextTargetError::new(kind, message)
        })
    }

    fn worker(&self) -> Result<&WorkerProxy, TextTargetError> {
        match self.worker.get_or_init(WorkerProxy::spawn) {
            Ok(worker) => Ok(worker),
            Err(error) => Err(TextTargetError::new(
                TextTargetErrorKind::Platform,
                error.clone(),
            )),
        }
    }
}

struct WorkerProxy {
    sender: mpsc::Sender<Command>,
}

impl WorkerProxy {
    fn spawn() -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("spick-macos-ax".into())
            .spawn(move || Worker::new().run(receiver))
            .map_err(|error| format!("could not start the macOS text-target worker: {error}"))?;
        Ok(Self { sender })
    }
}

enum Command {
    PermissionStatus {
        reply: SyncSender<AccessibilityPermissionStatus>,
    },
    RequestPermission {
        reply: SyncSender<Result<AccessibilityPermissionStatus, TextTargetError>>,
    },
    Capture {
        deadline: Instant,
        expected_bundle_identifier: Option<String>,
        #[cfg(feature = "macos-input-method-compatibility-harness")]
        expected_selection: Option<CompatibilitySelection>,
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        expected_selection: Option<()>,
        reply: SyncSender<Result<CapturedTextTarget, TextTargetError>>,
    },
    Commit {
        token: u64,
        text: String,
        deadline: Instant,
        reply: SyncSender<Result<TextInsertionReceipt, TextTargetError>>,
    },
    Discard {
        token: u64,
    },
}

struct Worker {
    next_token: u64,
    targets: HashMap<u64, CapturedTarget>,
    run_loop: CFRetained<CFRunLoop>,
}

impl Worker {
    fn new() -> Self {
        Self {
            next_token: 0,
            targets: HashMap::new(),
            run_loop: CFRunLoop::current().expect("the AX owner thread must have a run loop"),
        }
    }

    fn run(mut self, receiver: Receiver<Command>) {
        loop {
            pump_run_loop();
            let command = match receiver.recv_timeout(RUN_LOOP_POLL) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            objc2::rc::autoreleasepool(|_| match command {
                Command::PermissionStatus { reply } => {
                    let _ = reply.send(permission_status());
                }
                Command::RequestPermission { reply } => {
                    let _ = reply.send(request_permission());
                }
                Command::Capture {
                    deadline,
                    expected_bundle_identifier,
                    expected_selection,
                    reply,
                } => {
                    let result = self.capture(
                        deadline,
                        expected_bundle_identifier.as_deref(),
                        expected_selection,
                    );
                    let token = result
                        .as_ref()
                        .ok()
                        .map(|target| target.token.platform_value());
                    if reply.send(result).is_err() {
                        if let Some(token) = token {
                            self.targets.remove(&token);
                        }
                    }
                }
                Command::Commit {
                    token,
                    text,
                    deadline,
                    reply,
                } => {
                    let result = self.commit(token, &text, deadline);
                    let _ = reply.send(result);
                }
                Command::Discard { token } => {
                    self.targets.remove(&token);
                }
            });
        }
    }

    fn capture(
        &mut self,
        deadline: Instant,
        expected_bundle_identifier: Option<&str>,
        #[cfg(feature = "macos-input-method-compatibility-harness")] expected_selection: Option<
            CompatibilitySelection,
        >,
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        _expected_selection: Option<()>,
    ) -> Result<CapturedTextTarget, TextTargetError> {
        check_deadline(deadline)?;
        if !is_trusted() {
            return Err(TextTargetError::new(
                TextTargetErrorKind::AccessibilityMissing,
                "Turn on Accessibility for Spick before using the shortcut",
            ));
        }
        ensure_secure_event_input_disabled()?;

        let focused = read_focused_context(deadline)?;
        let application = focused.application;
        let focus_anchor = focused.focus_anchor;
        let application_pid = focused.pid;
        if application_pid == std::process::id() as pid_t {
            return Err(TextTargetError::new(
                TextTargetErrorKind::OwnApplication,
                "Click a text field in another app before holding the shortcut",
            ));
        }

        #[cfg(feature = "macos-input-method-prototype")]
        let bundle_identifier = application_bundle_identifier(application_pid);
        #[cfg(feature = "macos-input-method-compatibility-harness")]
        if let Some(expected) = expected_bundle_identifier {
            if bundle_identifier.as_deref() != Some(expected) {
                return Err(TextTargetError::new(
                    TextTargetErrorKind::ExpectedApplicationMismatch,
                    "Focus the exact app named by this compatibility case before using the shortcut",
                ));
            }
        }
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        let _ = expected_bundle_identifier;

        #[cfg(feature = "macos-input-method-compatibility-harness")]
        ensure_not_secure(&application, deadline)
            .map_err(|error| error.with_compatibility_target_pid(application_pid))?;
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        ensure_not_secure(&application, deadline)?;

        #[cfg(feature = "macos-input-method-compatibility-harness")]
        let editable = resolve_editable_target(&focus_anchor, deadline)
            .map_err(|error| error.with_compatibility_target_pid(application_pid))?;
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        let editable = resolve_editable_target(&focus_anchor, deadline)?;
        #[cfg(feature = "macos-input-method-compatibility-harness")]
        if let Some(expected) = expected_selection {
            let selection = editable.selection.ok_or_else(|| {
                TextTargetError::new(
                    TextTargetErrorKind::ExpectedSelectionMismatch,
                    "This field does not expose the selection required by the compatibility case",
                )
            })?;
            let has_selection = selection.length > 0;
            let matches = match expected {
                CompatibilitySelection::Any => true,
                CompatibilitySelection::Caret => !has_selection,
                CompatibilitySelection::Range => has_selection,
            };
            if !matches {
                return Err(TextTargetError::new(
                    TextTargetErrorKind::ExpectedSelectionMismatch,
                    "Prepare the caret or fixed selection required by this compatibility case",
                ));
            }
        }
        check_deadline(deadline)?;
        let observer = ObserverLease::install(
            &self.run_loop,
            application_pid,
            &application,
            &focus_anchor,
            &editable.element,
        )?;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let token = self.next_token;
        #[cfg(feature = "macos-input-method-prototype")]
        let input_method_lease = match (bundle_identifier.as_deref(), editable.selection) {
            (Some(identifier), Some(selection)) => {
                try_arm_input_method(token, selection, identifier, deadline)?
            }
            _ => None,
        };
        pump_run_loop();
        if observer.was_invalidated() {
            return Err(TextTargetError::new(
                TextTargetErrorKind::FocusChanged,
                "The field changed before Spick could start listening",
            ));
        }
        let confirmed_editable = revalidate_capture_snapshot(
            &application,
            &focus_anchor,
            application_pid,
            &editable,
            deadline,
        )?;
        pump_run_loop();
        if observer.was_invalidated() {
            return Err(TextTargetError::new(
                TextTargetErrorKind::FocusChanged,
                "The field changed before Spick could start listening",
            ));
        }
        check_deadline(deadline)?;
        let target_app = read_optional_string(&application, AX_TITLE)
            .ok()
            .flatten()
            .and_then(sanitize_application_name);
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        let insertion_path = insertion_path(
            confirmed_editable.selected_text_settable,
            confirmed_editable.selection.is_some(),
            confirmed_editable.prefers_paste,
        );
        self.targets.insert(
            token,
            CapturedTarget {
                application,
                focus_anchor,
                element: confirmed_editable.element,
                pid: application_pid,
                #[cfg(feature = "macos-input-method-prototype")]
                bundle_identifier,
                #[cfg(feature = "macos-input-method-prototype")]
                input_method_lease,
                selection: confirmed_editable.selection,
                #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
                insertion_path,
                observer,
            },
        );

        Ok(CapturedTextTarget {
            token: TextTargetToken::from_platform(token),
            target_app,
            #[cfg(feature = "macos-input-method-compatibility-harness")]
            compatibility_target_pid: application_pid,
        })
    }

    fn commit(
        &mut self,
        token: u64,
        transcript: &str,
        deadline: Instant,
    ) -> Result<TextInsertionReceipt, TextTargetError> {
        #[allow(unused_mut)]
        let mut target = self.targets.remove(&token).ok_or_else(|| {
            TextTargetError::new(
                TextTargetErrorKind::TargetGone,
                "the captured text field is no longer available",
            )
        })?;
        ensure_secure_event_input_disabled()?;
        let revalidated = revalidate_captured_target(&target, deadline)?;
        #[cfg(feature = "macos-input-method-compatibility-harness")]
        let _ = revalidated;

        #[cfg(feature = "macos-input-method-compatibility-harness")]
        {
            commit_through_input_method(&mut target, token, transcript, deadline)?;
            Ok(TextInsertionReceipt {
                target_app: None,
                caret_repositioned: true,
                #[cfg(feature = "macos-input-method-compatibility-harness")]
                compatibility_peer_cd_hash: target
                    .input_method_lease
                    .as_ref()
                    .map(|lease| lease.peer_cd_hash.clone()),
            })
        }

        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        {
            #[cfg(feature = "macos-input-method-prototype")]
            if target.input_method_lease.is_some() {
                commit_through_input_method(&mut target, token, transcript, deadline)?;
                return Ok(TextInsertionReceipt {
                    target_app: None,
                    caret_repositioned: true,
                });
            }

            match target.insertion_path {
                InsertionPath::ElementAddressed => {
                    if !revalidated.selected_text_settable {
                        return Err(TextTargetError::new(
                            TextTargetErrorKind::FocusChanged,
                            "The field stopped accepting direct text before Spick could type",
                        ));
                    }
                    commit_through_accessibility(&target, transcript, deadline)
                }
                InsertionPath::ClipboardPaste => {
                    commit_through_clipboard_paste(&target, transcript, deadline)
                }
            }
        }
    }
}

struct CapturedTarget {
    application: CFRetained<AXUIElement>,
    focus_anchor: CFRetained<AXUIElement>,
    element: CFRetained<AXUIElement>,
    pid: pid_t,
    #[cfg(feature = "macos-input-method-prototype")]
    bundle_identifier: Option<String>,
    #[cfg(feature = "macos-input-method-prototype")]
    input_method_lease: Option<InputMethodLease>,
    selection: Option<CFRange>,
    #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
    insertion_path: InsertionPath,
    observer: ObserverLease,
}

fn revalidate_captured_target(
    target: &CapturedTarget,
    deadline: Instant,
) -> Result<EditableTarget, TextTargetError> {
    pump_run_loop();
    if target.observer.was_invalidated() {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The field changed while Spick was listening, so nothing was typed",
        ));
    }
    check_deadline(deadline)?;
    if !is_trusted() {
        return Err(TextTargetError::new(
            TextTargetErrorKind::AccessibilityMissing,
            "Accessibility was turned off before Spick could type",
        ));
    }

    let editable = read_current_editable_target(target, deadline)?;
    if let Some(original_selection) = target.selection {
        if !editable
            .selection
            .is_some_and(|selection| ranges_equal(selection, original_selection))
        {
            return Err(TextTargetError::new(
                TextTargetErrorKind::SelectionChanged,
                "The selection changed, so Spick did not type over it",
            ));
        }
    }
    pump_run_loop();
    if target.observer.was_invalidated() {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The field changed while Spick was listening, so nothing was typed",
        ));
    }
    check_deadline(deadline)?;

    #[cfg(feature = "macos-input-method-prototype")]
    if let Some(expected) = target.bundle_identifier.as_deref() {
        if application_bundle_identifier(target.pid).as_deref() != Some(expected) {
            return Err(TextTargetError::new(
                TextTargetErrorKind::FocusChanged,
                "The target app identity changed, so Spick did not type the transcript",
            ));
        }
    }
    Ok(editable)
}

fn revalidate_capture_snapshot(
    application: &AXUIElement,
    focus_anchor: &AXUIElement,
    pid: pid_t,
    original: &EditableTarget,
    deadline: Instant,
) -> Result<EditableTarget, TextTargetError> {
    ensure_secure_event_input_disabled()?;
    let focused = read_focused_context(deadline)?;
    if focused.pid != pid || !elements_equal(&focused.application, application) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The active app changed before Spick could start listening",
        ));
    }
    if !elements_equal(&focused.focus_anchor, focus_anchor) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The focused field changed before Spick could start listening",
        ));
    }
    ensure_not_secure(&focused.application, deadline)?;
    let current = resolve_editable_target(&focused.focus_anchor, deadline)?;
    if !elements_equal(&current.element, &original.element)
        || !optional_ranges_equal(current.selection, original.selection)
    {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The text target changed before Spick could start listening",
        ));
    }
    ensure_secure_event_input_disabled()?;
    Ok(current)
}

fn read_current_editable_target(
    target: &CapturedTarget,
    deadline: Instant,
) -> Result<EditableTarget, TextTargetError> {
    let focused = read_focused_context(deadline)?;
    let current_application = focused.application;
    if !elements_equal(&current_application, &target.application) || focused.pid != target.pid {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The active app changed, so Spick did not type the transcript",
        ));
    }
    ensure_not_secure(&current_application, deadline)?;
    check_deadline(deadline)?;

    let current_anchor = focused.focus_anchor;
    set_element_timeout(&current_anchor, deadline)?;
    if !elements_equal(&current_anchor, &target.focus_anchor)
        || read_pid(&current_anchor)? != target.pid
    {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The cursor moved to another field, so Spick did not type the transcript",
        ));
    }

    let editable = resolve_editable_target(&current_anchor, deadline)?;
    if !elements_equal(&editable.element, &target.element) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The editable field changed, so Spick did not type the transcript",
        ));
    }
    check_deadline(deadline)?;
    Ok(editable)
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn commit_through_accessibility(
    target: &CapturedTarget,
    transcript: &str,
    deadline: Instant,
) -> Result<TextInsertionReceipt, TextTargetError> {
    check_deadline(deadline)?;
    ensure_secure_event_input_disabled()?;
    let selection = target.selection.ok_or_else(|| {
        TextTargetError::new(
            TextTargetErrorKind::Unsupported,
            "The field does not expose a selection for direct insertion",
        )
    })?;
    let (inserted_range, expected_caret) = insertion_ranges(selection, transcript)?;
    set_element_timeout(&target.element, deadline)?;
    let attribute = CFString::from_static_str(AX_SELECTED_TEXT);
    let value = CFString::from_str(transcript);
    let value: &CFType = &value;
    let result = unsafe { target.element.set_attribute_value(&attribute, value) };
    if result != AXError::Success {
        return Err(TextTargetError::new(
            TextTargetErrorKind::Indeterminate,
            "The field did not confirm whether it accepted the transcript; check it before copying",
        ));
    }

    confirm_element_addressed_write(
        &target.element,
        inserted_range,
        expected_caret,
        transcript,
        deadline,
    )
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertionPath {
    ElementAddressed,
    ClipboardPaste,
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
fn insertion_path(
    selected_text_settable: bool,
    has_selection: bool,
    prefers_paste: bool,
) -> InsertionPath {
    if selected_text_settable && has_selection && !prefers_paste {
        InsertionPath::ElementAddressed
    } else {
        InsertionPath::ClipboardPaste
    }
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
struct PasteboardEntrySnapshot {
    data_type: Retained<NSPasteboardType>,
    data: Retained<NSData>,
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
struct PasteboardItemSnapshot {
    entries: Vec<PasteboardEntrySnapshot>,
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
struct PasteboardSnapshot {
    items: Vec<PasteboardItemSnapshot>,
    source_change_count: isize,
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
impl PasteboardSnapshot {
    fn capture(pasteboard: &NSPasteboard, deadline: Instant) -> Result<Self, TextTargetError> {
        check_deadline(deadline)?;
        let source_change_count = pasteboard.changeCount();
        let items = pasteboard.pasteboardItems();
        check_deadline(deadline)?;
        let Some(items) = items else {
            // A zero change count is the only nil case that proves the general
            // pasteboard has never held contents in this server session. At a
            // nonzero count nil can also mean access was denied, so fail closed
            // instead of clearing contents Spick could not preserve.
            if source_change_count == 0 && pasteboard.changeCount() == 0 {
                return Ok(Self {
                    items: Vec::new(),
                    source_change_count,
                });
            }
            return Err(clipboard_snapshot_unavailable_error());
        };
        if items.len() > PASTEBOARD_SNAPSHOT_MAX_ITEMS {
            return Err(clipboard_snapshot_too_large_error());
        }

        let mut total_bytes = 0_usize;
        let mut total_types = 0_usize;
        let mut snapshots = Vec::with_capacity(items.len());
        for item in items.iter() {
            let types = item.types();
            check_deadline(deadline)?;
            total_types = total_types
                .checked_add(types.len())
                .ok_or_else(clipboard_snapshot_too_large_error)?;
            if total_types > PASTEBOARD_SNAPSHOT_MAX_TYPES {
                return Err(clipboard_snapshot_too_large_error());
            }

            let mut entries = Vec::with_capacity(types.len());
            for data_type in types.iter() {
                let data = item.dataForType(&data_type);
                check_deadline(deadline)?;
                let data = data.ok_or_else(|| {
                    TextTargetError::new(
                        TextTargetErrorKind::Unsupported,
                        "Spick could not preserve every clipboard item before pasting",
                    )
                })?;
                total_bytes = checked_pasteboard_snapshot_size(total_bytes, data.length())
                    .ok_or_else(clipboard_snapshot_too_large_error)?;
                entries.push(PasteboardEntrySnapshot { data_type, data });
            }
            snapshots.push(PasteboardItemSnapshot { entries });
        }
        if !pasteboard_source_count_is_stable(source_change_count, pasteboard.changeCount()) {
            return Err(clipboard_changed_during_snapshot_error());
        }
        Ok(Self {
            items: snapshots,
            source_change_count,
        })
    }

    fn rebuild_items(
        &self,
        deadline: Instant,
    ) -> Result<Vec<Retained<NSPasteboardItem>>, TextTargetError> {
        self.items
            .iter()
            .map(|snapshot| {
                check_deadline(deadline)?;
                let item = NSPasteboardItem::new();
                for entry in &snapshot.entries {
                    if !item.setData_forType(&entry.data, &entry.data_type) {
                        return Err(TextTargetError::new(
                            TextTargetErrorKind::Indeterminate,
                            "Spick could not rebuild the previous clipboard contents",
                        ));
                    }
                    check_deadline(deadline)?;
                }
                Ok(item)
            })
            .collect()
    }
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
struct PasteboardTransaction {
    pasteboard: Retained<NSPasteboard>,
    restore_items: Option<Vec<Retained<NSPasteboardItem>>>,
    owned_change_count: isize,
    owner_type: Retained<NSPasteboardType>,
    owner_marker: Retained<NSString>,
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
impl PasteboardTransaction {
    fn begin(transcript: &str, deadline: Instant) -> Result<Self, TextTargetError> {
        check_deadline(deadline)?;
        let pasteboard = NSPasteboard::generalPasteboard();
        let snapshot = PasteboardSnapshot::capture(&pasteboard, deadline)?;
        let restore_items = snapshot.rebuild_items(deadline)?;
        let item = NSPasteboardItem::new();
        let transcript = NSString::from_str(transcript);
        if !item.setString_forType(&transcript, unsafe { NSPasteboardTypeString }) {
            return Err(TextTargetError::new(
                TextTargetErrorKind::Platform,
                "macOS could not prepare the transcript for cross-app insertion",
            ));
        }
        let owner_type = NSString::from_str(PASTEBOARD_OWNER_TYPE);
        let owner_marker = NSString::from_str(&uuid::Uuid::new_v4().simple().to_string());
        if !item.setString_forType(&owner_marker, &owner_type) {
            return Err(TextTargetError::new(
                TextTargetErrorKind::Platform,
                "macOS could not mark the temporary clipboard handoff",
            ));
        }
        check_deadline(deadline)?;
        if !pasteboard_source_count_is_stable(
            snapshot.source_change_count,
            pasteboard.changeCount(),
        ) {
            return Err(clipboard_changed_during_snapshot_error());
        }
        check_deadline(deadline)?;

        let cleared_change_count = pasteboard.clearContents();
        if !pasteboard_change_count_advanced_once(
            snapshot.source_change_count,
            cleared_change_count,
        ) {
            return Err(TextTargetError::new(
                TextTargetErrorKind::Indeterminate,
                "Clipboard ownership changed during the non-atomic paste handoff; check the clipboard before continuing",
            ));
        }
        if !write_pasteboard_items(&pasteboard, &[item]) {
            let current_change_count = pasteboard.changeCount();
            let restore_change_count =
                if pasteboard_change_count_is_owned(cleared_change_count, current_change_count)
                    || pasteboard_contains_owned_marker(
                        &pasteboard,
                        current_change_count,
                        &owner_type,
                        &owner_marker,
                    )
                {
                    Some(current_change_count)
                } else {
                    None
                };
            if let Some(restore_change_count) = restore_change_count {
                restore_items_after_failed_stage(
                    &pasteboard,
                    restore_change_count,
                    &restore_items,
                )?;
            }
            return Err(TextTargetError::new(
                TextTargetErrorKind::Indeterminate,
                "macOS could not complete the non-atomic clipboard handoff; check the clipboard before continuing",
            ));
        }
        let owned_change_count = pasteboard.changeCount();
        Ok(Self {
            pasteboard,
            restore_items: Some(restore_items),
            owned_change_count,
            owner_type,
            owner_marker,
        })
    }

    fn owns_current_contents(&self) -> bool {
        pasteboard_contains_owned_marker(
            &self.pasteboard,
            self.owned_change_count,
            &self.owner_type,
            &self.owner_marker,
        )
    }

    fn validate_marker(&self, deadline: Instant) -> Result<(), TextTargetError> {
        check_deadline(deadline)?;
        let owns_contents = self.owns_current_contents();
        check_deadline(deadline)?;
        if owns_contents {
            Ok(())
        } else {
            Err(TextTargetError::new(
                TextTargetErrorKind::FocusChanged,
                "The clipboard changed before Spick could paste, so no command was sent",
            ))
        }
    }

    fn ensure_change_count_owned(&self) -> Result<(), TextTargetError> {
        if pasteboard_change_count_is_owned(self.owned_change_count, self.pasteboard.changeCount())
        {
            Ok(())
        } else {
            Err(TextTargetError::new(
                TextTargetErrorKind::FocusChanged,
                "The clipboard changed before Spick could paste, so no command was sent",
            ))
        }
    }

    fn restore_if_owned(&mut self) -> Result<bool, TextTargetError> {
        if !self.owns_current_contents() {
            self.restore_items.take();
            return Ok(false);
        }
        let Some(items) = self.restore_items.take() else {
            return Ok(true);
        };
        let cleared_change_count = self.pasteboard.clearContents();
        if !pasteboard_change_count_advanced_once(self.owned_change_count, cleared_change_count) {
            return Err(TextTargetError::new(
                TextTargetErrorKind::Indeterminate,
                "Clipboard ownership changed during non-atomic restoration; check the clipboard before continuing",
            ));
        }
        if !items.is_empty() && !write_pasteboard_items(&self.pasteboard, &items) {
            return Err(TextTargetError::new(
                TextTargetErrorKind::Indeterminate,
                "Spick could not restore the previous clipboard contents",
            ));
        }
        Ok(true)
    }
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
impl Drop for PasteboardTransaction {
    fn drop(&mut self) {
        let _ = self.restore_if_owned();
    }
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn write_pasteboard_items(pasteboard: &NSPasteboard, items: &[Retained<NSPasteboardItem>]) -> bool {
    let writers = items
        .iter()
        .map(|item| ProtocolObject::from_ref(&**item))
        .collect::<Vec<&ProtocolObject<dyn NSPasteboardWriting>>>();
    let writers = NSArray::from_slice(&writers);
    pasteboard.writeObjects(&writers)
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn clipboard_snapshot_too_large_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::Unsupported,
        "The current clipboard is too large or complex to preserve safely",
    )
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn clipboard_snapshot_unavailable_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::Unsupported,
        "macOS did not allow Spick to preserve the current clipboard before pasting",
    )
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn clipboard_changed_during_snapshot_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::FocusChanged,
        "The clipboard changed while Spick prepared to paste, so no command was sent",
    )
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn checked_pasteboard_snapshot_size(current: usize, addition: usize) -> Option<usize> {
    current
        .checked_add(addition)
        .filter(|total| *total <= PASTEBOARD_SNAPSHOT_MAX_BYTES)
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn pasteboard_change_count_is_owned(expected: isize, current: isize) -> bool {
    expected == current
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn pasteboard_contains_owned_marker(
    pasteboard: &NSPasteboard,
    expected_change_count: isize,
    owner_type: &NSPasteboardType,
    owner_marker: &NSString,
) -> bool {
    if !pasteboard_change_count_is_owned(expected_change_count, pasteboard.changeCount()) {
        return false;
    }
    let marker_matches = pasteboard
        .pasteboardItems()
        .filter(|items| items.len() == 1)
        .and_then(|items| items.iter().next())
        .and_then(|item| item.stringForType(owner_type))
        .is_some_and(|marker| marker.to_string() == owner_marker.to_string());
    marker_matches
        && pasteboard_change_count_is_owned(expected_change_count, pasteboard.changeCount())
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn pasteboard_source_count_is_stable(expected: isize, current: isize) -> bool {
    expected == current
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn pasteboard_change_count_advanced_once(previous: isize, current: isize) -> bool {
    previous.checked_add(1) == Some(current)
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn restore_items_after_failed_stage(
    pasteboard: &NSPasteboard,
    expected_change_count: isize,
    items: &[Retained<NSPasteboardItem>],
) -> Result<(), TextTargetError> {
    if !pasteboard_change_count_is_owned(expected_change_count, pasteboard.changeCount()) {
        return Ok(());
    }
    let cleared_change_count = pasteboard.clearContents();
    if !pasteboard_change_count_advanced_once(expected_change_count, cleared_change_count) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::Indeterminate,
            "Clipboard ownership changed while Spick recovered from a failed paste handoff; check the clipboard before continuing",
        ));
    }
    if items.is_empty() || write_pasteboard_items(pasteboard, items) {
        Ok(())
    } else {
        Err(TextTargetError::new(
            TextTargetErrorKind::Indeterminate,
            "Spick could not restore the previous clipboard after the paste handoff failed",
        ))
    }
}

/// Guarded fallback for web, Electron, and custom controls that
/// need an ordinary paste command rather than an Accessibility value mutation.
/// The route is selected before any write and the exact target is revalidated
/// again immediately before one Cmd-V. NSPasteboard replacement and restore
/// are inherently non-atomic, so ownership checks only narrow the remaining
/// race. It therefore reports indeterminate delivery instead of retrying.
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn commit_through_clipboard_paste(
    target: &CapturedTarget,
    transcript: &str,
    deadline: Instant,
) -> Result<TextInsertionReceipt, TextTargetError> {
    check_deadline(deadline)?;
    ensure_secure_event_input_disabled()?;
    if !clipboard_paste_transcript_is_safe(transcript) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::Unsupported,
            "Automatic compatibility paste is limited to one line so terminal controls cannot execute dictated commands",
        ));
    }
    let expected_ranges = target
        .selection
        .map(|selection| insertion_ranges(selection, transcript))
        .transpose()?;
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| {
        TextTargetError::new(
            TextTargetErrorKind::Platform,
            "macOS could not create a private keyboard-event source",
        )
    })?;
    let key_down =
        CGEvent::new_keyboard_event(source.clone(), ANSI_VIRTUAL_KEY_V, true).map_err(|_| {
            TextTargetError::new(
                TextTargetErrorKind::Platform,
                "macOS could not create the paste command",
            )
        })?;
    let key_up = CGEvent::new_keyboard_event(source, ANSI_VIRTUAL_KEY_V, false).map_err(|_| {
        TextTargetError::new(
            TextTargetErrorKind::Platform,
            "macOS could not complete the paste command",
        )
    })?;
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);

    let mut pasteboard = PasteboardTransaction::begin(transcript, deadline)?;
    // Marker validation can invoke a lazy provider or a system privacy check.
    // Do it before the final exact target and secure-input revalidation.
    if let Err(error) = pasteboard.validate_marker(deadline) {
        return Err(restore_after_pre_dispatch_error(&mut pasteboard, error));
    }
    if let Err(target_error) = revalidate_captured_target(target, deadline) {
        return Err(restore_after_pre_dispatch_error(
            &mut pasteboard,
            target_error,
        ));
    }
    if let Err(error) = ensure_secure_event_input_disabled() {
        return Err(restore_after_pre_dispatch_error(&mut pasteboard, error));
    }
    if let Err(error) = check_deadline(deadline) {
        return Err(restore_after_pre_dispatch_error(&mut pasteboard, error));
    }
    if let Err(error) = pasteboard.ensure_change_count_owned() {
        return Err(restore_after_pre_dispatch_error(&mut pasteboard, error));
    }
    if let Err(error) = ensure_post_dispatch_budget(deadline) {
        return Err(restore_after_pre_dispatch_error(&mut pasteboard, error));
    }
    key_down.post_to_pid(target.pid);
    key_up.post_to_pid(target.pid);

    let delivery = confirm_clipboard_paste(target, transcript, expected_ranges, deadline);
    pasteboard.restore_if_owned()?;
    delivery
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn restore_after_pre_dispatch_error(
    pasteboard: &mut PasteboardTransaction,
    original: TextTargetError,
) -> TextTargetError {
    match pasteboard.restore_if_owned() {
        Ok(_) => original,
        Err(restore_error) => restore_error,
    }
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
fn clipboard_paste_transcript_is_safe(transcript: &str) -> bool {
    !transcript.contains(['\r', '\n'])
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn ensure_post_dispatch_budget(deadline: Instant) -> Result<(), TextTargetError> {
    if has_post_dispatch_budget(deadline.saturating_duration_since(Instant::now())) {
        Ok(())
    } else {
        Err(TextTargetError::new(
            TextTargetErrorKind::TimedOut,
            "The target took too long to prepare, so Spick did not send a paste command",
        ))
    }
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn has_post_dispatch_budget(remaining: Duration) -> bool {
    remaining >= INSERTION_CONFIRMATION_BUDGET
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn confirm_clipboard_paste(
    target: &CapturedTarget,
    transcript: &str,
    expected_ranges: Option<(CFRange, CFRange)>,
    deadline: Instant,
) -> Result<TextInsertionReceipt, TextTargetError> {
    let confirmation_deadline = deadline.min(Instant::now() + INSERTION_CONFIRMATION_BUDGET);
    if expected_ranges.is_none() {
        wait_for_paste_consumer(confirmation_deadline);
        return Err(indeterminate_paste_delivery_error());
    }
    loop {
        pump_run_loop();
        let editable = match read_current_editable_target(target, confirmation_deadline) {
            Ok(editable) => editable,
            Err(error)
                if insertion_confirmation_error_is_retryable(error.kind)
                    && Instant::now() < confirmation_deadline =>
            {
                sleep_for_retry(confirmation_deadline);
                continue;
            }
            Err(_) => {
                wait_for_paste_consumer(confirmation_deadline);
                return Err(indeterminate_paste_delivery_error());
            }
        };
        let (inserted_range, expected_caret) =
            expected_ranges.expect("selectionless paste returned before confirmation");
        let Some(current_selection) = editable.selection else {
            wait_for_paste_consumer(confirmation_deadline);
            return Err(indeterminate_paste_delivery_error());
        };
        let original_selection = target
            .selection
            .expect("confirmed paste ranges require an original selection");
        match keyboard_selection_state(current_selection, original_selection, expected_caret) {
            KeyboardSelectionState::Confirmed => {
                if set_element_timeout(&target.element, confirmation_deadline).is_err() {
                    wait_for_paste_consumer(confirmation_deadline);
                    return Err(indeterminate_paste_delivery_error());
                }
                match read_string_for_range(&target.element, inserted_range) {
                    Ok(readback)
                        if matches!(
                            insertion_readback_state(readback.as_deref(), transcript),
                            InsertionReadbackState::Confirmed | InsertionReadbackState::Unavailable
                        ) =>
                    {
                        return Ok(TextInsertionReceipt {
                            target_app: None,
                            caret_repositioned: true,
                        });
                    }
                    Ok(_) => {}
                    Err(error) if insertion_confirmation_error_is_retryable(error.kind) => {}
                    Err(_) => {
                        wait_for_paste_consumer(confirmation_deadline);
                        return Err(indeterminate_paste_delivery_error());
                    }
                }
            }
            KeyboardSelectionState::Pending => {}
            KeyboardSelectionState::Shifted => {
                wait_for_paste_consumer(confirmation_deadline);
                return Err(indeterminate_paste_delivery_error());
            }
        }
        if Instant::now() >= confirmation_deadline {
            return Err(indeterminate_paste_delivery_error());
        }
        sleep_for_retry(confirmation_deadline);
    }
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn wait_for_paste_consumer(deadline: Instant) {
    while Instant::now() < deadline {
        pump_run_loop();
        sleep_for_retry(deadline);
    }
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn confirm_element_addressed_write(
    element: &AXUIElement,
    inserted_range: CFRange,
    expected_caret: CFRange,
    transcript: &str,
    deadline: Instant,
) -> Result<TextInsertionReceipt, TextTargetError> {
    let confirmation_deadline = deadline.min(Instant::now() + INSERTION_CONFIRMATION_BUDGET);
    loop {
        pump_run_loop();
        set_element_timeout(element, confirmation_deadline)
            .map_err(|_| indeterminate_element_write_error())?;
        let caret_repositioned = read_optional_range(element, AX_SELECTED_TEXT_RANGE)
            .ok()
            .flatten()
            .is_some_and(|selection| ranges_equal(selection, expected_caret));
        set_element_timeout(element, confirmation_deadline)
            .map_err(|_| indeterminate_element_write_error())?;
        match read_string_for_range(element, inserted_range) {
            Ok(readback)
                if insertion_readback_state(readback.as_deref(), transcript)
                    == InsertionReadbackState::Confirmed =>
            {
                return Ok(TextInsertionReceipt {
                    target_app: None,
                    caret_repositioned,
                });
            }
            Ok(readback)
                if caret_repositioned
                    && insertion_readback_state(readback.as_deref(), transcript)
                        == InsertionReadbackState::Unavailable =>
            {
                return Ok(TextInsertionReceipt {
                    target_app: None,
                    caret_repositioned: true,
                });
            }
            Ok(_) => {}
            Err(error) if insertion_confirmation_error_is_retryable(error.kind) => {}
            Err(_) => return Err(indeterminate_element_write_error()),
        }
        if Instant::now() >= confirmation_deadline {
            return Err(indeterminate_element_write_error());
        }
        sleep_for_retry(confirmation_deadline);
    }
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
fn insertion_confirmation_error_is_retryable(kind: TextTargetErrorKind) -> bool {
    focused_context_error_is_transient(kind)
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertionReadbackState {
    Confirmed,
    Unavailable,
    Mismatch,
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
fn insertion_readback_state(readback: Option<&str>, transcript: &str) -> InsertionReadbackState {
    match readback {
        Some(value) if value == transcript => InsertionReadbackState::Confirmed,
        Some(_) => InsertionReadbackState::Mismatch,
        None => InsertionReadbackState::Unavailable,
    }
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyboardSelectionState {
    Pending,
    Confirmed,
    Shifted,
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
fn keyboard_selection_state(
    current: CFRange,
    original: CFRange,
    expected: CFRange,
) -> KeyboardSelectionState {
    if ranges_equal(current, expected) {
        KeyboardSelectionState::Confirmed
    } else if ranges_equal(current, original) {
        KeyboardSelectionState::Pending
    } else {
        KeyboardSelectionState::Shifted
    }
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn indeterminate_element_write_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::Indeterminate,
        "The field accepted a write but could not confirm its contents; check it before copying",
    )
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn indeterminate_paste_delivery_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::Indeterminate,
        "macOS sent the paste command but could not confirm the exact field and contents; check it before copying",
    )
}

#[cfg(any(test, not(feature = "macos-input-method-compatibility-harness")))]
fn insertion_ranges(
    selection: CFRange,
    transcript: &str,
) -> Result<(CFRange, CFRange), TextTargetError> {
    validate_range_shape(selection)?;
    let inserted_length =
        isize::try_from(transcript.encode_utf16().count()).map_err(|_| invalid_range_error())?;
    let caret_location = selection
        .location
        .checked_add(inserted_length)
        .ok_or_else(invalid_range_error)?;
    Ok((
        CFRange {
            location: selection.location,
            length: inserted_length,
        },
        CFRange {
            location: caret_location,
            length: 0,
        },
    ))
}

#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn read_string_for_range(
    element: &AXUIElement,
    range: CFRange,
) -> Result<Option<String>, TextTargetError> {
    let mut range = range;
    let parameter = unsafe {
        AXValue::new(
            AXValueType::CFRange,
            NonNull::from(&mut range).cast::<c_void>(),
        )
    }
    .ok_or_else(|| {
        TextTargetError::new(
            TextTargetErrorKind::Platform,
            "macOS could not encode the inserted text range",
        )
    })?;
    let parameter: &CFType = &parameter;
    let attribute = CFString::from_static_str(AX_STRING_FOR_RANGE);
    let mut raw: *const CFType = ptr::null();
    let result = unsafe {
        element.copy_parameterized_attribute_value(&attribute, parameter, NonNull::from(&mut raw))
    };
    if matches!(result, AXError::AttributeUnsupported | AXError::NoValue) {
        return Ok(None);
    }
    if result != AXError::Success {
        return Err(map_ax_error(result, "read back the inserted text"));
    }
    let raw = NonNull::new(raw.cast_mut()).ok_or_else(|| {
        TextTargetError::new(
            TextTargetErrorKind::Platform,
            "macOS returned an empty inserted-text value",
        )
    })?;
    let value = unsafe { CFRetained::<CFType>::from_raw(raw) };
    value
        .downcast::<CFString>()
        .map(|value| Some(value.to_string()))
        .map_err(|_| {
            TextTargetError::new(
                TextTargetErrorKind::Unsupported,
                "macOS returned an unexpected inserted-text value",
            )
        })
}

#[cfg(feature = "macos-input-method-prototype")]
struct InputMethodLease {
    request_id: u64,
    lease_id: u64,
    #[cfg(feature = "macos-input-method-compatibility-harness")]
    peer_cd_hash: String,
}

#[cfg(feature = "macos-input-method-prototype")]
impl InputMethodLease {
    fn mark_consumed(&mut self) {
        self.lease_id = 0;
    }
}

#[cfg(feature = "macos-input-method-prototype")]
impl Drop for InputMethodLease {
    fn drop(&mut self) {
        if self.lease_id != 0 {
            disarm_input_method_best_effort(self.request_id, self.lease_id);
        }
    }
}

#[cfg(feature = "macos-input-method-prototype")]
fn try_arm_input_method(
    request_id: u64,
    selection: CFRange,
    bundle_identifier: &str,
    deadline: Instant,
) -> Result<Option<InputMethodLease>, TextTargetError> {
    let (selection_location, selection_length) = selection_parts(selection)?;
    let request = encode_arm_request(
        request_id,
        deadline_epoch_milliseconds(deadline)?,
        selection_location,
        selection_length,
        bundle_identifier,
    )
    .map_err(|message| TextTargetError::new(TextTargetErrorKind::Unsupported, message))?;
    let mut connection = match InputMethodConnection::connect(deadline) {
        Ok(connection) => connection,
        Err(error) if error.kind == TextTargetErrorKind::Unsupported => return Ok(None),
        Err(error) => return Err(error),
    };
    let response = connection.exchange(&request, request_id, false, deadline)?;
    match response.status {
        InputMethodResponseStatus::Armed => Ok(Some(InputMethodLease {
            request_id,
            lease_id: response.lease_id,
            #[cfg(feature = "macos-input-method-compatibility-harness")]
            peer_cd_hash: connection.peer_cd_hash.clone(),
        })),
        InputMethodResponseStatus::SecureInput => Err(TextTargetError::new(
            TextTargetErrorKind::SecureField,
            "Spick Input refused to arm while secure input was active",
        )),
        InputMethodResponseStatus::TargetMismatch => Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The input-method client changed before recording began",
        )),
        InputMethodResponseStatus::SelectionChanged => Err(TextTargetError::new(
            TextTargetErrorKind::SelectionChanged,
            "The selection changed before recording began",
        )),
        InputMethodResponseStatus::NoActiveClient | InputMethodResponseStatus::Unsupported => {
            Ok(None)
        }
        InputMethodResponseStatus::RequestExpired => Err(TextTargetError::new(
            TextTargetErrorKind::TimedOut,
            "Spick Input could not arm the field before its deadline",
        )),
        InputMethodResponseStatus::Confirmed
        | InputMethodResponseStatus::Dispatched
        | InputMethodResponseStatus::InvalidRequest
        | InputMethodResponseStatus::InternalError
        | InputMethodResponseStatus::Disarmed
        | InputMethodResponseStatus::LeaseExpired
        | InputMethodResponseStatus::LeaseMissingOrConsumed => Err(input_method_platform_error()),
    }
}

#[cfg(feature = "macos-input-method-prototype")]
fn commit_through_input_method(
    target: &mut CapturedTarget,
    request_id: u64,
    transcript: &str,
    deadline: Instant,
) -> Result<(), TextTargetError> {
    let bundle_identifier = target.bundle_identifier.as_deref().ok_or_else(|| {
        TextTargetError::new(
            TextTargetErrorKind::Unsupported,
            "The focused app does not expose an identity Spick can verify",
        )
    })?;
    let lease_id = target
        .input_method_lease
        .as_ref()
        .map(|lease| lease.lease_id)
        .ok_or_else(|| {
            TextTargetError::new(
                TextTargetErrorKind::Unsupported,
                "Spick Input was not active when recording began",
            )
        })?;
    let selection = target.selection.ok_or_else(|| {
        TextTargetError::new(
            TextTargetErrorKind::Unsupported,
            "Spick Input requires a field that exposes its selection",
        )
    })?;
    let (selection_location, selection_length) = selection_parts(selection)?;
    let request = encode_insert_request(
        request_id,
        lease_id,
        deadline_epoch_milliseconds(deadline)?,
        selection_location,
        selection_length,
        bundle_identifier,
        transcript,
    )
    .map_err(|message| TextTargetError::new(TextTargetErrorKind::Unsupported, message))?;

    // Establish and authenticate the connection before the final AX snapshot.
    // No transcript bytes have crossed the process boundary at this point.
    let mut connection = InputMethodConnection::connect(deadline)?;
    #[cfg(feature = "macos-input-method-compatibility-harness")]
    if target
        .input_method_lease
        .as_ref()
        .is_some_and(|lease| lease.peer_cd_hash != connection.peer_cd_hash)
    {
        return Err(TextTargetError::new(
            TextTargetErrorKind::Platform,
            "The authenticated input-method helper changed during this compatibility attempt",
        ));
    }
    revalidate_captured_target(target, deadline)?;
    let response = connection.exchange(&request, request_id, true, deadline)?;
    if response.status != InputMethodResponseStatus::RequestExpired {
        if let Some(lease) = target.input_method_lease.as_mut() {
            // The helper consumes an Insert lease before every terminal response.
            lease.mark_consumed();
        }
    }

    map_insert_response_status(response.status)
}

#[cfg(feature = "macos-input-method-prototype")]
fn map_insert_response_status(status: InputMethodResponseStatus) -> Result<(), TextTargetError> {
    match status {
        InputMethodResponseStatus::Confirmed => Ok(()),
        InputMethodResponseStatus::Dispatched => Err(indeterminate_input_method_error()),
        InputMethodResponseStatus::NoActiveClient | InputMethodResponseStatus::Unsupported => {
            Err(TextTargetError::new(
                TextTargetErrorKind::Unsupported,
                "This field does not support verified input-method insertion yet",
            ))
        }
        InputMethodResponseStatus::TargetMismatch => Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The input-method client changed, so Spick did not type the transcript",
        )),
        InputMethodResponseStatus::SelectionChanged => Err(TextTargetError::new(
            TextTargetErrorKind::SelectionChanged,
            "The selection changed before the input method could type",
        )),
        InputMethodResponseStatus::SecureInput => Err(TextTargetError::new(
            TextTargetErrorKind::SecureField,
            "Spick Input refused to type while secure input was active",
        )),
        InputMethodResponseStatus::LeaseExpired => Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The original input-method session expired, so nothing was typed",
        )),
        InputMethodResponseStatus::RequestExpired => Err(TextTargetError::new(
            TextTargetErrorKind::TimedOut,
            "Spick Input could not claim the request before its deadline",
        )),
        InputMethodResponseStatus::LeaseMissingOrConsumed => {
            Err(indeterminate_input_method_error())
        }
        InputMethodResponseStatus::InvalidRequest | InputMethodResponseStatus::InternalError => {
            Err(TextTargetError::new(
                TextTargetErrorKind::Platform,
                "Spick Input could not process the insertion request",
            ))
        }
        InputMethodResponseStatus::Armed | InputMethodResponseStatus::Disarmed => {
            Err(indeterminate_input_method_error())
        }
    }
}

#[cfg(feature = "macos-input-method-prototype")]
struct InputMethodConnection {
    stream: UnixStream,
    #[cfg(feature = "macos-input-method-compatibility-harness")]
    peer_cd_hash: String,
}

#[cfg(feature = "macos-input-method-prototype")]
impl InputMethodConnection {
    fn connect(deadline: Instant) -> Result<Self, TextTargetError> {
        let socket_path = input_method_socket_path();
        validate_input_method_socket(&socket_path)?;
        let stream = connect_unix_with_deadline(&socket_path, deadline)
            .map_err(map_input_method_connect_error)?;
        #[cfg(feature = "macos-input-method-compatibility-harness")]
        let peer_cd_hash = authenticate_input_method_peer(&stream)?;
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        authenticate_input_method_peer(&stream)?;
        remaining_input_method_time(deadline)?;
        Ok(Self {
            stream,
            #[cfg(feature = "macos-input-method-compatibility-harness")]
            peer_cd_hash,
        })
    }

    fn exchange(
        &mut self,
        request: &[u8],
        request_id: u64,
        mutation_may_follow: bool,
        deadline: Instant,
    ) -> Result<InputMethodResponse, TextTargetError> {
        let mut written = 0;
        while written < request.len() {
            wait_for_socket_io(self.stream.as_raw_fd(), libc::POLLOUT, deadline)
                .map_err(|error| map_input_method_io_error(error, mutation_may_follow, written))?;
            match self.stream.write(&request[written..]) {
                Ok(0) => {
                    return Err(map_input_method_io_error(
                        std::io::Error::new(
                            std::io::ErrorKind::WriteZero,
                            "the input-method connection stopped accepting data",
                        ),
                        mutation_may_follow,
                        written,
                    ));
                }
                Ok(count) => written += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => {
                    return Err(map_input_method_io_error(
                        error,
                        mutation_may_follow,
                        written,
                    ));
                }
            }
        }

        let mut response = [0_u8; RESPONSE_LENGTH];
        let mut read = 0;
        while read < response.len() {
            wait_for_socket_io(self.stream.as_raw_fd(), libc::POLLIN, deadline)
                .map_err(|error| map_input_method_io_error(error, mutation_may_follow, written))?;
            match self.stream.read(&mut response[read..]) {
                Ok(0) => {
                    return Err(map_input_method_io_error(
                        std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "the input-method helper closed its response early",
                        ),
                        mutation_may_follow,
                        written,
                    ));
                }
                Ok(count) => read += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => {
                    return Err(map_input_method_io_error(
                        error,
                        mutation_may_follow,
                        written,
                    ));
                }
            }
        }
        decode_response(&response, request_id).map_err(|_| {
            if mutation_may_follow {
                indeterminate_input_method_error()
            } else {
                input_method_platform_error()
            }
        })
    }
}

#[cfg(feature = "macos-input-method-prototype")]
fn map_input_method_io_error(
    error: std::io::Error,
    mutation_may_follow: bool,
    bytes_written: usize,
) -> TextTargetError {
    if mutation_may_follow && bytes_written > 0 {
        indeterminate_input_method_error()
    } else if error.kind() == std::io::ErrorKind::TimedOut {
        TextTargetError::new(
            TextTargetErrorKind::TimedOut,
            "Spick Input took too long to answer, so nothing was sent",
        )
    } else {
        input_method_platform_error()
    }
}

#[cfg(feature = "macos-input-method-prototype")]
fn map_input_method_connect_error(error: std::io::Error) -> TextTargetError {
    match error.kind() {
        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound => {
            TextTargetError::new(
                TextTargetErrorKind::Unsupported,
                "Spick Input is not active; the transcript is ready to copy",
            )
        }
        std::io::ErrorKind::TimedOut => TextTargetError::new(
            TextTargetErrorKind::TimedOut,
            "Spick Input took too long to accept a private connection",
        ),
        _ => input_method_platform_error(),
    }
}

#[cfg(feature = "macos-input-method-prototype")]
fn input_method_platform_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::Platform,
        "macOS could not use the private Spick Input connection",
    )
}

#[cfg(feature = "macos-input-method-prototype")]
fn selection_parts(selection: CFRange) -> Result<(usize, usize), TextTargetError> {
    let location = usize::try_from(selection.location).map_err(|_| invalid_range_error())?;
    let length = usize::try_from(selection.length).map_err(|_| invalid_range_error())?;
    location
        .checked_add(length)
        .ok_or_else(invalid_range_error)?;
    Ok((location, length))
}

#[cfg(feature = "macos-input-method-prototype")]
fn validate_input_method_socket(path: &Path) -> Result<(), TextTargetError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| {
        TextTargetError::new(
            TextTargetErrorKind::Unsupported,
            "Spick Input is not installed and active yet; the transcript is ready to copy",
        )
    })?;
    if metadata.file_type().is_socket()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o077 == 0
    {
        Ok(())
    } else {
        Err(TextTargetError::new(
            TextTargetErrorKind::Unsupported,
            "Spick Input did not expose a private local connection",
        ))
    }
}

#[cfg(feature = "macos-input-method-prototype")]
fn connect_unix_with_deadline(path: &Path, deadline: Instant) -> std::io::Result<UnixStream> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid local socket path",
        )
    })?;
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let descriptor = unsafe { OwnedFd::from_raw_fd(raw) };
    let descriptor_flags = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFD) };
    if descriptor_flags < 0
        || unsafe {
            libc::fcntl(
                descriptor.as_raw_fd(),
                libc::F_SETFD,
                descriptor_flags | libc::FD_CLOEXEC,
            )
        } < 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let flags = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe {
            libc::fcntl(
                descriptor.as_raw_fd(),
                libc::F_SETFL,
                flags | libc::O_NONBLOCK,
            )
        } < 0
    {
        return Err(std::io::Error::last_os_error());
    }

    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    address.sun_len = std::mem::size_of::<libc::sockaddr_un>() as u8;
    let path_bytes = path.as_bytes_with_nul();
    if path_bytes.len() > address.sun_path.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "local socket path is too long",
        ));
    }
    for (target, source) in address.sun_path.iter_mut().zip(path_bytes.iter().copied()) {
        *target = source as libc::c_char;
    }
    let result = unsafe {
        libc::connect(
            descriptor.as_raw_fd(),
            std::ptr::addr_of!(address).cast::<libc::sockaddr>(),
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(error);
        }
        wait_for_socket_connection(descriptor.as_raw_fd(), deadline)?;
    }
    let mut socket_error = 0;
    let mut socket_error_length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            descriptor.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            std::ptr::addr_of_mut!(socket_error).cast(),
            &mut socket_error_length,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if socket_error != 0 {
        return Err(std::io::Error::from_raw_os_error(socket_error));
    }
    Ok(UnixStream::from(descriptor))
}

#[cfg(feature = "macos-input-method-prototype")]
fn wait_for_socket_connection(descriptor: libc::c_int, deadline: Instant) -> std::io::Result<()> {
    wait_for_socket_io(descriptor, libc::POLLOUT, deadline)
}

#[cfg(feature = "macos-input-method-prototype")]
fn wait_for_socket_io(
    descriptor: libc::c_int,
    events: libc::c_short,
    deadline: Instant,
) -> std::io::Result<()> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "local socket connection timed out",
            ));
        }
        let milliseconds = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        let mut poll = libc::pollfd {
            fd: descriptor,
            events,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut poll, 1, milliseconds) };
        if result > 0 {
            if poll.revents & events != 0 {
                return Ok(());
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the local input-method connection closed",
            ));
        }
        if result == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "local socket connection timed out",
            ));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(feature = "macos-input-method-prototype")]
#[cfg(not(feature = "macos-input-method-compatibility-harness"))]
fn authenticate_input_method_peer(stream: &UnixStream) -> Result<(), TextTargetError> {
    let descriptor = stream.as_raw_fd();
    let trust = unsafe {
        SpickVerifyPeerSocket(
            descriptor,
            DESKTOP_SIGNING_IDENTIFIER.as_ptr().cast(),
            INPUT_METHOD_SIGNING_IDENTIFIER.as_ptr().cast(),
        )
    };
    let unsafe_development_allowed = unsafe { SpickPeerAuthenticationAllowsUnsafeDevelopment() };
    if trust == PEER_TRUST_SECURE
        || (trust == PEER_TRUST_UNSAFE_DEVELOPMENT && unsafe_development_allowed)
    {
        Ok(())
    } else {
        Err(untrusted_input_method_error())
    }
}

#[cfg(feature = "macos-input-method-compatibility-harness")]
fn authenticate_input_method_peer(stream: &UnixStream) -> Result<String, TextTargetError> {
    let descriptor = stream.as_raw_fd();
    let mut cd_hash = [0_i8; 65];
    let trust = unsafe {
        SpickVerifyPeerSocketWithCDHash(
            descriptor,
            DESKTOP_SIGNING_IDENTIFIER.as_ptr().cast(),
            INPUT_METHOD_SIGNING_IDENTIFIER.as_ptr().cast(),
            cd_hash.as_mut_ptr(),
            cd_hash.len(),
        )
    };
    let unsafe_development_allowed = unsafe { SpickPeerAuthenticationAllowsUnsafeDevelopment() };
    if trust != PEER_TRUST_SECURE
        && !(trust == PEER_TRUST_UNSAFE_DEVELOPMENT && unsafe_development_allowed)
    {
        return Err(untrusted_input_method_error());
    }
    let cd_hash = unsafe { CStr::from_ptr(cd_hash.as_ptr()) }
        .to_str()
        .map_err(|_| untrusted_input_method_error())?;
    if !matches!(cd_hash.len(), 40 | 64)
        || !cd_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(untrusted_input_method_error());
    }
    Ok(cd_hash.into())
}

#[cfg(feature = "macos-input-method-prototype")]
fn untrusted_input_method_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::Platform,
        "Spick refused an unverified input-method connection",
    )
}

#[cfg(feature = "macos-input-method-prototype")]
fn remaining_input_method_time(deadline: Instant) -> Result<Duration, TextTargetError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(TextTargetError::new(
            TextTargetErrorKind::TimedOut,
            "The focused app took too long to answer, so Spick did not type",
        ))
    } else {
        Ok(remaining.min(INPUT_METHOD_TIMEOUT))
    }
}

#[cfg(feature = "macos-input-method-prototype")]
fn deadline_epoch_milliseconds(deadline: Instant) -> Result<u64, TextTargetError> {
    let remaining = remaining_input_method_time(deadline)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        TextTargetError::new(
            TextTargetErrorKind::Platform,
            "the system clock is unavailable",
        )
    })?;
    u64::try_from((now + remaining).as_millis()).map_err(|_| {
        TextTargetError::new(TextTargetErrorKind::Platform, "the system clock is invalid")
    })
}

#[cfg(feature = "macos-input-method-prototype")]
fn indeterminate_input_method_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::Indeterminate,
        "Spick Input did not confirm the write; check the field before copying",
    )
}

#[cfg(feature = "macos-input-method-prototype")]
fn input_method_socket_path() -> PathBuf {
    std::env::temp_dir().join(INPUT_METHOD_SOCKET_NAME)
}

#[cfg(feature = "macos-input-method-prototype")]
fn application_bundle_identifier(pid: pid_t) -> Option<String> {
    objc2::rc::autoreleasepool(|_| {
        let application = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
        let identifier = application.bundleIdentifier()?.to_string();
        if identifier.is_empty()
            || identifier.len() > 512
            || identifier.chars().any(char::is_control)
        {
            None
        } else {
            Some(identifier)
        }
    })
}

#[cfg(feature = "macos-input-method-prototype")]
fn disarm_input_method_best_effort(request_id: u64, lease_id: u64) {
    let deadline = Instant::now() + Duration::from_millis(150);
    let Ok(expiry) = deadline_epoch_milliseconds(deadline) else {
        return;
    };
    let Ok(request) = encode_disarm_request(request_id, lease_id, expiry) else {
        return;
    };
    let Ok(mut connection) = InputMethodConnection::connect(deadline) else {
        return;
    };
    let _ = connection.exchange(&request, request_id, false, deadline);
}

struct InvalidationContext {
    invalidated: AtomicBool,
}

struct ObserverLease {
    observer: Option<CFRetained<AXObserver>>,
    source: Option<CFRetained<CFRunLoopSource>>,
    run_loop: CFRetained<CFRunLoop>,
    context: Box<InvalidationContext>,
}

impl ObserverLease {
    fn install(
        run_loop: &CFRunLoop,
        pid: pid_t,
        application: &AXUIElement,
        focus_anchor: &AXUIElement,
        edit_element: &AXUIElement,
    ) -> Result<Self, TextTargetError> {
        let mut raw_observer: *mut AXObserver = ptr::null_mut();
        let created = unsafe {
            AXObserver::create(
                pid,
                Some(invalidate_target),
                NonNull::from(&mut raw_observer),
            )
        };
        if created != AXError::Success {
            return Err(map_ax_error(created, "watch the focused text field"));
        }
        let raw_observer = NonNull::new(raw_observer).ok_or_else(|| {
            TextTargetError::new(
                TextTargetErrorKind::Platform,
                "macOS did not create the text-field observer",
            )
        })?;
        let observer = unsafe { CFRetained::<AXObserver>::from_raw(raw_observer) };
        let mut context = Box::new(InvalidationContext {
            invalidated: AtomicBool::new(false),
        });
        let context_pointer = (&mut *context as *mut InvalidationContext).cast::<c_void>();

        register_notification(
            &observer,
            application,
            AX_APPLICATION_DEACTIVATED,
            context_pointer,
        )?;
        register_notification(
            &observer,
            application,
            AX_FOCUSED_UI_ELEMENT_CHANGED,
            context_pointer,
        )?;
        register_notification(
            &observer,
            edit_element,
            AX_SELECTED_TEXT_CHANGED,
            context_pointer,
        )?;
        register_notification(&observer, edit_element, AX_VALUE_CHANGED, context_pointer)?;
        register_notification(
            &observer,
            edit_element,
            AX_UI_ELEMENT_DESTROYED,
            context_pointer,
        )?;
        if !elements_equal(focus_anchor, edit_element) {
            register_notification(
                &observer,
                focus_anchor,
                AX_UI_ELEMENT_DESTROYED,
                context_pointer,
            )?;
        }

        let source = unsafe { observer.run_loop_source() };
        let mode = unsafe { kCFRunLoopDefaultMode }.ok_or_else(|| {
            TextTargetError::new(
                TextTargetErrorKind::Platform,
                "macOS did not provide the default run-loop mode",
            )
        })?;
        run_loop.add_source(Some(&source), Some(mode));

        Ok(Self {
            observer: Some(observer),
            source: Some(source),
            run_loop: unsafe { CFRetained::retain(run_loop.into()) },
            context,
        })
    }

    fn was_invalidated(&self) -> bool {
        self.context.invalidated.load(Ordering::Acquire)
    }
}

impl Drop for ObserverLease {
    fn drop(&mut self) {
        if let (Some(source), Some(mode)) = (self.source.as_ref(), unsafe { kCFRunLoopDefaultMode })
        {
            self.run_loop.remove_source(Some(source), Some(mode));
        }
        // Drop the observer before its callback context. The source is already
        // detached, so no later callback can observe a freed refcon pointer.
        self.observer.take();
        self.source.take();
    }
}

unsafe extern "C-unwind" fn invalidate_target(
    _observer: NonNull<AXObserver>,
    _element: NonNull<AXUIElement>,
    _notification: NonNull<CFString>,
    refcon: *mut c_void,
) {
    let Some(context) = (unsafe { refcon.cast::<InvalidationContext>().as_ref() }) else {
        return;
    };
    context.invalidated.store(true, Ordering::Release);
}

fn register_notification(
    observer: &AXObserver,
    element: &AXUIElement,
    notification: &'static str,
    context: *mut c_void,
) -> Result<(), TextTargetError> {
    let notification = CFString::from_static_str(notification);
    let result = unsafe { observer.add_notification(element, &notification, context) };
    if notification_registration_is_best_effort_success(result) {
        Ok(())
    } else {
        Err(map_ax_error(result, "watch the focused text field"))
    }
}

fn notification_registration_is_best_effort_success(result: AXError) -> bool {
    matches!(
        result,
        AXError::Success
            | AXError::NotificationAlreadyRegistered
            | AXError::NotificationUnsupported
            | AXError::AttributeUnsupported
            | AXError::NoValue
            | AXError::NotImplemented
    )
}

struct EditableTarget {
    element: CFRetained<AXUIElement>,
    selection: Option<CFRange>,
    #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
    selected_text_settable: bool,
    prefers_paste: bool,
}

struct FocusedContext {
    application: CFRetained<AXUIElement>,
    focus_anchor: CFRetained<AXUIElement>,
    pid: pid_t,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusedApplicationFallbackError {
    Missing,
    PidMismatch,
}

fn select_coherent_focused_pid(
    system_focused_element_pid: Option<pid_t>,
    accessibility_application_pid: Option<pid_t>,
    frontmost_application_pid: Option<pid_t>,
) -> Result<pid_t, FocusedApplicationFallbackError> {
    let mut reported = [
        system_focused_element_pid,
        accessibility_application_pid,
        frontmost_application_pid,
    ]
    .into_iter()
    .flatten();
    let Some(pid) = reported.next() else {
        return Err(FocusedApplicationFallbackError::Missing);
    };
    if reported.all(|reported_pid| reported_pid == pid) {
        Ok(pid)
    } else {
        Err(FocusedApplicationFallbackError::PidMismatch)
    }
}

fn permission_status() -> AccessibilityPermissionStatus {
    AccessibilityPermissionStatus {
        state: if is_trusted() {
            AccessibilityPermissionState::Granted
        } else {
            AccessibilityPermissionState::Missing
        },
        can_request: true,
    }
}

fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

fn secure_event_input_enabled() -> bool {
    unsafe { IsSecureEventInputEnabled() != 0 }
}

fn ensure_secure_event_input_disabled() -> Result<(), TextTargetError> {
    if secure_event_input_enabled() {
        Err(TextTargetError::new(
            TextTargetErrorKind::SecureField,
            "Spick does not record or type while macOS Secure Event Input is active",
        ))
    } else {
        Ok(())
    }
}

fn request_permission() -> Result<AccessibilityPermissionStatus, TextTargetError> {
    let prompt_key: &CFType = unsafe { kAXTrustedCheckOptionPrompt };
    let prompt_value: &CFBoolean = unsafe { kCFBooleanTrue }.ok_or_else(|| {
        TextTargetError::new(
            TextTargetErrorKind::Platform,
            "macOS did not provide the Accessibility prompt option",
        )
    })?;
    let prompt_value: &CFType = prompt_value;
    let options = CFDictionary::<CFType, CFType>::from_slices(&[prompt_key], &[prompt_value]);
    let granted = unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) };
    Ok(AccessibilityPermissionStatus {
        state: if granted {
            AccessibilityPermissionState::Granted
        } else {
            AccessibilityPermissionState::Missing
        },
        can_request: true,
    })
}

/// Read one coherent app/field snapshot. The system-wide focused element is
/// authoritative because Chromium and Electron can keep AXFocusedApplication
/// populated while the application's own focus proxy is stale or overly broad.
/// Every other source is used only as a coherence check or a last-resort
/// fallback, and conflicting PIDs are retried as an in-flight app switch.
fn read_focused_context(deadline: Instant) -> Result<FocusedContext, TextTargetError> {
    let retry_deadline = deadline.min(Instant::now() + FOCUSED_CONTEXT_RETRY_BUDGET);

    loop {
        check_deadline(retry_deadline)?;
        match read_focused_context_once(retry_deadline) {
            Ok(context) => {
                check_deadline(retry_deadline)?;
                return Ok(context);
            }
            Err(error) if focused_context_error_is_transient(error.kind) => {
                if Instant::now() >= retry_deadline {
                    return Err(error);
                }
                sleep_for_retry(retry_deadline);
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_focused_context_once(deadline: Instant) -> Result<FocusedContext, TextTargetError> {
    check_deadline(deadline)?;
    let system = unsafe { AXUIElement::new_system_wide() };
    let system_anchor = read_optional_focus_element(&system, AX_FOCUSED_UI_ELEMENT, deadline)?;
    check_deadline(deadline)?;
    let system_anchor_pid = system_anchor
        .as_ref()
        .map(|anchor| read_pid(anchor))
        .transpose()?;
    let reported_application =
        read_optional_focus_element(&system, AX_FOCUSED_APPLICATION, deadline)?;
    check_deadline(deadline)?;
    let reported_application_pid = reported_application
        .as_ref()
        .map(|application| read_pid(application))
        .transpose()?;
    let frontmost_pid = frontmost_application_pid();
    check_deadline(deadline)?;
    let pid =
        select_coherent_focused_pid(system_anchor_pid, reported_application_pid, frontmost_pid)
            .map_err(|error| match error {
                FocusedApplicationFallbackError::Missing => TextTargetError::new(
                    TextTargetErrorKind::NoFocusedTarget,
                    "macOS did not report a focused application or text field",
                ),
                FocusedApplicationFallbackError::PidMismatch => TextTargetError::new(
                    TextTargetErrorKind::NoFocusedTarget,
                    "macOS reported inconsistent focused application and text field",
                ),
            })?;
    let application = match reported_application {
        Some(application) if reported_application_pid == Some(pid) => application,
        _ => unsafe { AXUIElement::new_application(pid) },
    };
    check_deadline(deadline)?;
    set_element_timeout(&application, deadline)?;
    if read_pid(&application)? != pid {
        return Err(TextTargetError::new(
            TextTargetErrorKind::NoFocusedTarget,
            "macOS reported an inconsistent focused application",
        ));
    }
    check_deadline(deadline)?;
    let mut application_anchor =
        read_optional_focus_element(&application, AX_FOCUSED_UI_ELEMENT, deadline)?;
    if application_anchor.is_none() {
        enable_manual_accessibility_best_effort(&application);
        check_deadline(deadline)?;
        set_element_timeout(&application, deadline)?;
        application_anchor =
            read_optional_focus_element(&application, AX_FOCUSED_UI_ELEMENT, deadline)?;
    }
    // The application-level proxy may remain a web-area container while the
    // system-wide element points at the actual editor. PID coherence above is
    // the cross-source safety boundary; element equality would reject the
    // exact Chromium/Electron controls this fallback exists to reach.
    let focus_anchor = match (system_anchor, application_anchor) {
        (Some(system_anchor), _) => system_anchor,
        (None, Some(application_anchor)) => application_anchor,
        (None, None) => {
            return Err(TextTargetError::new(
                TextTargetErrorKind::NoFocusedTarget,
                "macOS did not report a focused text field",
            ));
        }
    };
    set_element_timeout(&focus_anchor, deadline)?;
    if read_pid(&focus_anchor)? != pid {
        return Err(TextTargetError::new(
            TextTargetErrorKind::NoFocusedTarget,
            "macOS reported an inconsistent focused field",
        ));
    }
    check_deadline(deadline)?;

    Ok(FocusedContext {
        application,
        focus_anchor,
        pid,
    })
}

fn read_optional_focus_element(
    element: &AXUIElement,
    attribute: &'static str,
    deadline: Instant,
) -> Result<Option<CFRetained<AXUIElement>>, TextTargetError> {
    check_deadline(deadline)?;
    let result = read_optional_element(element, attribute);
    check_deadline(deadline)?;
    match result {
        Err(error) if focused_context_error_is_transient(error.kind) => Ok(None),
        result => result,
    }
}

fn frontmost_application_pid() -> Option<pid_t> {
    let workspace = NSWorkspace::sharedWorkspace();
    let application = workspace.frontmostApplication()?;
    let pid = application.processIdentifier();
    (pid > 0).then_some(pid)
}

fn enable_manual_accessibility_best_effort(application: &AXUIElement) {
    let Some(enabled) = (unsafe { kCFBooleanTrue }) else {
        return;
    };
    let attribute = CFString::from_static_str(AX_MANUAL_ACCESSIBILITY);
    let enabled: &CFType = enabled;
    let _ = unsafe { application.set_attribute_value(&attribute, enabled) };
}

fn focused_context_error_is_transient(kind: TextTargetErrorKind) -> bool {
    matches!(
        kind,
        TextTargetErrorKind::NoFocusedTarget
            | TextTargetErrorKind::TargetGone
            | TextTargetErrorKind::TimedOut
    )
}

fn resolve_editable_target(
    focus_anchor: &AXUIElement,
    deadline: Instant,
) -> Result<EditableTarget, TextTargetError> {
    let mut current = retain_element(focus_anchor);
    let mut candidate = None;
    let mut saw_web_area = false;
    for depth in 0..MAX_PARENT_DEPTH {
        check_deadline(deadline)?;
        ensure_not_secure(&current, deadline)?;
        check_deadline(deadline)?;
        set_element_timeout(&current, deadline)?;
        if read_optional_bool(&current, AX_ENABLED)?.is_some_and(|enabled| !enabled) {
            return Err(TextTargetError::new(
                TextTargetErrorKind::NotEditable,
                "The focused field is disabled",
            ));
        }

        set_element_timeout(&current, deadline)?;
        saw_web_area |=
            read_optional_string(&current, AX_ROLE)?.as_deref() == Some(AX_WEB_AREA_ROLE);
        if candidate.is_none() {
            candidate = editable_snapshot(&current, depth == 0, deadline)?;
        }
        check_deadline(deadline)?;
        set_element_timeout(&current, deadline)?;
        let parent = read_optional_element(&current, AX_PARENT)?;
        if depth + 1 == MAX_PARENT_DEPTH {
            if parent.is_some() {
                return Err(TextTargetError::new(
                    TextTargetErrorKind::Unsupported,
                    "The text field hierarchy is too deep for Spick to verify safely",
                ));
            }
            break;
        }
        let Some(parent) = parent else {
            break;
        };
        check_deadline(deadline)?;
        current = parent;
    }

    match candidate {
        Some(mut candidate) => {
            candidate.prefers_paste |= saw_web_area;
            Ok(candidate)
        }
        None => Err(TextTargetError::new(
            TextTargetErrorKind::NotEditable,
            "Click an editable text field before holding the shortcut",
        )),
    }
}

fn editable_snapshot(
    element: &AXUIElement,
    is_focus_anchor: bool,
    deadline: Instant,
) -> Result<Option<EditableTarget>, TextTargetError> {
    set_element_timeout(element, deadline)?;
    let role = read_optional_string(element, AX_ROLE)?;
    check_deadline(deadline)?;
    set_element_timeout(element, deadline)?;
    let selected_text_settable = is_settable(element, AX_SELECTED_TEXT)?;
    check_deadline(deadline)?;
    if !has_editable_text_capability(role.as_deref(), selected_text_settable) {
        return Ok(None);
    }
    set_element_timeout(element, deadline)?;
    let selection = read_optional_range(element, AX_SELECTED_TEXT_RANGE)?;
    check_deadline(deadline)?;
    if let Some(selection) = selection {
        validate_range_shape(selection)?;
    } else {
        set_element_timeout(element, deadline)?;
        let value_settable = is_settable(element, AX_VALUE)?;
        check_deadline(deadline)?;
        if !can_capture_without_selection(role.as_deref(), value_settable, is_focus_anchor) {
            return Ok(None);
        }
    }

    if selection.is_some() {
        set_element_timeout(element, deadline)?;
        if let Some(ranges) = read_optional_array(element, AX_SELECTED_TEXT_RANGES)? {
            if ranges.len() > 1 {
                return Err(TextTargetError::new(
                    TextTargetErrorKind::Unsupported,
                    "Spick does not type over multiple selections yet",
                ));
            }
        }
    }
    check_deadline(deadline)?;

    Ok(Some(EditableTarget {
        element: retain_element(element),
        selection,
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        selected_text_settable,
        prefers_paste: selection.is_none() || !selected_text_settable,
    }))
}

fn is_editable_text_role(role: Option<&str>) -> bool {
    matches!(role, Some(AX_TEXT_FIELD_ROLE | AX_TEXT_AREA_ROLE))
}

fn has_editable_text_capability(role: Option<&str>, selected_text_settable: bool) -> bool {
    is_editable_text_role(role) || selected_text_settable
}

fn can_capture_without_selection(
    role: Option<&str>,
    value_settable: bool,
    is_focus_anchor: bool,
) -> bool {
    is_focus_anchor && value_settable && is_editable_text_role(role)
}

fn ensure_not_secure(element: &AXUIElement, deadline: Instant) -> Result<(), TextTargetError> {
    set_element_timeout(element, deadline)?;
    let subrole = read_optional_string(element, AX_SUBROLE)?;
    check_deadline(deadline)?;
    set_element_timeout(element, deadline)?;
    let contains_protected_content = read_optional_bool(element, AX_CONTAINS_PROTECTED_CONTENT)?;
    check_deadline(deadline)?;
    if has_secure_marker(subrole.as_deref(), contains_protected_content) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::SecureField,
            "Spick does not record or type in password fields",
        ));
    }
    Ok(())
}

fn has_secure_marker(subrole: Option<&str>, contains_protected_content: Option<bool>) -> bool {
    subrole == Some(AX_SECURE_TEXT_FIELD) || contains_protected_content == Some(true)
}

fn set_element_timeout(element: &AXUIElement, deadline: Instant) -> Result<(), TextTargetError> {
    check_deadline(deadline)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    let timeout = bounded_messaging_timeout_seconds(remaining).ok_or_else(deadline_error)?;
    let result = unsafe { element.set_messaging_timeout(timeout) };
    if result == AXError::Success {
        check_deadline(deadline)
    } else {
        Err(map_ax_error(result, "set an Accessibility timeout"))
    }
}

fn bounded_messaging_timeout_seconds(remaining: Duration) -> Option<f32> {
    (!remaining.is_zero()).then(|| remaining.as_secs_f32().min(APPLICATION_TIMEOUT_SECONDS))
}

fn read_pid(element: &AXUIElement) -> Result<pid_t, TextTargetError> {
    let mut pid = 0;
    let result = unsafe { element.pid(NonNull::from(&mut pid)) };
    if result == AXError::Success && pid > 0 {
        Ok(pid)
    } else if result == AXError::Success {
        Err(TextTargetError::new(
            TextTargetErrorKind::TargetGone,
            "macOS did not identify the focused application",
        ))
    } else {
        Err(map_ax_error(result, "identify the focused application"))
    }
}

fn read_optional_element(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<CFRetained<AXUIElement>>, TextTargetError> {
    read_optional_attribute(element, attribute)?
        .map(|value| {
            value.downcast::<AXUIElement>().map_err(|_| {
                TextTargetError::new(
                    TextTargetErrorKind::Unsupported,
                    "macOS returned an unexpected Accessibility element type",
                )
            })
        })
        .transpose()
}

fn read_optional_string(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<String>, TextTargetError> {
    read_optional_attribute(element, attribute)?
        .map(|value| {
            value
                .downcast::<CFString>()
                .map(|string| string.to_string())
                .map_err(|_| {
                    TextTargetError::new(
                        TextTargetErrorKind::Unsupported,
                        "macOS returned an unexpected text-field value type",
                    )
                })
        })
        .transpose()
}

fn read_optional_bool(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<bool>, TextTargetError> {
    read_optional_attribute(element, attribute)?
        .map(|value| {
            value
                .downcast::<CFBoolean>()
                .map(|boolean| boolean.value())
                .map_err(|_| {
                    TextTargetError::new(
                        TextTargetErrorKind::Unsupported,
                        "macOS returned an unexpected Accessibility state type",
                    )
                })
        })
        .transpose()
}

fn read_optional_range(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<CFRange>, TextTargetError> {
    read_optional_attribute(element, attribute)?
        .map(|value| {
            let value = value.downcast::<AXValue>().map_err(|_| {
                TextTargetError::new(
                    TextTargetErrorKind::Unsupported,
                    "macOS returned an unexpected selection type",
                )
            })?;
            if unsafe { value.r#type() } != AXValueType::CFRange {
                return Err(TextTargetError::new(
                    TextTargetErrorKind::Unsupported,
                    "macOS returned an unsupported selection range",
                ));
            }
            let mut range = CFRange {
                location: 0,
                length: 0,
            };
            let decoded = unsafe {
                value.value(
                    AXValueType::CFRange,
                    NonNull::from(&mut range).cast::<c_void>(),
                )
            };
            if decoded {
                Ok(range)
            } else {
                Err(TextTargetError::new(
                    TextTargetErrorKind::Unsupported,
                    "macOS could not decode the selection range",
                ))
            }
        })
        .transpose()
}

fn read_optional_array(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<CFRetained<CFArray>>, TextTargetError> {
    read_optional_attribute(element, attribute)?
        .map(|value| {
            value.downcast::<CFArray>().map_err(|_| {
                TextTargetError::new(
                    TextTargetErrorKind::Unsupported,
                    "macOS returned an unexpected multiple-selection type",
                )
            })
        })
        .transpose()
}

fn read_optional_attribute(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<CFRetained<CFType>>, TextTargetError> {
    let attribute = CFString::from_static_str(attribute);
    let mut raw: *const CFType = ptr::null();
    let result = unsafe { element.copy_attribute_value(&attribute, NonNull::from(&mut raw)) };
    if matches!(result, AXError::AttributeUnsupported | AXError::NoValue) {
        return Ok(None);
    }
    if result != AXError::Success {
        return Err(map_ax_error(result, "read the focused text field"));
    }
    let raw = NonNull::new(raw.cast_mut()).ok_or_else(|| {
        TextTargetError::new(
            TextTargetErrorKind::Platform,
            "macOS returned an empty Accessibility value",
        )
    })?;
    Ok(Some(unsafe { CFRetained::<CFType>::from_raw(raw) }))
}

fn is_settable(element: &AXUIElement, attribute: &'static str) -> Result<bool, TextTargetError> {
    let attribute = CFString::from_static_str(attribute);
    let mut settable = 0;
    let result = unsafe { element.is_attribute_settable(&attribute, NonNull::from(&mut settable)) };
    if result == AXError::Success {
        Ok(settable != 0)
    } else if matches!(result, AXError::AttributeUnsupported | AXError::NoValue) {
        Ok(false)
    } else {
        Err(map_ax_error(
            result,
            "check whether the text field is editable",
        ))
    }
}

fn retain_element(element: &AXUIElement) -> CFRetained<AXUIElement> {
    unsafe { CFRetained::retain(element.into()) }
}

fn elements_equal(left: &AXUIElement, right: &AXUIElement) -> bool {
    let left: &CFType = left;
    let right: &CFType = right;
    left == right
}

fn ranges_equal(left: CFRange, right: CFRange) -> bool {
    left.location == right.location && left.length == right.length
}

fn optional_ranges_equal(left: Option<CFRange>, right: Option<CFRange>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => ranges_equal(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn validate_range_shape(range: CFRange) -> Result<(), TextTargetError> {
    let start = usize::try_from(range.location).map_err(|_| invalid_range_error())?;
    let length = usize::try_from(range.length).map_err(|_| invalid_range_error())?;
    start
        .checked_add(length)
        .map(|_| ())
        .ok_or_else(invalid_range_error)
}

fn invalid_range_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::Unsupported,
        "macOS reported an invalid text selection",
    )
}

fn check_deadline(deadline: Instant) -> Result<(), TextTargetError> {
    if Instant::now() <= deadline {
        Ok(())
    } else {
        Err(deadline_error())
    }
}

fn deadline_error() -> TextTargetError {
    TextTargetError::new(
        TextTargetErrorKind::TimedOut,
        "The focused app took too long to answer, so Spick did not type",
    )
}

fn sleep_for_retry(deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        thread::sleep(remaining.min(FOCUSED_CONTEXT_RETRY_DELAY));
    }
}

fn pump_run_loop() {
    if let Some(mode) = unsafe { kCFRunLoopDefaultMode } {
        let _ = CFRunLoop::run_in_mode(Some(mode), 0.001, true);
    }
}

fn map_ax_error(error: AXError, action: &str) -> TextTargetError {
    let kind = match error {
        AXError::APIDisabled => TextTargetErrorKind::AccessibilityMissing,
        AXError::CannotComplete => TextTargetErrorKind::TimedOut,
        AXError::InvalidUIElement => TextTargetErrorKind::TargetGone,
        AXError::AttributeUnsupported
        | AXError::NotificationUnsupported
        | AXError::NoValue
        | AXError::NotImplemented => TextTargetErrorKind::Unsupported,
        _ => TextTargetErrorKind::Platform,
    };
    TextTargetError::new(kind, format!("macOS could not {action}"))
}

fn sanitize_application_name(name: String) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        None
    } else {
        Some(name.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "macos-input-method-prototype")]
    fn input_method_response(status: u8, request_id: u64, lease_id: u64) -> [u8; RESPONSE_LENGTH] {
        let mut response = [0_u8; RESPONSE_LENGTH];
        response[..4].copy_from_slice(b"SPR2");
        response[4] = 2;
        response[5] = status;
        response[8..16].copy_from_slice(&request_id.to_be_bytes());
        response[16..24].copy_from_slice(&lease_id.to_be_bytes());
        response
    }

    fn range(location: isize, length: isize) -> CFRange {
        CFRange { location, length }
    }

    #[test]
    fn invalid_selection_ranges_are_rejected() {
        let error = validate_range_shape(range(-1, 0)).unwrap_err();
        assert_eq!(error.kind, TextTargetErrorKind::Unsupported);
        assert!(validate_range_shape(range(3, 0)).is_ok());
    }

    #[test]
    fn insertion_ranges_use_utf16_offsets_and_replace_the_original_selection() {
        assert_eq!(
            insertion_ranges(range(4, 3), "hello").unwrap(),
            (range(4, 5), range(9, 0))
        );
        assert_eq!(
            insertion_ranges(range(2, 0), "🙂").unwrap(),
            (range(2, 2), range(4, 0))
        );
        assert_eq!(
            insertion_ranges(range(7, 1), "e\u{301}").unwrap(),
            (range(7, 2), range(9, 0))
        );
    }

    #[test]
    fn focused_context_retries_only_transient_snapshot_failures() {
        assert!(focused_context_error_is_transient(
            TextTargetErrorKind::NoFocusedTarget
        ));
        assert!(focused_context_error_is_transient(
            TextTargetErrorKind::TimedOut
        ));
        assert!(focused_context_error_is_transient(
            TextTargetErrorKind::TargetGone
        ));
        assert!(!focused_context_error_is_transient(
            TextTargetErrorKind::AccessibilityMissing
        ));
        assert!(!focused_context_error_is_transient(
            TextTargetErrorKind::SecureField
        ));
    }

    #[test]
    fn focused_sources_must_describe_one_coherent_application() {
        assert_eq!(
            select_coherent_focused_pid(Some(41), Some(41), Some(41)),
            Ok(41)
        );
        assert_eq!(
            select_coherent_focused_pid(Some(41), None, Some(41)),
            Ok(41)
        );
        assert_eq!(select_coherent_focused_pid(None, Some(41), None), Ok(41));
    }

    #[test]
    fn frontmost_application_is_the_last_resort() {
        assert_eq!(select_coherent_focused_pid(None, None, Some(52)), Ok(52));
        assert_eq!(
            select_coherent_focused_pid(None, None, None),
            Err(FocusedApplicationFallbackError::Missing)
        );
    }

    #[test]
    fn mismatched_focused_and_frontmost_pids_are_retried() {
        assert_eq!(
            select_coherent_focused_pid(Some(41), Some(41), Some(52)),
            Err(FocusedApplicationFallbackError::PidMismatch)
        );
        assert_eq!(
            select_coherent_focused_pid(Some(41), Some(52), None),
            Err(FocusedApplicationFallbackError::PidMismatch)
        );
        assert!(focused_context_error_is_transient(
            TextTargetErrorKind::NoFocusedTarget
        ));
    }

    #[test]
    fn supported_roles_are_explicit() {
        assert!(is_editable_text_role(Some(AX_TEXT_FIELD_ROLE)));
        assert!(is_editable_text_role(Some(AX_TEXT_AREA_ROLE)));
        assert!(!is_editable_text_role(Some("AXStaticText")));
        assert!(!is_editable_text_role(None));
    }

    #[test]
    fn editable_capability_accepts_standard_roles_or_a_proven_text_setter() {
        assert!(has_editable_text_capability(
            Some(AX_TEXT_FIELD_ROLE),
            false
        ));
        assert!(has_editable_text_capability(Some(AX_TEXT_AREA_ROLE), false));
        assert!(has_editable_text_capability(Some("AXWebArea"), true));
        assert!(!has_editable_text_capability(Some("AXStaticText"), false));
        assert!(!has_editable_text_capability(None, false));
    }

    #[test]
    fn only_proven_native_selection_setters_use_direct_accessibility() {
        assert_eq!(
            insertion_path(true, true, false),
            InsertionPath::ElementAddressed
        );
        assert_eq!(
            insertion_path(false, true, true),
            InsertionPath::ClipboardPaste
        );
        assert_eq!(
            insertion_path(true, false, true),
            InsertionPath::ClipboardPaste
        );
        assert_eq!(
            insertion_path(true, true, true),
            InsertionPath::ClipboardPaste
        );
    }

    #[test]
    fn selectionless_capture_requires_the_exact_focused_editable_role() {
        assert!(can_capture_without_selection(
            Some(AX_TEXT_AREA_ROLE),
            true,
            true
        ));
        assert!(!can_capture_without_selection(
            Some(AX_TEXT_AREA_ROLE),
            false,
            true
        ));
        assert!(!can_capture_without_selection(
            Some(AX_TEXT_AREA_ROLE),
            true,
            false
        ));
        assert!(!can_capture_without_selection(
            Some(AX_WEB_AREA_ROLE),
            true,
            true
        ));
    }

    #[test]
    #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
    fn clipboard_restoration_requires_unchanged_transaction_ownership() {
        assert!(pasteboard_change_count_is_owned(41, 41));
        assert!(!pasteboard_change_count_is_owned(41, 42));
        assert!(!pasteboard_change_count_is_owned(-1, 0));
        assert!(pasteboard_source_count_is_stable(41, 41));
        assert!(!pasteboard_source_count_is_stable(41, 42));
        assert!(pasteboard_change_count_advanced_once(41, 42));
        assert!(!pasteboard_change_count_advanced_once(41, 43));
        assert!(!pasteboard_change_count_advanced_once(
            isize::MAX,
            isize::MIN
        ));
    }

    #[test]
    #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
    fn clipboard_snapshot_size_is_bounded_before_mutation() {
        assert_eq!(checked_pasteboard_snapshot_size(10, 20), Some(30));
        assert_eq!(
            checked_pasteboard_snapshot_size(PASTEBOARD_SNAPSHOT_MAX_BYTES, 0),
            Some(PASTEBOARD_SNAPSHOT_MAX_BYTES)
        );
        assert_eq!(
            checked_pasteboard_snapshot_size(PASTEBOARD_SNAPSHOT_MAX_BYTES, 1),
            None
        );
        assert_eq!(checked_pasteboard_snapshot_size(usize::MAX, 1), None);
    }

    #[test]
    #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
    fn web_confirmation_budget_allows_async_accessibility_updates() {
        assert!(INSERTION_CONFIRMATION_BUDGET >= Duration::from_millis(750));
        assert!(has_post_dispatch_budget(INSERTION_CONFIRMATION_BUDGET));
        assert!(!has_post_dispatch_budget(
            INSERTION_CONFIRMATION_BUDGET - Duration::from_millis(1)
        ));
    }

    #[test]
    fn compatibility_paste_refuses_terminal_command_separators() {
        assert!(clipboard_paste_transcript_is_safe("write this sentence"));
        assert!(!clipboard_paste_transcript_is_safe("first\nsecond"));
        assert!(!clipboard_paste_transcript_is_safe("first\rsecond"));
    }

    #[test]
    fn unavailable_readback_is_not_treated_as_a_content_mismatch() {
        assert_eq!(
            insertion_readback_state(Some("hello"), "hello"),
            InsertionReadbackState::Confirmed
        );
        assert_eq!(
            insertion_readback_state(None, "hello"),
            InsertionReadbackState::Unavailable
        );
        assert_eq!(
            insertion_readback_state(Some("goodbye"), "hello"),
            InsertionReadbackState::Mismatch
        );
    }

    #[test]
    fn keyboard_confirmation_distinguishes_pending_and_shifted_selections() {
        let original = range(4, 2);
        let expected = range(9, 0);
        assert_eq!(
            keyboard_selection_state(original, original, expected),
            KeyboardSelectionState::Pending
        );
        assert_eq!(
            keyboard_selection_state(expected, original, expected),
            KeyboardSelectionState::Confirmed
        );
        assert_eq!(
            keyboard_selection_state(range(5, 0), original, expected),
            KeyboardSelectionState::Shifted
        );
    }

    #[test]
    fn insertion_confirmation_retries_only_transient_ax_failures() {
        assert!(insertion_confirmation_error_is_retryable(
            TextTargetErrorKind::TimedOut
        ));
        assert!(insertion_confirmation_error_is_retryable(
            TextTargetErrorKind::TargetGone
        ));
        assert!(!insertion_confirmation_error_is_retryable(
            TextTargetErrorKind::Unsupported
        ));
        assert!(!insertion_confirmation_error_is_retryable(
            TextTargetErrorKind::SecureField
        ));
    }

    #[test]
    fn accessibility_timeout_is_clamped_to_the_remaining_budget() {
        assert_eq!(bounded_messaging_timeout_seconds(Duration::ZERO), None);
        assert_eq!(
            bounded_messaging_timeout_seconds(Duration::from_millis(500)),
            Some(APPLICATION_TIMEOUT_SECONDS)
        );
        let short = bounded_messaging_timeout_seconds(Duration::from_millis(40)).unwrap();
        assert!((short - 0.04).abs() < f32::EPSILON * 8.0);
    }

    #[test]
    fn unsupported_observer_notifications_do_not_block_capture() {
        for result in [
            AXError::Success,
            AXError::NotificationAlreadyRegistered,
            AXError::NotificationUnsupported,
            AXError::AttributeUnsupported,
            AXError::NoValue,
            AXError::NotImplemented,
        ] {
            assert!(notification_registration_is_best_effort_success(result));
        }
        assert!(!notification_registration_is_best_effort_success(
            AXError::APIDisabled
        ));
        assert!(!notification_registration_is_best_effort_success(
            AXError::CannotComplete
        ));
    }

    #[test]
    fn application_names_are_bounded_and_control_free() {
        assert_eq!(
            sanitize_application_name(" Notes ".into()).as_deref(),
            Some("Notes")
        );
        assert_eq!(sanitize_application_name("bad\nname".into()), None);
        assert_eq!(sanitize_application_name("x".repeat(81)), None);
    }

    #[test]
    fn every_native_protected_content_marker_is_blocked() {
        assert!(has_secure_marker(Some(AX_SECURE_TEXT_FIELD), None));
        assert!(has_secure_marker(None, Some(true)));
        assert!(!has_secure_marker(None, Some(false)));
        assert!(!has_secure_marker(None, None));
    }

    #[cfg(feature = "macos-input-method-prototype")]
    #[test]
    fn fragmented_input_method_response_respects_one_absolute_deadline() {
        let (client, mut helper) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let request = vec![0x5a; 64];
        let expected_request = request.clone();
        let helper_thread = std::thread::spawn(move || {
            let mut received = vec![0_u8; expected_request.len()];
            helper.read_exact(&mut received).unwrap();
            assert_eq!(received, expected_request);
            for chunk in input_method_response(1, 42, 0).chunks(3) {
                helper.write_all(chunk).unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
        });

        let mut connection = InputMethodConnection {
            stream: client,
            #[cfg(feature = "macos-input-method-compatibility-harness")]
            peer_cd_hash: "0".repeat(40),
        };
        let response = connection
            .exchange(
                &request,
                42,
                true,
                Instant::now() + Duration::from_millis(250),
            )
            .unwrap();
        assert_eq!(response.status, InputMethodResponseStatus::Confirmed);
        helper_thread.join().unwrap();
    }

    #[cfg(feature = "macos-input-method-prototype")]
    #[test]
    fn slow_partial_response_is_indeterminate_after_insert_bytes_cross() {
        let (client, mut helper) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let request = vec![0x5a; 64];
        let request_length = request.len();
        let helper_thread = std::thread::spawn(move || {
            let mut received = vec![0_u8; request_length];
            helper.read_exact(&mut received).unwrap();
            for byte in input_method_response(1, 42, 0) {
                if helper.write_all(&[byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        });

        let started = Instant::now();
        let mut connection = InputMethodConnection {
            stream: client,
            #[cfg(feature = "macos-input-method-compatibility-harness")]
            peer_cd_hash: "0".repeat(40),
        };
        let error = connection
            .exchange(&request, 42, true, started + Duration::from_millis(45))
            .unwrap_err();
        assert_eq!(error.kind, TextTargetErrorKind::Indeterminate);
        assert!(started.elapsed() < Duration::from_millis(200));
        drop(connection);
        helper_thread.join().unwrap();
    }

    #[cfg(feature = "macos-input-method-prototype")]
    #[test]
    fn insert_transport_is_definite_only_before_its_first_byte() {
        let before = map_input_method_io_error(
            std::io::Error::new(std::io::ErrorKind::TimedOut, "late"),
            true,
            0,
        );
        assert_eq!(before.kind, TextTargetErrorKind::TimedOut);

        let after = map_input_method_io_error(
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed"),
            true,
            1,
        );
        assert_eq!(after.kind, TextTargetErrorKind::Indeterminate);
    }

    #[cfg(feature = "macos-input-method-prototype")]
    #[test]
    fn every_insert_status_has_an_explicit_delivery_outcome() {
        use InputMethodResponseStatus as Status;

        assert!(map_insert_response_status(Status::Confirmed).is_ok());
        let cases = [
            (Status::Dispatched, TextTargetErrorKind::Indeterminate),
            (Status::NoActiveClient, TextTargetErrorKind::Unsupported),
            (Status::TargetMismatch, TextTargetErrorKind::FocusChanged),
            (
                Status::SelectionChanged,
                TextTargetErrorKind::SelectionChanged,
            ),
            (Status::Unsupported, TextTargetErrorKind::Unsupported),
            (Status::SecureInput, TextTargetErrorKind::SecureField),
            (Status::InvalidRequest, TextTargetErrorKind::Platform),
            (Status::InternalError, TextTargetErrorKind::Platform),
            (Status::Armed, TextTargetErrorKind::Indeterminate),
            (Status::Disarmed, TextTargetErrorKind::Indeterminate),
            (Status::LeaseExpired, TextTargetErrorKind::FocusChanged),
            (Status::RequestExpired, TextTargetErrorKind::TimedOut),
            (
                Status::LeaseMissingOrConsumed,
                TextTargetErrorKind::Indeterminate,
            ),
        ];
        for (status, expected_kind) in cases {
            assert_eq!(
                map_insert_response_status(status).unwrap_err().kind,
                expected_kind
            );
        }
    }

    #[cfg(feature = "macos-input-method-prototype")]
    #[test]
    fn peer_authentication_build_mode_is_explicit() {
        let allows_unsafe_development = unsafe { SpickPeerAuthenticationAllowsUnsafeDevelopment() };
        assert_eq!(
            allows_unsafe_development,
            cfg!(feature = "macos-input-method-unsafe-dev-peers")
        );

        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        let invalid_socket_result = unsafe {
            SpickVerifyPeerSocket(
                -1,
                DESKTOP_SIGNING_IDENTIFIER.as_ptr().cast(),
                INPUT_METHOD_SIGNING_IDENTIFIER.as_ptr().cast(),
            )
        };
        #[cfg(feature = "macos-input-method-compatibility-harness")]
        let invalid_socket_result = unsafe {
            let mut cd_hash = [0_i8; 65];
            SpickVerifyPeerSocketWithCDHash(
                -1,
                DESKTOP_SIGNING_IDENTIFIER.as_ptr().cast(),
                INPUT_METHOD_SIGNING_IDENTIFIER.as_ptr().cast(),
                cd_hash.as_mut_ptr(),
                cd_hash.len(),
            )
        };
        assert_eq!(invalid_socket_result, 10);
    }
}
