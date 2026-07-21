use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "macos")]
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, TryRecvError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[cfg(target_os = "macos")]
use crate::domain::SessionState;
use crate::{commands, domain::SessionTrigger, state::AppState};

#[cfg(any(target_os = "macos", test))]
mod gesture;
#[cfg(target_os = "macos")]
mod macos_option;

#[cfg(target_os = "macos")]
use gesture::{GestureAction, GestureEvent, GestureInput, GestureMachine};

pub const OPTION_SHORTCUT: &str = "Option";
pub const OPTION_FALLBACK_SHORTCUT: &str = "CommandOrControl+Shift+Space";

#[cfg(target_os = "macos")]
const OPTION_EVENT_CAPACITY: usize = 64;
#[cfg(target_os = "macos")]
const IDLE_GESTURE_POLL: Duration = Duration::from_millis(250);
#[cfg(target_os = "macos")]
const OPTION_WATCHDOG_POLL: Duration = Duration::from_millis(250);
#[cfg(target_os = "macos")]
const OPTION_RECOVERY_GRACE: Duration = Duration::from_millis(750);
#[cfg(target_os = "macos")]
const OPTION_RETRY_INITIAL: Duration = Duration::from_millis(750);
#[cfg(target_os = "macos")]
const OPTION_RETRY_MAX: Duration = Duration::from_secs(30);

#[cfg(target_os = "macos")]
static OPTION_WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);

static SHORTCUT_CONTROLLER: OnceLock<Mutex<ShortcutController>> = OnceLock::new();

#[cfg(target_os = "macos")]
#[derive(Default)]
struct ChordQueueFlags {
    keyboard: AtomicBool,
    pointer: AtomicBool,
    hud_pointer: AtomicBool,
}

