use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
};

use tempfile::Builder as TempFileBuilder;

use crate::{
    audio::AudioCaptureController,
    cloud::CloudRuntime,
    domain::{
        AppSettings, LEGACY_SETTINGS_SCHEMA_VERSION, MULTILINGUAL_SETTINGS_SCHEMA_VERSION,
        OPTION_DEFAULT_SETTINGS_SCHEMA_VERSION, SETTINGS_SCHEMA_VERSION,
        TRANSIENT_HUD_SETTINGS_SCHEMA_VERSION,
    },
    engines::{DictationTranscript, WhisperCppRuntime},
    latency::{DictationLatencyEvent, StartupLatencyTrace},
    local_data::LocalDataStore,
    model_store::ModelStore,
    notes::NoteStore,
    platform::{CapturedTextTarget, TextTargetController},
    session::SessionController,
};

/// Process-wide state managed by Tauri.
///
/// Settings are read far more often than they are replaced, while session
/// transitions must be serialized. The lock choices mirror those access
/// patterns and keep the UI/event layer independent from provider runtimes.
pub struct AppState {
    pub settings: RwLock<AppSettings>,
    pub session: Mutex<SessionController>,
    pub audio: Mutex<AudioCaptureController>,
    pub models: Arc<ModelStore>,
    pub whisper: WhisperCppRuntime,
    pub local_data: LocalDataStore,
    pub text_targets: TextTargetController,
    pub cloud: CloudRuntime,
    pub notes: NoteStore,
    /// Serializes model selection/removal with settings writes so an active
    /// model cannot disappear between verification and persistence.
    pub model_configuration: Mutex<()>,
    /// Serializes every settings transaction without blocking dictation.
    ///
    /// Shortcut replacement can stop and join the Option gesture worker. That
    /// worker reads settings while starting a session, so this lock must stay
    /// separate from both `settings` and `model_configuration` and must never
    /// be acquired by the dictation path. Settings writers acquire locks in
    /// this order: `settings_update`, optional `model_configuration`, then
    /// `settings`.
    pub settings_update: Mutex<()>,
    /// Owns the fallback HUD's click-through protection. A lease prevents a
    /// delayed worker from an older session from making a newer session's HUD
    /// interactive and stealing its captured text focus.
    pub hud_target_protection: Mutex<HudTargetProtection>,
    /// Allows a first-run shortcut attempt while Spick itself owns focus.
    /// It changes only target capture: audio and engine paths remain identical.
    pub in_app_dictation: AtomicBool,
    transcripts: Mutex<TranscriptStore>,
    active_dictation_latency: Mutex<Option<StartupLatencyTrace>>,
    latest_dictation_latency: RwLock<Option<DictationLatencyEvent>>,
    local_data_revision: AtomicU64,
    settings_path: PathBuf,
}

pub struct TranscriptionOperation {
    pub cancellation: Arc<AtomicBool>,
    pub target: Option<CapturedTextTarget>,
    pub hud_target_lease: Option<HudTargetProtectionLease>,
    pub vocabulary: Arc<[String]>,
    pub allow_cloud_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudTargetProtectionLease {
    session_id: String,
    generation: u64,
}

#[derive(Debug, Default)]
pub struct HudTargetProtection {
    next_generation: u64,
    owner: Option<HudTargetProtectionLease>,
}

impl HudTargetProtection {
    pub fn claim(&mut self, session_id: String) -> HudTargetProtectionLease {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let lease = HudTargetProtectionLease {
            session_id,
            generation: self.next_generation,
        };
        self.owner = Some(lease.clone());
        lease
    }

    pub fn is_current(&self, lease: &HudTargetProtectionLease) -> bool {
        self.owner.as_ref() == Some(lease)
    }

    pub fn has_owner(&self) -> bool {
        self.owner.is_some()
    }

    pub fn release_if_current(&mut self, lease: &HudTargetProtectionLease) -> bool {
        if !self.is_current(lease) {
            return false;
        }
        self.owner = None;
        true
    }
}

impl AppState {
    #[cfg(test)]
    pub fn load(settings_path: PathBuf) -> Result<Self, String> {
        let data_path = parent_directory(&settings_path);
        let models_path = data_path.join("models");
        let database_path = data_path.join("spick.sqlite3");
        Self::load_with_paths(settings_path, models_path, database_path)
    }

