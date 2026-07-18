use std::{sync::Arc, thread, time::Duration};

use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::{
    audio::{
        AudioCaptureFailure, AudioCaptureStatus, CaptureFinalizer, ErrorSink, LevelSink,
        AUDIO_LEVEL_EVENT,
    },
    domain::{AppSettings, DictationStateEvent, SessionState, SessionTrigger},
    hud, platform, shortcut,
    state::AppState,
};

pub const DICTATION_STATE_EVENT: &str = "dictation://state";
const SUCCESS_HUD_DWELL: Duration = Duration::from_millis(650);
const FAILURE_HUD_DWELL: Duration = Duration::from_secs(2);

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
        if !platform::current_platform_capabilities().supports_global_shortcut {
            return Err(
                "global shortcuts are unavailable in the current desktop session; settings were not changed"
                    .into(),
            );
        }
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
pub fn get_audio_capture_status(state: State<'_, AppState>) -> Result<AudioCaptureStatus, String> {
    state
        .audio
        .lock()
        .map(|audio| audio.status())
        .map_err(|_| "microphone capture lock is poisoned".into())
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
    let (event, cleanup) = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        let session_id = active_session_id(&session.snapshot())?;
        let mut audio = state
            .audio
            .lock()
            .map_err(|_| "microphone capture lock is poisoned".to_string())?;
        let cleanup = audio.take_matching(&session_id);
        let event = session.cancel(reason).map_err(|error| error.to_string())?;
        (event, cleanup)
    };

    discard_on_worker(cleanup);

    emit_state(&app, &event)?;
    if let Err(error) = hud::hide(&app) {
        eprintln!("dictation was cancelled but the HUD could not be hidden: {error}");
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
    let settings = state.settings_snapshot()?;
    let language_policy = settings.language_policy;
    let hud_position = settings.hud.position;
    let level_app = app.clone();
    let level_sink: LevelSink = Arc::new(move |payload| {
        if let Err(error) = level_app.emit(AUDIO_LEVEL_EVENT, payload) {
            eprintln!("could not emit microphone level: {error}");
        }
    });
    let error_app = app.clone();
    let error_sink: ErrorSink = Arc::new(move |failure| {
        handle_audio_capture_failure(&error_app, failure);
    });

    // Both locks are held only while creating a session and spawning the owner
    // thread. No microphone API is called on this shortcut-handler path.
    let (event, start_error) = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        let mut audio = state
            .audio
            .lock()
            .map_err(|_| "microphone capture lock is poisoned".to_string())?;
        let listening = session
            .start(trigger, language_policy)
            .map_err(|error| error.to_string())?;
        let session_id = active_session_id(&listening)?;

        match audio.start(session_id, level_sink, error_sink) {
            Ok(_) => (listening, None),
            Err(error) => {
                let failed = session
                    .fail(error.clone())
                    .map_err(|transition| transition.to_string())?;
                (failed, Some(error))
            }
        }
    };

    if let Some(error) = start_error {
        let _ = emit_state(app, &event);
        hide_after(app, FAILURE_HUD_DWELL, event.revision);
        return Err(error);
    }

    if let Err(error) = hud::show(app, hud_position) {
        eprintln!("dictation started but the HUD could not be shown: {error}");
    }
    if let Err(error) = emit_state(app, &event) {
        eprintln!("could not emit listening state: {error}");
    }
    Ok(event)
}

pub(crate) fn stop_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> Result<DictationStateEvent, String> {
    let (processing, session_id, finalizer) = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        let session_id = active_session_id(&session.snapshot())?;
        let mut audio = state
            .audio
            .lock()
            .map_err(|_| "microphone capture lock is poisoned".to_string())?;
        let processing = session.stop().map_err(|error| error.to_string())?;
        let finalizer = audio.take_for_session(&session_id);
        (processing, session_id, finalizer)
    };

    if let Err(error) = emit_state(app, &processing) {
        eprintln!("could not emit processing state: {error}");
    }

    // Permission waits, stream teardown, resampling handoff, and the terminal
    // transition all run off the shortcut callback. The response above remains
    // Processing; the emitted terminal revision is authoritative.
    let worker_app = app.clone();
    let spawn_result = thread::Builder::new()
        .name("spick-capture-finalize".into())
        .spawn(move || finalize_capture(&worker_app, session_id, finalizer));
    if let Err(error) = spawn_result {
        let message = format!("could not start microphone finalization: {error}");
        if let Some(failed) = fail_session_if_matching(state, &processing, message)? {
            let _ = emit_state(app, &failed);
            hide_after(app, FAILURE_HUD_DWELL, failed.revision);
        }
    }

    Ok(processing)
}