#[cfg(target_os = "macos")]
impl ChordQueueFlags {
    fn claim(&self, input: GestureInput) -> bool {
        let flag = match input {
            GestureInput::KeyboardChord => &self.keyboard,
            GestureInput::PointerChord => &self.pointer,
            GestureInput::HudPointerChord => &self.hud_pointer,
            _ => return true,
        };
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn release(&self, input: GestureInput) {
        match input {
            GestureInput::KeyboardChord => self.keyboard.store(false, Ordering::Release),
            GestureInput::PointerChord => self.pointer.store(false, Ordering::Release),
            GestureInput::HudPointerChord => self.hud_pointer.store(false, Ordering::Release),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InputMonitoringAccess {
    Granted,
    Denied,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unknown,
}

impl InputMonitoringAccess {
    fn is_granted(self) -> bool {
        self == Self::Granted
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutStatus {
    pub option_selected: bool,
    pub option_listener_active: bool,
    pub input_monitoring_granted: bool,
    pub input_monitoring_access: InputMonitoringAccess,
    pub fallback_shortcut: Option<&'static str>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct OptionRetryState {
    failures: u32,
    next_attempt_at: Option<Instant>,
    observed_access: Option<InputMonitoringAccess>,
}

#[cfg(target_os = "macos")]
impl OptionRetryState {
    fn should_attempt(&mut self, now: Instant, access: InputMonitoringAccess) -> bool {
        if self.observed_access != Some(access) {
            self.failures = 0;
            self.next_attempt_at = None;
            self.observed_access = Some(access);
        }
        self.next_attempt_at.is_none_or(|retry_at| now >= retry_at)
    }

    fn record_failure(&mut self, now: Instant, access: InputMonitoringAccess) {
        self.observed_access = Some(access);
        self.next_attempt_at = Some(now + option_retry_delay(self.failures));
        self.failures = self.failures.saturating_add(1);
    }

    fn record_success(&mut self, access: InputMonitoringAccess) {
        self.failures = 0;
        self.next_attempt_at = None;
        self.observed_access = Some(access);
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(target_os = "macos")]
fn option_retry_delay(failures: u32) -> Duration {
    let multiplier = 1_u32 << failures.min(16);
    OPTION_RETRY_INITIAL
        .saturating_mul(multiplier)
        .min(OPTION_RETRY_MAX)
}

#[derive(Default)]
struct ShortcutController {
    accelerator: Option<String>,
    option_selected: bool,
    fallback_registered: bool,
    #[cfg(target_os = "macos")]
    option_runtime: Option<OptionRuntime>,
    #[cfg(target_os = "macos")]
    option_retry: OptionRetryState,
}

impl ShortcutController {
    fn activate<R: Runtime>(&mut self, app: &AppHandle<R>, shortcut: &str) -> Result<(), String> {
        if shortcut == OPTION_SHORTCUT {
            #[cfg(target_os = "macos")]
            return self.activate_option(app);

            #[cfg(not(target_os = "macos"))]
            return Err("the Option-only shortcut is available on macOS".into());
        }

        self.activate_accelerator(app, shortcut)
    }

    fn activate_accelerator<R: Runtime>(
        &mut self,
        app: &AppHandle<R>,
        shortcut: &str,
    ) -> Result<(), String> {
        let shortcuts = app.global_shortcut();
        let was_registered = shortcuts.is_registered(shortcut);
        if !was_registered {
            shortcuts
                .register(shortcut)
                .map_err(|error| format!("could not register push-to-talk shortcut: {error}"))?;
        }

        let previous_accelerator = self.accelerator.clone();
        let previous_fallback = self.fallback_registered;
        let cleanup_result = (|| {
            if let Some(previous) = previous_accelerator.as_deref() {
                if previous != shortcut {
                    unregister_accelerator(app, previous)?;
                }
            }
            if previous_fallback && shortcut != OPTION_FALLBACK_SHORTCUT {
                unregister_accelerator(app, OPTION_FALLBACK_SHORTCUT)?;
            }
            Ok::<(), String>(())
        })();

        if let Err(error) = cleanup_result {
            if !was_registered {
                let _ = unregister_accelerator(app, shortcut);
            }
            return Err(error);
        }

        #[cfg(target_os = "macos")]
        if let Some(mut runtime) = self.option_runtime.take() {
            runtime.stop();
        }
        self.accelerator = Some(shortcut.to_owned());
        self.option_selected = false;
        self.fallback_registered = false;
        #[cfg(target_os = "macos")]
        self.option_retry.clear();
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn activate_option<R: Runtime>(&mut self, app: &AppHandle<R>) -> Result<(), String> {
        // A recovering worker is not a usable shortcut. Try a fresh listener
        // (or install the fallback if permission was revoked) unless the
        // current listener is actually receiving events.
        let input_monitoring_access = macos_option::input_monitoring_access();
        if self.option_selected && self.option_listener_active() {
            self.option_retry.record_success(input_monitoring_access);
            return Ok(());
        }

        let listener = if input_monitoring_access.is_granted() {
            OptionRuntime::start(app.clone())
        } else {
            Err("Input Monitoring is not allowed for Spick yet".into())
        };
        match listener {
            Ok(mut new_runtime) => {
                let cleanup_result = (|| {
                    if let Some(previous) = self.accelerator.as_deref() {
                        unregister_accelerator(app, previous)?;
                    }
                    if self.fallback_registered {
                        unregister_accelerator(app, OPTION_FALLBACK_SHORTCUT)?;
                    }
                    Ok::<(), String>(())
                })();
                if let Err(error) = cleanup_result {
                    new_runtime.stop();
                    self.option_retry
                        .record_failure(Instant::now(), input_monitoring_access);
                    return Err(error);
                }

                if let Some(mut previous) = self.option_runtime.take() {
                    previous.stop();
                }
                self.option_runtime = Some(new_runtime);
                self.accelerator = None;
                self.option_selected = true;
                self.fallback_registered = false;
                self.option_retry.record_success(input_monitoring_access);
                Ok(())
            }
            Err(listener_error) => {
                let fallback_already_active = self.fallback_registered;
                self.option_retry
                    .record_failure(Instant::now(), input_monitoring_access);
                let shortcuts = app.global_shortcut();
                let fallback_was_registered = shortcuts.is_registered(OPTION_FALLBACK_SHORTCUT);
                if !fallback_was_registered {
                    shortcuts
                        .register(OPTION_FALLBACK_SHORTCUT)
                        .map_err(|fallback_error| {
                            format!(
                                "{listener_error}; the temporary fallback shortcut could not be registered: {fallback_error}"
                            )
                        })?;
                }

                if let Some(previous) = self.accelerator.as_deref() {
                    if previous != OPTION_FALLBACK_SHORTCUT {
                        if let Err(error) = unregister_accelerator(app, previous) {
                            if !fallback_was_registered {
                                let _ = unregister_accelerator(app, OPTION_FALLBACK_SHORTCUT);
                            }
                            return Err(error);
                        }
                    }
                }

                if let Some(mut previous) = self.option_runtime.take() {
                    previous.stop();
                }
                if !fallback_already_active {
                    eprintln!(
                        "Option-key dictation is using its fallback while macOS rejects the passive listener: {listener_error}"
                    );
                }
                self.accelerator = None;
                self.option_selected = true;
                self.fallback_registered = true;
                Ok(())
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn option_listener_active(&self) -> bool {
        self.option_runtime
            .as_ref()
            .is_some_and(OptionRuntime::is_active)
    }

    fn status(&self) -> ShortcutStatus {
        #[cfg(target_os = "macos")]
        let input_monitoring_access = macos_option::input_monitoring_access();
        #[cfg(not(target_os = "macos"))]
        let input_monitoring_access = InputMonitoringAccess::Unknown;
        ShortcutStatus {
            option_selected: self.option_selected,
            #[cfg(target_os = "macos")]
            option_listener_active: self.option_selected && self.option_listener_active(),
            #[cfg(not(target_os = "macos"))]
            option_listener_active: false,
            input_monitoring_granted: input_monitoring_access.is_granted(),
            input_monitoring_access,
            fallback_shortcut: (self.option_selected && self.fallback_registered)
                .then_some(OPTION_FALLBACK_SHORTCUT),
        }
    }
}

#[cfg(target_os = "macos")]
struct OptionRuntime {
    listener: macos_option::ListenerHandle,
    sender: mpsc::SyncSender<GestureEvent>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(target_os = "macos")]
impl OptionRuntime {
    fn start<R: Runtime>(app: AppHandle<R>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(OPTION_EVENT_CAPACITY);
        let overflowed = Arc::new(AtomicBool::new(false));
        let chord_queue = Arc::new(ChordQueueFlags::default());
        let listener = macos_option::start_listener(
            sender.clone(),
            Arc::clone(&overflowed),
            Arc::clone(&chord_queue),
        )?;
        let worker = match thread::Builder::new()
            .name("spick-option-gesture".into())
            .spawn(move || run_gesture_worker(app, receiver, overflowed, chord_queue))
        {
            Ok(worker) => worker,
            Err(error) => {
                let mut listener = listener;
                listener.stop();
                return Err(format!(
                    "could not start the Option gesture worker: {error}"
                ));
            }
        };

        Ok(Self {
            listener,
            sender,
            worker: Some(worker),
        })
    }

    fn is_active(&self) -> bool {
        self.listener.is_active()
    }

    fn stop(&mut self) {
        self.listener.stop();
        let _ = self.sender.send(GestureEvent::now(GestureInput::Shutdown));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for OptionRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn validate(shortcut: &str) -> Result<(), String> {
    if shortcut == OPTION_SHORTCUT {
        #[cfg(target_os = "macos")]
        return Ok(());

        #[cfg(not(target_os = "macos"))]
        return Err("the Option-only shortcut is available on macOS".into());
    }
    shortcut
        .parse::<Shortcut>()
        .map(|_| ())
        .map_err(|error| format!("invalid push-to-talk shortcut: {error}"))
}

pub fn register<R: Runtime>(app: &AppHandle<R>, shortcut: &str) -> Result<(), String> {
    validate(shortcut)?;
    #[cfg(target_os = "macos")]
    if shortcut == OPTION_SHORTCUT {
        ensure_option_watchdog(app);
    }
    let mut controller = shortcut_controller()
        .lock()
        .map_err(|_| "shortcut controller lock is poisoned".to_string())?;
    controller.activate(app, shortcut)
}

pub fn status<R: Runtime>(app: &AppHandle<R>) -> Result<ShortcutStatus, String> {
    let mut controller = shortcut_controller()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    #[cfg(target_os = "macos")]
    {
        let input_monitoring_access = macos_option::input_monitoring_access();
        if option_status_recovery_required(
            controller.option_selected,
            controller.option_listener_active(),
        ) && !dictation_is_active(app)
            && controller
                .option_retry
                .should_attempt(Instant::now(), input_monitoring_access)
        {
            // This runs only from the dashboard status command, never from the
            // gesture worker being replaced, so stopping an unhealthy runtime
            // cannot join the current thread.
            controller.activate_option(app)?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(controller.status())
}

pub fn request_input_monitoring_permission<R: Runtime>(app: &AppHandle<R>) -> bool {
    #[cfg(target_os = "macos")]
    {
        // This is the only path that asks CoreGraphics to enroll the process
        // in the Input Monitoring privacy service. Status and watchdog checks
        // never prompt on their own.
        let input_monitoring_access = macos_option::request_input_monitoring_access();
        let mut controller = shortcut_controller()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if option_activation_required(
            controller.option_selected,
            controller.option_listener_active(),
        ) && !dictation_is_active(app)
        {
            if let Err(error) = controller.activate_option(app) {
                eprintln!("Option shortcut activation failed after its permission check: {error}");
            }
        }
        input_monitoring_access.is_granted()
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        false
    }
}

/// Replace the active shortcut while retaining the previous backend if the new
/// binding cannot be installed or the old plugin binding cannot be removed.
pub fn replace<R: Runtime>(app: &AppHandle<R>, _previous: &str, next: &str) -> Result<(), String> {
    validate(next)?;
    #[cfg(target_os = "macos")]
    if next == OPTION_SHORTCUT {
        ensure_option_watchdog(app);
    }
    let mut controller = shortcut_controller()
        .lock()
        .map_err(|_| "shortcut controller lock is poisoned".to_string())?;
    controller.activate(app, next)
}

fn shortcut_controller() -> &'static Mutex<ShortcutController> {
    SHORTCUT_CONTROLLER.get_or_init(|| Mutex::new(ShortcutController::default()))
}

#[cfg(target_os = "macos")]
fn ensure_option_watchdog<R: Runtime>(app: &AppHandle<R>) {
    if OPTION_WATCHDOG_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    let app = app.clone();
    if let Err(error) = thread::Builder::new()
        .name("spick-option-watchdog".into())
        .spawn(move || run_option_watchdog(app))
    {
        OPTION_WATCHDOG_STARTED.store(false, Ordering::Release);
        eprintln!("could not start Option shortcut recovery: {error}");
    }
}

#[cfg(target_os = "macos")]
fn run_option_watchdog<R: Runtime>(app: AppHandle<R>) {
    let mut unhealthy_since = None;
    loop {
        thread::sleep(OPTION_WATCHDOG_POLL);
        let recovery_needed = {
            let mut controller = shortcut_controller()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let listener_active = controller.option_listener_active();
            let recovery_needed =
                option_status_recovery_required(controller.option_selected, listener_active);
            if controller.option_selected && listener_active {
                controller
                    .option_retry
                    .record_success(macos_option::input_monitoring_access());
            }
            recovery_needed
        };
        if !recovery_needed {
            unhealthy_since = None;
            continue;
        }

        let now = Instant::now();
        let unhealthy_at = *unhealthy_since.get_or_insert(now);
        if now.duration_since(unhealthy_at) < OPTION_RECOVERY_GRACE || dictation_is_active(&app) {
            continue;
        }

        let mut controller = shortcut_controller()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let still_needs_recovery = option_status_recovery_required(
            controller.option_selected,
            controller.option_listener_active(),
        );
        let retry_due = still_needs_recovery
            && controller
                .option_retry
                .should_attempt(now, macos_option::input_monitoring_access());
        if retry_due {
            if let Err(error) = controller.activate_option(&app) {
                eprintln!("Option shortcut recovery failed: {error}");
            }
            unhealthy_since = None;
        }
    }
}

#[cfg(target_os = "macos")]
fn dictation_is_active<R: Runtime>(app: &AppHandle<R>) -> bool {
    let state = app.try_state::<AppState>();
    state.is_some_and(|state| {
        state.session.lock().map_or(true, |session| {
            matches!(
                session.snapshot().state,
                SessionState::Starting
                    | SessionState::Listening
                    | SessionState::Processing
                    | SessionState::Inserting
            )
        })
    })
}

#[cfg(target_os = "macos")]
fn option_activation_required(option_selected: bool, listener_active: bool) -> bool {
    option_selected && !listener_active
}

#[cfg(target_os = "macos")]
fn option_status_recovery_required(option_selected: bool, listener_active: bool) -> bool {
    // A listener can become unhealthy after permission is granted, and a
    // permission transition can make a previously unavailable listener ready.
    option_selected && !listener_active
}

fn unregister_accelerator<R: Runtime>(app: &AppHandle<R>, shortcut: &str) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    if shortcuts.is_registered(shortcut) {
        shortcuts
            .unregister(shortcut)
            .map_err(|error| format!("could not unregister push-to-talk shortcut: {error}"))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_gesture_worker<R: Runtime>(
    app: AppHandle<R>,
    receiver: mpsc::Receiver<GestureEvent>,
    overflowed: Arc<AtomicBool>,
    chord_queue: Arc<ChordQueueFlags>,
) {
    let mut machine = GestureMachine::default();
    loop {
        reconcile_gesture(&app, &mut machine);
        if overflowed.swap(false, Ordering::AcqRel) {
            if let Some(action) = machine.handle(GestureInput::ListenerDisabled, Instant::now()) {
                execute_gesture_action(&app, &mut machine, action);
            }
            let mut shutdown = false;
            loop {
                match receiver.try_recv() {
                    Ok(event) => {
                        chord_queue.release(event.input);
                        if event.input == GestureInput::Shutdown {
                            shutdown = true;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        shutdown = true;
                        break;
                    }
                }
            }
            if shutdown {
                break;
            }
            machine.quarantine(Instant::now());
        }

        let wait = machine
            .deadline()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(IDLE_GESTURE_POLL);
        let received = match receiver.try_recv() {
            Ok(event) => Ok(event),
            Err(TryRecvError::Empty) => receiver.recv_timeout(wait),
            Err(TryRecvError::Disconnected) => Err(RecvTimeoutError::Disconnected),
        };
        match received {
            Ok(event) => {
                chord_queue.release(event.input);
                reconcile_gesture(&app, &mut machine);
                let shutdown = event.input == GestureInput::Shutdown;
                for action in machine.handle_timestamped(event).into_iter().flatten() {
                    execute_gesture_action(&app, &mut machine, action);
                }
                if shutdown {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                reconcile_gesture(&app, &mut machine);
                if let Some(action) = machine.handle_timeout(Instant::now()) {
                    execute_gesture_action(&app, &mut machine, action);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(action) = machine.handle(GestureInput::ListenerDisabled, Instant::now())
                {
                    execute_gesture_action(&app, &mut machine, action);
                }
                break;
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn reconcile_gesture<R: Runtime>(app: &AppHandle<R>, machine: &mut GestureMachine) {
    let state = app.state::<AppState>();
    let recording = state
        .session
        .lock()
        .map(|session| {
            matches!(
                session.snapshot().state,
                SessionState::Starting | SessionState::Listening
            )
        })
        .unwrap_or(false);
    machine.reconcile(recording);
}

#[cfg(target_os = "macos")]
fn execute_gesture_action<R: Runtime>(
    app: &AppHandle<R>,
    machine: &mut GestureMachine,
    action: GestureAction,
) {
    let state = app.state::<AppState>();
    let result = match action {
        GestureAction::Start => {
            commands::start_session(app, state.inner(), SessionTrigger::Shortcut)
        }
        GestureAction::Stop => commands::stop_session(app, state.inner()),
        GestureAction::Cancel => commands::cancel_session(
            app,
            state.inner(),
            Some("Option was used with another input".into()),
        ),
    };
    if let Err(error) = result {
        machine.reset();
        eprintln!("Option-key dictation gesture was ignored: {error}");
    }
}

pub fn handle_event<R: Runtime>(app: &AppHandle<R>, event_state: ShortcutState) {
    #[cfg(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    ))]
    if crate::compatibility::is_active() {
        crate::compatibility::handle_shortcut(app, event_state);
        return;
    }

    let state = app.state::<AppState>();
    let result = match event_state {
        ShortcutState::Pressed => {
            commands::start_session(app, state.inner(), SessionTrigger::Shortcut)
        }
        ShortcutState::Released => commands::stop_session(app, state.inner()),
    };

    // Repeated key events and focus changes may race with a UI cancellation.
    // They are intentionally non-fatal to the desktop process.
    if let Err(error) = result {
        eprintln!("push-to-talk event ignored: {error}");
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{
        option_activation_required, option_retry_delay, option_status_recovery_required,
        ChordQueueFlags, GestureInput, InputMonitoringAccess, OptionRetryState, ShortcutController,
        ShortcutStatus, OPTION_FALLBACK_SHORTCUT, OPTION_RETRY_INITIAL, OPTION_RETRY_MAX,
    };

    #[test]
    fn status_identifies_the_selected_backend() {
        let custom = ShortcutController::default();
        assert!(!custom.status().option_selected);

        let option = ShortcutController {
            option_selected: true,
            ..ShortcutController::default()
        };
        assert!(option.status().option_selected);
    }

    #[test]
    fn input_monitoring_state_extends_the_compatible_status_shape() {
        let value = serde_json::to_value(ShortcutStatus {
            option_selected: true,
            option_listener_active: false,
            input_monitoring_granted: false,
            input_monitoring_access: InputMonitoringAccess::Denied,
            fallback_shortcut: Some(OPTION_FALLBACK_SHORTCUT),
        })
        .expect("shortcut status should serialize");

        assert_eq!(value["inputMonitoringGranted"], false);
        assert_eq!(value["inputMonitoringAccess"], "denied");
        assert_eq!(value["fallbackShortcut"], "CommandOrControl+Shift+Space");
    }

    #[test]
    fn inactive_selected_option_backend_is_always_reactivated() {
        // Recovery must not depend on whether a fallback happens to be
        // registered. A revoked listener needs the fallback; a listener whose
        // event tap stopped while permission remains needs a fresh listener.
        assert!(option_activation_required(true, false));
        assert!(!option_activation_required(true, true));
        assert!(!option_activation_required(false, false));
    }

    #[test]
    fn revoked_listener_self_heals_to_fallback_then_retries_after_grant() {
        // Listener health, rather than the privacy-pane state or presence of a
        // fallback, determines whether recovery is needed.
        assert!(option_status_recovery_required(true, false));
        assert!(!option_status_recovery_required(true, true));
        assert!(!option_status_recovery_required(false, false));
    }

    #[test]
    fn option_retry_backoff_is_capped_and_permission_changes_retry_immediately() {
        let started_at = std::time::Instant::now();
        let mut retry = OptionRetryState::default();

        assert!(retry.should_attempt(started_at, InputMonitoringAccess::Denied));
        retry.record_failure(started_at, InputMonitoringAccess::Denied);
        assert!(!retry.should_attempt(
            started_at + OPTION_RETRY_INITIAL - std::time::Duration::from_millis(1),
            InputMonitoringAccess::Denied
        ));
        assert!(retry.should_attempt(
            started_at + OPTION_RETRY_INITIAL,
            InputMonitoringAccess::Denied
        ));

        // A real Input Monitoring state transition bypasses the old state's delay.
        assert!(retry.should_attempt(
            started_at + std::time::Duration::from_millis(1),
            InputMonitoringAccess::Granted
        ));
        assert_eq!(option_retry_delay(100), OPTION_RETRY_MAX);
    }

    #[test]
    fn a_healthy_listener_clears_prior_retry_failures() {
        let started_at = std::time::Instant::now();
        let mut retry = OptionRetryState::default();
        retry.record_failure(started_at, InputMonitoringAccess::Unknown);
        retry.record_success(InputMonitoringAccess::Unknown);
        assert!(retry.should_attempt(started_at, InputMonitoringAccess::Unknown));
    }

    #[test]
    fn noisy_pointer_chords_cannot_consume_the_keyboard_chord_slot() {
        let queued = ChordQueueFlags::default();
        assert!(queued.claim(GestureInput::PointerChord));
        assert!(!queued.claim(GestureInput::PointerChord));
        assert!(queued.claim(GestureInput::HudPointerChord));
        assert!(!queued.claim(GestureInput::HudPointerChord));
        assert!(queued.claim(GestureInput::KeyboardChord));
        assert!(!queued.claim(GestureInput::KeyboardChord));

        queued.release(GestureInput::PointerChord);
        assert!(queued.claim(GestureInput::PointerChord));
        queued.release(GestureInput::HudPointerChord);
        assert!(queued.claim(GestureInput::HudPointerChord));
        queued.release(GestureInput::KeyboardChord);
        assert!(queued.claim(GestureInput::KeyboardChord));
    }
}
