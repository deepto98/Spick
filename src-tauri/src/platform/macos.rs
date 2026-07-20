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
#[cfg(all(
    debug_assertions,
    not(feature = "macos-input-method-compatibility-harness")
))]
use core_graphics::{
    event::CGEvent,
    event_source::{CGEventSource, CGEventSourceStateID},
};
use libc::pid_t;
#[cfg(feature = "macos-input-method-prototype")]
use objc2_app_kit::NSRunningApplication;
use objc2_app_kit::NSWorkspace;
use objc2_application_services::{
    kAXTrustedCheckOptionPrompt, AXError, AXIsProcessTrusted, AXIsProcessTrustedWithOptions,
    AXObserver, AXUIElement, AXValue, AXValueType,
};
use objc2_core_foundation::{
    kCFBooleanTrue, kCFRunLoopDefaultMode, CFArray, CFBoolean, CFDictionary, CFRange, CFRetained,
    CFRunLoop, CFRunLoopSource, CFString, CFType,
};

const AX_FOCUSED_APPLICATION: &str = "AXFocusedApplication";
const AX_FOCUSED_UI_ELEMENT: &str = "AXFocusedUIElement";
const AX_PARENT: &str = "AXParent";
const AX_TITLE: &str = "AXTitle";
const AX_ROLE: &str = "AXRole";
const AX_SUBROLE: &str = "AXSubrole";
const AX_CONTAINS_PROTECTED_CONTENT: &str = "AXContainsProtectedContent";
const AX_ENABLED: &str = "AXEnabled";
const AX_SELECTED_TEXT: &str = "AXSelectedText";
const AX_SELECTED_TEXT_RANGE: &str = "AXSelectedTextRange";
const AX_SELECTED_TEXT_RANGES: &str = "AXSelectedTextRanges";
#[cfg(all(
    debug_assertions,
    not(feature = "macos-input-method-compatibility-harness")
))]
const AX_STRING_FOR_RANGE: &str = "AXStringForRange";
const AX_SECURE_TEXT_FIELD: &str = "AXSecureTextField";
const AX_TEXT_FIELD_ROLE: &str = "AXTextField";
const AX_TEXT_AREA_ROLE: &str = "AXTextArea";
const AX_APPLICATION_DEACTIVATED: &str = "AXApplicationDeactivated";
const AX_FOCUSED_UI_ELEMENT_CHANGED: &str = "AXFocusedUIElementChanged";
const AX_SELECTED_TEXT_CHANGED: &str = "AXSelectedTextChanged";
const AX_VALUE_CHANGED: &str = "AXValueChanged";
const AX_UI_ELEMENT_DESTROYED: &str = "AXUIElementDestroyed";
const AX_MANUAL_ACCESSIBILITY: &str = "AXManualAccessibility";

