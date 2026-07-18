use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
};

use tempfile::Builder as TempFileBuilder;

use crate::{
    audio::AudioCaptureController,
    domain::{AppSettings, SETTINGS_SCHEMA_VERSION},
    engines::{DictationTranscript, WhisperCppRuntime},
    model_store::ModelStore,
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
    /// Serializes model selection/removal with settings writes so an active
    /// model cannot disappear between verification and persistence.
    pub model_configuration: Mutex<()>,
    transcripts: Mutex<TranscriptStore>,
    settings_path: PathBuf,
}

impl AppState {
    #[cfg(test)]
    pub fn load(settings_path: PathBuf) -> Result<Self, String> {
        let models_path = parent_directory(&settings_path).join("models");
        Self::load_with_models(settings_path, models_path)
    }

    pub fn load_with_models(settings_path: PathBuf, models_path: PathBuf) -> Result<Self, String> {
        let settings = load_settings(&settings_path)?;
        Ok(Self {
            settings: RwLock::new(settings),
            session: Mutex::new(SessionController::default()),
            audio: Mutex::new(AudioCaptureController::default()),
            models: Arc::new(ModelStore::new(models_path)),
            whisper: WhisperCppRuntime::default(),
            model_configuration: Mutex::new(()),
            transcripts: Mutex::new(TranscriptStore::default()),
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

    pub fn begin_transcription(&self, session_id: String) -> Result<Arc<AtomicBool>, String> {
        self.transcripts
            .lock()
            .map(|mut transcripts| transcripts.begin(session_id))
            .map_err(|_| "transcript store is unavailable".into())
    }

    pub fn cancel_transcription(&self, session_id: &str) -> Result<(), String> {
        self.transcripts
            .lock()
            .map(|mut transcripts| transcripts.cancel(session_id))
            .map_err(|_| "transcript store is unavailable".into())
    }

    pub fn transcription_cancellation(
        &self,
        session_id: &str,
    ) -> Result<Option<Arc<AtomicBool>>, String> {
        self.transcripts
            .lock()
            .map(|transcripts| {
                transcripts
                    .active
                    .as_ref()
                    .filter(|active| active.session_id == session_id)
                    .map(|active| Arc::clone(&active.cancellation))
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
}

#[derive(Default)]
struct TranscriptStore {
    active: Option<ActiveTranscription>,
    latest: Option<DictationTranscript>,
}

struct ActiveTranscription {
    session_id: String,
    cancellation: Arc<AtomicBool>,
}

impl TranscriptStore {
    fn begin(&mut self, session_id: String) -> Arc<AtomicBool> {
        if let Some(active) = self.active.take() {
            active.cancellation.store(true, Ordering::Relaxed);
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        self.active = Some(ActiveTranscription {
            session_id,
            cancellation: Arc::clone(&cancellation),
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

    match parse_settings(&bytes) {
        Ok(settings) => Ok(settings),
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
    let settings: AppSettings = serde_json::from_slice(bytes)
        .map_err(|error| SettingsParseError::InvalidJson(error.to_string()))?;
    if settings.schema_version != SETTINGS_SCHEMA_VERSION {
        return Err(SettingsParseError::UnsupportedSchema(
            settings.schema_version,
        ));
    }
    settings
        .validate()
        .map_err(SettingsParseError::InvalidSettings)?;
    Ok(settings)
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
        domain::{EngineLocation, EngineProvider, LanguagePolicy},
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
        let cancellation = store.begin("session-a".into());
        assert!(!cancellation.load(Ordering::Relaxed));
        assert!(!store.complete(transcript("session-b", "stale")));

        store.cancel("session-a");
        assert!(cancellation.load(Ordering::Relaxed));
        assert!(!store.complete(transcript("session-a", "cancelled")));
        assert!(store.latest.is_none());

        store.begin("session-c".into());
        assert!(store.complete(transcript("session-c", "kept")));
        assert_eq!(store.latest.as_ref().unwrap().transcript.text, "kept");

        store.begin("session-d".into());
        assert!(store.latest.is_none());
    }

    fn transcript(session_id: &str, text: &str) -> DictationTranscript {
        DictationTranscript {
            session_id: session_id.into(),
            engine_id: "whisper-test".into(),
            transcript: TranscriptResult::final_text(text),
        }
    }
}
