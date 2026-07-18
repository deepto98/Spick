//! Serializable domain types shared by the Tauri commands and the web UI.
//!
//! Keep this module free of Tauri and operating-system APIs. That makes the
//! dictation lifecycle testable and gives every platform adapter the same
//! contract.

use serde::{Deserialize, Serialize};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_PUSH_TO_TALK_SHORTCUT: &str = "CommandOrControl+Shift+Space";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LanguagePolicy {
    /// Let the active transcription engine detect a language for each session.
    #[default]
    Auto,
    /// Pin transcription to a single BCP-47 language tag.
    Fixed { language: String },
    /// Auto-detect from a smaller user-selected language set.
    Preferred { languages: Vec<String> },
    /// Preserve code-switching between the listed languages.
    Mixed { languages: Vec<String> },
    /// Transcribe from the selected sources and normalize to one output language.
    Translate {
        source_languages: Vec<String>,
        output_language: String,
    },
}

impl LanguagePolicy {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Auto => Ok(()),
            Self::Fixed { language } => validate_language_tag(language),
            Self::Preferred { languages } | Self::Mixed { languages } => {
                validate_language_list(languages)
            }
            Self::Translate {
                source_languages,
                output_language,
            } => {
                validate_language_list(source_languages)?;
                validate_language_tag(output_language)
            }
        }
    }
}

fn validate_language_list(languages: &[String]) -> Result<(), String> {
    if languages.is_empty() {
        return Err("at least one language must be selected".into());
    }

    for language in languages {
        validate_language_tag(language)?;
    }

    Ok(())
}

fn validate_language_tag(language: &str) -> Result<(), String> {
    let language = language.trim();
    if language.is_empty() {
        return Err("language tags cannot be empty".into());
    }

    // A deliberately small BCP-47 guard. Providers support different subsets,
    // so provider-specific capability validation belongs in an engine adapter.
    if language.len() > 35
        || language.starts_with('-')
        || language.ends_with('-')
        || !language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(format!("invalid language tag: {language}"));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineProvider {
    BuiltIn,
    WhisperCpp,
    LlamaCpp,
    OpenAi,
    Gemini,
    XAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineLocation {
    Local,
    Cloud,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
    pub provider: EngineProvider,
    pub model: String,
    pub location: EngineLocation,
}

impl EngineConfig {
    pub fn local(provider: EngineProvider, model: impl Into<String>) -> Self {
        Self {
            provider,
            model: model.into(),
            location: EngineLocation::Local,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.model.trim().is_empty() {
            return Err("engine model cannot be empty".into());
        }

        let requires_local = matches!(
            self.provider,
            EngineProvider::BuiltIn | EngineProvider::WhisperCpp | EngineProvider::LlamaCpp
        );
        let requires_cloud = matches!(
            self.provider,
            EngineProvider::OpenAi | EngineProvider::Gemini | EngineProvider::XAi
        );

        if requires_local && self.location != EngineLocation::Local {
            return Err(format!(
                "provider {:?} is only available as a local engine",
                self.provider
            ));
        }
        if requires_cloud && self.location != EngineLocation::Cloud {
            return Err(format!(
                "provider {:?} is only available as a cloud engine",
                self.provider
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HudPosition {
    BottomLeft,
    #[default]
    BottomCenter,
    BottomRight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct HudSettings {
    pub position: HudPosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub push_to_talk_shortcut: String,
    pub language_policy: LanguagePolicy,
    pub transcription_engine: EngineConfig,
    pub cleanup_engine: Option<EngineConfig>,
    pub hud: HudSettings,
    pub allow_cloud_fallback: bool,
    pub save_transcript_history: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            push_to_talk_shortcut: DEFAULT_PUSH_TO_TALK_SHORTCUT.into(),
            language_policy: LanguagePolicy::Auto,
            transcription_engine: EngineConfig::local(EngineProvider::WhisperCpp, "small-q5_1"),
            cleanup_engine: Some(EngineConfig::local(EngineProvider::BuiltIn, "readable-v1")),
            hud: HudSettings::default(),
            allow_cloud_fallback: false,
            save_transcript_history: false,
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(format!(
                "unsupported settings schema version: {}",
                self.schema_version
            ));
        }
        if self.push_to_talk_shortcut.trim().is_empty() {
            return Err("push-to-talk shortcut cannot be empty".into());
        }

        self.language_policy.validate()?;
        self.transcription_engine.validate()?;
        if let Some(cleanup_engine) = &self.cleanup_engine {
            cleanup_engine.validate()?;
        }

        if !self.allow_cloud_fallback && self.transcription_engine.location == EngineLocation::Cloud
        {
            // Selecting a cloud engine explicitly is different from allowing an
            // automatic fallback, so this combination remains valid.
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    Idle,
    Listening,
    Processing,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionTrigger {
    Shortcut,
    UserInterface,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationSession {
    pub id: String,
    pub state: SessionState,
    pub trigger: SessionTrigger,
    /// Snapshot the policy so an in-flight session is deterministic even if
    /// settings are edited concurrently.
    pub language_policy: LanguagePolicy,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub cancel_reason: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationStateEvent {
    /// Monotonic lifecycle revision used by webviews to ignore stale command
    /// responses when an emitted transition has already superseded them.
    pub revision: u64,
    pub state: SessionState,
    pub session: Option<DictationSession>,
}

impl DictationStateEvent {
    pub fn idle() -> Self {
        Self {
            revision: 0,
            state: SessionState::Idle,
            session: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_are_local_private_and_multilingual() {
        let settings = AppSettings::default();

        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(
            settings.push_to_talk_shortcut,
            "CommandOrControl+Shift+Space"
        );
        assert_eq!(settings.language_policy, LanguagePolicy::Auto);
        assert_eq!(
            settings.transcription_engine.provider,
            EngineProvider::WhisperCpp
        );
        assert_eq!(
            settings.transcription_engine.location,
            EngineLocation::Local
        );
        assert!(!settings.allow_cloud_fallback);
        assert!(!settings.save_transcript_history);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn settings_json_never_has_a_secret_field() {
        let value = serde_json::to_value(AppSettings::default()).unwrap();
        let object = value.as_object().unwrap();

        assert!(!object.contains_key("apiKey"));
        assert!(!object.contains_key("secret"));
        assert!(!object.contains_key("token"));
    }

    #[test]
    fn language_policy_requires_well_formed_non_empty_language_tags() {
        assert!(LanguagePolicy::Mixed {
            languages: vec!["en-IN".into(), "hi-IN".into()]
        }
        .validate()
        .is_ok());
        assert!(LanguagePolicy::Preferred { languages: vec![] }
            .validate()
            .is_err());
        assert!(LanguagePolicy::Fixed {
            language: "not a language".into()
        }
        .validate()
        .is_err());
    }
}