fn emit_state<R: Runtime>(app: &AppHandle<R>, event: &DictationStateEvent) -> Result<(), String> {
    app.emit(DICTATION_STATE_EVENT, event)
        .map_err(|error| format!("could not emit dictation state: {error}"))
}

fn active_session_id(event: &DictationStateEvent) -> Result<String, String> {
    event
        .session
        .as_ref()
        .map(|session| session.id.clone())
        .ok_or_else(|| "dictation session identity is unavailable".into())
}

fn finalize_capture<R: Runtime>(
    app: &AppHandle<R>,
    session_id: String,
    finalizer: Result<CaptureFinalizer, String>,
) {
    let capture_result = finalizer.and_then(CaptureFinalizer::finalize);
    match capture_result {
        Ok(mut capture) => {
            let status = capture.status();
            // Capture-only completion has no consumer yet. Exercise the future
            // provider handoff seam, then release PCM before marking terminal.
            let mut pcm = capture.take_pcm_16khz();
            let sample_count = pcm.len();
            pcm.fill(0.0);
            pcm.clear();
            drop(pcm);
            drop(capture);

            if sample_count == 0 || status.sample_count == 0 {
                fail_and_emit_if_matching(
                    app,
                    &session_id,
                    "no microphone audio was captured".into(),
                );
                return;
            }

            match complete_session_if_matching(app.state::<AppState>().inner(), &session_id) {
                Ok(Some(completed)) => {
                    if let Err(error) = emit_state(app, &completed) {
                        eprintln!("could not emit capture completion: {error}");
                    }
                    hide_after(app, SUCCESS_HUD_DWELL, completed.revision);
                }
                Ok(None) => {}
                Err(error) => eprintln!("could not complete capture session: {error}"),
            }
        }
        Err(error) => fail_and_emit_if_matching(app, &session_id, error),
    }
}

fn handle_audio_capture_failure<R: Runtime>(app: &AppHandle<R>, failure: AudioCaptureFailure) {
    let state = app.state::<AppState>();
    let transition =
        (|| -> Result<Option<(DictationStateEvent, Option<CaptureFinalizer>)>, String> {
            let mut session = state
                .session
                .lock()
                .map_err(|_| "dictation session lock is poisoned".to_string())?;
            let snapshot = session.snapshot();
            if !session_matches(&snapshot, &failure.session_id) {
                return Ok(None);
            }

            let mut audio = state
                .audio
                .lock()
                .map_err(|_| "microphone capture lock is poisoned".to_string())?;
            let cleanup = audio.take_matching(&failure.session_id);
            let failed = session
                .fail(failure.message)
                .map_err(|error| error.to_string())?;
            Ok(Some((failed, cleanup)))
        })();

    match transition {
        Ok(Some((event, cleanup))) => {
            discard_on_worker(cleanup);
            if let Err(emit_error) = emit_state(app, &event) {
                eprintln!("could not emit microphone failure: {emit_error}");
            }
            hide_after(app, FAILURE_HUD_DWELL, event.revision);
        }
        Ok(None) => {}
        Err(error) => eprintln!("could not handle microphone failure: {error}"),
    }
}

fn session_matches(event: &DictationStateEvent, session_id: &str) -> bool {
    matches!(
        event.state,
        SessionState::Listening | SessionState::Processing
    ) && event
        .session
        .as_ref()
        .is_some_and(|session| session.id == session_id)
}

fn complete_session_if_matching(
    state: &AppState,
    session_id: &str,
) -> Result<Option<DictationStateEvent>, String> {
    let mut session = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?;
    let snapshot = session.snapshot();
    if snapshot.state != SessionState::Processing || !session_matches(&snapshot, session_id) {
        return Ok(None);
    }
    session
        .complete()
        .map(Some)
        .map_err(|error| error.to_string())
}