const OWNER_RESPONSE_TIMEOUT: Duration = Duration::from_millis(1_800);
const CAPTURE_DEADLINE: Duration = Duration::from_millis(700);
const COMMIT_DEADLINE: Duration = Duration::from_millis(950);
const APPLICATION_TIMEOUT_SECONDS: f32 = 0.25;
const MAX_PARENT_DEPTH: usize = 12;
const FOCUSED_CONTEXT_RETRY_BUDGET: Duration = Duration::from_millis(600);
const FOCUSED_CONTEXT_RETRY_DELAY: Duration = Duration::from_millis(12);
const RUN_LOOP_POLL: Duration = Duration::from_millis(4);
#[cfg(all(
    debug_assertions,
    not(feature = "macos-input-method-compatibility-harness")
))]
const KEYBOARD_EVENT_CONFIRMATION_BUDGET: Duration = Duration::from_millis(160);
#[cfg(all(
    debug_assertions,
    not(feature = "macos-input-method-compatibility-harness")
))]
const UNMAPPED_VIRTUAL_KEY: u16 = u16::MAX;
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
            match command {
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
            }
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
        ensure_not_secure(&application)
            .map_err(|error| error.with_compatibility_target_pid(application_pid))?;
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        ensure_not_secure(&application)?;

        #[cfg(feature = "macos-input-method-compatibility-harness")]
        let editable = resolve_editable_target(&focus_anchor)
            .map_err(|error| error.with_compatibility_target_pid(application_pid))?;
        #[cfg(not(feature = "macos-input-method-compatibility-harness"))]
        let editable = resolve_editable_target(&focus_anchor)?;
        #[cfg(feature = "macos-input-method-compatibility-harness")]
        if let Some(expected) = expected_selection {
            let has_selection = editable.selection.length > 0;
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
        let target_app = read_optional_string(&application, AX_TITLE)
            .ok()
            .flatten()
            .and_then(sanitize_application_name);

        self.next_token = self.next_token.wrapping_add(1).max(1);
        let token = self.next_token;
        #[cfg(feature = "macos-input-method-prototype")]
        let input_method_lease = match bundle_identifier.as_deref() {
            Some(identifier) => {
                try_arm_input_method(token, editable.selection, identifier, deadline)?
            }
            None => None,
        };
        pump_run_loop();
        if observer.was_invalidated() {
            return Err(TextTargetError::new(
                TextTargetErrorKind::FocusChanged,
                "The field changed before Spick could start listening",
            ));
        }
        check_deadline(deadline)?;
        self.targets.insert(
            token,
            CapturedTarget {
                application,
                focus_anchor,
                element: editable.element,
                pid: application_pid,
                #[cfg(feature = "macos-input-method-prototype")]
                bundle_identifier,
                #[cfg(feature = "macos-input-method-prototype")]
                input_method_lease,
                selection: editable.selection,
                #[cfg(all(
                    debug_assertions,
                    not(feature = "macos-input-method-compatibility-harness")
                ))]
                selected_text_settable: editable.selected_text_settable,
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
        revalidate_captured_target(&target, deadline)?;

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

        #[cfg(all(
            debug_assertions,
            not(feature = "macos-input-method-compatibility-harness")
        ))]
        {
            #[cfg(feature = "macos-input-method-prototype")]
            if target.input_method_lease.is_some() {
                commit_through_input_method(&mut target, token, transcript, deadline)?;
                return Ok(TextInsertionReceipt {
                    target_app: None,
                    caret_repositioned: true,
                });
            }

            if target.selected_text_settable {
                commit_through_accessibility(&target, transcript, deadline)
            } else {
                commit_through_unicode_keyboard_event(&target, transcript, deadline)
            }
        }

        #[cfg(all(
            not(debug_assertions),
            feature = "macos-input-method-prototype",
            not(feature = "macos-input-method-compatibility-harness")
        ))]
        {
            commit_through_input_method(&mut target, token, transcript, deadline)?;
            Ok(TextInsertionReceipt {
                target_app: None,
                caret_repositioned: true,
            })
        }

        #[cfg(all(not(debug_assertions), not(feature = "macos-input-method-prototype")))]
        {
            let _ = (transcript, &mut target);
            Err(TextTargetError::new(
                TextTargetErrorKind::Unsupported,
                "Automatic typing is not enabled in this build; the transcript is ready to copy",
            ))
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
    selection: CFRange,
    #[cfg(all(
        debug_assertions,
        not(feature = "macos-input-method-compatibility-harness")
    ))]
    selected_text_settable: bool,
    observer: ObserverLease,
}

fn revalidate_captured_target(
    target: &CapturedTarget,
    deadline: Instant,
) -> Result<(), TextTargetError> {
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

    let focused = read_focused_context(deadline)?;
    let current_application = focused.application;
    if !elements_equal(&current_application, &target.application) || focused.pid != target.pid {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The active app changed, so Spick did not type the transcript",
        ));
    }
    set_element_timeout(&current_application)?;
    ensure_not_secure(&current_application)?;

    let current_anchor = focused.focus_anchor;
    if !elements_equal(&current_anchor, &target.focus_anchor)
        || read_pid(&current_anchor)? != target.pid
    {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The cursor moved to another field, so Spick did not type the transcript",
        ));
    }

    let editable = resolve_editable_target(&current_anchor)?;
    if !elements_equal(&editable.element, &target.element) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::FocusChanged,
            "The editable field changed, so Spick did not type the transcript",
        ));
    }
    if !ranges_equal(editable.selection, target.selection) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::SelectionChanged,
            "The selection changed, so Spick did not type over it",
        ));
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
    Ok(())
}