    pub fn load_with_paths(
        settings_path: PathBuf,
        models_path: PathBuf,
        database_path: PathBuf,
    ) -> Result<Self, String> {
        let settings = load_settings(&settings_path)?;
        let credentials_path = parent_directory(&database_path).join("cloud-credentials.json");
        let notes_path = parent_directory(&database_path).join("notes.json");
        Ok(Self {
            settings: RwLock::new(settings),
            session: Mutex::new(SessionController::default()),
            audio: Mutex::new(AudioCaptureController::default()),
            models: Arc::new(ModelStore::new(models_path)?),
            whisper: WhisperCppRuntime::default(),
            local_data: LocalDataStore::open(database_path),
            text_targets: TextTargetController::default(),
            cloud: CloudRuntime::new(credentials_path),
            notes: NoteStore::open(notes_path)?,
            model_configuration: Mutex::new(()),
            settings_update: Mutex::new(()),
            hud_target_protection: Mutex::new(HudTargetProtection::default()),
            in_app_dictation: AtomicBool::new(false),
            transcripts: Mutex::new(TranscriptStore::default()),
            active_dictation_latency: Mutex::new(None),
            latest_dictation_latency: RwLock::new(None),
            local_data_revision: AtomicU64::new(0),
            settings_path,
        })
    }

    pub fn settings_snapshot(&self) -> Result<AppSettings, String> {
        self.settings
            .read()
            .map(|settings| settings.clone())
            .map_err(|_| "settings lock is poisoned".into())
    }

    pub fn persist_settings(&self, settings: &AppSettings) -> Result<(), String> {
        write_settings(&self.settings_path, settings)
    }

    pub fn begin_transcription(
        &self,
        session_id: String,
        target: Option<CapturedTextTarget>,
        hud_target_lease: Option<HudTargetProtectionLease>,
        vocabulary: Arc<[String]>,
        allow_cloud_fallback: bool,
    ) -> Result<Arc<AtomicBool>, String> {
        self.transcripts
            .lock()
            .map(|mut transcripts| {
                transcripts.begin(
                    session_id,
                    target,
                    hud_target_lease,
                    vocabulary,
                    allow_cloud_fallback,
                )
            })
            .map_err(|_| "transcript store is unavailable".into())
    }

    pub fn cancel_transcription(&self, session_id: &str) -> Result<(), String> {
        self.transcripts
            .lock()
            .map(|mut transcripts| transcripts.cancel(session_id))
            .map_err(|_| "transcript store is unavailable".into())
    }

    /// Atomically establishes the point after which cancellation can suppress
    /// delivery but cannot promise that a provider upload was never started.
    pub fn claim_cloud_upload(&self, session_id: &str) -> Result<bool, String> {
        self.transcripts
            .lock()
            .map(|mut transcripts| transcripts.claim_cloud_upload(session_id))
            .map_err(|_| "transcript store is unavailable".into())
    }

    pub fn transcription_operation(
        &self,
        session_id: &str,
    ) -> Result<Option<TranscriptionOperation>, String> {
        self.transcripts
            .lock()
            .map(|transcripts| {
                transcripts
                    .active
                    .as_ref()
                    .filter(|active| active.session_id == session_id)
                    .map(|active| TranscriptionOperation {
                        cancellation: Arc::clone(&active.cancellation),
                        target: active.target.clone(),
                        hud_target_lease: active.hud_target_lease.clone(),
                        vocabulary: Arc::clone(&active.vocabulary),
                        allow_cloud_fallback: active.allow_cloud_fallback,
                    })
            })
            .map_err(|_| "transcript store is unavailable".into())
    }

    pub fn finish_transcription(&self, session_id: &str) -> Result<(), String> {
        self.transcripts
            .lock()
            .map(|mut transcripts| transcripts.finish(session_id))
            .map_err(|_| "transcript store is unavailable".into())
    }

    pub fn complete_transcription(&self, transcript: DictationTranscript) -> Result<bool, String> {
        self.transcripts
            .lock()
            .map(|mut transcripts| transcripts.complete(transcript))
            .map_err(|_| "transcript store is unavailable".into())
    }

    pub fn latest_transcript(&self) -> Result<Option<DictationTranscript>, String> {
        self.transcripts
            .lock()
            .map(|transcripts| transcripts.latest.clone())
            .map_err(|_| "transcript store is unavailable".into())
    }

    pub fn clear_latest_transcript(&self) -> Result<Option<String>, String> {
        self.transcripts
            .lock()
            .map(|mut transcripts| {
                transcripts
                    .latest
                    .take()
                    .map(|transcript| transcript.session_id)
            })
            .map_err(|_| "transcript store is unavailable".into())
    }

    /// Register the one startup trace owned by the active dictation. A newer
    /// session replaces an orphaned older trace, while exact-id lookups below
    /// ensure stale native callbacks can never mutate or finish the replacement.
    pub fn register_dictation_latency(&self, trace: StartupLatencyTrace) -> Result<(), String> {
        if trace.session_id().is_none() {
            return Err("dictation latency session identity is unavailable".into());
        }
        self.active_dictation_latency
            .lock()
            .map(|mut active| *active = Some(trace))
            .map_err(|_| "dictation latency diagnostics are unavailable".into())
    }

