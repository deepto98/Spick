//! Serializable domain types shared by the Tauri commands and the web UI.
//!
//! Keep this module free of Tauri and operating-system APIs. That makes the
//! dictation lifecycle testable and gives every platform adapter the same
//! contract.

use serde::{Deserialize, Serialize};

pub(crate) const LEGACY_SETTINGS_SCHEMA_VERSION: u32 = 1;
pub(crate) const OPTION_DEFAULT_SETTINGS_SCHEMA_VERSION: u32 = 2;
pub(crate) const MULTILINGUAL_SETTINGS_SCHEMA_VERSION: u32 = 3;
pub(crate) const TRANSIENT_HUD_SETTINGS_SCHEMA_VERSION: u32 = 4;
pub const SETTINGS_SCHEMA_VERSION: u32 = 5;
#[cfg(target_os = "macos")]
pub const DEFAULT_PUSH_TO_TALK_SHORTCUT: &str = "Option";
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_PUSH_TO_TALK_SHORTCUT: &str = "CommandOrControl+Shift+Space";
pub(crate) const LEGACY_DEFAULT_PUSH_TO_TALK_SHORTCUT: &str = "CommandOrControl+Shift+Space";
pub const BUILTIN_READABLE_CLEANUP_MODEL: &str = "readable-v1";

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

    pub fn is_builtin_readable_cleanup(&self) -> bool {
        self.provider == EngineProvider::BuiltIn
            && self.location == EngineLocation::Local
            && self.model == BUILTIN_READABLE_CLEANUP_MODEL
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HudPosition {
    BottomLeft,
    BottomCenter,
    #[default]
    BottomRight,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HudPresentation {
    Expanded,
    #[default]
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HudCoordinates {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HudSettings {
    pub position: HudPosition,
    pub presentation: HudPresentation,
    pub custom_position: Option<HudCoordinates>,
    pub visible: bool,
}

impl Default for HudSettings {
    fn default() -> Self {
        Self {
            position: HudPosition::default(),
            presentation: HudPresentation::default(),
            custom_position: None,
            // A fresh install reveals the widget from the final onboarding
            // step. Once acknowledged, the persisted choice shows it at
            // startup on every later launch.
            visible: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub push_to_talk_shortcut: String,
    pub language_policy: LanguagePolicy,
    pub transcription_engine: EngineConfig,
    pub cleanup_engine: Option<EngineConfig>,
    /// `None` follows the operating system's current default input device.
    /// A named choice is snapshotted when a session begins.
    pub input_device_name: Option<String>,
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
            transcription_engine: EngineConfig::local(
                EngineProvider::WhisperCpp,
                "whisper-tiny-multilingual-f16",
            ),
            // Cleanup changes a user's words, so it is an explicit opt-in.
            cleanup_engine: None,
            input_device_name: None,
            hud: HudSettings::default(),
            allow_cloud_fallback: false,
            save_transcript_history: false,
        }
    }
}

impl AppSettings {
    /// Upgrade settings defaults without overwriting an explicit user choice.
    ///
    /// Schema v1 could contain a cleanup selection even though cleanup was not
    /// connected, so that selection is disabled. Schema v2 also predates the
    /// macOS Option default. Schema v3 predates persisted microphone/HUD
    /// visibility choices. Schema v4 predates the persistent corner widget;
    /// its implicit bottom-center placement moves to bottom-right unless the
    /// user already dragged the HUD. All migrations preserve explicit choices.
    pub(crate) fn migrate_legacy_schema(&mut self) -> bool {
        match self.schema_version {
            LEGACY_SETTINGS_SCHEMA_VERSION => {
                self.cleanup_engine = None;
                #[cfg(target_os = "macos")]
                if self.push_to_talk_shortcut == LEGACY_DEFAULT_PUSH_TO_TALK_SHORTCUT {
                    self.push_to_talk_shortcut = DEFAULT_PUSH_TO_TALK_SHORTCUT.into();
                }
                self.hud.visible = true;
                self.migrate_implicit_hud_position();
                self.schema_version = SETTINGS_SCHEMA_VERSION;
                true
            }
            OPTION_DEFAULT_SETTINGS_SCHEMA_VERSION => {
                #[cfg(target_os = "macos")]
                if self.push_to_talk_shortcut == LEGACY_DEFAULT_PUSH_TO_TALK_SHORTCUT {
                    self.push_to_talk_shortcut = DEFAULT_PUSH_TO_TALK_SHORTCUT.into();
                }
                self.hud.visible = true;
                self.migrate_implicit_hud_position();
                self.schema_version = SETTINGS_SCHEMA_VERSION;
                true
            }
            MULTILINGUAL_SETTINGS_SCHEMA_VERSION => {
                self.hud.visible = true;
                self.migrate_implicit_hud_position();
                self.schema_version = SETTINGS_SCHEMA_VERSION;
                true
            }
            TRANSIENT_HUD_SETTINGS_SCHEMA_VERSION => {
                self.migrate_implicit_hud_position();
                self.schema_version = SETTINGS_SCHEMA_VERSION;
                true
            }
            _ => false,
        }
    }

    fn migrate_implicit_hud_position(&mut self) {
        if self.hud.custom_position.is_none() && self.hud.position == HudPosition::BottomCenter {
            self.hud.position = HudPosition::BottomRight;
        }
    }

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
        if let Some(name) = self.input_device_name.as_deref() {
            let name = name.trim();
            if name.is_empty() {
                return Err("selected microphone name cannot be empty".into());
            }
            if name.len() > 512 || name.chars().any(char::is_control) {
                return Err("selected microphone name is invalid".into());
            }
        }

        self.language_policy.validate()?;
        self.transcription_engine.validate()?;
        if let Some(cleanup_engine) = &self.cleanup_engine {
            cleanup_engine.validate()?;
            if !cleanup_engine.is_builtin_readable_cleanup() {
                return Err(
                    "only the built-in readable-v1 cleanup engine is available right now".into(),
                );
            }
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
    Starting,
    Listening,
    Processing,
    Inserting,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationDeliveryStatus {
    Inserted,
    FocusChanged,
    SecureField,
    AccessibilityMissing,
    Unsupported,
    Failed,
    /// A native write returned an ambiguous result. Spick must not retry or
    /// offer one-click copy until the user checks the target for duplicates.
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationDelivery {
    pub status: DictationDeliveryStatus,
    pub transcript_available: bool,
    pub target_app: Option<String>,
    pub caret_repositioned: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionTrigger {
    Shortcut,
    FloatingWidget,
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
    /// Snapshot the engine for the same reason. A model switch only affects the
    /// next dictation session.
    pub transcription_engine: EngineConfig,
    /// `None` means as-spoken output. The selected cleanup engine is captured
    /// with the session so a settings edit cannot alter words already being
    /// transcribed.
    pub cleanup_engine: Option<EngineConfig>,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub cancel_reason: Option<String>,
    pub error: Option<String>,
    /// Structured terminal delivery result. This intentionally contains no
    /// transcript text, field contents, process IDs, or native identifiers.
    pub delivery: Option<DictationDelivery>,
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
            DEFAULT_PUSH_TO_TALK_SHORTCUT
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
        assert_eq!(
            settings.transcription_engine.model,
            "whisper-tiny-multilingual-f16"
        );
        assert_eq!(settings.cleanup_engine, None);
        assert_eq!(settings.input_device_name, None);
        assert_eq!(settings.hud.position, HudPosition::BottomRight);
        assert_eq!(settings.hud.presentation, HudPresentation::Compact);
        assert!(!settings.hud.visible);
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

    #[test]
    fn settings_accept_as_spoken_or_the_available_deterministic_cleanup_only() {
        let mut settings = AppSettings {
            cleanup_engine: None,
            ..AppSettings::default()
        };
        assert!(settings.validate().is_ok());

        settings.cleanup_engine = Some(EngineConfig::local(
            EngineProvider::BuiltIn,
            BUILTIN_READABLE_CLEANUP_MODEL,
        ));
        assert!(settings.validate().is_ok());

        settings.cleanup_engine = Some(EngineConfig::local(
            EngineProvider::LlamaCpp,
            "unconnected-polisher",
        ));
        assert_eq!(
            settings.validate(),
            Err("only the built-in readable-v1 cleanup engine is available right now".into())
        );
    }

    #[test]
    fn schema_v1_migration_disables_every_legacy_cleanup_selection() {
        for cleanup_engine in [
            EngineConfig::local(EngineProvider::BuiltIn, BUILTIN_READABLE_CLEANUP_MODEL),
            EngineConfig::local(EngineProvider::LlamaCpp, "old-local-polisher"),
        ] {
            let mut legacy = AppSettings {
                schema_version: LEGACY_SETTINGS_SCHEMA_VERSION,
                push_to_talk_shortcut: "Control+Option+D".into(),
                cleanup_engine: Some(cleanup_engine),
                ..AppSettings::default()
            };

            assert!(legacy.migrate_legacy_schema());
            assert_eq!(legacy.schema_version, SETTINGS_SCHEMA_VERSION);
            assert_eq!(legacy.cleanup_engine, None);
            assert_eq!(legacy.push_to_talk_shortcut, "Control+Option+D");
            assert!(legacy.validate().is_ok());
        }

        let mut current = AppSettings::default();
        assert!(!current.migrate_legacy_schema());
        assert_eq!(current, AppSettings::default());
    }

    #[test]
    fn shortcut_default_migration_preserves_explicit_user_choices() {
        let mut previous_default = AppSettings {
            schema_version: OPTION_DEFAULT_SETTINGS_SCHEMA_VERSION,
            push_to_talk_shortcut: LEGACY_DEFAULT_PUSH_TO_TALK_SHORTCUT.into(),
            ..AppSettings::default()
        };
        assert!(previous_default.migrate_legacy_schema());
        assert_eq!(previous_default.schema_version, SETTINGS_SCHEMA_VERSION);
        #[cfg(target_os = "macos")]
        assert_eq!(
            previous_default.push_to_talk_shortcut,
            DEFAULT_PUSH_TO_TALK_SHORTCUT
        );

        let mut custom = AppSettings {
            schema_version: OPTION_DEFAULT_SETTINGS_SCHEMA_VERSION,
            push_to_talk_shortcut: "CommandOrControl+Shift+D".into(),
            ..AppSettings::default()
        };
        assert!(custom.migrate_legacy_schema());
        assert_eq!(custom.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(custom.push_to_talk_shortcut, "CommandOrControl+Shift+D");

        let mut current = AppSettings {
            push_to_talk_shortcut: LEGACY_DEFAULT_PUSH_TO_TALK_SHORTCUT.into(),
            ..AppSettings::default()
        };
        assert!(!current.migrate_legacy_schema());
        assert_eq!(
            current.push_to_talk_shortcut,
            LEGACY_DEFAULT_PUSH_TO_TALK_SHORTCUT
        );
    }

    #[test]
    fn schema_v3_migration_enables_the_preexisting_hud_by_default() {
        let mut previous = AppSettings {
            schema_version: MULTILINGUAL_SETTINGS_SCHEMA_VERSION,
            input_device_name: None,
            hud: HudSettings {
                visible: false,
                ..HudSettings::default()
            },
            ..AppSettings::default()
        };

        assert!(previous.migrate_legacy_schema());
        assert_eq!(previous.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(previous.hud.visible);
        assert_eq!(previous.input_device_name, None);
    }

    #[test]
    fn schema_v4_moves_only_the_implicit_hud_position_to_a_corner() {
        let mut untouched = AppSettings {
            schema_version: TRANSIENT_HUD_SETTINGS_SCHEMA_VERSION,
            hud: HudSettings {
                position: HudPosition::BottomCenter,
                custom_position: None,
                ..HudSettings::default()
            },
            ..AppSettings::default()
        };
        assert!(untouched.migrate_legacy_schema());
        assert_eq!(untouched.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(untouched.hud.position, HudPosition::BottomRight);

        let dragged_position = HudCoordinates { x: 1456, y: 880 };
        let mut dragged = AppSettings {
            schema_version: TRANSIENT_HUD_SETTINGS_SCHEMA_VERSION,
            hud: HudSettings {
                position: HudPosition::BottomCenter,
                custom_position: Some(dragged_position),
                ..HudSettings::default()
            },
            ..AppSettings::default()
        };
        assert!(dragged.migrate_legacy_schema());
        assert_eq!(dragged.hud.position, HudPosition::BottomCenter);
        assert_eq!(dragged.hud.custom_position, Some(dragged_position));
    }

    #[test]
    fn selected_microphone_names_are_bounded_and_printable() {
        let mut settings = AppSettings {
            input_device_name: Some("Studio microphone".into()),
            ..AppSettings::default()
        };
        assert!(settings.validate().is_ok());

        settings.input_device_name = Some("\n".into());
        assert_eq!(
            settings.validate(),
            Err("selected microphone name cannot be empty".into())
        );

        settings.input_device_name = Some("microphone\0name".into());
        assert_eq!(
            settings.validate(),
            Err("selected microphone name is invalid".into())
        );
    }
}
