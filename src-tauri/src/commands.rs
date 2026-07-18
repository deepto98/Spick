use std::{sync::Arc, thread, time::Duration};

use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};

use crate::{
    audio::{
        AudioCaptureFailure, AudioCaptureStatus, CaptureFinalizer, ErrorSink, LevelSink,
        AUDIO_LEVEL_EVENT,
    },
    domain::{
        AppSettings, DictationDelivery, DictationDeliveryStatus, DictationSession,
        DictationStateEvent, EngineConfig, EngineLocation, EngineProvider, LanguagePolicy,
        SessionState, SessionTrigger,
    },
    engines::{
        resolve_curated_whisper_model, validate_whisper_model_policy, AudioInput, CleanupEngine,
        CleanupRequest, DictationTranscript, EngineError, ModelLanguageSet, RuleBasedCleanupEngine,
        TranscriptResult, TranscriptionRequest,
    },
    hud,
    model_store::{LocalModelSummary, ModelDownloadProgress, MODEL_DOWNLOAD_PROGRESS_EVENT},
    platform::{self, TextTargetError, TextTargetErrorKind},
    shortcut,
    state::AppState,
};

pub const DICTATION_STATE_EVENT: &str = "dictation://state";
pub const DICTATION_TRANSCRIPT_EVENT: &str = "dictation://transcript";
const MAIN_WINDOW_LABEL: &str = "main";
const SUCCESS_HUD_DWELL: Duration = Duration::from_millis(650);
const FAILURE_HUD_DWELL: Duration = Duration::from_secs(2);

struct PendingDictationTranscript {
    session_id: String,
    engine_id: String,
    transcript: crate::engines::TranscriptResult,
}

impl PendingDictationTranscript {
    fn finish(self, delivery: DictationDelivery) -> DictationTranscript {
        DictationTranscript {
            session_id: self.session_id,
            engine_id: self.engine_id,
            transcript: self.transcript,
            delivery,
        }
    }
}

#[tauri::command]
pub fn get_settings(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    require_main_window(&window)?;
    state.settings_snapshot()
}

#[tauri::command]
pub fn update_settings(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    require_main_window(&window)?;
    settings.validate()?;
    shortcut::validate(&settings.push_to_talk_shortcut)?;

    let _model_configuration = state
        .model_configuration
        .lock()
        .map_err(|_| "model configuration is unavailable".to_string())?;

    let mut current = state
        .settings
        .write()
        .map_err(|_| "settings lock is poisoned".to_string())?;
    if settings.transcription_engine != current.transcription_engine {
        return Err("choose transcription models from Engines".into());
    }
    validate_selected_transcription(&settings)?;
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
pub fn get_last_transcript(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<DictationTranscript>, String> {
    require_main_window(&window)?;
    state.latest_transcript()
}

#[tauri::command]
pub fn list_local_models(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Vec<LocalModelSummary>, String> {
    require_main_window(&window)?;
    let settings = state.settings_snapshot()?;
    Ok(state.models.catalog(&settings.transcription_engine.model))
}

#[tauri::command]
pub async fn install_local_model(
    window: WebviewWindow,
    app: AppHandle,
    model_id: String,
) -> Result<LocalModelSummary, String> {
    require_main_window(&window)?;
    let models = Arc::clone(&app.state::<AppState>().models);
    let progress_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        models.install(&model_id, |progress: ModelDownloadProgress| {
            if let Err(error) =
                progress_app.emit_to(MAIN_WINDOW_LABEL, MODEL_DOWNLOAD_PROGRESS_EVENT, progress)
            {
                eprintln!("could not emit model download progress: {error}");
            }
        })
    })
    .await
    .map_err(|error| format!("local model download worker failed: {error}"))?
}

#[tauri::command]
pub fn cancel_local_model_install(
    window: WebviewWindow,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<bool, String> {
    require_main_window(&window)?;
    state.models.cancel_download(&model_id)
}

#[tauri::command]
pub async fn activate_local_model(
    window: WebviewWindow,
    app: AppHandle,
    model_id: String,
) -> Result<AppSettings, String> {
    require_main_window(&window)?;
    let model = resolve_curated_whisper_model(&model_id)
        .ok_or_else(|| format!("unknown local model: {model_id}"))?;
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        // The first verification may hash hundreds of megabytes. Do that
        // before the short settings transaction so dictation can still start
        // with the current model. Recheck from the fingerprint cache while the
        // transaction lock is held before persisting the selection.
        let model_path = state.models.verified_model_path(&model.id)?;
        state
            .whisper
            .load(&model, &model_path)
            .map_err(engine_error_message)?;
        let _model_configuration = state
            .model_configuration
            .lock()
            .map_err(|_| "model configuration is unavailable".to_string())?;
        let mut updated = state.settings_snapshot()?;
        updated.language_policy = policy_for_model_activation(&updated.language_policy, &model);
        updated.transcription_engine =
            EngineConfig::local(EngineProvider::WhisperCpp, model.id.clone());
        updated.validate()?;
        validate_selected_transcription(&updated)?;
        state.models.verified_model_path(&model.id)?;
        state.persist_settings(&updated)?;
        *state
            .settings
            .write()
            .map_err(|_| "settings lock is poisoned".to_string())? = updated.clone();
        Ok(updated)
    })
    .await
    .map_err(|error| format!("local model activation worker failed: {error}"))?
}