#[cfg(all(
    debug_assertions,
    not(feature = "macos-input-method-compatibility-harness")
))]
fn commit_through_accessibility(
    target: &CapturedTarget,
    transcript: &str,
    deadline: Instant,
) -> Result<TextInsertionReceipt, TextTargetError> {
    if !target.selected_text_settable {
        return Err(TextTargetError::new(
            TextTargetErrorKind::Unsupported,
            "This field does not accept direct text insertion; the transcript is ready to copy",
        ));
    }
    check_deadline(deadline)?;
    let (inserted_range, expected_caret) = insertion_ranges(target.selection, transcript)?;
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

    // AX setters are synchronous from the caller's perspective, but give the
    // target's Accessibility bridge one run-loop turn before asking it to read
    // the exact UTF-16 range back.
    pump_run_loop();
    let readback = read_string_for_range(&target.element, inserted_range).map_err(|_| {
        TextTargetError::new(
            TextTargetErrorKind::Indeterminate,
            "The field accepted a write but could not confirm its contents; check it before copying",
        )
    })?;
    if readback.as_deref() != Some(transcript) {
        return Err(TextTargetError::new(
            TextTargetErrorKind::Indeterminate,
            "The field accepted a write but its contents did not match; check it before copying",
        ));
    }
    let caret_repositioned = read_optional_range(&target.element, AX_SELECTED_TEXT_RANGE)
        .map(|selection| selection.is_some_and(|selection| ranges_equal(selection, expected_caret)))
        .unwrap_or(false);

    Ok(TextInsertionReceipt {
        target_app: None,
        caret_repositioned,
    })
}

/// Development fallback for controls (including Notes versions) that expose a
/// readable selection but no `AXSelectedText` setter. The event is posted only
/// to the captured PID after exact app, element, and selection revalidation.
/// An unmapped virtual key is deliberate: a client that ignores the Unicode
/// payload should insert nothing rather than a layout-dependent character.
#[cfg(all(
    debug_assertions,
    not(feature = "macos-input-method-compatibility-harness")
))]
fn commit_through_unicode_keyboard_event(
    target: &CapturedTarget,
    transcript: &str,
    deadline: Instant,
) -> Result<TextInsertionReceipt, TextTargetError> {
    check_deadline(deadline)?;
    let (inserted_range, expected_caret) = insertion_ranges(target.selection, transcript)?;
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| {
        TextTargetError::new(
            TextTargetErrorKind::Platform,
            "macOS could not create a private keyboard-event source",
        )
    })?;
    let key_down = CGEvent::new_keyboard_event(source.clone(), UNMAPPED_VIRTUAL_KEY, true)
        .map_err(|_| {
            TextTargetError::new(
                TextTargetErrorKind::Platform,
                "macOS could not create the text insertion event",
            )
        })?;
    let key_up =
        CGEvent::new_keyboard_event(source, UNMAPPED_VIRTUAL_KEY, false).map_err(|_| {
            TextTargetError::new(
                TextTargetErrorKind::Platform,
                "macOS could not create the text insertion completion event",
            )
        })?;
    key_down.set_string(transcript);
    key_down.post_to_pid(target.pid);
    key_up.post_to_pid(target.pid);

    // CGEventPostToPid is asynchronous and has no delivery result. Treat the
    // readable selection that qualified this target as a bounded acknowledgement.
    // Exact range readback, where implemented, adds a content check.
    let confirmation_deadline = deadline.min(Instant::now() + KEYBOARD_EVENT_CONFIRMATION_BUDGET);
    loop {
        pump_run_loop();
        match read_optional_range(&target.element, AX_SELECTED_TEXT_RANGE) {
            Ok(Some(selection)) if ranges_equal(selection, expected_caret) => {
                if let Some(readback) = read_string_for_range(&target.element, inserted_range)? {
                    if readback != transcript {
                        return Err(TextTargetError::new(
                            TextTargetErrorKind::Indeterminate,
                            "The field moved its caret but did not confirm the inserted text; check it before copying",
                        ));
                    }
                }
                return Ok(TextInsertionReceipt {
                    target_app: None,
                    caret_repositioned: true,
                });
            }
            Ok(_) => {}
            Err(error) if focused_context_error_is_transient(error.kind) => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= confirmation_deadline {
            return Err(TextTargetError::new(
                TextTargetErrorKind::Indeterminate,
                "macOS sent the text event but the field did not confirm its new caret; check it before copying",
            ));
        }
        thread::sleep(FOCUSED_CONTEXT_RETRY_DELAY);
    }
}

