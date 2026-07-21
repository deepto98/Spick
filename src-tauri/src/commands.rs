use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, TryLockError,
    },
    thread,
    time::{Duration, Instant},
};

use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, FilePath};

use crate::{
    audio::{
        self, AudioCaptureFailure, AudioCaptureReady, AudioCaptureStatus, AudioInputDevice,
        CaptureFinalizer, ErrorSink, LevelSink, ReadySink, AUDIO_LEVEL_EVENT,
    },
    cloud::{
        provider_for_engine, validate_provider_language_policy, CloudProviderId, CloudTranscription,
    },
    domain::{
        AppSettings, DictationDelivery, DictationDeliveryStatus, DictationSession,
        DictationStateEvent, EngineConfig, EngineLocation, EngineProvider, HudPresentation,
        HudSettings, LanguagePolicy, SessionState, SessionTrigger,
    },
    engines::{
        validate_whisper_model_policy, AudioInput, CleanupEngine, CleanupRequest,
        DictationTranscript, EngineError, ModelLanguageSet, RuleBasedCleanupEngine,
        TranscriptResult, TranscriptionRequest, WhisperModelManifest,
    },
    hud,
    latency::{
        DictationLatencyEvent, DictationLatencyOutcome, ProcessingLatencyTrace,
        StartupLatencyTrace, DICTATION_LATENCY_EVENT,
    },
    local_data::{
        ClearLocalDataResult, ClearLocalDataScope, CompletedDictationRecord,
        DeleteVocabularyResult, HistoryCursor, HistoryPage, LocalDataChangedEvent, LocalDataDomain,
        RecordOutcome, UsageDashboard, VocabularyEntryDto, VocabularyInput,
        LOCAL_DATA_CHANGED_EVENT,
    },
    microphone_permission::{self, MicrophonePermissionState, MicrophonePermissionStatus},
    model_store::{LocalModelSummary, ModelDownloadProgress, MODEL_DOWNLOAD_PROGRESS_EVENT},
    platform::{self, TextTargetError, TextTargetErrorKind, TextTargetToken},
    session::SessionController,
    shortcut,
    state::{AppState, HudTargetProtectionLease},
};

#[cfg(test)]
use crate::engines::resolve_curated_whisper_model;

pub const DICTATION_STATE_EVENT: &str = "dictation://state";
pub const DICTATION_TRANSCRIPT_EVENT: &str = "dictation://transcript";
const MAIN_WINDOW_LABEL: &str = "main";
const SUCCESS_HUD_DWELL: Duration = Duration::from_millis(650);
const FAILURE_HUD_DWELL: Duration = Duration::from_secs(2);
const HUD_DRAG_INITIAL_SETTLE: Duration = Duration::from_millis(300);
const HUD_DRAG_POLL: Duration = Duration::from_millis(100);
const HUD_DRAG_STABLE_POLLS: usize = 3;
const HUD_DRAG_MAX_POLLS: usize = 50;
const HUD_SETTLE_RETRY_DELAY: Duration = Duration::from_millis(25);
const HUD_SETTLE_MAX_RETRIES: usize = 80;
static HUD_DRAG_REVISION: AtomicU64 = AtomicU64::new(0);

struct PendingDictationTranscript {
    session_id: String,
    engine_id: String,
    transcript: crate::engines::TranscriptResult,
}

struct AudioFailureTransition {
    event: DictationStateEvent,
    cleanup: Option<CaptureFinalizer>,
    target: Option<TextTargetToken>,
    hud_target_lease: Option<HudTargetProtectionLease>,
}

#[derive(Default)]
struct StartPublicationGate {
    published: Mutex<bool>,
    changed: Condvar,
}