#[tauri::command]
pub async fn remove_local_model(
    window: WebviewWindow,
    app: AppHandle,
    model_id: String,
) -> Result<(), String> {
    require_main_window(&window)?;
    let model = resolve_curated_whisper_model(&model_id)
        .ok_or_else(|| format!("unknown local model: {model_id}"))?;
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        let _model_configuration = state
            .model_configuration
            .lock()
            .map_err(|_| "model configuration is unavailable".to_string())?;
        let settings = state.settings_snapshot()?;
        let active_id = resolve_curated_whisper_model(&settings.transcription_engine.model)
            .map(|active| active.id.clone());
        if active_id.as_deref() == Some(model.id.as_str()) {
            return Err("choose another local model before removing the active one".into());
        }

        let in_use = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?
            .snapshot();
        if session_uses_model(&in_use, &model.id) {
            return Err("wait for the current dictation before removing this model".into());
        }

        state.models.remove(&model.id)?;
        state.whisper.unload(&model.id);
        Ok(())
    })
    .await
    .map_err(|error| format!("local model removal worker failed: {error}"))?
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
    let (event, cleanup, target) = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        let session_id = active_session_id(&session.snapshot())?;
        let target = state
            .transcription_operation(&session_id)?
            .and_then(|operation| operation.target)
            .map(|target| target.token);
        // This transition is the cancellation linearization point. Once a
        // worker has claimed Inserting, cancellation returns an error and must
        // not clear its target or claim that no text was written.
        let event = session.cancel(reason).map_err(|error| error.to_string())?;
        let mut audio = state
            .audio
            .lock()
            .map_err(|_| "microphone capture lock is poisoned".to_string())?;
        let cleanup = audio.take_matching(&session_id);
        state.cancel_transcription(&session_id)?;
        state.finish_transcription(&session_id)?;
        (event, cleanup, target)
    };

    if let Some(target) = target {
        state.text_targets.discard(target);
    }
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

#[tauri::command]
pub fn get_accessibility_permission_status(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<platform::AccessibilityPermissionStatus, String> {
    require_main_window(&window)?;
    Ok(state.text_targets.permission_status())
}

#[tauri::command]
pub fn request_accessibility_permission(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<platform::AccessibilityPermissionStatus, String> {
    require_main_window(&window)?;
    state
        .text_targets
        .request_permission()
        .map_err(|error| error.to_string())
}

fn require_main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("this command is only available from the Spick dashboard".into())
    }
}

fn policy_for_model_activation(
    current: &LanguagePolicy,
    model: &crate::engines::WhisperModelManifest,
) -> LanguagePolicy {
    if model.languages == ModelLanguageSet::EnglishOnly {
        LanguagePolicy::Fixed {
            language: "en".into(),
        }
    } else {
        current.clone()
    }
}