#[cfg(any(
    test,
    all(
        debug_assertions,
        not(feature = "macos-input-method-compatibility-harness")
    )
))]
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

#[cfg(all(
    debug_assertions,
    not(feature = "macos-input-method-compatibility-harness")
))]
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
    let (selection_location, selection_length) = selection_parts(target.selection)?;
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
    selection: CFRange,
    #[cfg(all(
        debug_assertions,
        not(feature = "macos-input-method-compatibility-harness")
    ))]
    selected_text_settable: bool,
}

struct FocusedContext {
    application: CFRetained<AXUIElement>,
    focus_anchor: CFRetained<AXUIElement>,
    pid: pid_t,
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

/// Read one coherent app/field snapshot. Some Chromium and Electron apps
/// transiently omit AXFocusedApplication while still exposing the system-wide
/// AXFocusedUIElement. Deriving the application from that element's PID avoids
/// treating the gap as an unsupported editor, while the PID equality check
/// prevents a mixed snapshot during an app switch.
fn read_focused_context(deadline: Instant) -> Result<FocusedContext, TextTargetError> {
    let retry_deadline = deadline.min(Instant::now() + FOCUSED_CONTEXT_RETRY_BUDGET);

    loop {
        check_deadline(deadline)?;
        match read_focused_context_once() {
            Ok(context) => return Ok(context),
            Err(error) if focused_context_error_is_transient(error.kind) => {
                if Instant::now() >= retry_deadline {
                    return Err(error);
                }
                thread::sleep(FOCUSED_CONTEXT_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_focused_context_once() -> Result<FocusedContext, TextTargetError> {
    let system = unsafe { AXUIElement::new_system_wide() };
    let mut system_anchor = None;
    let application = match read_optional_focus_element(&system, AX_FOCUSED_APPLICATION)? {
        Some(application) => application,
        None => match frontmost_application_pid() {
            Some(pid) => unsafe { AXUIElement::new_application(pid) },
            None => {
                system_anchor = read_optional_focus_element(&system, AX_FOCUSED_UI_ELEMENT)?;
                let anchor = system_anchor.as_ref().ok_or_else(|| {
                    TextTargetError::new(
                        TextTargetErrorKind::NoFocusedTarget,
                        "macOS did not report a focused application or text field",
                    )
                })?;
                let pid = read_pid(anchor)?;
                unsafe { AXUIElement::new_application(pid) }
            }
        },
    };
    set_element_timeout(&application)?;
    let pid = read_pid(&application)?;
    let mut application_anchor = read_optional_focus_element(&application, AX_FOCUSED_UI_ELEMENT)?;
    if application_anchor.is_none() {
        enable_manual_accessibility_best_effort(&application);
        application_anchor = read_optional_focus_element(&application, AX_FOCUSED_UI_ELEMENT)?;
    }
    let focus_anchor = match application_anchor {
        Some(anchor) => anchor,
        None => {
            let fallback = match system_anchor {
                Some(anchor) => Some(anchor),
                None => read_optional_focus_element(&system, AX_FOCUSED_UI_ELEMENT)?,
            };
            fallback.ok_or_else(|| {
                TextTargetError::new(
                    TextTargetErrorKind::NoFocusedTarget,
                    "macOS did not report a focused text field",
                )
            })?
        }
    };
    if read_pid(&focus_anchor)? != pid {
        return Err(TextTargetError::new(
            TextTargetErrorKind::NoFocusedTarget,
            "macOS reported an inconsistent focused field",
        ));
    }

    Ok(FocusedContext {
        application,
        focus_anchor,
        pid,
    })
}

fn read_optional_focus_element(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<CFRetained<AXUIElement>>, TextTargetError> {
    match read_optional_element(element, attribute) {
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

fn resolve_editable_target(focus_anchor: &AXUIElement) -> Result<EditableTarget, TextTargetError> {
    let mut current = retain_element(focus_anchor);
    for depth in 0..MAX_PARENT_DEPTH {
        set_element_timeout(&current)?;
        ensure_not_secure(&current)?;
        if read_optional_bool(&current, AX_ENABLED)?.is_some_and(|enabled| !enabled) {
            return Err(TextTargetError::new(
                TextTargetErrorKind::NotEditable,
                "The focused field is disabled",
            ));
        }

        if let Some(candidate) = editable_snapshot(&current)? {
            return Ok(candidate);
        }
        if depth + 1 == MAX_PARENT_DEPTH {
            break;
        }
        let Some(parent) = read_optional_element(&current, AX_PARENT)? else {
            break;
        };
        current = parent;
    }

    Err(TextTargetError::new(
        TextTargetErrorKind::NotEditable,
        "Click an editable text field before holding the shortcut",
    ))
}

fn editable_snapshot(element: &AXUIElement) -> Result<Option<EditableTarget>, TextTargetError> {
    let role = read_optional_string(element, AX_ROLE)?;
    let selected_text_settable = is_settable(element, AX_SELECTED_TEXT)?;
    if !has_editable_text_capability(role.as_deref(), selected_text_settable) {
        return Ok(None);
    }
    let Some(selection) = read_optional_range(element, AX_SELECTED_TEXT_RANGE)? else {
        return Ok(None);
    };
    validate_range_shape(selection)?;

    if let Some(ranges) = read_optional_array(element, AX_SELECTED_TEXT_RANGES)? {
        if ranges.len() > 1 {
            return Err(TextTargetError::new(
                TextTargetErrorKind::Unsupported,
                "Spick does not type over multiple selections yet",
            ));
        }
    }

    Ok(Some(EditableTarget {
        element: retain_element(element),
        selection,
        #[cfg(all(
            debug_assertions,
            not(feature = "macos-input-method-compatibility-harness")
        ))]
        selected_text_settable,
    }))
}

fn is_editable_text_role(role: Option<&str>) -> bool {
    matches!(role, Some(AX_TEXT_FIELD_ROLE | AX_TEXT_AREA_ROLE))
}

fn has_editable_text_capability(role: Option<&str>, selected_text_settable: bool) -> bool {
    is_editable_text_role(role) || selected_text_settable
}

fn ensure_not_secure(element: &AXUIElement) -> Result<(), TextTargetError> {
    let subrole = read_optional_string(element, AX_SUBROLE)?;
    let contains_protected_content = read_optional_bool(element, AX_CONTAINS_PROTECTED_CONTENT)?;
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

fn set_element_timeout(element: &AXUIElement) -> Result<(), TextTargetError> {
    let result = unsafe { element.set_messaging_timeout(APPLICATION_TIMEOUT_SECONDS) };
    if result == AXError::Success {
        Ok(())
    } else {
        Err(map_ax_error(result, "set an Accessibility timeout"))
    }
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
        Err(TextTargetError::new(
            TextTargetErrorKind::TimedOut,
            "The focused app took too long to answer, so Spick did not type",
        ))
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