impl StartPublicationGate {
    fn wait(&self) {
        let mut published = self
            .published
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*published {
            published = self
                .changed
                .wait(published)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn publish(&self) {
        let mut published = self
            .published
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *published = true;
        self.changed.notify_all();
    }
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
pub fn get_usage_dashboard(
    window: WebviewWindow,
    state: State<'_, AppState>,
    days: u16,
) -> Result<UsageDashboard, String> {
    require_main_window(&window)?;
    state.local_data.usage_dashboard(days)
}

#[tauri::command]
pub fn list_transcript_history(
    window: WebviewWindow,
    state: State<'_, AppState>,
    cursor: Option<HistoryCursor>,
    limit: Option<u16>,
) -> Result<HistoryPage, String> {
    require_main_window(&window)?;
    state.local_data.transcript_history(cursor, limit)
}

#[tauri::command]
pub fn list_vocabulary(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Vec<VocabularyEntryDto>, String> {
    require_main_window(&window)?;
    state.local_data.list_vocabulary()
}

#[tauri::command]
pub fn create_vocabulary_entry(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    input: VocabularyInput,
) -> Result<VocabularyEntryDto, String> {
    require_main_window(&window)?;
    let entry = state.local_data.create_vocabulary(input)?;
    emit_local_data_changed(&app, state.inner(), vec![LocalDataDomain::Vocabulary]);
    Ok(entry)
}

#[tauri::command]
pub fn update_vocabulary_entry(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    input: VocabularyInput,
) -> Result<VocabularyEntryDto, String> {
    require_main_window(&window)?;
    let entry = state.local_data.update_vocabulary(&id, input)?;
    emit_local_data_changed(&app, state.inner(), vec![LocalDataDomain::Vocabulary]);
    Ok(entry)
}

#[tauri::command]
pub fn delete_vocabulary_entry(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<DeleteVocabularyResult, String> {
    require_main_window(&window)?;
    let result = state.local_data.delete_vocabulary(&id)?;
    if result.deleted {
        emit_local_data_changed(&app, state.inner(), vec![LocalDataDomain::Vocabulary]);
    }
    Ok(result)
}

#[tauri::command]
pub fn clear_local_data(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    scope: ClearLocalDataScope,
) -> Result<ClearLocalDataResult, String> {
    require_main_window(&window)?;
    // This uses the settings/history gate so an acknowledged history opt-out
    // cannot race with a final transcript write.
    let _settings_update = state
        .settings_update
        .lock()
        .map_err(|_| "settings update is unavailable".to_string())?;
    // A pre-commit storage/checkpoint failure leaves memory untouched. Once
    // this returns, database deletion has committed; a post-commit WAL warning
    // is carried in the successful result rather than hidden by an error.
    let mut result = state.local_data.clear(scope)?;
    if scope.clears_history() || scope.clears_usage() {
        apply_latest_clear_outcome(&mut result, state.clear_latest_transcript());
    }
    let mut domains = Vec::with_capacity(3);
    // History count is part of UsageDashboard, so a history-only clear also
    // invalidates the usage domain.
    if scope.clears_usage() || scope.clears_history() {
        domains.push(LocalDataDomain::Usage);
    }
    if scope.clears_history() {
        domains.push(LocalDataDomain::History);
    }
    if scope.clears_vocabulary() {
        domains.push(LocalDataDomain::Vocabulary);
    }
    emit_local_data_changed(&app, state.inner(), domains);
    Ok(result)
}

fn apply_latest_clear_outcome(
    result: &mut ClearLocalDataResult,
    outcome: Result<Option<String>, String>,
) {
    match outcome {
        Ok(session_id) => {
            result.cleared_latest_transcript = session_id.is_some();
            result.cleared_latest_session_id = session_id;
        }
        Err(error) => {
            // SQLite deletion has already committed. Preserve successful
            // counts/event delivery and make the rare poisoned-memory failure
            // explicit instead of turning committed work into a command Err.
            result.memory_cleanup_complete = false;
            result.memory_cleanup_warning = Some(format!(
                "local database rows were deleted, but the in-memory latest transcript could not be cleared: {error}"
            ));
        }
    }
}

#[tauri::command]
pub fn update_settings(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: AppSettings,
) -> Result<AppSettings, String> {
    require_main_window(&window)?;
    settings.validate()?;
    shortcut::validate(&settings.push_to_talk_shortcut)?;
    let requested_hud_visibility = settings.hud.visible;

    // Dashboard saves are serialized by a lock the dictation worker never
    // acquires. Do not hold either the settings or model locks while replacing
    // a shortcut: stopping Option joins a worker which may be reading both.
    let _settings_update = state
        .settings_update
        .lock()
        .map_err(|_| "settings update is unavailable".to_string())?;
    let previous = {
        let _model_configuration = state
            .model_configuration
            .lock()
            .map_err(|_| "model configuration is unavailable".to_string())?;
        let current = state
            .settings
            .read()
            .map_err(|_| "settings lock is poisoned".to_string())?;
        // HUD geometry is owned by the HUD webview. Ignore the dashboard's
        // potentially stale copy and begin with the latest native state.
        settings.hud = current.hud.clone();
        settings.hud.visible = requested_hud_visibility;
        if settings.transcription_engine != current.transcription_engine {
            return Err("choose transcription models from Engines".into());
        }
        validate_selected_transcription(&state.models, &settings)?;
        current.clone()
    };
    let shortcut_changed = previous.push_to_talk_shortcut != settings.push_to_talk_shortcut;
    let hud_visibility_change = hud_visibility_update(previous.hud.visible, settings.hud.visible);
    // A visibility failure must leave the dashboard on its last acknowledged
    // value. Snapshot actual window visibility before any settings or shortcut
    // state changes so a partially completed native action can be restored.
    let hud_was_visible = match hud_visibility_change {
        HudVisibilityUpdate::Unchanged => false,
        HudVisibilityUpdate::DisableNow | HudVisibilityUpdate::EnableNowIfIdle => {
            hud::is_visible(&app)?
        }
    };

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

    let commit = (|| {
        let _model_configuration = state
            .model_configuration
            .lock()
            .map_err(|_| "model configuration is unavailable".to_string())?;
        let mut current = state
            .settings
            .write()
            .map_err(|_| "settings lock is poisoned".to_string())?;
        let next = rebase_settings_update(&previous, &settings, &current)?;
        validate_selected_transcription(&state.models, &next)?;
        state.persist_settings(&next)?;
        *current = next.clone();
        Ok::<AppSettings, String>(next)
    })();

    match commit {
        Ok(next) => {
            reconcile_committed_hud_visibility(
                hud_visibility_change,
                || show_idle_hud_if_inactive(&app, state.inner(), &next.hud).map(|_| ()),
                || hud::hide(&app),
                || {
                    rollback_failed_hud_visibility(
                        &app,
                        state.inner(),
                        &previous,
                        &next,
                        shortcut_changed,
                        hud_was_visible,
                    )
                },
            )?;
            Ok(next)
        }
        Err(error) => {
            // All AppState guards from the commit attempt are gone before a
            // rollback can stop and join an Option worker.
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
            Err(error)
        }
    }
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
pub async fn list_audio_input_devices(
    window: WebviewWindow,
) -> Result<Vec<AudioInputDevice>, String> {
    require_main_window(&window)?;
    drop(window);
    tauri::async_runtime::spawn_blocking(audio::list_input_devices)
        .await
        .map_err(|error| format!("microphone discovery worker failed: {error}"))?
}

#[tauri::command]
pub fn get_shortcut_status(window: WebviewWindow) -> Result<shortcut::ShortcutStatus, String> {
    require_main_window(&window)?;
    shortcut::status(window.app_handle())
}

#[tauri::command]
pub fn get_hud_settings(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<HudSettings, String> {
    require_hud_window(&window)?;
    Ok(state.settings_snapshot()?.hud)
}

#[tauri::command]
pub fn mark_hud_renderer_ready(window: WebviewWindow) -> Result<(), String> {
    require_hud_window(&window)?;
    hud::mark_renderer_ready(window.app_handle())
}

#[tauri::command]
pub fn set_hud_presentation(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    presentation: HudPresentation,
) -> Result<HudSettings, String> {
    require_hud_window(&window)?;
    let _settings_update = state
        .settings_update
        .lock()
        .map_err(|_| "settings update is unavailable".to_string())?;
    let mut current = state
        .settings
        .write()
        .map_err(|_| "settings lock is poisoned".to_string())?;
    let previous_hud = current.hud.clone();
    let mut next = current.clone();
    next.hud.presentation = presentation;

    hud::apply(&app, &next.hud)?;
    if let Err(error) = state.persist_settings(&next) {
        if let Err(rollback_error) = hud::apply(&app, &previous_hud) {
            return Err(format!(
                "{error}; the HUD geometry rollback also failed: {rollback_error}"
            ));
        }
        return Err(error);
    }

    *current = next.clone();
    Ok(next.hud)
}

#[tauri::command]
pub fn start_hud_drag(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_hud_window(&window)?;
    let revision = HUD_DRAG_REVISION.fetch_add(1, Ordering::AcqRel) + 1;
    let settings_update = state
        .settings_update
        .lock()
        .map_err(|_| "settings update is unavailable".to_string())?;
    let completed_synchronously = hud::start_drag(&app)?;
    if completed_synchronously {
        let position = hud::current_position(&app)?;
        persist_hud_position_with_gate(state.inner(), position)?;
        drop(settings_update);
    } else {
        // The portable drag API can return before the window stops moving. It
        // must not hold the settings gate while the settling watcher runs.
        drop(settings_update);
        schedule_hud_position_save(app, revision);
    }
    Ok(())
}

#[tauri::command]
pub fn request_input_monitoring_permission(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<bool, String> {
    require_main_window(&window)?;
    Ok(shortcut::request_input_monitoring_permission(&app))
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
pub fn get_last_dictation_latency(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<DictationLatencyEvent>, String> {
    require_main_window(&window)?;
    state.latest_dictation_latency()
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
pub async fn import_local_model(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<Option<LocalModelSummary>, String> {
    require_main_window(&window)?;
    let selected = window
        .dialog()
        .file()
        .set_title("Import a whisper.cpp model")
        .add_filter("whisper.cpp GGML model", &["bin"])
        .blocking_pick_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = match selected {
        FilePath::Path(path) => path,
        FilePath::Url(_) => {
            return Err("choose a model stored as a local file".into());
        }
    };
    let models = Arc::clone(&app.state::<AppState>().models);
    tauri::async_runtime::spawn_blocking(move || models.import_from_path(&path))
        .await
        .map_err(|error| format!("local model import worker failed: {error}"))?
        .map(Some)
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
    let model = app
        .state::<AppState>()
        .models
        .resolve(&model_id)
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
        commit_local_model_activation(state.inner(), &model, || {
            state.models.verified_model_path(&model.id).map(|_| ())
        })
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
    let model = app
        .state::<AppState>()
        .models
        .resolve(&model_id)
        .ok_or_else(|| format!("unknown local model: {model_id}"))?;
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        let _model_configuration = state
            .model_configuration
            .lock()
            .map_err(|_| "model configuration is unavailable".to_string())?;
        let settings = state.settings_snapshot()?;
        let active_id = state
            .models
            .resolve(&settings.transcription_engine.model)
            .map(|active| active.id.clone());
        if active_id.as_deref() == Some(model.id.as_str()) {
            return Err("choose another local model before removing the active one".into());
        }

        let in_use = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?
            .snapshot();
        if session_uses_model(&state.models, &in_use, &model.id) {
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
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DictationStateEvent, String> {
    start_session(
        &app,
        state.inner(),
        session_trigger_for_window(window.label())?,
    )
}

#[tauri::command]
pub fn set_onboarding_practice_mode(
    window: WebviewWindow,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    require_main_window(&window)?;
    state.onboarding_practice.store(enabled, Ordering::Release);
    Ok(())
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
    cancel_session(&app, state.inner(), reason)
}

pub(crate) fn cancel_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    reason: Option<String>,
) -> Result<DictationStateEvent, String> {
    let (event, cleanup, target, hud_target_lease, session_id) = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        let session_id = active_session_id(&session.snapshot())?;
        let operation = state.transcription_operation(&session_id)?;
        let target = operation
            .as_ref()
            .and_then(|operation| operation.target.as_ref())
            .map(|target| target.token);
        let hud_target_lease = operation.and_then(|operation| operation.hud_target_lease);
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
        (event, cleanup, target, hud_target_lease, session_id)
    };

    if let Some(target) = target {
        state.text_targets.discard(target);
    }
    release_hud_target(app, state, hud_target_lease.as_ref());
    discard_on_worker(cleanup);

    let emit_result = emit_state(app, &event);
    finish_dictation_latency(
        app,
        &session_id,
        event.revision,
        DictationLatencyOutcome::Cancelled,
        None,
    );
    settle_after(app, Duration::ZERO, event.revision);
    emit_result?;
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

#[tauri::command]
pub fn get_microphone_permission_status(
    window: WebviewWindow,
) -> Result<MicrophonePermissionStatus, String> {
    require_main_window(&window)?;
    microphone_permission::status()
}

#[tauri::command]
pub async fn request_microphone_permission(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<MicrophonePermissionStatus, String> {
    require_main_window(&window)?;
    drop(window);

    let current = microphone_permission::status()?;
    if current.state == MicrophonePermissionState::Missing && !current.can_request {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let _ = sender.send(microphone_permission::open_microphone_privacy_settings());
        })
        .map_err(|error| format!("could not open microphone settings: {error}"))?;
        tauri::async_runtime::spawn_blocking(move || {
            receiver
                .recv()
                .map_err(|_| "the microphone settings request stopped unexpectedly".to_string())?
        })
        .await
        .map_err(|error| format!("microphone settings worker failed: {error}"))??;
        return microphone_permission::status();
    }
    if current.state != MicrophonePermissionState::Missing || !current.can_request {
        return Ok(current);
    }

    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let callback_sender = sender.clone();
        if let Err(error) = microphone_permission::request_access(move |result| {
            let _ = callback_sender.send(result);
        }) {
            let _ = sender.send(Err(error));
        }
    })
    .map_err(|error| format!("could not request microphone access: {error}"))?;

    tauri::async_runtime::spawn_blocking(move || {
        receiver
            .recv()
            .map_err(|_| "the microphone permission request stopped unexpectedly".to_string())?
    })
    .await
    .map_err(|error| format!("microphone permission worker failed: {error}"))?
}

fn require_main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("this command is only available from the Spick dashboard".into())
    }
}

fn require_hud_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == hud::HUD_WINDOW_LABEL {
        Ok(())
    } else {
        Err("this command is only available to the dictation HUD".into())
    }
}

fn schedule_hud_position_save<R: Runtime>(app: AppHandle<R>, revision: u64) {
    let spawn_result = thread::Builder::new()
        .name("spick-hud-position".into())
        .spawn(move || {
            thread::sleep(HUD_DRAG_INITIAL_SETTLE);
            let mut last_position = None;
            let mut stable_polls = 0;

            for _ in 0..HUD_DRAG_MAX_POLLS {
                if HUD_DRAG_REVISION.load(Ordering::Acquire) != revision {
                    return;
                }
                thread::sleep(HUD_DRAG_POLL);
                let Ok(position) = hud::current_position(&app) else {
                    continue;
                };
                if last_position == Some(position) {
                    stable_polls += 1;
                } else {
                    last_position = Some(position);
                    stable_polls = 0;
                }
                if stable_polls >= HUD_DRAG_STABLE_POLLS {
                    break;
                }
            }

            let Some(position) = last_position else {
                return;
            };
            if let Err(error) = persist_hud_position_if_current(&app, position, revision) {
                eprintln!("could not save the dictation HUD position: {error}");
            }
        });

    if let Err(error) = spawn_result {
        eprintln!("could not watch the dictation HUD drag: {error}");
    }
}

fn persist_hud_position_if_current<R: Runtime>(
    app: &AppHandle<R>,
    position: crate::domain::HudCoordinates,
    revision: u64,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _settings_update = state
        .settings_update
        .lock()
        .map_err(|_| "settings update is unavailable".to_string())?;
    // Recheck after acquiring the serialization gate. A newer drag may have
    // started while this watcher was waiting behind a presentation change.
    if HUD_DRAG_REVISION.load(Ordering::Acquire) != revision {
        return Ok(());
    }
    persist_hud_position_with_gate(state.inner(), position)
}

#[cfg(test)]
fn persist_hud_position_for_state(
    state: &AppState,
    position: crate::domain::HudCoordinates,
) -> Result<(), String> {
    let _settings_update = state
        .settings_update
        .lock()
        .map_err(|_| "settings update is unavailable".to_string())?;
    persist_hud_position_with_gate(state, position)
}

/// Persists a HUD position while the caller owns `settings_update`.
fn persist_hud_position_with_gate(
    state: &AppState,
    position: crate::domain::HudCoordinates,
) -> Result<(), String> {
    let mut current = state
        .settings
        .write()
        .map_err(|_| "settings lock is poisoned".to_string())?;
    if current.hud.custom_position == Some(position) {
        return Ok(());
    }
    let mut next = current.clone();
    next.hud.custom_position = Some(position);
    state.persist_settings(&next)?;
    *current = next;
    Ok(())
}

fn commit_local_model_activation<F>(
    state: &AppState,
    model: &crate::engines::WhisperModelManifest,
    verify_installed: F,
) -> Result<AppSettings, String>
where
    F: FnOnce() -> Result<(), String>,
{
    let _settings_update = state
        .settings_update
        .lock()
        .map_err(|_| "settings update is unavailable".to_string())?;
    let _model_configuration = state
        .model_configuration
        .lock()
        .map_err(|_| "model configuration is unavailable".to_string())?;
    verify_installed()?;

    let mut current = state
        .settings
        .write()
        .map_err(|_| "settings lock is poisoned".to_string())?;
    let mut updated = current.clone();
    updated.language_policy = policy_for_model_activation(&updated.language_policy, model);
    updated.transcription_engine =
        EngineConfig::local(EngineProvider::WhisperCpp, model.id.clone());
    updated.validate()?;
    validate_selected_transcription(&state.models, &updated)?;
    state.persist_settings(&updated)?;
    *current = updated.clone();
    Ok(updated)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HudVisibilityUpdate {
    Unchanged,
    DisableNow,
    /// Show immediately when no session owns a captured target. Otherwise the
    /// terminal settle path reveals the newly enabled persistent widget.
    EnableNowIfIdle,
}

fn hud_visibility_update(previous: bool, next: bool) -> HudVisibilityUpdate {
    match (previous, next) {
        (true, false) => HudVisibilityUpdate::DisableNow,
        (false, true) => HudVisibilityUpdate::EnableNowIfIdle,
        _ => HudVisibilityUpdate::Unchanged,
    }
}

fn reconcile_committed_hud_visibility<FShow, FHide, FRollback>(
    update: HudVisibilityUpdate,
    show: FShow,
    hide: FHide,
    rollback: FRollback,
) -> Result<(), String>
where
    FShow: FnOnce() -> Result<(), String>,
    FHide: FnOnce() -> Result<(), String>,
    FRollback: FnOnce() -> Result<(), String>,
{
    let result = match update {
        HudVisibilityUpdate::Unchanged => return Ok(()),
        HudVisibilityUpdate::DisableNow => hide(),
        HudVisibilityUpdate::EnableNowIfIdle => show(),
    };
    let Err(action_error) = result else {
        return Ok(());
    };
    let operation = match update {
        HudVisibilityUpdate::DisableNow => "hidden",
        HudVisibilityUpdate::EnableNowIfIdle => "shown",
        HudVisibilityUpdate::Unchanged => unreachable!(),
    };
    match rollback() {
        Ok(()) => Err(format!(
            "floating HUD settings were not changed because the HUD could not be {operation}: {action_error}"
        )),
        Err(rollback_error) => Err(format!(
            "the HUD could not be {operation}: {action_error}; settings rollback also failed: {rollback_error}"
        )),
    }
}

fn rollback_failed_hud_visibility<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    previous: &AppSettings,
    attempted: &AppSettings,
    shortcut_changed: bool,
    hud_was_visible: bool,
) -> Result<(), String> {
    let mut rollback_errors = Vec::new();

    if let Err(error) = restore_settings_snapshot(state, previous) {
        rollback_errors.push(format!("settings: {error}"));
    }
    if shortcut_changed {
        if let Err(error) = shortcut::replace(
            app,
            &attempted.push_to_talk_shortcut,
            &previous.push_to_talk_shortcut,
        ) {
            rollback_errors.push(format!("shortcut: {error}"));
        }
    }
    if hud_was_visible {
        let target_is_live = state
            .hud_target_protection
            .lock()
            .map(|protection| protection.has_owner())
            .map_err(|_| "HUD target protection is unavailable".to_string());
        match target_is_live
            .and_then(|target_is_live| hud::show(app, &previous.hud, target_is_live))
        {
            Ok(()) => {}
            Err(error) => rollback_errors.push(format!("HUD visibility: {error}")),
        }
    } else if let Err(error) = hud::hide(app) {
        rollback_errors.push(format!("HUD visibility: {error}"));
    }

    if rollback_errors.is_empty() {
        Ok(())
    } else {
        Err(rollback_errors.join("; "))
    }
}

fn show_idle_hud_if_inactive<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    settings: &HudSettings,
) -> Result<bool, String> {
    // Hold both guards through the native show so a shortcut cannot capture a
    // target between the idle check and panel presentation.
    let session = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?;
    if matches!(
        session.snapshot().state,
        SessionState::Starting
            | SessionState::Listening
            | SessionState::Processing
            | SessionState::Inserting
    ) {
        return Ok(false);
    }
    let protection = state
        .hud_target_protection
        .lock()
        .map_err(|_| "HUD target protection is unavailable".to_string())?;
    if protection.has_owner() {
        return Ok(false);
    }
    hud::show(app, settings, false)?;
    Ok(true)
}

fn restore_settings_snapshot(state: &AppState, settings: &AppSettings) -> Result<(), String> {
    let _model_configuration = state
        .model_configuration
        .lock()
        .map_err(|_| "model configuration is unavailable".to_string())?;
    let mut current = state
        .settings
        .write()
        .map_err(|_| "settings lock is poisoned".to_string())?;
    state.persist_settings(settings)?;
    *current = settings.clone();
    Ok(())
}

/// Rebases a dashboard settings edit onto native-owned HUD state.
///
/// Every production settings writer holds `settings_update`, so `current`
/// should remain stable while the shortcut backend is replaced. The comparison
/// remains a defensive guard, and the HUD copy is always native-owned rather
/// than trusted from a potentially stale dashboard payload.
fn rebase_settings_update(
    previous: &AppSettings,
    requested: &AppSettings,
    current: &AppSettings,
) -> Result<AppSettings, String> {
    let mut comparable = current.clone();
    comparable.hud = previous.hud.clone();
    if comparable != *previous {
        return Err("settings changed while this update was being applied; try again".into());
    }

    let mut next = requested.clone();
    next.hud = current.hud.clone();
    next.hud.visible = requested.hud.visible;
    Ok(next)
}

fn validate_selected_transcription(
    models: &crate::model_store::ModelStore,
    settings: &AppSettings,
) -> Result<(), String> {
    if let Some(provider) = provider_for_engine(&settings.transcription_engine) {
        return validate_provider_language_policy(provider, &settings.language_policy);
    }
    if settings.transcription_engine.location == EngineLocation::Cloud {
        return Err("The selected cloud transcription engine is not supported.".into());
    }
    if settings.transcription_engine.provider != EngineProvider::WhisperCpp
        || settings.transcription_engine.location != EngineLocation::Local
    {
        return Ok(());
    }

    let model = models
        .resolve(&settings.transcription_engine.model)
        .ok_or_else(|| {
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

fn validate_dictation_engine_readiness<CloudReady, LocalReady>(
    models: &crate::model_store::ModelStore,
    settings: &AppSettings,
    ensure_cloud_ready: CloudReady,
    ensure_local_ready: LocalReady,
) -> Result<(), String>
where
    CloudReady: FnOnce(CloudProviderId) -> Result<(), String>,
    LocalReady: FnOnce(&WhisperModelManifest) -> Result<(), String>,
{
    if let Some(provider) = provider_for_engine(&settings.transcription_engine) {
        validate_provider_language_policy(provider, &settings.language_policy)?;
        return ensure_cloud_ready(provider);
    }
    if settings.transcription_engine.location == EngineLocation::Cloud {
        return Err("The selected cloud transcription engine is not supported.".into());
    }
    if settings.transcription_engine.provider != EngineProvider::WhisperCpp
        || settings.transcription_engine.location != EngineLocation::Local
    {
        return Err("The selected transcription engine is not supported.".into());
    }

    let model = models
        .resolve(&settings.transcription_engine.model)
        .ok_or_else(|| {
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
    })?;
    ensure_local_ready(model.as_ref())
}

fn ensure_local_whisper_engine_ready<Verify, Load>(
    model: &WhisperModelManifest,
    verify: Verify,
    load: Load,
) -> Result<(), String>
where
    Verify: FnOnce(&str) -> Result<std::path::PathBuf, String>,
    Load: FnOnce(&WhisperModelManifest, &std::path::Path) -> Result<(), EngineError>,
{
    let model_path = verify(&model.id)?;
    load(model, &model_path).map_err(engine_error_message)
}

fn preflight_selected_transcription_engine(
    state: &AppState,
    settings: &AppSettings,
) -> Result<(), String> {
    validate_dictation_engine_readiness(
        &state.models,
        settings,
        |provider| state.cloud.ensure_credential_configured(provider),
        |model| {
            ensure_local_whisper_engine_ready(
                model,
                |model_id| state.models.verified_model_path(model_id),
                |model, path| state.whisper.load(model, path),
            )
        },
    )
}

fn session_uses_model(
    models: &crate::model_store::ModelStore,
    event: &DictationStateEvent,
    model_id: &str,
) -> bool {
    matches!(
        event.state,
        SessionState::Starting
            | SessionState::Listening
            | SessionState::Processing
            | SessionState::Inserting
    ) && event.session.as_ref().is_some_and(|session| {
        models
            .resolve(&session.transcription_engine.model)
            .is_some_and(|model| model.id == model_id)
    })
}

pub(crate) fn start_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    trigger: SessionTrigger,
) -> Result<DictationStateEvent, String> {
    let startup_latency = StartupLatencyTrace::start();
    if let Err(error) = microphone_permission::ensure_capture_allowed() {
        return fail_microphone_preflight(app, state, trigger, error, startup_latency);
    }

    // Keep the selected engine stable from this snapshot through microphone
    // ownership. A missing model or API key must fail before Spick observes an
    // external field, much less starts recording into a doomed session.
    let model_configuration = state
        .model_configuration
        .lock()
        .map_err(|_| "model configuration is unavailable".to_string())?;
    let settings = state.settings_snapshot()?;
    if let Err(error) = preflight_selected_transcription_engine(state, &settings) {
        drop(model_configuration);
        return fail_engine_preflight(app, state, trigger, settings, error, startup_latency);
    }

    // External-target sessions must prove a concrete, non-secure editable
    // target before any session state or audio capture is created. The HUD is
    // nonactivating, so a click there still leaves the other app focused.
    let captured_target = if trigger_uses_external_target(trigger) {
        startup_latency.mark_target_capture_started();
        let capture = state.text_targets.capture();
        startup_latency.mark_target_capture_returned();
        match capture {
            Ok(target) => Some(target),
            Err(error)
                if error.kind == TextTargetErrorKind::OwnApplication
                    && state.onboarding_practice.load(Ordering::Acquire) =>
            {
                None
            }
            Err(error) => {
                return fail_target_preflight(app, state, trigger, error, startup_latency)
            }
        }
    } else {
        // The dashboard owns focus for UI-triggered recordings. Those remain
        // an explicit transcript/copy test and never type into another app.
        None
    };
    let target_token = captured_target.as_ref().map(|target| target.token);
    let input_device_name = settings.input_device_name.clone();
    let language_policy = settings.language_policy;
    let transcription_engine = settings.transcription_engine;
    let cleanup_engine = settings.cleanup_engine;
    let allow_cloud_fallback = settings.allow_cloud_fallback;
    let hud_settings = settings.hud.clone();
    // Vocabulary is a per-session snapshot. Edits that win the SQLite read
    // race are included; every later edit applies to the next session only.
    let vocabulary = state.local_data.vocabulary_prompt_snapshot();
    let level_app = app.clone();
    let level_sink: LevelSink = Arc::new(move |payload| {
        if let Err(error) = level_app.emit(AUDIO_LEVEL_EVENT, payload) {
            eprintln!("could not emit microphone level: {error}");
        }
    });
    let start_publication = Arc::new(StartPublicationGate::default());
    let error_app = app.clone();
    let error_publication = Arc::clone(&start_publication);
    let error_sink: ErrorSink = Arc::new(move |failure| {
        error_publication.wait();
        handle_audio_capture_failure(&error_app, failure);
    });
    let ready_app = app.clone();
    let ready_publication = Arc::clone(&start_publication);
    let ready_latency = startup_latency.clone();
    let ready_sink: ReadySink = Arc::new(move |ready| {
        // Preserve the native play-return timestamp before waiting for HUD
        // publication. This trace is bound to this session only; a late
        // callback can neither update a replacement nor mutate a finished one.
        ready_latency.mark_microphone_ready_at(ready.stream_play_returned_at);
        ready_publication.wait();
        handle_audio_capture_ready(&ready_app, ready);
    });

    // The session/audio guards begin here and live only through microphone
    // owner startup. `model_configuration` began before target capture so the
    // engine proven ready above cannot be removed or replaced in between.
    let mut claimed_hud_target_lease = None;
    let transaction = (|| -> Result<(DictationStateEvent, Option<String>), String> {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        let mut audio = state
            .audio
            .lock()
            .map_err(|_| "microphone capture lock is poisoned".to_string())?;
        let starting = session
            .start(
                trigger,
                language_policy,
                transcription_engine,
                cleanup_engine,
            )
            .map_err(|error| error.to_string())?;
        let session_id = active_session_id(&starting)?;
        if !startup_latency.bind_session(&session_id) {
            eprintln!("could not bind dictation startup latency diagnostics");
        } else if let Err(error) = state.register_dictation_latency(startup_latency.clone()) {
            eprintln!("could not start dictation latency diagnostics: {error}");
        }
        let hud_target_lease = if captured_target.is_some() {
            match state.hud_target_protection.lock() {
                Ok(mut protection) => {
                    let lease = protection.claim(session_id.clone());
                    claimed_hud_target_lease = Some(lease.clone());
                    Some(lease)
                }
                Err(_) => {
                    let error = "HUD target protection is unavailable".to_string();
                    let failed = session
                        .fail(error.clone())
                        .map_err(|transition| transition.to_string())?;
                    return Ok((failed, Some(error)));
                }
            }
        } else {
            None
        };
        if let Err(error) = state.begin_transcription(
            session_id.clone(),
            captured_target.clone(),
            hud_target_lease.clone(),
            Arc::clone(&vocabulary),
            allow_cloud_fallback,
        ) {
            let failed = session
                .fail(error.clone())
                .map_err(|transition| transition.to_string())?;
            return Ok((failed, Some(error)));
        }

        match audio.start(
            session_id,
            input_device_name,
            level_sink,
            ready_sink,
            error_sink,
        ) {
            Ok(_) => {
                startup_latency.mark_audio_owner_spawned();
                Ok((starting, None))
            }
            Err(error) => {
                state.finish_transcription(&active_session_id(&starting)?)?;
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
            release_hud_target(app, state, claimed_hud_target_lease.as_ref());
            start_publication.publish();
            return Err(error);
        }
    };

    if let Some(error) = start_error {
        if let Some(target) = target_token {
            state.text_targets.discard(target);
        }
        release_hud_target(app, state, claimed_hud_target_lease.as_ref());
        match show_hud_for_target(app, state, &hud_settings, None) {
            Ok(true) => startup_latency.mark_hud_show_returned(),
            Ok(false) => {}
            Err(show_error) => {
                eprintln!("dictation failed but the HUD could not be shown: {show_error}")
            }
        }
        let _ = emit_state(app, &event);
        start_publication.publish();
        if let Ok(session_id) = active_session_id(&event) {
            finish_dictation_latency(
                app,
                &session_id,
                event.revision,
                DictationLatencyOutcome::Failed,
                None,
            );
        }
        settle_after(app, FAILURE_HUD_DWELL, event.revision);
        return Err(error);
    }

    if let Err(error) = emit_state(app, &event) {
        eprintln!("could not emit microphone starting state: {error}");
    } else {
        startup_latency.mark_starting_emitted();
    }
    match show_hud_for_target(app, state, &hud_settings, claimed_hud_target_lease.as_ref()) {
        Ok(true) => startup_latency.mark_hud_show_returned(),
        Ok(false) => {}
        Err(error) => eprintln!("dictation started but the HUD could not be shown: {error}"),
    }
    // Ready, failure, and timeout callbacks wait on this publication boundary,
    // so none can overtake the user-visible Starting revision.
    start_publication.publish();
    Ok(event)
}

fn fail_engine_preflight<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    trigger: SessionTrigger,
    settings: AppSettings,
    error: String,
    startup_latency: StartupLatencyTrace,
) -> Result<DictationStateEvent, String> {
    let hud_settings = settings.hud.clone();
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
            .and_then(|_| session.fail(error.clone()))
            .map_err(|transition| transition.to_string())?
    };
    let session_id = active_session_id(&failed)?;
    if !startup_latency.bind_session(&session_id) {
        eprintln!("could not bind engine preflight latency diagnostics");
    }

    match show_hud_for_target(app, state, &hud_settings, None) {
        Ok(true) => startup_latency.mark_hud_show_returned(),
        Ok(false) => {}
        Err(show_error) => {
            eprintln!("engine check failed but the HUD could not be shown: {show_error}")
        }
    }
    if let Err(emit_error) = emit_state(app, &failed) {
        eprintln!("could not emit transcription engine failure: {emit_error}");
    }
    emit_unregistered_dictation_latency(
        app,
        &startup_latency,
        failed.revision,
        DictationLatencyOutcome::Failed,
    );
    settle_after(app, FAILURE_HUD_DWELL, failed.revision);
    Err(error)
}

fn fail_microphone_preflight<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    trigger: SessionTrigger,
    error: String,
    startup_latency: StartupLatencyTrace,
) -> Result<DictationStateEvent, String> {
    let settings = state.settings_snapshot()?;
    let hud_settings = settings.hud.clone();
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
            .and_then(|_| session.fail(error.clone()))
            .map_err(|transition| transition.to_string())?
    };
    let session_id = active_session_id(&failed)?;
    if !startup_latency.bind_session(&session_id) {
        eprintln!("could not bind microphone preflight latency diagnostics");
    }

    match show_hud_for_target(app, state, &hud_settings, None) {
        Ok(true) => startup_latency.mark_hud_show_returned(),
        Ok(false) => {}
        Err(show_error) => {
            eprintln!("microphone check failed but the HUD could not be shown: {show_error}")
        }
    }
    if let Err(emit_error) = emit_state(app, &failed) {
        eprintln!("could not emit microphone permission failure: {emit_error}");
    }
    emit_unregistered_dictation_latency(
        app,
        &startup_latency,
        failed.revision,
        DictationLatencyOutcome::Failed,
    );
    settle_after(app, FAILURE_HUD_DWELL, failed.revision);
    Err(error)
}

fn session_trigger_for_window(label: &str) -> Result<SessionTrigger, String> {
    match label {
        MAIN_WINDOW_LABEL => Ok(SessionTrigger::UserInterface),
        hud::HUD_WINDOW_LABEL => Ok(SessionTrigger::FloatingWidget),
        _ => Err("dictation can only be started from a Spick window".into()),
    }
}

fn trigger_uses_external_target(trigger: SessionTrigger) -> bool {
    matches!(
        trigger,
        SessionTrigger::Shortcut | SessionTrigger::FloatingWidget
    )
}

fn fail_target_preflight<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    trigger: SessionTrigger,
    error: TextTargetError,
    startup_latency: StartupLatencyTrace,
) -> Result<DictationStateEvent, String> {
    let settings = state.settings_snapshot()?;
    let hud_settings = settings.hud.clone();
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
    let session_id = active_session_id(&failed)?;
    if !startup_latency.bind_session(&session_id) {
        eprintln!("could not bind text-target preflight latency diagnostics");
    }

    match show_hud_for_target(app, state, &hud_settings, None) {
        Ok(true) => startup_latency.mark_hud_show_returned(),
        Ok(false) => {}
        Err(show_error) => {
            eprintln!("target check failed but the HUD could not be shown: {show_error}")
        }
    }
    if let Err(emit_error) = emit_state(app, &failed) {
        eprintln!("could not emit text-target failure: {emit_error}");
    }
    emit_unregistered_dictation_latency(
        app,
        &startup_latency,
        failed.revision,
        DictationLatencyOutcome::Failed,
    );
    settle_after(app, FAILURE_HUD_DWELL, failed.revision);
    Err(error.to_string())
}

pub(crate) fn stop_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> Result<DictationStateEvent, String> {
    let microphone_is_starting = state
        .session
        .lock()
        .map_err(|_| "dictation session lock is poisoned".to_string())?
        .snapshot()
        .state
        == SessionState::Starting;
    if microphone_is_starting {
        return cancel_session(
            app,
            state,
            Some("Stopped before the microphone was ready".into()),
        );
    }

    // Start at the user's stop gesture, before locks, stream teardown, or the
    // worker handoff, so the total matches the delay they actually feel.
    let stop_requested_at = Instant::now();
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
    let processing_session_id = active_session_id(&processing)?;
    let mut latency =
        ProcessingLatencyTrace::start(processing_session_id.clone(), stop_requested_at);
    latency.mark_processing_emitted();
    let hud_target_lease = state
        .transcription_operation(&processing_session_id)?
        .and_then(|operation| operation.hud_target_lease);

    // Permission waits, stream teardown, resampling handoff, and the terminal
    // transition all run off the shortcut callback. The response above remains
    // Processing; the emitted terminal revision is authoritative.
    let worker_app = app.clone();
    let worker_processing = processing.clone();
    let worker_latency = latency.clone();
    let spawn_result = thread::Builder::new()
        .name("spick-capture-finalize".into())
        .spawn(move || finalize_capture(&worker_app, worker_processing, finalizer, worker_latency));
    if let Err(error) = spawn_result {
        let message = format!("could not start microphone finalization: {error}");
        discard_target_for_session(state, &processing_session_id);
        release_hud_target(app, state, hud_target_lease.as_ref());
        if let Some(failed) = fail_session_if_matching(state, &processing, message)? {
            let _ = emit_state(app, &failed);
            finish_dictation_latency(
                app,
                &processing_session_id,
                failed.revision,
                DictationLatencyOutcome::Failed,
                Some(latency),
            );
            settle_after(app, FAILURE_HUD_DWELL, failed.revision);
        }
    }

    Ok(processing)
}

fn emit_state<R: Runtime>(app: &AppHandle<R>, event: &DictationStateEvent) -> Result<(), String> {
    app.emit(DICTATION_STATE_EVENT, event)
        .map_err(|error| format!("could not emit dictation state: {error}"))
}

fn emit_unregistered_dictation_latency<R: Runtime>(
    app: &AppHandle<R>,
    startup: &StartupLatencyTrace,
    revision: u64,
    outcome: DictationLatencyOutcome,
) {
    if let Some(event) = startup.finish(revision, outcome, None) {
        emit_dictation_latency(app, event);
    }
}

/// Atomically claims the startup trace for this exact session before building
/// its terminal event. A stale worker or callback cannot take a replacement
/// trace, and `StartupLatencyTrace::finish` provides a second at-most-once
/// boundary for any outstanding Arc clone.
fn finish_dictation_latency<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    revision: u64,
    outcome: DictationLatencyOutcome,
    processing: Option<ProcessingLatencyTrace>,
) {
    if processing
        .as_ref()
        .is_some_and(|processing| processing.session_id() != session_id)
    {
        return;
    }
    let state = app.state::<AppState>();
    let startup = match state.take_dictation_latency_trace(session_id) {
        Ok(Some(startup)) => startup,
        Ok(None) => return,
        Err(error) => {
            eprintln!("could not finish dictation latency diagnostics: {error}");
            return;
        }
    };
    let processing = processing.map(ProcessingLatencyTrace::finish);
    if let Some(event) = startup.finish(revision, outcome, processing) {
        emit_dictation_latency(app, event);
    }
}

fn emit_dictation_latency<R: Runtime>(app: &AppHandle<R>, latency: DictationLatencyEvent) {
    let state = app.state::<AppState>();
    match state.record_dictation_latency(latency.clone()) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            // Diagnostics are best-effort and must never change dictation's
            // terminal result. The live event can still reach an open window.
            eprintln!("could not cache dictation latency diagnostics: {error}");
        }
    }
    if let Err(error) = app.emit_to(MAIN_WINDOW_LABEL, DICTATION_LATENCY_EVENT, latency) {
        eprintln!("could not emit dictation latency diagnostics: {error}");
    }
}

fn emit_local_data_changed<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    domains: Vec<LocalDataDomain>,
) {
    if domains.is_empty() {
        return;
    }
    let event = LocalDataChangedEvent {
        revision: state.next_local_data_revision(),
        domains,
    };
    if let Err(error) = app.emit_to(MAIN_WINDOW_LABEL, LOCAL_DATA_CHANGED_EVENT, event) {
        eprintln!("could not emit local data change: {error}");
    }
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
    mut latency: ProcessingLatencyTrace,
) {
    let session_id = match active_session_id(&processing) {
        Ok(session_id) => session_id,
        Err(error) => {
            eprintln!("could not identify the transcription session: {error}");
            return;
        }
    };
    let state = app.state::<AppState>();
    let hud_target_lease = state
        .transcription_operation(&session_id)
        .ok()
        .flatten()
        .and_then(|operation| operation.hud_target_lease);
    let capture_finalize_started = Instant::now();
    let capture_result = finalizer.and_then(CaptureFinalizer::finalize);
    latency.mark_capture_finalize(capture_finalize_started);
    match capture_result {
        Ok(capture) => {
            let status = capture.status();
            latency.set_audio_duration(status.captured_ms);
            let sample_count = capture.pcm_16khz().len();

            if sample_count == 0 || status.sample_count == 0 {
                fail_processing_with_latency(
                    app,
                    &session_id,
                    "no microphone audio was captured".into(),
                    latency,
                );
                return;
            }

            let Some(session) = processing.session.as_ref() else {
                fail_processing_with_latency(
                    app,
                    &session_id,
                    "dictation settings were unavailable".into(),
                    latency,
                );
                return;
            };
            let transcription_started = Instant::now();
            let result = transcribe_capture(state.inner(), session, capture.pcm_16khz());
            latency.mark_transcription(transcription_started);
            // The decoder is finished. Wipe and release raw PCM before text
            // delivery, event emission, or optional SQLite accounting.
            drop(capture);

            match result {
                Ok(transcript) if transcript.transcript.text.trim().is_empty() => {
                    fail_processing_with_latency(
                        app,
                        &session_id,
                        "No speech was recognized. Try again a little closer to the microphone."
                            .into(),
                        latency,
                    );
                }
                Ok(transcript) => {
                    let inserting = match claim_session_insertion(state.inner(), &session_id) {
                        Ok(Some(inserting)) => inserting,
                        Ok(None) => {
                            discard_target_for_session(state.inner(), &session_id);
                            release_hud_target(app, state.inner(), hud_target_lease.as_ref());
                            return;
                        }
                        Err(error) => {
                            fail_processing_with_latency(app, &session_id, error, latency);
                            return;
                        }
                    };
                    if let Err(error) = emit_state(app, &inserting) {
                        eprintln!("could not emit insertion state: {error}");
                    }

                    let delivery_started = Instant::now();
                    let delivery =
                        deliver_transcript(state.inner(), &session_id, &transcript.transcript.text);
                    latency.mark_delivery(delivery_started);
                    release_hud_target(app, state.inner(), hud_target_lease.as_ref());
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
                            finish_dictation_latency(
                                app,
                                &session_id,
                                completed.revision,
                                DictationLatencyOutcome::Completed,
                                Some(latency),
                            );
                            match delivery.status {
                                DictationDeliveryStatus::Inserted => {
                                    settle_after(app, SUCCESS_HUD_DWELL, completed.revision)
                                }
                                DictationDeliveryStatus::SecureField => {
                                    settle_after(app, FAILURE_HUD_DWELL, completed.revision)
                                }
                                DictationDeliveryStatus::FocusChanged
                                | DictationDeliveryStatus::AccessibilityMissing
                                | DictationDeliveryStatus::Unsupported
                                | DictationDeliveryStatus::Failed
                                | DictationDeliveryStatus::Indeterminate => {
                                    settle_after(app, FAILURE_HUD_DWELL, completed.revision)
                                }
                            }
                            // User-visible terminal events and HUD behavior
                            // must never wait behind optional SQLite work.
                            persist_completed_dictation(
                                app,
                                state.inner(),
                                &completed,
                                &transcript,
                                status.captured_ms,
                            );
                        }
                        Ok(None) => {}
                        Err(error) => {
                            eprintln!("could not complete transcription session: {error}")
                        }
                    }
                }
                Err(EngineError::Cancelled) => {}
                Err(error) => fail_processing_with_latency(
                    app,
                    &session_id,
                    engine_error_message(error),
                    latency,
                ),
            }
        }
        Err(error) => fail_processing_with_latency(app, &session_id, error, latency),
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
    let operation = state
        .transcription_operation(&session.id)
        .map_err(EngineError::Backend)?
        .ok_or(EngineError::Cancelled)?;
    let cancellation = operation.cancellation;
    let vocabulary = operation
        .vocabulary
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    validate_cleanup_selection(session.cleanup_engine.as_ref())?;
    let clean = session.cleanup_engine.is_some();
    let request = || TranscriptionRequest {
        audio: AudioInput {
            samples: pcm_16khz,
            sample_rate_hz: 16_000,
            channels: 1,
        },
        language_policy: &session.language_policy,
        vocabulary: &vocabulary,
        cancellation: Some(cancellation.as_ref()),
    };