fn validate_selected_transcription(settings: &AppSettings) -> Result<(), String> {
    if settings.transcription_engine.provider != EngineProvider::WhisperCpp
        || settings.transcription_engine.location != EngineLocation::Local
    {
        return Ok(());
    }

    let model =
        resolve_curated_whisper_model(&settings.transcription_engine.model).ok_or_else(|| {
            format!(
                "unknown local model: {}",
                settings.transcription_engine.model
            )
        })?;
    validate_whisper_model_policy(&settings.language_policy, &model).map_err(|error| {
        format!(
            "{} can’t use the current language setting: {}",
            model.display_name,
            engine_error_message(error)
        )
    })
}

fn session_uses_model(event: &DictationStateEvent, model_id: &str) -> bool {
    matches!(
        event.state,
        SessionState::Listening | SessionState::Processing | SessionState::Inserting
    ) && event.session.as_ref().is_some_and(|session| {
        resolve_curated_whisper_model(&session.transcription_engine.model)
            .is_some_and(|model| model.id == model_id)
    })
}

pub(crate) fn start_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    trigger: SessionTrigger,
) -> Result<DictationStateEvent, String> {
    // Shortcut sessions must prove a concrete, non-secure editable target
    // before any session state, audio capture, or HUD window is created.
    let captured_target = if trigger == SessionTrigger::Shortcut {
        match state.text_targets.capture() {
            Ok(target) => Some(target),
            Err(error) => return fail_target_preflight(app, state, trigger, error),
        }
    } else {
        // The dashboard owns focus for UI-triggered recordings. Those remain
        // an explicit transcript/copy test and never type into another app.
        None
    };
    let target_token = captured_target.as_ref().map(|target| target.token);

    let model_configuration = match state.model_configuration.lock() {
        Ok(configuration) => configuration,
        Err(_) => {
            if let Some(target) = target_token {
                state.text_targets.discard(target);
            }
            return Err("model configuration is unavailable".into());
        }
    };
    let settings = match state.settings_snapshot() {
        Ok(settings) => settings,
        Err(error) => {
            if let Some(target) = target_token {
                state.text_targets.discard(target);
            }
            return Err(error);
        }
    };
    let language_policy = settings.language_policy;
    let transcription_engine = settings.transcription_engine;
    let cleanup_engine = settings.cleanup_engine;
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

    // These locks are held only while creating the session and spawning the
    // microphone owner thread. Accessibility calls completed above.
    let transaction = (|| -> Result<(DictationStateEvent, Option<String>), String> {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        let mut audio = state
            .audio
            .lock()
            .map_err(|_| "microphone capture lock is poisoned".to_string())?;
        let listening = session
            .start(
                trigger,
                language_policy,
                transcription_engine,
                cleanup_engine,
            )
            .map_err(|error| error.to_string())?;
        let session_id = active_session_id(&listening)?;
        if let Err(error) = state.begin_transcription(session_id.clone(), captured_target.clone()) {
            let failed = session
                .fail(error.clone())
                .map_err(|transition| transition.to_string())?;
            return Ok((failed, Some(error)));
        }

        match audio.start(session_id, level_sink, error_sink) {
            Ok(_) => Ok((listening, None)),
            Err(error) => {
                state.finish_transcription(&active_session_id(&listening)?)?;
                let failed = session
                    .fail(error.clone())
                    .map_err(|transition| transition.to_string())?;
                Ok((failed, Some(error)))
            }
        }
    })();
    drop(model_configuration);
    let (event, start_error) = match transaction {
        Ok(result) => result,
        Err(error) => {
            if let Some(target) = target_token {
                state.text_targets.discard(target);
            }
            return Err(error);
        }
    };

    if let Some(error) = start_error {
        if let Some(target) = target_token {
            state.text_targets.discard(target);
        }
        if let Err(show_error) = hud::show(app, hud_position) {
            eprintln!("dictation failed but the HUD could not be shown: {show_error}");
        }
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

fn fail_target_preflight<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    trigger: SessionTrigger,
    error: TextTargetError,
) -> Result<DictationStateEvent, String> {
    let settings = state.settings_snapshot()?;
    let hud_position = settings.hud.position;
    let delivery = delivery_for_target_error(&error, false, None);
    let failed = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        session
            .start(
                trigger,
                settings.language_policy,
                settings.transcription_engine,
                settings.cleanup_engine,
            )
            .and_then(|_| session.fail_with_delivery(error.to_string(), delivery))
            .map_err(|transition| transition.to_string())?
    };

    if let Err(show_error) = hud::show(app, hud_position) {
        eprintln!("target check failed but the HUD could not be shown: {show_error}");
    }
    if let Err(emit_error) = emit_state(app, &failed) {
        eprintln!("could not emit text-target failure: {emit_error}");
    }
    hide_after(app, FAILURE_HUD_DWELL, failed.revision);
    Err(error.to_string())
}

