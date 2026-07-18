use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
};

use crate::{domain::AppSettings, session::SessionController};

/// Process-wide state managed by Tauri.
///
/// Settings are read far more often than they are replaced, while session
/// transitions must be serialized. The lock choices mirror those access
/// patterns and keep the UI/event layer independent from provider runtimes.
pub struct AppState {
    pub settings: RwLock<AppSettings>,
    pub session: Mutex<SessionController>,
    settings_path: PathBuf,
}

impl AppState {
    pub fn load(settings_path: PathBuf) -> Result<Self, String> {
        let settings = load_settings(&settings_path)?;
        Ok(Self {
            settings: RwLock::new(settings),
            session: Mutex::new(SessionController::default()),
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
}

fn load_settings(path: &Path) -> Result<AppSettings, String> {
    if !path.exists() {
        let settings = AppSettings::default();
        write_settings(path, &settings)?;
        return Ok(settings);
    }

    let bytes = fs::read(path)
        .map_err(|error| format!("could not read settings from {}: {error}", path.display()))?;
    let settings: AppSettings = serde_json::from_slice(&bytes)
        .map_err(|error| format!("could not parse settings from {}: {error}", path.display()))?;
    settings.validate()?;
    Ok(settings)
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
    fs::write(path, json)
        .map_err(|error| format!("could not save settings to {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use super::*;
    use crate::domain::{EngineLocation, EngineProvider, LanguagePolicy};

    fn test_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "spick-{name}-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
    }

    #[test]
    fn missing_settings_file_is_created_with_defaults() {
        let path = test_path("settings-default");
        let _ = fs::remove_file(&path);

        let state = AppState::load(path.clone()).unwrap();
        assert_eq!(state.settings_snapshot().unwrap(), AppSettings::default());
        assert!(path.exists());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn persisted_settings_round_trip_without_credentials() {
        let path = test_path("settings-round-trip");
        let _ = fs::remove_file(&path);
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

        let _ = fs::remove_file(path);
    }
}
