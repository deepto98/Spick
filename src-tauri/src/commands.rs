use tauri::{AppHandle, Emitter, Runtime, State};

use crate::{
    domain::{AppSettings, DictationStateEvent, SessionTrigger},
    hud, platform, shortcut,
    state::AppState,
};

pub const DICTATION_STATE_EVENT: &str = "dictation://state";

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state.settings_snapshot()
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    settings.validate()?;
    shortcut::validate(&settings.push_to_talk_shortcut)?;

    let mut current = state
        .settings
        .write()
        .map_err(|_| "settings lock is poisoned".to_string())?;
    let previous = current.clone();
    let shortcut_changed = previous.push_to_talk_shortcut != settings.push_to_talk_shortcut;

    if shortcut_changed {
        shortcut::replace(
            &app,
            &previous.push_to_talk_shortcut,
            &settings.push_to_talk_shortcut,
        )?;
    }

    if let Err(error) = state.persist_settings(&settings) {
        if shortcut_changed {
            let rollback = shortcut::replace(
                &app,
                &settings.push_to_talk_shortcut,
                &previous.push_to_talk_shortcut,
            );
            if let Err(rollback_error) = rollback {
                return Err(format!(
                    "{error}; shortcut rollback also failed: {rollback_error}"
                ));
            }
        }
        return Err(error);
    }

    let hud_position_changed = previous.hud.position != settings.hud.position;
    *current = settings.clone();
    drop(current);

    if hud_position_changed {
        if let Err(error) = hud::reposition(&app, settings.hud.position) {
            eprintln!("saved HUD position but could not move its window: {error}");
        }
    }

    Ok(settings)
}

#[tauri::command]
pub fn get_dictation_session(state: State<'_, AppState>) -> Result<DictationStateEvent, String> {
    state
        .session
        .lock()
        .map(|session| session.snapshot())
        .map_err(|_| "dictation session lock is poisoned".into())
}

#[tauri::command]
pub fn start_dictation_session(
    app: AppHandle,
    state: State<'_, AppState>,
    trigger: Option<SessionTrigger>,
) -> Result<DictationStateEvent, String> {
    start_session(
        &app,
        state.inner(),
        trigger.unwrap_or(SessionTrigger::UserInterface),
    )
}

#[tauri::command]
pub fn stop_dictation_session(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DictationStateEvent, String> {
    stop_session(&app, state.inner())
}

#[tauri::command]
pub fn cancel_dictation_session(
    app: AppHandle,
    state: State<'_, AppState>,
    reason: Option<String>,
) -> Result<DictationStateEvent, String> {
    let event = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?
        .cancel(reason)
        .map_err(|error| error.to_string())?;

    emit_state(&app, &event)?;
    if let Err(error) = hud::hide(&app) {
        eprintln!("dictation was cancelled but the HUD could not be hidden: {error}");
    }
    Ok(event)
}

/// Provider adapters call this after transcription and cleanup finish. It is
/// also useful to close out a mocked session while the audio pipeline is being
/// developed independently.
#[tauri::command]
pub fn complete_dictation_session(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DictationStateEvent, String> {
    let event = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?
        .complete()
        .map_err(|error| error.to_string())?;

    emit_state(&app, &event)?;
    if let Err(error) = hud::hide(&app) {
        eprintln!("dictation completed but the HUD could not be hidden: {error}");
    }
    Ok(event)
}

#[tauri::command]
pub fn fail_dictation_session(
    app: AppHandle,
    state: State<'_, AppState>,
    error: String,
) -> Result<DictationStateEvent, String> {
    let event = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?
        .fail(error)
        .map_err(|error| error.to_string())?;

    emit_state(&app, &event)?;
    if let Err(error) = hud::hide(&app) {
        eprintln!("dictation failed but the HUD could not be hidden: {error}");
    }
    Ok(event)
}

#[tauri::command]
pub fn get_platform_capabilities() -> platform::PlatformCapabilities {
    platform::current_platform_capabilities()
}

pub(crate) fn start_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    trigger: SessionTrigger,
) -> Result<DictationStateEvent, String> {
    let language_policy = state.settings_snapshot()?.language_policy;
    let event = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?
        .start(trigger, language_policy)
        .map_err(|error| error.to_string())?;

    if let Err(error) = hud::show(app) {
        eprintln!("dictation started but the HUD could not be shown: {error}");
    }
    emit_state(app, &event)?;
    Ok(event)
}

pub(crate) fn stop_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> Result<DictationStateEvent, String> {
    let event = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?
        .stop()
        .map_err(|error| error.to_string())?;

    emit_state(app, &event)?;
    Ok(event)
}

fn emit_state<R: Runtime>(app: &AppHandle<R>, event: &DictationStateEvent) -> Result<(), String> {
    app.emit(DICTATION_STATE_EVENT, event)
        .map_err(|error| format!("could not emit dictation state: {error}"))
}