fn fail_session_if_matching(
    state: &AppState,
    expected: &DictationStateEvent,
    error: String,
) -> Result<Option<DictationStateEvent>, String> {
    let session_id = active_session_id(expected)?;
    let mut session = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?;
    if !session_matches(&session.snapshot(), &session_id) {
        return Ok(None);
    }
    session
        .fail(error)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn fail_and_emit_if_matching<R: Runtime>(app: &AppHandle<R>, session_id: &str, error: String) {
    let state = app.state::<AppState>();
    let expected = state.session.lock().map(|session| session.snapshot());
    let event = match expected {
        Ok(expected) if session_matches(&expected, session_id) => {
            fail_session_if_matching(state.inner(), &expected, error)
        }
        Ok(_) => return,
        Err(_) => Err("dictation session lock is poisoned".into()),
    };

    match event {
        Ok(Some(failed)) => {
            if let Err(emit_error) = emit_state(app, &failed) {
                eprintln!("could not emit capture failure: {emit_error}");
            }
            hide_after(app, FAILURE_HUD_DWELL, failed.revision);
        }
        Ok(None) => {}
        Err(error) => eprintln!("could not fail capture session: {error}"),
    }
}

fn discard_on_worker(finalizer: Option<CaptureFinalizer>) {
    let Some(finalizer) = finalizer else {
        return;
    };
    let session_id = finalizer.session_id().to_string();
    let spawn_result = thread::Builder::new()
        .name("spick-capture-discard".into())
        .spawn(move || {
            if let Err(error) = finalizer.discard() {
                eprintln!("could not discard microphone for session {session_id}: {error}");
            }
        });
    if let Err(error) = spawn_result {
        eprintln!("could not start microphone cleanup: {error}");
    }
}

fn hide_after<R: Runtime>(app: &AppHandle<R>, delay: Duration, revision: u64) {
    let app = app.clone();
    let spawn_result = thread::Builder::new()
        .name("spick-hud-hide".into())
        .spawn(move || {
            thread::sleep(delay);

            // Do not let an old completion timer hide a newly-started session.
            let should_hide = app
                .state::<AppState>()
                .session
                .lock()
                .map(|session| should_hide_for_revision(&session.snapshot(), revision))
                .unwrap_or(false);

            if should_hide {
                if let Err(error) = hud::hide(&app) {
                    eprintln!("could not hide the dictation HUD: {error}");
                }
            }
        });

    if let Err(error) = spawn_result {
        eprintln!("could not schedule the dictation HUD dismissal: {error}");
    }
}

fn should_hide_for_revision(event: &DictationStateEvent, revision: u64) -> bool {
    event.revision == revision
        && matches!(
            event.state,
            SessionState::Completed | SessionState::Cancelled | SessionState::Failed
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DictationSession, LanguagePolicy};

    fn event(id: &str, state: SessionState, revision: u64) -> DictationStateEvent {
        DictationStateEvent {
            revision,
            state,
            session: Some(DictationSession {
                id: id.into(),
                state,
                trigger: SessionTrigger::Shortcut,
                language_policy: LanguagePolicy::Auto,
                started_at_ms: 1,
                ended_at_ms: None,
                cancel_reason: None,
                error: None,
            }),
        }
    }

    #[test]
    fn asynchronous_results_only_match_their_originating_active_session() {
        assert!(session_matches(
            &event("session-a", SessionState::Listening, 1),
            "session-a"
        ));
        assert!(session_matches(
            &event("session-a", SessionState::Processing, 2),
            "session-a"
        ));
        assert!(!session_matches(
            &event("session-b", SessionState::Processing, 3),
            "session-a"
        ));
        assert!(!session_matches(
            &event("session-a", SessionState::Completed, 3),
            "session-a"
        ));
    }

    #[test]
    fn hud_timer_requires_the_exact_terminal_revision() {
        let completed = event("session-a", SessionState::Completed, 4);
        assert!(should_hide_for_revision(&completed, 4));
        assert!(!should_hide_for_revision(&completed, 3));
        assert!(!should_hide_for_revision(
            &event("session-b", SessionState::Listening, 5),
            4
        ));
    }
}