    pub fn dictation_latency_trace(
        &self,
        session_id: &str,
    ) -> Result<Option<StartupLatencyTrace>, String> {
        self.active_dictation_latency
            .lock()
            .map(|active| {
                active
                    .as_ref()
                    .filter(|trace| trace.session_id().as_deref() == Some(session_id))
                    .cloned()
            })
            .map_err(|_| "dictation latency diagnostics are unavailable".into())
    }

    pub fn take_dictation_latency_trace(
        &self,
        session_id: &str,
    ) -> Result<Option<StartupLatencyTrace>, String> {
        self.active_dictation_latency
            .lock()
            .map(|mut active| {
                if active
                    .as_ref()
                    .is_some_and(|trace| trace.session_id().as_deref() == Some(session_id))
                {
                    active.take()
                } else {
                    None
                }
            })
            .map_err(|_| "dictation latency diagnostics are unavailable".into())
    }

    pub fn record_dictation_latency(&self, event: DictationLatencyEvent) -> Result<bool, String> {
        self.latest_dictation_latency
            .write()
            .map(|mut latest| {
                if latest
                    .as_ref()
                    .is_some_and(|current| current.revision >= event.revision)
                {
                    return false;
                }
                *latest = Some(event);
                true
            })
            .map_err(|_| "dictation latency diagnostics are unavailable".into())
    }

    pub fn latest_dictation_latency(&self) -> Result<Option<DictationLatencyEvent>, String> {
        self.latest_dictation_latency
            .read()
            .map(|latest| latest.clone())
            .map_err(|_| "dictation latency diagnostics are unavailable".into())
    }

    pub fn next_local_data_revision(&self) -> u64 {
        self.local_data_revision.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[derive(Default)]
struct TranscriptStore {
    active: Option<ActiveTranscription>,
    latest: Option<DictationTranscript>,
}

struct ActiveTranscription {
    session_id: String,
    cancellation: Arc<AtomicBool>,
    target: Option<CapturedTextTarget>,
    hud_target_lease: Option<HudTargetProtectionLease>,
    vocabulary: Arc<[String]>,
    allow_cloud_fallback: bool,
    cloud_upload_claimed: bool,
}

impl TranscriptStore {
    fn begin(
        &mut self,
        session_id: String,
        target: Option<CapturedTextTarget>,
        hud_target_lease: Option<HudTargetProtectionLease>,
        vocabulary: Arc<[String]>,
        allow_cloud_fallback: bool,
    ) -> Arc<AtomicBool> {
        if let Some(active) = self.active.take() {
            active.cancellation.store(true, Ordering::Relaxed);
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        self.active = Some(ActiveTranscription {
            session_id,
            cancellation: Arc::clone(&cancellation),
            target,
            hud_target_lease,
            vocabulary,
            allow_cloud_fallback,
            cloud_upload_claimed: false,
        });
        self.latest = None;
        cancellation
    }

    fn cancel(&mut self, session_id: &str) {
        if let Some(active) = self
            .active
            .as_ref()
            .filter(|active| active.session_id == session_id)
        {
            active.cancellation.store(true, Ordering::Relaxed);
        }
    }

    fn claim_cloud_upload(&mut self, session_id: &str) -> bool {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.session_id == session_id)
        else {
            return false;
        };
        if active.cancellation.load(Ordering::Relaxed) || active.cloud_upload_claimed {
            return false;
        }
        active.cloud_upload_claimed = true;
        true
    }

    fn finish(&mut self, session_id: &str) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.session_id == session_id)
        {
            self.active = None;
        }
    }

    fn complete(&mut self, transcript: DictationTranscript) -> bool {
        let Some(active) = self
            .active
            .as_ref()
            .filter(|active| active.session_id == transcript.session_id)
        else {
            return false;
        };
        if active.cancellation.load(Ordering::Relaxed) {
            self.active = None;
            return false;
        }

        self.active = None;
        self.latest = Some(transcript);
        true
    }
}

fn load_settings(path: &Path) -> Result<AppSettings, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let settings = load_valid_backup(path)?.unwrap_or_default();
            write_settings(path, &settings)?;
            return Ok(settings);
        }
        Err(error) => {
            return Err(format!(
                "could not read settings from {}: {error}",
                path.display()
            ))
        }
    };

    match parse_settings_document(&bytes) {
        Ok(parsed) => {
            if parsed.migrated_legacy_schema {
                write_settings(path, &parsed.settings)?;
            }
            Ok(parsed.settings)
        }
        Err(SettingsParseError::UnsupportedSchema(version)) => {
            Err(unsupported_schema_message(path, version))
        }
        Err(SettingsParseError::InvalidJson(_) | SettingsParseError::InvalidSettings(_)) => {
            let settings = load_valid_backup(path)?.unwrap_or_default();
            write_settings(path, &settings)?;
            Ok(settings)
        }
    }
}