pub(crate) fn stop_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> Result<DictationStateEvent, String> {
    let (processing, finalizer) = {
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
        (processing, finalizer)
    };

    if let Err(error) = emit_state(app, &processing) {
        eprintln!("could not emit processing state: {error}");
    }

    // Permission waits, stream teardown, resampling handoff, and the terminal
    // transition all run off the shortcut callback. The response above remains
    // Processing; the emitted terminal revision is authoritative.
    let worker_app = app.clone();
    let worker_processing = processing.clone();
    let spawn_result = thread::Builder::new()
        .name("spick-capture-finalize".into())
        .spawn(move || finalize_capture(&worker_app, worker_processing, finalizer));
    if let Err(error) = spawn_result {
        let message = format!("could not start microphone finalization: {error}");
        discard_target_for_session(state, &active_session_id(&processing)?);
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
    processing: DictationStateEvent,
    finalizer: Result<CaptureFinalizer, String>,
) {
    let session_id = match active_session_id(&processing) {
        Ok(session_id) => session_id,
        Err(error) => {
            eprintln!("could not identify the transcription session: {error}");
            return;
        }
    };
    let capture_result = finalizer.and_then(CaptureFinalizer::finalize);
    match capture_result {
        Ok(capture) => {
            let status = capture.status();
            let sample_count = capture.pcm_16khz().len();

            if sample_count == 0 || status.sample_count == 0 {
                fail_and_emit_if_matching(
                    app,
                    &session_id,
                    "no microphone audio was captured".into(),
                );
                return;
            }

            let Some(session) = processing.session.as_ref() else {
                fail_and_emit_if_matching(
                    app,
                    &session_id,
                    "dictation settings were unavailable".into(),
                );
                return;
            };
            let state = app.state::<AppState>();
            let result = transcribe_capture(state.inner(), session, capture.pcm_16khz());

            match result {
                Ok(transcript) if transcript.transcript.text.trim().is_empty() => {
                    fail_and_emit_if_matching(
                        app,
                        &session_id,
                        "No speech was recognized. Try again a little closer to the microphone."
                            .into(),
                    );
                }
                Ok(transcript) => {
                    let inserting = match claim_session_insertion(state.inner(), &session_id) {
                        Ok(Some(inserting)) => inserting,
                        Ok(None) => {
                            discard_target_for_session(state.inner(), &session_id);
                            return;
                        }
                        Err(error) => {
                            fail_and_emit_if_matching(app, &session_id, error);
                            return;
                        }
                    };
                    if let Err(error) = emit_state(app, &inserting) {
                        eprintln!("could not emit insertion state: {error}");
                    }

                    let delivery =
                        deliver_transcript(state.inner(), &session_id, &transcript.transcript.text);
                    let transcript = transcript.finish(delivery.clone());
                    match complete_session_with_transcript(state.inner(), &session_id, transcript) {
                        Ok(Some((completed, transcript))) => {
                            if transcript.delivery.transcript_available {
                                if let Err(error) = app.emit_to(
                                    MAIN_WINDOW_LABEL,
                                    DICTATION_TRANSCRIPT_EVENT,
                                    &transcript,
                                ) {
                                    eprintln!("could not emit completed transcript: {error}");
                                }
                            }
                            if let Err(error) = emit_state(app, &completed) {
                                eprintln!("could not emit transcription completion: {error}");
                            }
                            match delivery.status {
                                DictationDeliveryStatus::Inserted => {
                                    hide_after(app, SUCCESS_HUD_DWELL, completed.revision)
                                }
                                DictationDeliveryStatus::SecureField => {
                                    hide_after(app, FAILURE_HUD_DWELL, completed.revision)
                                }
                                DictationDeliveryStatus::FocusChanged
                                | DictationDeliveryStatus::AccessibilityMissing
                                | DictationDeliveryStatus::Unsupported
                                | DictationDeliveryStatus::Failed
                                | DictationDeliveryStatus::Indeterminate => {}
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            eprintln!("could not complete transcription session: {error}")
                        }
                    }
                }
                Err(EngineError::Cancelled) => {}
                Err(error) => {
                    fail_and_emit_if_matching(app, &session_id, engine_error_message(error))
                }
            }
        }
        Err(error) => fail_and_emit_if_matching(app, &session_id, error),
    }
}

fn engine_error_message(error: EngineError) -> String {
    match error {
        EngineError::InvalidRequest(reason)
        | EngineError::Backend(reason)
        | EngineError::InvalidResult(reason) => reason,
        EngineError::UnsupportedPolicy(reason) => reason.to_string(),
        EngineError::Cancelled => "Dictation was cancelled".into(),
    }
}

fn transcribe_capture(
    state: &AppState,
    session: &DictationSession,
    pcm_16khz: &[f32],
) -> Result<PendingDictationTranscript, EngineError> {
    if session.transcription_engine.provider != EngineProvider::WhisperCpp
        || session.transcription_engine.location != EngineLocation::Local
    {
        return Err(EngineError::Backend(
            "the selected transcription engine is not connected yet".into(),
        ));
    }

    let model =
        resolve_curated_whisper_model(&session.transcription_engine.model).ok_or_else(|| {
            EngineError::InvalidRequest(format!(
                "unknown local model: {}",
                session.transcription_engine.model
            ))
        })?;
    let cancellation = state
        .transcription_cancellation(&session.id)
        .map_err(EngineError::Backend)?
        .ok_or(EngineError::Cancelled)?;
    let model_path = state
        .models
        .verified_model_path_cancellable(&model.id, cancellation.as_ref())
        .map_err(|error| {
            if cancellation.load(std::sync::atomic::Ordering::Relaxed) {
                EngineError::Cancelled
            } else {
                EngineError::Backend(error)
            }
        })?;
    let mut result = state.whisper.transcribe(
        Arc::clone(&model),
        &model_path,
        TranscriptionRequest {
            audio: AudioInput {
                samples: pcm_16khz,
                sample_rate_hz: 16_000,
                channels: 1,
            },
            language_policy: &session.language_policy,
            vocabulary: &[],
            cancellation: Some(cancellation.as_ref()),
        },
    )?;
    apply_configured_cleanup(session.cleanup_engine.as_ref(), &mut result)?;

    Ok(PendingDictationTranscript {
        session_id: session.id.clone(),
        engine_id: model.id.clone(),
        transcript: result,
    })
}

fn apply_configured_cleanup(
    cleanup_engine: Option<&EngineConfig>,
    transcript: &mut TranscriptResult,
) -> Result<(), EngineError> {
    let Some(cleanup_engine) = cleanup_engine else {
        return Ok(());
    };
    if !cleanup_engine.is_builtin_readable_cleanup() {
        return Err(EngineError::InvalidRequest(
            "the selected cleanup engine is not connected yet".into(),
        ));
    }

    let cleanup = RuleBasedCleanupEngine::default().cleanup(CleanupRequest {
        transcript,
        output_language: None,
    })?;
    if cleanup.changed {
        transcript.text = cleanup.text;
        // Timing segments describe the raw recognizer output. Once tokens are
        // removed, keeping them would expose stale filler text and imply
        // offsets that no longer match the delivered transcript.
        transcript.segments.clear();
    }
    Ok(())
}

fn claim_session_insertion(
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
    let Some(operation) = state.transcription_operation(session_id)? else {
        return Ok(None);
    };
    if operation
        .cancellation
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Ok(None);
    }
    session
        .begin_insertion()
        .map(Some)
        .map_err(|error| error.to_string())
}

fn deliver_transcript(state: &AppState, session_id: &str, transcript: &str) -> DictationDelivery {
    let operation = state.transcription_operation(session_id);
    let Ok(Some(operation)) = operation else {
        return DictationDelivery {
            status: DictationDeliveryStatus::Failed,
            transcript_available: true,
            target_app: None,
            caret_repositioned: None,
        };
    };
    let target = operation.target;
    if operation
        .cancellation
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return DictationDelivery {
            status: DictationDeliveryStatus::Failed,
            transcript_available: true,
            target_app: target.and_then(|target| target.target_app),
            caret_repositioned: None,
        };
    }

    let Some(target) = target else {
        return DictationDelivery {
            status: DictationDeliveryStatus::Unsupported,
            transcript_available: true,
            target_app: None,
            caret_repositioned: None,
        };
    };
    let target_app = target.target_app.clone();
    match state.text_targets.commit(target.token, transcript) {
        Ok(receipt) => DictationDelivery {
            status: DictationDeliveryStatus::Inserted,
            transcript_available: true,
            target_app: receipt.target_app.or(target_app),
            caret_repositioned: Some(receipt.caret_repositioned),
        },
        Err(error) => delivery_for_target_error(&error, true, target_app),
    }
}