    if let Some(provider) = provider_for_engine(&session.transcription_engine) {
        let cloud = state.cloud.transcribe(provider, request(), clean, || {
            state.claim_cloud_upload(&session.id)
        })?;
        return finish_cloud_transcription(session, cloud);
    }

    if session.transcription_engine.provider != EngineProvider::WhisperCpp
        || session.transcription_engine.location != EngineLocation::Local
    {
        return Err(EngineError::InvalidRequest(
            "The selected transcription engine is not supported.".into(),
        ));
    }

    let local = (|| {
        let model = state
            .models
            .resolve(&session.transcription_engine.model)
            .ok_or_else(|| {
                EngineError::InvalidRequest(format!(
                    "unknown local model: {}",
                    session.transcription_engine.model
                ))
            })?;
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
        let mut result = state
            .whisper
            .transcribe(Arc::clone(&model), &model_path, request())?;
        apply_configured_cleanup(session.cleanup_engine.as_ref(), &mut result)?;
        Ok(PendingDictationTranscript {
            session_id: session.id.clone(),
            engine_id: model.id.clone(),
            transcript: result,
        })
    })();

    let local_error = match local {
        Ok(transcript) => return Ok(transcript),
        Err(error) => error,
    };
    if !operation.allow_cloud_fallback || !cloud_fallback_eligible(&local_error) {
        return Err(local_error);
    }
    if cancellation.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    let Some(provider) = state
        .cloud
        .first_configured_compatible(&session.language_policy)?
    else {
        return Err(local_error);
    };
    // One fallback attempt means one upload. A provider failure is surfaced;
    // Spick never fans the same recording out to another cloud service.
    let cloud = state.cloud.transcribe(provider, request(), clean, || {
        state.claim_cloud_upload(&session.id)
    })?;
    finish_cloud_transcription(session, cloud)
}

