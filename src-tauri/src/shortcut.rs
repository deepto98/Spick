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
use gesture::{GestureAction, GestureInput, GestureMachine};

pub const OPTION_SHORTCUT: &str = "Option";
pub const OPTION_FALLBACK_SHORTCUT: &str = "CommandOrControl+Shift+Space";

#[cfg(target_os = "macos")]
const OPTION_EVENT_CAPACITY: usize = 64;
#[cfg(target_os = "macos")]
const IDLE_GESTURE_POLL: Duration = Duration::from_millis(250);

static SHORTCUT_CONTROLLER: OnceLock<Mutex<ShortcutController>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutStatus {
    pub option_selected: bool,
    pub option_listener_active: bool,
    pub input_monitoring_granted: bool,
    pub fallback_shortcut: Option<&'static str>,
}

#[derive(Default)]
struct ShortcutController {
    accelerator: Option<String>,
    option_selected: bool,
    fallback_registered: bool,
    #[cfg(target_os = "macos")]
    option_runtime: Option<OptionRuntime>,
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
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn activate_option<R: Runtime>(&mut self, app: &AppHandle<R>) -> Result<(), String> {
        // A recovering worker is not a usable shortcut. Try a fresh listener
        // (or install the fallback if permission was revoked) unless the
        // current listener is actually receiving events.
        if self.option_selected
            && self.option_listener_active()
            && macos_option::listen_access_granted()
        {
            return Ok(());
        }

        match OptionRuntime::start(app.clone()) {
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
                    return Err(error);
                }

                if let Some(mut previous) = self.option_runtime.take() {
                    previous.stop();
                }
                self.option_runtime = Some(new_runtime);
                self.accelerator = None;
                self.option_selected = true;
                self.fallback_registered = false;
                Ok(())
            }
            Err(listener_error) => {
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
                eprintln!("Option-key dictation is waiting for Input Monitoring: {listener_error}");
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
        ShortcutStatus {
            option_selected: self.option_selected,
            #[cfg(target_os = "macos")]
            option_listener_active: self.option_selected && self.option_listener_active(),
            #[cfg(not(target_os = "macos"))]
            option_listener_active: false,
            #[cfg(target_os = "macos")]
            input_monitoring_granted: macos_option::listen_access_granted(),
            #[cfg(not(target_os = "macos"))]
            input_monitoring_granted: false,
            fallback_shortcut: (self.option_selected && self.fallback_registered)
                .then_some(OPTION_FALLBACK_SHORTCUT),
        }
    }
}

#[cfg(target_os = "macos")]
struct OptionRuntime {
    listener: macos_option::ListenerHandle,
    sender: mpsc::SyncSender<GestureInput>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(target_os = "macos")]
impl OptionRuntime {
    fn start<R: Runtime>(app: AppHandle<R>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(OPTION_EVENT_CAPACITY);
        let overflowed = Arc::new(AtomicBool::new(false));
        let listener = macos_option::start_listener(sender.clone(), Arc::clone(&overflowed))?;
        let worker = match thread::Builder::new()
            .name("spick-option-gesture".into())
            .spawn(move || run_gesture_worker(app, receiver, overflowed))
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
        let _ = self.sender.send(GestureInput::Shutdown);
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
        let permission_granted = macos_option::listen_access_granted();
        if option_status_recovery_required(
            controller.option_selected,
            controller.option_listener_active(),
            controller.fallback_registered,
            permission_granted,
        ) {
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
        // CoreGraphics documents the request API as requesting access "if
        // absent". Preflight first anyway so normal status recovery never
        // repeats a potentially prompting call after access is already set.
        let granted =
            macos_option::listen_access_granted() || macos_option::request_listen_access();
        let mut controller = shortcut_controller()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if option_activation_required(
            controller.option_selected,
            controller.option_listener_active() && granted,
        ) {
            if let Err(error) = controller.activate_option(app) {
                eprintln!("Option shortcut activation failed after its permission check: {error}");
            }
        }
        granted
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
    let mut controller = shortcut_controller()
        .lock()
        .map_err(|_| "shortcut controller lock is poisoned".to_string())?;
    controller.activate(app, next)
}

fn shortcut_controller() -> &'static Mutex<ShortcutController> {
    SHORTCUT_CONTROLLER.get_or_init(|| Mutex::new(ShortcutController::default()))
}

#[cfg(target_os = "macos")]
fn option_activation_required(option_selected: bool, listener_active: bool) -> bool {
    option_selected && !listener_active
}

#[cfg(target_os = "macos")]
fn option_status_recovery_required(
    option_selected: bool,
    listener_active: bool,
    fallback_registered: bool,
    permission_granted: bool,
) -> bool {
    if !option_selected || (listener_active && permission_granted) {
        return false;
    }

    // A denied installation with an existing fallback is already usable.
    // Retry automatically once permission appears, or immediately when an
    // unhealthy listener has not yet been replaced by the fallback.
    permission_granted || !fallback_registered
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
    receiver: mpsc::Receiver<GestureInput>,
    overflowed: Arc<AtomicBool>,
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
                    Ok(GestureInput::Shutdown) => shutdown = true,
                    Ok(_) => {}
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
        match receiver.recv_timeout(wait) {
            Ok(input) => {
                reconcile_gesture(&app, &mut machine);
                let shutdown = input == GestureInput::Shutdown;
                if let Some(action) = machine.handle(input, Instant::now()) {
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
    let listening = state
        .session
        .lock()
        .map(|session| session.snapshot().state == SessionState::Listening)
        .unwrap_or(false);
    machine.reconcile(listening);
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
    use super::{option_activation_required, option_status_recovery_required, ShortcutController};

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
        // The first dashboard status check after revocation must replace the
        // unhealthy listener with the already-supported fallback.
        assert!(option_status_recovery_required(true, false, false, false));
        // Once installed, the fallback is a stable, usable denied state and
        // status polling does not repeatedly rebuild it or prompt.
        assert!(!option_status_recovery_required(true, false, true, false));
        // Returning from System Settings after granting access promotes the
        // fallback back to the Option listener automatically.
        assert!(option_status_recovery_required(true, false, true, true));
        assert!(!option_status_recovery_required(true, true, false, true));
        // Treat a stale "active" health bit as unusable once access is gone.
        assert!(option_status_recovery_required(true, true, false, false));
    }
}