fn delivery_for_target_error(
    error: &TextTargetError,
    transcript_available: bool,
    target_app: Option<String>,
) -> DictationDelivery {
    let status = match error.kind {
        TextTargetErrorKind::AccessibilityMissing => DictationDeliveryStatus::AccessibilityMissing,
        TextTargetErrorKind::SecureField => DictationDeliveryStatus::SecureField,
        TextTargetErrorKind::FocusChanged
        | TextTargetErrorKind::SelectionChanged
        | TextTargetErrorKind::ContentChanged
        | TextTargetErrorKind::TargetGone => DictationDeliveryStatus::FocusChanged,
        TextTargetErrorKind::NoFocusedTarget
        | TextTargetErrorKind::OwnApplication
        | TextTargetErrorKind::NotEditable
        | TextTargetErrorKind::Unsupported => DictationDeliveryStatus::Unsupported,
        TextTargetErrorKind::Indeterminate => DictationDeliveryStatus::Indeterminate,
        TextTargetErrorKind::TimedOut | TextTargetErrorKind::Platform => {
            DictationDeliveryStatus::Failed
        }
    };
    DictationDelivery {
        status,
        transcript_available: transcript_available
            && status != DictationDeliveryStatus::SecureField,
        target_app,
        caret_repositioned: None,
    }
}