fn validate_cleanup_selection(cleanup_engine: Option<&EngineConfig>) -> Result<(), EngineError> {
    if cleanup_engine.is_some_and(|engine| !engine.is_builtin_readable_cleanup()) {
        Err(EngineError::InvalidRequest(
            "The selected cleanup engine is not supported.".into(),
        ))
    } else {
        Ok(())
    }
}

fn finish_cloud_transcription(
    session: &DictationSession,
    cloud: CloudTranscription,
) -> Result<PendingDictationTranscript, EngineError> {
    let mut result = cloud.transcript;
    if !cloud.cleanup_applied {
        apply_configured_cleanup(session.cleanup_engine.as_ref(), &mut result)?;
    }
    Ok(PendingDictationTranscript {
        session_id: session.id.clone(),
        engine_id: cloud.engine_id.into(),
        transcript: result,
    })
}

fn cloud_fallback_eligible(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::Backend(_) | EngineError::InvalidResult(_)
    )
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
        | TextTargetErrorKind::ExpectedApplicationMismatch
        | TextTargetErrorKind::ExpectedSelectionMismatch
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

fn handle_audio_capture_ready<R: Runtime>(app: &AppHandle<R>, ready: AudioCaptureReady) {
    let state = app.state::<AppState>();
    let (event, startup_latency) = {
        let mut session = match state.session.lock() {
            Ok(session) => session,
            Err(_) => {
                eprintln!(
                    "could not mark the microphone ready: dictation session lock is poisoned"
                );
                return;
            }
        };
        if !starting_session_matches(&session.snapshot(), &ready.session_id) {
            return;
        }
        let startup_latency = match state.dictation_latency_trace(&ready.session_id) {
            Ok(trace) => trace,
            Err(error) => {
                eprintln!("could not read microphone startup diagnostics: {error}");
                None
            }
        };
        match mark_session_ready_if_matching(&mut session, &ready.session_id) {
            Ok(Some(event)) => (event, startup_latency),
            Ok(None) => return,
            Err(error) => {
                eprintln!("could not mark the microphone ready: {error}");
                return;
            }
        }
    };

    if let Err(error) = emit_state(app, &event) {
        eprintln!("could not emit listening state: {error}");
    } else if let Some(trace) = startup_latency {
        trace.mark_listening_emitted();
    }
}

