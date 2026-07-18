use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{commands, domain::SessionTrigger, state::AppState};

pub fn validate(shortcut: &str) -> Result<(), String> {
    shortcut
        .parse::<Shortcut>()
        .map(|_| ())
        .map_err(|error| format!("invalid push-to-talk shortcut: {error}"))
}

pub fn register<R: Runtime>(app: &AppHandle<R>, shortcut: &str) -> Result<(), String> {
    validate(shortcut)?;
    app.global_shortcut()
        .register(shortcut)
        .map_err(|error| format!("could not register push-to-talk shortcut: {error}"))
}

/// Replace the registered shortcut while retaining the old binding if the new
/// one is invalid or unavailable.
pub fn replace<R: Runtime>(app: &AppHandle<R>, previous: &str, next: &str) -> Result<(), String> {
    if previous == next {
        return Ok(());
    }

    register(app, next)?;
    let shortcuts = app.global_shortcut();
    if shortcuts.is_registered(previous) {
        if let Err(error) = shortcuts.unregister(previous) {
            let _ = shortcuts.unregister(next);
            return Err(format!(
                "could not unregister the previous push-to-talk shortcut: {error}"
            ));
        }
    }

    Ok(())
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