fn handle_audio_capture_failure<R: Runtime>(app: &AppHandle<R>, failure: AudioCaptureFailure) {
    let state = app.state::<AppState>();
    discard_target_for_session(state.inner(), &failure.session_id);
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
            state.finish_transcription(&failure.session_id)?;
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
        SessionState::Listening | SessionState::Processing | SessionState::Inserting
    ) && event
        .session
        .as_ref()
        .is_some_and(|session| session.id == session_id)
}

fn complete_session_with_transcript(
    state: &AppState,
    session_id: &str,
    mut transcript: DictationTranscript,
) -> Result<Option<(DictationStateEvent, DictationTranscript)>, String> {
    let mut session = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?;
    let snapshot = session.snapshot();
    if snapshot.state != SessionState::Inserting || !session_matches(&snapshot, session_id) {
        return Ok(None);
    }
    let mut delivery = transcript.delivery.clone();
    if delivery.transcript_available {
        if !state.complete_transcription(transcript.clone())? {
            delivery.status = DictationDeliveryStatus::Failed;
            delivery.transcript_available = false;
            delivery.caret_repositioned = None;
            transcript.delivery = delivery.clone();
            state.finish_transcription(session_id)?;
        }
    } else {
        state.finish_transcription(session_id)?;
    }
    let completed = session
        .complete(delivery)
        .map_err(|error| error.to_string())?;
    Ok(Some((completed, transcript)))
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
    state.finish_transcription(&session_id)?;
    session
        .fail(error)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn fail_and_emit_if_matching<R: Runtime>(app: &AppHandle<R>, session_id: &str, error: String) {
    let state = app.state::<AppState>();
    discard_target_for_session(state.inner(), session_id);
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

fn discard_target_for_session(state: &AppState, session_id: &str) {
    let target = state
        .transcription_operation(session_id)
        .ok()
        .flatten()
        .and_then(|operation| operation.target);
    if let Some(target) = target {
        state.text_targets.discard(target.token);
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
    use crate::domain::{AppSettings, DictationSession, LanguagePolicy};

    fn event(id: &str, state: SessionState, revision: u64) -> DictationStateEvent {
        DictationStateEvent {
            revision,
            state,
            session: Some(DictationSession {
                id: id.into(),
                state,
                trigger: SessionTrigger::Shortcut,
                language_policy: LanguagePolicy::Auto,
                transcription_engine: AppSettings::default().transcription_engine,
                cleanup_engine: AppSettings::default().cleanup_engine,
                started_at_ms: 1,
                ended_at_ms: None,
                cancel_reason: None,
                error: None,
                delivery: None,
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

    #[test]
    fn backend_errors_are_shown_without_internal_engine_prefixes() {
        assert_eq!(
            engine_error_message(EngineError::Backend("Download a model first".into())),
            "Download a model first"
        );
    }

    #[test]
    fn no_cleanup_engine_preserves_as_spoken_text_exactly() {
        let mut transcript = TranscriptResult::final_text("Um,  keep this as spoken.");
        transcript.detected_language = Some("en".into());

        apply_configured_cleanup(None, &mut transcript).unwrap();

        assert_eq!(transcript.text, "Um,  keep this as spoken.");
    }

    #[test]
    fn configured_readable_cleanup_changes_the_live_transcript_text() {
        let cleanup_engine = EngineConfig::local(
            EngineProvider::BuiltIn,
            crate::domain::BUILTIN_READABLE_CLEANUP_MODEL,
        );
        let mut transcript = TranscriptResult::final_text("Um,  this is, uh, ready.");
        transcript.detected_language = Some("en-US".into());
        transcript.segments.push(crate::engines::TranscriptSegment {
            text: transcript.text.clone(),
            start_ms: 0,
            end_ms: 800,
            language: Some("en".into()),
            confidence: None,
        });

        apply_configured_cleanup(Some(&cleanup_engine), &mut transcript).unwrap();

        assert_eq!(transcript.text, "this is ready.");
        assert!(transcript.segments.is_empty());
    }

    #[test]
    fn live_pipeline_rejects_an_unconnected_polishing_engine() {
        let unsupported = EngineConfig::local(EngineProvider::LlamaCpp, "local-polisher");
        let mut transcript = TranscriptResult::final_text("Um, hello.");
        transcript.detected_language = Some("en".into());

        assert_eq!(
            apply_configured_cleanup(Some(&unsupported), &mut transcript),
            Err(EngineError::InvalidRequest(
                "the selected cleanup engine is not connected yet".into()
            ))
        );
        assert_eq!(transcript.text, "Um, hello.");
    }

    #[test]
    fn english_only_activation_explicitly_pins_speech_to_english() {
        let english_only = resolve_curated_whisper_model("whisper-base-english-q5-1").unwrap();
        assert_eq!(
            policy_for_model_activation(&LanguagePolicy::Auto, &english_only),
            LanguagePolicy::Fixed {
                language: "en".into()
            }
        );

        let hindi = LanguagePolicy::Fixed {
            language: "hi".into(),
        };
        assert_eq!(
            policy_for_model_activation(&hindi, &english_only),
            LanguagePolicy::Fixed {
                language: "en".into()
            }
        );
    }

    #[test]
    fn in_flight_session_keeps_its_snapshotted_model() {
        let mut active = event("session-a", SessionState::Processing, 2);
        active.session.as_mut().unwrap().transcription_engine =
            EngineConfig::local(EngineProvider::WhisperCpp, "whisper-tiny-multilingual-f16");

        assert!(session_uses_model(&active, "whisper-tiny-multilingual-f16"));
        assert!(!session_uses_model(
            &active,
            "whisper-small-multilingual-q5-1"
        ));
        active.state = SessionState::Completed;
        assert!(!session_uses_model(
            &active,
            "whisper-tiny-multilingual-f16"
        ));
    }
}