fn mark_session_ready_if_matching(
    session: &mut SessionController,
    session_id: &str,
) -> Result<Option<DictationStateEvent>, String> {
    let snapshot = session.snapshot();
    if !starting_session_matches(&snapshot, session_id) {
        return Ok(None);
    }
    session.ready().map(Some).map_err(|error| error.to_string())
}

fn starting_session_matches(event: &DictationStateEvent, session_id: &str) -> bool {
    event.state == SessionState::Starting
        && event
            .session
            .as_ref()
            .is_some_and(|session| session.id == session_id)
}

fn audio_failure_matches_session(
    event: &DictationStateEvent,
    failure: &AudioCaptureFailure,
) -> bool {
    if failure.requires_starting {
        starting_session_matches(event, &failure.session_id)
    } else {
        session_matches(event, &failure.session_id)
    }
}

fn handle_audio_capture_failure<R: Runtime>(app: &AppHandle<R>, failure: AudioCaptureFailure) {
    let state = app.state::<AppState>();
    let failure_session_id = failure.session_id.clone();
    let transition = (|| -> Result<Option<AudioFailureTransition>, String> {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "dictation session lock is poisoned".to_string())?;
        let snapshot = session.snapshot();
        if !audio_failure_matches_session(&snapshot, &failure) {
            return Ok(None);
        }

        let operation = state.transcription_operation(&failure.session_id)?;
        let target = operation
            .as_ref()
            .and_then(|operation| operation.target.as_ref())
            .map(|target| target.token);
        let hud_target_lease = operation.and_then(|operation| operation.hud_target_lease);
        let mut audio = state
            .audio
            .lock()
            .map_err(|_| "microphone capture lock is poisoned".to_string())?;
        let cleanup = audio.take_matching(&failure.session_id);
        let failed = session
            .fail(failure.message)
            .map_err(|error| error.to_string())?;
        if let Err(error) = state.cancel_transcription(&failure.session_id) {
            eprintln!("could not cancel failed transcription state: {error}");
        }
        if let Err(error) = state.finish_transcription(&failure.session_id) {
            eprintln!("could not discard failed transcription state: {error}");
        }
        Ok(Some(AudioFailureTransition {
            event: failed,
            cleanup,
            target,
            hud_target_lease,
        }))
    })();

    match transition {
        Ok(Some(transition)) => {
            if let Some(target) = transition.target {
                state.text_targets.discard(target);
            }
            release_hud_target(app, state.inner(), transition.hud_target_lease.as_ref());
            discard_on_worker(transition.cleanup);
            if let Err(emit_error) = emit_state(app, &transition.event) {
                eprintln!("could not emit microphone failure: {emit_error}");
            }
            finish_dictation_latency(
                app,
                &failure_session_id,
                transition.event.revision,
                DictationLatencyOutcome::Failed,
                None,
            );
            settle_after(app, FAILURE_HUD_DWELL, transition.event.revision);
        }
        Ok(None) => {}
        Err(error) => eprintln!("could not handle microphone failure: {error}"),
    }
}

