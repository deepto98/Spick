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

use super::{
    AccessibilityPermissionState, AccessibilityPermissionStatus, CapturedTextTarget,
    TextInsertionReceipt, TextTargetError, TextTargetErrorKind, TextTargetToken,
};
use libc::pid_t;
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
const AX_SELECTED_TEXT_RANGE: &str = "AXSelectedTextRange";
const AX_SELECTED_TEXT_RANGES: &str = "AXSelectedTextRanges";
const AX_SECURE_TEXT_FIELD: &str = "AXSecureTextField";
const AX_TEXT_FIELD_ROLE: &str = "AXTextField";
const AX_TEXT_AREA_ROLE: &str = "AXTextArea";
const AX_APPLICATION_DEACTIVATED: &str = "AXApplicationDeactivated";
const AX_FOCUSED_UI_ELEMENT_CHANGED: &str = "AXFocusedUIElementChanged";
const AX_SELECTED_TEXT_CHANGED: &str = "AXSelectedTextChanged";
const AX_VALUE_CHANGED: &str = "AXValueChanged";
const AX_UI_ELEMENT_DESTROYED: &str = "AXUIElementDestroyed";

const OWNER_RESPONSE_TIMEOUT: Duration = Duration::from_millis(1_800);
const CAPTURE_DEADLINE: Duration = Duration::from_millis(700);
const COMMIT_DEADLINE: Duration = Duration::from_millis(950);
const APPLICATION_TIMEOUT_SECONDS: f32 = 0.25;
const MAX_PARENT_DEPTH: usize = 5;
const RUN_LOOP_POLL: Duration = Duration::from_millis(4);

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
                Command::Capture { deadline, reply } => {
                    let result = self.capture(deadline);
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

    fn capture(&mut self, deadline: Instant) -> Result<CapturedTextTarget, TextTargetError> {
        check_deadline(deadline)?;
        if !is_trusted() {
            return Err(TextTargetError::new(
                TextTargetErrorKind::AccessibilityMissing,
                "Turn on Accessibility for Spick before using the shortcut",
            ));
        }

        let system = unsafe { AXUIElement::new_system_wide() };
        let application = read_element(&system, AX_FOCUSED_APPLICATION, "focused application")?;
        set_element_timeout(&application)?;
        ensure_not_secure(&application)?;
        let application_pid = read_pid(&application)?;
        if application_pid == std::process::id() as pid_t {
            return Err(TextTargetError::new(
                TextTargetErrorKind::OwnApplication,
                "Click a text field in another app before holding the shortcut",
            ));
        }

        let focus_anchor = read_element(&application, AX_FOCUSED_UI_ELEMENT, "focused text field")?;
        let anchor_pid = read_pid(&focus_anchor)?;
        if anchor_pid != application_pid {
            return Err(TextTargetError::new(
                TextTargetErrorKind::NoFocusedTarget,
                "macOS reported an inconsistent focused field",
            ));
        }

        let editable = resolve_editable_target(&focus_anchor)?;
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
        self.targets.insert(
            token,
            CapturedTarget {
                application,
                focus_anchor,
                element: editable.element,
                pid: application_pid,
                selection: editable.selection,
                observer,
            },
        );

        Ok(CapturedTextTarget {
            token: TextTargetToken::from_platform(token),
            target_app,
        })
    }

    fn commit(
        &mut self,
        token: u64,
        transcript: &str,
        deadline: Instant,
    ) -> Result<TextInsertionReceipt, TextTargetError> {
        let target = self.targets.remove(&token).ok_or_else(|| {
            TextTargetError::new(
                TextTargetErrorKind::TargetGone,
                "the captured text field is no longer available",
            )
        })?;
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

        let system = unsafe { AXUIElement::new_system_wide() };
        let current_application =
            read_element(&system, AX_FOCUSED_APPLICATION, "focused application")?;
        if !elements_equal(&current_application, &target.application)
            || read_pid(&current_application)? != target.pid
        {
            return Err(TextTargetError::new(
                TextTargetErrorKind::FocusChanged,
                "The active app changed, so Spick did not type the transcript",
            ));
        }
        set_element_timeout(&current_application)?;
        ensure_not_secure(&current_application)?;

        let current_anchor = read_element(
            &current_application,
            AX_FOCUSED_UI_ELEMENT,
            "focused text field",
        )?;
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

        let _ = transcript;
        Err(TextTargetError::new(
            TextTargetErrorKind::Unsupported,
            "Automatic paste is still being hardened; the transcript is ready to copy from Spick",
        ))
    }
}

struct CapturedTarget {
    application: CFRetained<AXUIElement>,
    focus_anchor: CFRetained<AXUIElement>,
    element: CFRetained<AXUIElement>,
    pid: pid_t,
    selection: CFRange,
    observer: ObserverLease,
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
    if result == AXError::Success {
        Ok(())
    } else {
        Err(map_ax_error(result, "watch the focused text field"))
    }
}

struct EditableTarget {
    element: CFRetained<AXUIElement>,
    selection: CFRange,
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
    if !is_editable_text_role(role.as_deref()) {
        return Ok(None);
    }
    if !is_settable(element, AX_SELECTED_TEXT_RANGE)? {
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
    }))
}

fn is_editable_text_role(role: Option<&str>) -> bool {
    matches!(role, Some(AX_TEXT_FIELD_ROLE | AX_TEXT_AREA_ROLE))
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

fn read_element(
    element: &AXUIElement,
    attribute: &'static str,
    label: &'static str,
) -> Result<CFRetained<AXUIElement>, TextTargetError> {
    read_optional_element(element, attribute)?.ok_or_else(|| {
        TextTargetError::new(
            TextTargetErrorKind::NoFocusedTarget,
            format!("macOS did not report a {label}"),
        )
    })
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
    fn supported_roles_are_explicit() {
        assert!(is_editable_text_role(Some(AX_TEXT_FIELD_ROLE)));
        assert!(is_editable_text_role(Some(AX_TEXT_AREA_ROLE)));
        assert!(!is_editable_text_role(Some("AXStaticText")));
        assert!(!is_editable_text_role(None));
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
}