fn write_settings(path: &Path, settings: &AppSettings) -> Result<(), String> {
    settings.validate()?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create settings directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let mut json = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("could not serialize settings: {error}"))?;
    json.push(b'\n');

    match fs::read(path) {
        Ok(existing) => match parse_settings(&existing) {
            Ok(_) => write_file_atomically(&backup_path(path), &existing)?,
            Err(SettingsParseError::UnsupportedSchema(version)) => {
                return Err(unsupported_schema_message(path, version));
            }
            Err(SettingsParseError::InvalidJson(_) | SettingsParseError::InvalidSettings(_)) => {
                quarantine_settings(path, &existing)?;
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "could not preserve existing settings at {}: {error}",
                path.display()
            ));
        }
    }

    write_file_atomically(path, &json)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SettingsParseError {
    InvalidJson(String),
    UnsupportedSchema(u32),
    InvalidSettings(String),
}

impl fmt::Display for SettingsParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(reason) => write!(formatter, "settings JSON is invalid: {reason}"),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "settings schema {version} is not supported")
            }
            Self::InvalidSettings(reason) => write!(formatter, "settings are invalid: {reason}"),
        }
    }
}

fn parse_settings(bytes: &[u8]) -> Result<AppSettings, SettingsParseError> {
    parse_settings_document(bytes).map(|parsed| parsed.settings)
}

struct ParsedSettings {
    settings: AppSettings,
    migrated_legacy_schema: bool,
}

fn parse_settings_document(bytes: &[u8]) -> Result<ParsedSettings, SettingsParseError> {
    let mut settings: AppSettings = serde_json::from_slice(bytes)
        .map_err(|error| SettingsParseError::InvalidJson(error.to_string()))?;
    let migrated_legacy_schema = if matches!(
        settings.schema_version,
        LEGACY_SETTINGS_SCHEMA_VERSION
            | OPTION_DEFAULT_SETTINGS_SCHEMA_VERSION
            | MULTILINGUAL_SETTINGS_SCHEMA_VERSION
            | TRANSIENT_HUD_SETTINGS_SCHEMA_VERSION
            | SETTINGS_SCHEMA_VERSION
    ) {
        settings.migrate_legacy_schema()
    } else {
        return Err(SettingsParseError::UnsupportedSchema(
            settings.schema_version,
        ));
    };
    settings
        .validate()
        .map_err(SettingsParseError::InvalidSettings)?;
    Ok(ParsedSettings {
        settings,
        migrated_legacy_schema,
    })
}

fn unsupported_schema_message(path: &Path, version: u32) -> String {
    format!(
        "settings at {} use unsupported schema version {version}; the file was left unchanged",
        path.display()
    )
}

fn load_valid_backup(path: &Path) -> Result<Option<AppSettings>, String> {
    let backup = backup_path(path);
    let bytes = match fs::read(&backup) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not read settings backup from {}: {error}",
                backup.display()
            ))
        }
    };

    match parse_settings(&bytes) {
        Ok(settings) => Ok(Some(settings)),
        Err(SettingsParseError::UnsupportedSchema(version)) => {
            Err(unsupported_schema_message(&backup, version))
        }
        Err(SettingsParseError::InvalidJson(_) | SettingsParseError::InvalidSettings(_)) => {
            Ok(None)
        }
    }
}

fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = parent_directory(path);
    let mut temporary = TempFileBuilder::new()
        .prefix(".spick-settings-")
        .tempfile_in(parent)
        .map_err(|error| {
            format!(
                "could not create a temporary file in {}: {error}",
                parent.display()
            )
        })?;

    let write_result = (|| -> std::io::Result<()> {
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()
    })();
    if let Err(error) = write_result {
        return Err(format!(
            "could not write a temporary settings file in {}: {error}",
            parent.display()
        ));
    }

    temporary.persist(path).map_err(|error| {
        format!(
            "could not save settings to {}: {}",
            path.display(),
            error.error
        )
    })?;
    // The replacement is already visible at this point. Directory syncing is
    // best-effort because reporting a post-commit durability error would make
    // callers roll back in-memory state while the new file is on disk.
    sync_parent_directory(path);
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    sibling_path(path, ".bak")
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut file_name = path.file_name().unwrap_or_default().to_os_string();
    file_name.push(suffix);
    path.with_file_name(file_name)
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn quarantine_settings(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let parent = parent_directory(path);
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut quarantine = TempFileBuilder::new()
        .prefix(&format!("{file_name}.corrupt-"))
        .tempfile_in(parent)
        .map_err(|error| {
            format!(
                "could not preserve corrupt settings in {}: {error}",
                parent.display()
            )
        })?;
    quarantine
        .write_all(bytes)
        .and_then(|_| quarantine.as_file().sync_all())
        .map_err(|error| format!("could not preserve corrupt settings: {error}"))?;
    let (_, quarantine_path) = quarantine
        .keep()
        .map_err(|error| format!("could not retain corrupt settings: {}", error.error))?;
    sync_parent_directory(&quarantine_path);
    Ok(quarantine_path)
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) {
    let parent = parent_directory(path);
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) {}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        domain::{
            DictationDelivery, DictationDeliveryStatus, EngineConfig, EngineLocation,
            EngineProvider, HudPosition, HudSettings, LanguagePolicy,
            BUILTIN_READABLE_CLEANUP_MODEL,
        },
        engines::TranscriptResult,
    };
    use tempfile::TempDir;

    fn test_path() -> (TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        (directory, path)
    }

    #[test]
    fn missing_settings_file_is_created_with_defaults() {
        let (_directory, path) = test_path();

        let state = AppState::load(path.clone()).unwrap();
        assert_eq!(state.settings_snapshot().unwrap(), AppSettings::default());
        assert!(path.exists());
    }

    #[test]
    fn persisted_settings_round_trip_without_credentials() {
        let (_directory, path) = test_path();
        let state = AppState::load(path.clone()).unwrap();
        let mut changed = state.settings_snapshot().unwrap();
        changed.language_policy = LanguagePolicy::Mixed {
            languages: vec!["en-IN".into(), "hi-IN".into()],
        };
        changed.transcription_engine.provider = EngineProvider::OpenAi;
        changed.transcription_engine.model = "gpt-4o-transcribe".into();
        changed.transcription_engine.location = EngineLocation::Cloud;
        state.persist_settings(&changed).unwrap();

        let reloaded = AppState::load(path.clone()).unwrap();
        assert_eq!(reloaded.settings_snapshot().unwrap(), changed);

        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.to_ascii_lowercase().contains("apikey"));
        assert!(!raw.to_ascii_lowercase().contains("secret"));
    }

    #[test]
    fn legacy_unsupported_cleanup_is_deactivated_without_resetting_other_settings() {
        let (_directory, path) = test_path();
        let legacy = AppSettings {
            schema_version: LEGACY_SETTINGS_SCHEMA_VERSION,
            push_to_talk_shortcut: "Control+Option+D".into(),
            language_policy: LanguagePolicy::Mixed {
                languages: vec!["en-IN".into(), "hi-IN".into()],
            },
            transcription_engine: EngineConfig {
                provider: EngineProvider::OpenAi,
                model: "gpt-4o-transcribe".into(),
                location: EngineLocation::Cloud,
            },
            cleanup_engine: Some(EngineConfig::local(
                EngineProvider::LlamaCpp,
                "old-local-polisher",
            )),
            input_device_name: None,
            hud: HudSettings {
                position: HudPosition::BottomRight,
                ..HudSettings::default()
            },
            allow_cloud_fallback: true,
            save_transcript_history: true,
        };
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&path, &original).unwrap();

        let state = AppState::load(path.clone()).unwrap();
        let expected = AppSettings {
            schema_version: SETTINGS_SCHEMA_VERSION,
            cleanup_engine: None,
            hud: HudSettings {
                visible: true,
                ..legacy.hud.clone()
            },
            ..legacy
        };

        assert_eq!(state.settings_snapshot().unwrap(), expected);
        assert_eq!(parse_settings(&fs::read(&path).unwrap()).unwrap(), expected);
        assert_eq!(fs::read(backup_path(&path)).unwrap(), original);
    }

    #[test]
    fn legacy_builtin_cleanup_is_deactivated_without_implied_consent() {
        let (_directory, path) = test_path();
        let legacy = AppSettings {
            schema_version: LEGACY_SETTINGS_SCHEMA_VERSION,
            push_to_talk_shortcut: "Control+Option+D".into(),
            cleanup_engine: Some(EngineConfig::local(
                EngineProvider::BuiltIn,
                BUILTIN_READABLE_CLEANUP_MODEL,
            )),
            ..AppSettings::default()
        };
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&path, &original).unwrap();

        let state = AppState::load(path.clone()).unwrap();
        let expected = AppSettings {
            schema_version: SETTINGS_SCHEMA_VERSION,
            cleanup_engine: None,
            hud: HudSettings {
                visible: true,
                ..legacy.hud.clone()
            },
            ..legacy
        };

        assert_eq!(state.settings_snapshot().unwrap(), expected);
        assert_eq!(parse_settings(&fs::read(&path).unwrap()).unwrap(), expected);
        assert_eq!(fs::read(backup_path(&path)).unwrap(), original);
    }

    #[test]
    fn explicit_schema_v2_builtin_cleanup_remains_selected() {
        let (_directory, path) = test_path();
        let previous = AppSettings {
            schema_version: OPTION_DEFAULT_SETTINGS_SCHEMA_VERSION,
            push_to_talk_shortcut: "Control+Option+D".into(),
            cleanup_engine: Some(EngineConfig::local(
                EngineProvider::BuiltIn,
                BUILTIN_READABLE_CLEANUP_MODEL,
            )),
            ..AppSettings::default()
        };
        let original = serde_json::to_vec_pretty(&previous).unwrap();
        fs::write(&path, &original).unwrap();

        let state = AppState::load(path.clone()).unwrap();
        let expected = AppSettings {
            schema_version: SETTINGS_SCHEMA_VERSION,
            hud: HudSettings {
                visible: true,
                ..previous.hud.clone()
            },
            ..previous
        };

        assert_eq!(state.settings_snapshot().unwrap(), expected);
        assert_eq!(parse_settings(&fs::read(&path).unwrap()).unwrap(), expected);
        assert_eq!(fs::read(backup_path(&path)).unwrap(), original);
    }

    #[test]
    fn schema_v3_adds_device_and_hud_defaults_without_losing_choices() {
        let (_directory, path) = test_path();
        let mut value = serde_json::to_value(AppSettings {
            language_policy: LanguagePolicy::Fixed {
                language: "ja".into(),
            },
            save_transcript_history: true,
            ..AppSettings::default()
        })
        .unwrap();
        let object = value.as_object_mut().unwrap();
        object.insert(
            "schemaVersion".into(),
            serde_json::Value::from(MULTILINGUAL_SETTINGS_SCHEMA_VERSION),
        );
        object.remove("inputDeviceName");
        object
            .get_mut("hud")
            .and_then(serde_json::Value::as_object_mut)
            .unwrap()
            .remove("visible");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let loaded = AppState::load(path.clone())
            .unwrap()
            .settings_snapshot()
            .unwrap();

        assert_eq!(loaded.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(loaded.input_device_name, None);
        assert!(loaded.hud.visible);
        assert_eq!(
            loaded.language_policy,
            LanguagePolicy::Fixed {
                language: "ja".into()
            }
        );
        assert!(loaded.save_transcript_history);
    }

    #[test]
    fn schema_v4_moves_only_an_undragged_hud_and_preserves_the_original_backup() {
        let (_directory, path) = test_path();
        let previous = AppSettings {
            schema_version: TRANSIENT_HUD_SETTINGS_SCHEMA_VERSION,
            hud: HudSettings {
                position: HudPosition::BottomCenter,
                custom_position: None,
                ..HudSettings::default()
            },
            ..AppSettings::default()
        };
        let original = serde_json::to_vec_pretty(&previous).unwrap();
        fs::write(&path, &original).unwrap();

        let loaded = AppState::load(path.clone())
            .unwrap()
            .settings_snapshot()
            .unwrap();

        assert_eq!(loaded.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(loaded.hud.position, HudPosition::BottomRight);
        assert_eq!(loaded.hud.custom_position, None);
        assert_eq!(parse_settings(&fs::read(&path).unwrap()).unwrap(), loaded);
        assert_eq!(fs::read(backup_path(&path)).unwrap(), original);
    }

    #[test]
    fn schema_v4_preserves_a_dragged_hud_coordinate() {
        let (_directory, path) = test_path();
        let dragged_position = crate::domain::HudCoordinates { x: 1456, y: 880 };
        let previous = AppSettings {
            schema_version: TRANSIENT_HUD_SETTINGS_SCHEMA_VERSION,
            hud: HudSettings {
                position: HudPosition::BottomCenter,
                custom_position: Some(dragged_position),
                ..HudSettings::default()
            },
            ..AppSettings::default()
        };
        fs::write(&path, serde_json::to_vec_pretty(&previous).unwrap()).unwrap();

        let loaded = AppState::load(path).unwrap().settings_snapshot().unwrap();

        assert_eq!(loaded.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(loaded.hud.position, HudPosition::BottomCenter);
        assert_eq!(loaded.hud.custom_position, Some(dragged_position));
    }

    #[test]
    fn new_unsupported_cleanup_selection_is_rejected_without_touching_settings() {
        let (_directory, path) = test_path();
        let state = AppState::load(path.clone()).unwrap();
        let original = fs::read(&path).unwrap();
        let mut unsupported = state.settings_snapshot().unwrap();
        unsupported.cleanup_engine = Some(EngineConfig::local(
            EngineProvider::LlamaCpp,
            "new-local-polisher",
        ));

        let error = state.persist_settings(&unsupported).unwrap_err();

        assert!(error.contains("only the built-in readable-v1 cleanup engine"));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn corrupt_settings_recover_from_the_last_known_good_backup() {
        let (directory, path) = test_path();
        let backup = backup_path(&path);

        let state = AppState::load(path.clone()).unwrap();
        let mut first_change = state.settings_snapshot().unwrap();
        first_change.language_policy = LanguagePolicy::Fixed {
            language: "fr-FR".into(),
        };
        state.persist_settings(&first_change).unwrap();

        let mut second_change = first_change.clone();
        second_change.language_policy = LanguagePolicy::Fixed {
            language: "de-DE".into(),
        };
        state.persist_settings(&second_change).unwrap();
        fs::write(&path, b"{not valid json").unwrap();

        let recovered = AppState::load(path.clone()).unwrap();
        assert_eq!(recovered.settings_snapshot().unwrap(), first_change);
        assert_eq!(
            parse_settings(&fs::read(&path).unwrap()).unwrap(),
            first_change
        );

        let prefix = format!("{}.corrupt-", path.file_name().unwrap().to_string_lossy());
        assert!(fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix)));
        assert!(backup.exists());
    }

    #[test]
    fn corrupt_settings_without_a_backup_fall_back_to_defaults() {
        let (_directory, path) = test_path();
        fs::write(&path, b"[]").unwrap();

        let state = AppState::load(path.clone()).unwrap();
        assert_eq!(state.settings_snapshot().unwrap(), AppSettings::default());
        assert_eq!(
            parse_settings(&fs::read(&path).unwrap()).unwrap(),
            AppSettings::default()
        );
    }

    #[test]
    fn a_missing_main_file_is_restored_from_a_valid_backup() {
        let (_directory, path) = test_path();
        let backup = backup_path(&path);
        let expected = AppSettings {
            language_policy: LanguagePolicy::Fixed {
                language: "bn-IN".into(),
            },
            ..AppSettings::default()
        };
        let json = serde_json::to_vec_pretty(&expected).unwrap();
        fs::write(&backup, json).unwrap();

        let state = AppState::load(path.clone()).unwrap();

        assert_eq!(state.settings_snapshot().unwrap(), expected);
        assert_eq!(parse_settings(&fs::read(path).unwrap()).unwrap(), expected);
    }

    #[test]
    fn an_unknown_schema_is_preserved_and_never_replaced_with_defaults() {
        let (directory, path) = test_path();
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value["schemaVersion"] = serde_json::json!(SETTINGS_SCHEMA_VERSION + 1);
        let original = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&path, &original).unwrap();

        let error = AppState::load(path.clone()).err().unwrap();

        assert!(error.contains("unsupported schema version"));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn unreadable_settings_targets_are_not_overwritten() {
        let (_directory, path) = test_path();
        fs::create_dir(&path).unwrap();

        let error = AppState::load(path.clone()).err().unwrap();

        assert!(error.contains("could not read settings"));
        assert!(path.is_dir());
    }

    #[test]
    fn transcript_store_rejects_cancelled_and_stale_results() {
        let mut store = TranscriptStore::default();
        let cancellation = store.begin("session-a".into(), None, None, Arc::from([]), false);
        assert!(!cancellation.load(Ordering::Relaxed));
        assert!(!store.complete(transcript("session-b", "stale")));

        store.cancel("session-a");
        assert!(cancellation.load(Ordering::Relaxed));
        assert!(!store.complete(transcript("session-a", "cancelled")));
        assert!(store.latest.is_none());

        store.begin("session-c".into(), None, None, Arc::from([]), false);
        assert!(store.complete(transcript("session-c", "kept")));
        assert_eq!(store.latest.as_ref().unwrap().transcript.text, "kept");

        store.begin("session-d".into(), None, None, Arc::from([]), false);
        assert!(store.latest.is_none());
    }

    #[test]
    fn cloud_upload_claim_and_cancellation_have_one_ordered_boundary() {
        let mut store = TranscriptStore::default();
        store.begin("cancel-first".into(), None, None, Arc::from([]), true);
        store.cancel("cancel-first");
        assert!(!store.claim_cloud_upload("cancel-first"));

        let cancellation = store.begin("upload-first".into(), None, None, Arc::from([]), true);
        assert!(store.claim_cloud_upload("upload-first"));
        assert!(!store.claim_cloud_upload("upload-first"));
        store.cancel("upload-first");
        assert!(cancellation.load(Ordering::Relaxed));
    }

    #[test]
    fn clearing_latest_returns_the_exact_suppression_identity() {
        let (_directory, path) = test_path();
        let state = AppState::load(path).unwrap();
        state
            .begin_transcription("session-a".into(), None, None, Arc::from([]), false)
            .unwrap();
        assert!(state
            .complete_transcription(transcript("session-a", "queued text"))
            .unwrap());

        assert_eq!(
            state.clear_latest_transcript().unwrap(),
            Some("session-a".into())
        );
        assert_eq!(state.clear_latest_transcript().unwrap(), None);
    }

    #[test]
    fn latency_cache_keeps_only_the_newest_terminal_revision_in_memory() {
        let (_directory, path) = test_path();
        let state = AppState::load(path).unwrap();

        assert_eq!(state.latest_dictation_latency().unwrap(), None);
        assert!(state.record_dictation_latency(latency_event(8)).unwrap());
        assert!(!state.record_dictation_latency(latency_event(7)).unwrap());
        assert!(!state.record_dictation_latency(latency_event(8)).unwrap());
        assert!(state.record_dictation_latency(latency_event(9)).unwrap());
        assert_eq!(
            state.latest_dictation_latency().unwrap().unwrap().revision,
            9
        );
    }

    #[test]
    fn active_latency_trace_is_taken_only_by_its_exact_session() {
        let (_directory, path) = test_path();
        let state = AppState::load(path).unwrap();
        let trace = StartupLatencyTrace::start();
        assert!(trace.bind_session("session-a"));
        state.register_dictation_latency(trace).unwrap();

        assert!(state
            .take_dictation_latency_trace("session-b")
            .unwrap()
            .is_none());
        assert!(state
            .dictation_latency_trace("session-a")
            .unwrap()
            .is_some());
        assert!(state
            .take_dictation_latency_trace("session-a")
            .unwrap()
            .is_some());
        assert!(state
            .dictation_latency_trace("session-a")
            .unwrap()
            .is_none());
    }

    #[test]
    fn cloud_fallback_permission_is_snapshotted_per_session() {
        let (_directory, path) = test_path();
        let state = AppState::load(path).unwrap();
        state
            .begin_transcription("session-a".into(), None, None, Arc::from([]), true)
            .unwrap();
        assert!(
            state
                .transcription_operation("session-a")
                .unwrap()
                .unwrap()
                .allow_cloud_fallback
        );

        state.settings.write().unwrap().allow_cloud_fallback = false;
        assert!(
            state
                .transcription_operation("session-a")
                .unwrap()
                .unwrap()
                .allow_cloud_fallback
        );
    }

    #[test]
    fn stale_hud_lease_cannot_release_a_newer_session() {
        let mut protection = HudTargetProtection::default();
        let old = protection.claim("session-a".into());
        let current = protection.claim("session-b".into());

        assert!(!protection.release_if_current(&old));
        assert!(protection.is_current(&current));
        assert!(protection.release_if_current(&current));
        assert!(!protection.has_owner());
    }

    fn transcript(session_id: &str, text: &str) -> DictationTranscript {
        DictationTranscript {
            session_id: session_id.into(),
            engine_id: "whisper-test".into(),
            transcript: TranscriptResult::final_text(text),
            delivery: DictationDelivery {
                status: DictationDeliveryStatus::Unsupported,
                transcript_available: true,
                target_app: None,
                caret_repositioned: None,
            },
        }
    }

    fn latency_event(revision: u64) -> DictationLatencyEvent {
        DictationLatencyEvent {
            session_id: format!("session-{revision}"),
            revision,
            outcome: crate::latency::DictationLatencyOutcome::Completed,
            target_capture_ms: Some(1),
            start_to_target_capture_return_ms: Some(2),
            start_to_audio_owner_spawn_ms: Some(4),
            start_to_starting_emitted_ms: Some(5),
            start_to_hud_show_return_ms: Some(7),
            start_to_microphone_ready_ms: Some(6),
            start_to_listening_emitted_ms: Some(8),
            audio_duration_ms: Some(1_000),
            stop_to_processing_ms: Some(2),
            capture_finalize_ms: Some(3),
            transcription_ms: Some(80),
            delivery_ms: Some(4),
            stop_to_delivery_ms: Some(91),
            processing_total_ms: Some(94),
        }
    }
}