fn session_matches(event: &DictationStateEvent, session_id: &str) -> bool {
    matches!(
        event.state,
        SessionState::Starting
            | SessionState::Listening
            | SessionState::Processing
            | SessionState::Inserting
    ) && event
        .session
        .as_ref()
        .is_some_and(|session| session.id == session_id)
}

fn persist_completed_dictation<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    completed: &DictationStateEvent,
    transcript: &DictationTranscript,
    speech_duration_ms: u64,
) {
    let outcome =
        record_completed_dictation_locally(state, completed, transcript, speech_duration_ms);

    match outcome {
        Ok(outcome) if outcome.inserted_usage => {
            let mut domains = vec![LocalDataDomain::Usage];
            if outcome.inserted_history {
                domains.push(LocalDataDomain::History);
            }
            emit_local_data_changed(app, state, domains);
        }
        Ok(_) => {}
        Err(error) => {
            // The transcript was already delivered and the lifecycle already
            // completed. Never fail or retry insertion because optional local
            // accounting could not be written.
            eprintln!("could not record local usage: {error}");
        }
    }
}

fn record_completed_dictation_locally(
    state: &AppState,
    completed: &DictationStateEvent,
    transcript: &DictationTranscript,
    speech_duration_ms: u64,
) -> Result<RecordOutcome, String> {
    let Some(session) = completed.session.as_ref() else {
        return Ok(RecordOutcome::default());
    };
    let Some(completed_at_ms) = session.ended_at_ms else {
        return Ok(RecordOutcome::default());
    };

    // This gate is also held by settings writes and destructive clears. If an
    // opt-out acknowledgement returns, no older completion can subsequently
    // observe the stale `true` value and write transcript text.
    let _settings_update = match state.settings_update.lock() {
        Ok(guard) => guard,
        Err(_) => return Err("settings update is unavailable".into()),
    };
    let save_history = match state.settings.read() {
        Ok(settings) => settings.save_transcript_history,
        Err(_) => return Err("settings are unavailable".into()),
    };
    let language_tag = delivered_language_tag(session, &transcript.transcript);
    state
        .local_data
        .record_completed_dictation(CompletedDictationRecord {
            session_id: &session.id,
            started_at_ms: session.started_at_ms,
            completed_at_ms,
            text: &transcript.transcript.text,
            speech_duration_ms,
            language_tag: &language_tag,
            engine_id: &transcript.engine_id,
            target_app: transcript.delivery.target_app.as_deref(),
            delivery_status: transcript.delivery.status,
            save_history,
        })
}

fn delivered_language_tag(session: &DictationSession, transcript: &TranscriptResult) -> String {
    if let Some(language) = transcript
        .detected_language
        .as_deref()
        .map(str::trim)
        .filter(|language| !language.is_empty())
    {
        return language.to_owned();
    }
    match &session.language_policy {
        LanguagePolicy::Fixed { language } => language.clone(),
        LanguagePolicy::Translate {
            output_language, ..
        } => output_language.clone(),
        LanguagePolicy::Preferred { languages } | LanguagePolicy::Mixed { languages }
            if languages.len() == 1 =>
        {
            languages[0].clone()
        }
        LanguagePolicy::Auto | LanguagePolicy::Preferred { .. } | LanguagePolicy::Mixed { .. } => {
            "und".into()
        }
    }
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

fn fail_processing_with_latency<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    error: String,
    latency: ProcessingLatencyTrace,
) {
    let Some(failed) = fail_and_emit_if_matching(app, session_id, error) else {
        return;
    };
    finish_dictation_latency(
        app,
        session_id,
        failed.revision,
        DictationLatencyOutcome::Failed,
        Some(latency),
    );
    settle_after(app, FAILURE_HUD_DWELL, failed.revision);
}

fn fail_and_emit_if_matching<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    error: String,
) -> Option<DictationStateEvent> {
    let state = app.state::<AppState>();
    let hud_target_lease = state
        .transcription_operation(session_id)
        .ok()
        .flatten()
        .and_then(|operation| operation.hud_target_lease);
    discard_target_for_session(state.inner(), session_id);
    release_hud_target(app, state.inner(), hud_target_lease.as_ref());
    let expected = state.session.lock().map(|session| session.snapshot());
    let event = match expected {
        Ok(expected) if session_matches(&expected, session_id) => {
            fail_session_if_matching(state.inner(), &expected, error)
        }
        Ok(_) => return None,
        Err(_) => Err("dictation session lock is poisoned".into()),
    };

    match event {
        Ok(Some(failed)) => {
            if let Err(emit_error) = emit_state(app, &failed) {
                eprintln!("could not emit capture failure: {emit_error}");
            }
            Some(failed)
        }
        Ok(None) => None,
        Err(error) => {
            eprintln!("could not fail capture session: {error}");
            None
        }
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

fn show_hud_for_target<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    settings: &HudSettings,
    lease: Option<&HudTargetProtectionLease>,
) -> Result<bool, String> {
    if !settings.visible {
        return Ok(false);
    }
    let protection = state
        .hud_target_protection
        .lock()
        .map_err(|_| "HUD target protection is unavailable".to_string())?;
    match lease {
        Some(lease) if protection.is_current(lease) => {
            hud::show(app, settings, true)?;
            Ok(true)
        }
        Some(_) => Ok(false),
        None if !protection.has_owner() => {
            hud::show(app, settings, false)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn release_hud_target<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    lease: Option<&HudTargetProtectionLease>,
) {
    let Some(lease) = lease else {
        return;
    };
    let Ok(mut protection) = state.hud_target_protection.lock() else {
        eprintln!("could not release fallback HUD target protection: ownership is unavailable");
        return;
    };
    if !protection.is_current(lease) {
        return;
    }
    if let Err(error) = hud::protect_target(app, false) {
        eprintln!("could not release fallback HUD target protection: {error}");
        return;
    }
    protection.release_if_current(lease);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HudSettleAttempt {
    Applied,
    Obsolete,
    Retry,
}

fn try_mutex_guard<'a, T>(
    lock: &'a Mutex<T>,
    poisoned_message: &'static str,
) -> Result<Option<MutexGuard<'a, T>>, String> {
    match lock.try_lock() {
        Ok(guard) => Ok(Some(guard)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Poisoned(_)) => Err(poisoned_message.into()),
    }
}

fn try_read_guard<'a, T>(
    lock: &'a RwLock<T>,
    poisoned_message: &'static str,
) -> Result<Option<RwLockReadGuard<'a, T>>, String> {
    match lock.try_read() {
        Ok(guard) => Ok(Some(guard)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Poisoned(_)) => Err(poisoned_message.into()),
    }
}

fn try_settle_hud_for_revision<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    revision: u64,
) -> Result<HudSettleAttempt, String> {
    // This function runs on AppKit's main thread. Never block there on a guard:
    // another worker may own that guard while its panel operation is already
    // queued behind us. A short retry lets that operation run first and avoids
    // a main-thread/worker lock inversion.
    let Some(session) = try_mutex_guard(&state.session, "dictation session lock is poisoned")?
    else {
        return Ok(HudSettleAttempt::Retry);
    };
    if !should_settle_for_revision(&session.snapshot(), revision) {
        return Ok(HudSettleAttempt::Obsolete);
    }
    let Some(protection) = try_mutex_guard(
        &state.hud_target_protection,
        "HUD target protection is unavailable",
    )?
    else {
        return Ok(HudSettleAttempt::Retry);
    };
    if protection.has_owner() {
        return Ok(HudSettleAttempt::Retry);
    }
    let Some(settings) = try_read_guard(&state.settings, "settings lock is poisoned")? else {
        return Ok(HudSettleAttempt::Retry);
    };
    let hud_settings = settings.hud.clone();
    drop(settings);
    if hud_settings.visible {
        // The floating widget is a persistent idle control. Showing an
        // already-visible nonactivating panel also reapplies its saved size and
        // monitor-clamped position after the session releases its target.
        hud::show(app, &hud_settings, false)?;
    } else {
        hud::hide(app)?;
    }
    Ok(HudSettleAttempt::Applied)
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

fn settle_after<R: Runtime>(app: &AppHandle<R>, delay: Duration, revision: u64) {
    schedule_hud_settle(app, delay, revision, HUD_SETTLE_MAX_RETRIES);
}

fn schedule_hud_settle<R: Runtime>(
    app: &AppHandle<R>,
    delay: Duration,
    revision: u64,
    retries_remaining: usize,
) {
    let app = app.clone();
    let spawn_result = thread::Builder::new()
        .name("spick-hud-settle".into())
        .spawn(move || {
            if !delay.is_zero() {
                thread::sleep(delay);
            }

            let operation_app = app.clone();
            if let Err(error) = app.run_on_main_thread(move || {
                // The revision check and native panel operation share the same
                // guards on AppKit's thread, so an old timer cannot overwrite a
                // newly started session's target protection or presentation.
                let state = operation_app.state::<AppState>();
                match try_settle_hud_for_revision(&operation_app, state.inner(), revision) {
                    Ok(HudSettleAttempt::Applied | HudSettleAttempt::Obsolete) => {}
                    Ok(HudSettleAttempt::Retry) if retries_remaining > 0 => {
                        schedule_hud_settle(
                            &operation_app,
                            HUD_SETTLE_RETRY_DELAY,
                            revision,
                            retries_remaining - 1,
                        );
                    }
                    Ok(HudSettleAttempt::Retry) => {
                        eprintln!(
                            "could not return the dictation HUD to idle because its state remained busy"
                        );
                    }
                    Err(error) => {
                        eprintln!("could not return the dictation HUD to idle: {error}");
                    }
                }
            }) {
                eprintln!("could not schedule the dictation HUD reset on AppKit: {error}");
            }
        });

    if let Err(error) = spawn_result {
        eprintln!("could not schedule the dictation HUD reset: {error}");
    }
}

fn should_settle_for_revision(event: &DictationStateEvent, revision: u64) -> bool {
    event.revision == revision
        && matches!(
            event.state,
            SessionState::Completed | SessionState::Cancelled | SessionState::Failed
        )
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, sync::mpsc};

    use super::*;
    use crate::domain::{
        AppSettings, DictationSession, HudCoordinates, HudPresentation, LanguagePolicy,
    };

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
            &event("session-a", SessionState::Starting, 1),
            "session-a"
        ));
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
    fn microphone_readiness_cannot_revive_or_repeat_an_old_session() {
        let mut controller = SessionController::default();
        let settings = AppSettings::default();
        let first = controller
            .start(
                SessionTrigger::Shortcut,
                settings.language_policy.clone(),
                settings.transcription_engine.clone(),
                settings.cleanup_engine.clone(),
            )
            .unwrap();
        let first_id = active_session_id(&first).unwrap();
        controller.cancel(None).unwrap();
        let second = controller
            .start(
                SessionTrigger::Shortcut,
                settings.language_policy,
                settings.transcription_engine,
                settings.cleanup_engine,
            )
            .unwrap();
        let second_id = active_session_id(&second).unwrap();

        assert_eq!(
            mark_session_ready_if_matching(&mut controller, &first_id).unwrap(),
            None
        );
        let listening = mark_session_ready_if_matching(&mut controller, &second_id)
            .unwrap()
            .unwrap();
        assert_eq!(listening.state, SessionState::Listening);
        assert_eq!(
            mark_session_ready_if_matching(&mut controller, &second_id).unwrap(),
            None
        );
    }

    #[test]
    fn startup_timeout_cannot_fail_a_ready_cancelled_or_newer_session() {
        let mut controller = SessionController::default();
        let settings = AppSettings::default();
        let first = controller
            .start(
                SessionTrigger::Shortcut,
                settings.language_policy.clone(),
                settings.transcription_engine.clone(),
                settings.cleanup_engine.clone(),
            )
            .unwrap();
        let first_id = active_session_id(&first).unwrap();
        let timeout = AudioCaptureFailure {
            session_id: first_id.clone(),
            message: "timed out".into(),
            requires_starting: true,
        };

        assert!(audio_failure_matches_session(&first, &timeout));
        let listening = controller.ready().unwrap();
        assert!(!audio_failure_matches_session(&listening, &timeout));
        controller.cancel(None).unwrap();
        let second = controller
            .start(
                SessionTrigger::Shortcut,
                settings.language_policy,
                settings.transcription_engine,
                settings.cleanup_engine,
            )
            .unwrap();
        assert!(!audio_failure_matches_session(&second, &timeout));
    }

    #[test]
    fn ordinary_stream_failure_still_applies_after_readiness() {
        let listening = event("session-a", SessionState::Listening, 2);
        let failure = AudioCaptureFailure {
            session_id: "session-a".into(),
            message: "device disappeared".into(),
            requires_starting: false,
        };

        assert!(audio_failure_matches_session(&listening, &failure));
    }

    #[test]
    fn microphone_lifecycle_waits_until_starting_was_published() {
        let gate = Arc::new(StartPublicationGate::default());
        let worker_gate = Arc::clone(&gate);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            worker_gate.wait();
            sender.send(()).unwrap();
        });

        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(10)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        gate.publish();
        receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn only_external_controls_capture_another_apps_text_target() {
        assert_eq!(
            session_trigger_for_window(MAIN_WINDOW_LABEL).unwrap(),
            SessionTrigger::UserInterface
        );
        assert_eq!(
            session_trigger_for_window(hud::HUD_WINDOW_LABEL).unwrap(),
            SessionTrigger::FloatingWidget
        );
        assert!(session_trigger_for_window("unknown").is_err());
        assert!(trigger_uses_external_target(SessionTrigger::Shortcut));
        assert!(trigger_uses_external_target(SessionTrigger::FloatingWidget));
        assert!(!trigger_uses_external_target(SessionTrigger::UserInterface));
    }

    #[test]
    fn hud_settle_timer_requires_the_exact_terminal_revision() {
        let completed = event("session-a", SessionState::Completed, 4);
        assert!(should_settle_for_revision(&completed, 4));
        assert!(!should_settle_for_revision(&completed, 3));
        assert!(!should_settle_for_revision(
            &event("session-b", SessionState::Listening, 5),
            4
        ));
    }

    #[test]
    fn hud_settle_defers_instead_of_waiting_on_a_new_session_lock() {
        let session_gate = Mutex::new(());
        let new_session = session_gate.lock().unwrap();

        assert!(try_mutex_guard(&session_gate, "session lock is poisoned")
            .unwrap()
            .is_none());

        drop(new_session);
        assert!(try_mutex_guard(&session_gate, "session lock is poisoned")
            .unwrap()
            .is_some());
    }

    #[test]
    fn settings_update_keeps_hud_changes_committed_during_shortcut_replacement() {
        let previous = AppSettings::default();
        let mut requested = previous.clone();
        requested.save_transcript_history = true;
        requested.hud.visible = false;
        let mut current = previous.clone();
        current.hud.presentation = HudPresentation::Compact;
        current.hud.custom_position = Some(HudCoordinates { x: -320, y: 84 });

        let rebased = rebase_settings_update(&previous, &requested, &current).unwrap();

        assert!(rebased.save_transcript_history);
        assert!(!rebased.hud.visible);
        assert_eq!(rebased.hud.presentation, current.hud.presentation);
        assert_eq!(rebased.hud.custom_position, current.hud.custom_position);
    }

    #[test]
    fn hud_enablement_shows_the_persistent_widget_when_idle() {
        assert_eq!(
            hud_visibility_update(false, true),
            HudVisibilityUpdate::EnableNowIfIdle
        );

        let mut show_called = false;
        let mut hide_called = false;
        let mut rollback_called = false;
        reconcile_committed_hud_visibility(
            HudVisibilityUpdate::EnableNowIfIdle,
            || {
                show_called = true;
                Ok(())
            },
            || {
                hide_called = true;
                Ok(())
            },
            || {
                rollback_called = true;
                Ok(())
            },
        )
        .unwrap();

        assert!(show_called);
        assert!(!hide_called);
        assert!(!rollback_called);
    }

    #[test]
    fn hud_hide_failure_is_not_acknowledged_and_triggers_rollback() {
        let mut rollback_called = false;
        let error = reconcile_committed_hud_visibility(
            HudVisibilityUpdate::DisableNow,
            || Ok(()),
            || Err("native hide failed".into()),
            || {
                rollback_called = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(rollback_called);
        assert!(error.contains("settings were not changed"));
        assert!(error.contains("native hide failed"));
    }

    #[test]
    fn hud_show_failure_is_not_acknowledged_and_triggers_rollback() {
        let mut hide_called = false;
        let mut rollback_called = false;
        let error = reconcile_committed_hud_visibility(
            HudVisibilityUpdate::EnableNowIfIdle,
            || Err("native show failed".into()),
            || {
                hide_called = true;
                Ok(())
            },
            || {
                rollback_called = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(!hide_called);
        assert!(rollback_called);
        assert!(error.contains("settings were not changed"));
        assert!(error.contains("native show failed"));
    }

    #[test]
    fn successful_hud_hide_keeps_the_committed_settings() {
        let mut rollback_called = false;
        reconcile_committed_hud_visibility(
            HudVisibilityUpdate::DisableNow,
            || Ok(()),
            || Ok(()),
            || {
                rollback_called = true;
                Ok(())
            },
        )
        .unwrap();

        assert!(!rollback_called);
    }

    #[test]
    fn hud_hide_rollback_restores_memory_and_persisted_settings() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let state = AppState::load(settings_path.clone()).unwrap();
        let previous = state.settings_snapshot().unwrap();
        let mut attempted = previous.clone();
        attempted.hud.visible = false;
        attempted.save_transcript_history = !previous.save_transcript_history;

        state.persist_settings(&attempted).unwrap();
        *state.settings.write().unwrap() = attempted;
        restore_settings_snapshot(&state, &previous).unwrap();

        assert_eq!(state.settings_snapshot().unwrap(), previous);
        assert_eq!(
            AppState::load(settings_path)
                .unwrap()
                .settings_snapshot()
                .unwrap(),
            previous
        );
    }

    #[test]
    fn hud_hide_error_reports_a_failed_settings_rollback() {
        let error = reconcile_committed_hud_visibility(
            HudVisibilityUpdate::DisableNow,
            || Ok(()),
            || Err("native hide failed".into()),
            || Err("disk is read-only".into()),
        )
        .unwrap_err();

        assert!(error.contains("native hide failed"));
        assert!(error.contains("settings rollback also failed"));
        assert!(error.contains("disk is read-only"));
    }

    #[test]
    fn settings_update_rejects_a_concurrent_non_hud_change() {
        let previous = AppSettings::default();
        let mut requested = previous.clone();
        requested.save_transcript_history = true;
        let mut current = previous.clone();
        current.allow_cloud_fallback = true;

        let error = rebase_settings_update(&previous, &requested, &current).unwrap_err();

        assert!(error.contains("settings changed"));
    }

    #[test]
    fn acknowledged_history_opt_out_wins_before_a_waiting_completion_write() {
        let directory = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::load(directory.path().join("settings.json")).unwrap());
        state.settings.write().unwrap().save_transcript_history = true;
        let mut completed = event("session-a", SessionState::Completed, 4);
        completed.session.as_mut().unwrap().ended_at_ms = Some(2_000);
        let transcript = DictationTranscript {
            session_id: "session-a".into(),
            engine_id: "whisper-test".into(),
            transcript: TranscriptResult::final_text("clean delivered text"),
            delivery: DictationDelivery {
                status: DictationDeliveryStatus::Inserted,
                transcript_available: true,
                target_app: Some("Notes".into()),
                caret_repositioned: Some(true),
            },
        };

        let settings_gate = state.settings_update.lock().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            record_completed_dictation_locally(&worker_state, &completed, &transcript, 1_000)
        });
        started_rx.recv().unwrap();
        state.settings.write().unwrap().save_transcript_history = false;
        drop(settings_gate);

        let outcome = worker.join().unwrap().unwrap();
        assert!(outcome.inserted_usage);
        assert!(!outcome.inserted_history);
        assert!(state
            .local_data
            .transcript_history(None, None)
            .unwrap()
            .items
            .is_empty());
    }

    #[test]
    fn committed_clear_keeps_its_result_when_memory_cleanup_fails() {
        let mut result = ClearLocalDataResult {
            scope: ClearLocalDataScope::TranscriptHistory,
            deleted_usage_sessions: 0,
            deleted_transcripts: 2,
            deleted_vocabulary_entries: 0,
            cleared_latest_transcript: false,
            cleared_latest_session_id: None,
            storage_cleanup_complete: true,
            storage_cleanup_warning: None,
            memory_cleanup_complete: true,
            memory_cleanup_warning: None,
            cleared_at_ms: 10,
        };

        apply_latest_clear_outcome(&mut result, Err("transcript lock is poisoned".into()));

        assert_eq!(result.deleted_transcripts, 2);
        assert!(result.storage_cleanup_complete);
        assert!(!result.memory_cleanup_complete);
        assert!(result
            .memory_cleanup_warning
            .as_deref()
            .unwrap()
            .contains("in-memory"));
    }

    #[test]
    fn model_activation_and_hud_position_writes_serialize_without_lost_updates() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let state = Arc::new(AppState::load(settings_path.clone()).unwrap());
        let model = resolve_curated_whisper_model("whisper-tiny-multilingual-f16").unwrap();
        let position = HudCoordinates { x: -420, y: 96 };

        let (model_has_gate_tx, model_has_gate_rx) = mpsc::channel();
        let (release_model_tx, release_model_rx) = mpsc::channel();
        let model_state = Arc::clone(&state);
        let model_worker = thread::spawn(move || {
            commit_local_model_activation(model_state.as_ref(), &model, || {
                model_has_gate_tx.send(()).unwrap();
                release_model_rx.recv().unwrap();
                Ok(())
            })
        });
        model_has_gate_rx.recv().unwrap();

        let (position_started_tx, position_started_rx) = mpsc::channel();
        let (position_done_tx, position_done_rx) = mpsc::channel();
        let position_state = Arc::clone(&state);
        let position_worker = thread::spawn(move || {
            position_started_tx.send(()).unwrap();
            let result = persist_hud_position_for_state(position_state.as_ref(), position);
            position_done_tx.send(result).unwrap();
        });
        position_started_rx.recv().unwrap();
        assert!(matches!(
            position_done_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        release_model_tx.send(()).unwrap();
        let activated = model_worker.join().unwrap().unwrap();
        position_done_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        position_worker.join().unwrap();

        let current = state.settings_snapshot().unwrap();
        assert_eq!(
            current.transcription_engine.model,
            "whisper-tiny-multilingual-f16"
        );
        assert_eq!(current.hud.custom_position, Some(position));
        assert_eq!(activated.transcription_engine, current.transcription_engine);

        let reloaded = AppState::load(settings_path).unwrap();
        assert_eq!(reloaded.settings_snapshot().unwrap(), current);
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
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().join("settings.json")).unwrap();
        let mut active = event("session-a", SessionState::Processing, 2);
        active.session.as_mut().unwrap().transcription_engine =
            EngineConfig::local(EngineProvider::WhisperCpp, "whisper-tiny-multilingual-f16");

        assert!(session_uses_model(
            &state.models,
            &active,
            "whisper-tiny-multilingual-f16"
        ));
        assert!(!session_uses_model(
            &state.models,
            &active,
            "whisper-small-multilingual-q5-1"
        ));
        active.state = SessionState::Completed;
        assert!(!session_uses_model(
            &state.models,
            &active,
            "whisper-tiny-multilingual-f16"
        ));
    }

    #[test]
    fn cloud_fallback_never_runs_for_cancellation_or_configuration_errors() {
        assert!(cloud_fallback_eligible(&EngineError::Backend(
            "local runtime failed".into()
        )));
        assert!(cloud_fallback_eligible(&EngineError::InvalidResult(
            "invalid local result".into()
        )));
        assert!(!cloud_fallback_eligible(&EngineError::Cancelled));
        assert!(!cloud_fallback_eligible(&EngineError::InvalidRequest(
            "bad settings".into()
        )));
    }

    #[test]
    fn settings_validation_enforces_the_selected_cloud_provider_language_contract() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().join("settings.json")).unwrap();
        let bengali = LanguagePolicy::Fixed {
            language: "bn-IN".into(),
        };
        let openai = AppSettings {
            language_policy: bengali.clone(),
            transcription_engine: EngineConfig {
                provider: EngineProvider::OpenAi,
                model: "gpt-4o-transcribe".into(),
                location: EngineLocation::Cloud,
            },
            ..AppSettings::default()
        };
        assert!(validate_selected_transcription(&state.models, &openai).is_ok());

        let xai = AppSettings {
            language_policy: bengali.clone(),
            transcription_engine: EngineConfig {
                provider: EngineProvider::XAi,
                model: "speech-to-text".into(),
                location: EngineLocation::Cloud,
            },
            ..AppSettings::default()
        };
        assert!(validate_selected_transcription(&state.models, &xai).is_err());

        let unknown = AppSettings {
            language_policy: bengali,
            transcription_engine: EngineConfig {
                provider: EngineProvider::Gemini,
                model: "unregistered".into(),
                location: EngineLocation::Cloud,
            },
            ..AppSettings::default()
        };
        assert!(validate_selected_transcription(&state.models, &unknown).is_err());
    }

    #[test]
    fn dictation_readiness_checks_the_selected_cloud_key_without_touching_models() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().join("settings.json")).unwrap();
        let settings = AppSettings {
            transcription_engine: EngineConfig {
                provider: EngineProvider::OpenAi,
                model: "gpt-4o-transcribe".into(),
                location: EngineLocation::Cloud,
            },
            ..AppSettings::default()
        };
        let checked_provider = Cell::new(None);
        let local_checked = Cell::new(false);

        let error = validate_dictation_engine_readiness(
            &state.models,
            &settings,
            |provider| {
                checked_provider.set(Some(provider));
                Err("No API key is saved for OpenAI.".into())
            },
            |_| {
                local_checked.set(true);
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(checked_provider.get(), Some(CloudProviderId::OpenAi));
        assert!(!local_checked.get());
        assert_eq!(error, "No API key is saved for OpenAI.");
    }

    #[test]
    fn dictation_readiness_rejects_cloud_language_policy_before_key_access() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().join("settings.json")).unwrap();
        let settings = AppSettings {
            language_policy: LanguagePolicy::Fixed {
                language: "bn-IN".into(),
            },
            transcription_engine: EngineConfig {
                provider: EngineProvider::XAi,
                model: "speech-to-text".into(),
                location: EngineLocation::Cloud,
            },
            ..AppSettings::default()
        };
        let credential_checked = Cell::new(false);

        let error = validate_dictation_engine_readiness(
            &state.models,
            &settings,
            |_| {
                credential_checked.set(true);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap_err();

        assert!(!credential_checked.get());
        assert!(error.contains("xAI"));
    }

    #[test]
    fn dictation_readiness_verifies_only_the_selected_local_model() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().join("settings.json")).unwrap();
        let settings = AppSettings::default();
        let cloud_checked = Cell::new(false);
        let checked_model = std::cell::RefCell::new(None);

        let error = validate_dictation_engine_readiness(
            &state.models,
            &settings,
            |_| {
                cloud_checked.set(true);
                Ok(())
            },
            |model| {
                *checked_model.borrow_mut() = Some(model.id.clone());
                Err("selected model is unavailable".into())
            },
        )
        .unwrap_err();

        assert!(!cloud_checked.get());
        assert_eq!(
            checked_model.borrow().as_deref(),
            Some("whisper-small-multilingual-q5-1")
        );
        assert_eq!(error, "selected model is unavailable");
    }

    #[test]
    fn production_local_preflight_rejects_an_uninstalled_model() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().join("settings.json")).unwrap();
        let settings = state.settings_snapshot().unwrap();

        let error = preflight_selected_transcription_engine(&state, &settings).unwrap_err();

        assert!(error.contains("is not installed"));
        assert!(error.contains("Download it from Engines first"));
    }

    #[test]
    fn local_engine_readiness_surfaces_a_load_failure_after_verification() {
        let directory = tempfile::tempdir().unwrap();
        let model = resolve_curated_whisper_model("whisper-small-multilingual-q5-1").unwrap();
        let verified_id = std::cell::RefCell::new(None);
        let loaded_id = std::cell::RefCell::new(None);
        let verified_path = directory.path().join("verified-model.bin");

        let error = ensure_local_whisper_engine_ready(
            &model,
            |model_id| {
                *verified_id.borrow_mut() = Some(model_id.to_owned());
                Ok(verified_path.clone())
            },
            |loaded_model, path| {
                *loaded_id.borrow_mut() = Some(loaded_model.id.clone());
                assert_eq!(path, verified_path);
                Err(EngineError::Backend(
                    "whisper.cpp could not load the selected model".into(),
                ))
            },
        )
        .unwrap_err();

        assert_eq!(verified_id.borrow().as_deref(), Some(model.id.as_str()));
        assert_eq!(loaded_id.borrow().as_deref(), Some(model.id.as_str()));
        assert_eq!(
            error,
            "whisper.cpp could not load the selected model".to_string()
        );
    }

    #[test]
    fn dictation_readiness_rejects_an_unknown_local_manifest_before_verification() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().join("settings.json")).unwrap();
        let settings = AppSettings {
            transcription_engine: EngineConfig::local(
                EngineProvider::WhisperCpp,
                "whisper-does-not-exist",
            ),
            ..AppSettings::default()
        };
        let verifier_called = Cell::new(false);

        let error = validate_dictation_engine_readiness(
            &state.models,
            &settings,
            |_| Ok(()),
            |_| {
                verifier_called.set(true);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(!verifier_called.get());
        assert_eq!(error, "unknown local model: whisper-does-not-exist");
    }

    #[test]
    fn provider_side_cleanup_is_not_applied_twice() {
        let cleanup_engine = EngineConfig::local(
            EngineProvider::BuiltIn,
            crate::domain::BUILTIN_READABLE_CLEANUP_MODEL,
        );
        let mut session = event("session-a", SessionState::Processing, 2)
            .session
            .unwrap();
        session.cleanup_engine = Some(cleanup_engine);

        let already_cleaned = finish_cloud_transcription(
            &session,
            CloudTranscription {
                engine_id: "xai-speech-to-text",
                transcript: TranscriptResult {
                    detected_language: Some("en".into()),
                    ..TranscriptResult::final_text("Um, provider kept this intentionally.")
                },
                cleanup_applied: true,
            },
        )
        .unwrap();
        assert_eq!(
            already_cleaned.transcript.text,
            "Um, provider kept this intentionally."
        );

        let needs_cleanup = finish_cloud_transcription(
            &session,
            CloudTranscription {
                engine_id: "openai-gpt-4o-transcribe",
                transcript: TranscriptResult {
                    detected_language: Some("en".into()),
                    ..TranscriptResult::final_text("Um, this is ready.")
                },
                cleanup_applied: false,
            },
        )
        .unwrap();
        assert_eq!(needs_cleanup.transcript.text, "this is ready.");
    }
}
