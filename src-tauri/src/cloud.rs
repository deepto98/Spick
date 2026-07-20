use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};
use zeroize::Zeroizing;

use crate::{
    domain::{AppSettings, EngineConfig, EngineLocation, EngineProvider, LanguagePolicy},
    state::AppState,
};

const CREDENTIAL_SERVICE: &str = "app.spick.desktop";
const MAIN_WINDOW_LABEL: &str = "main";
const MIN_API_KEY_BYTES: usize = 8;
const MAX_API_KEY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudProviderId {
    OpenAi,
    XAi,
    Gemini,
}

impl CloudProviderId {
    pub const ORDERED: [Self; 3] = [Self::OpenAi, Self::XAi, Self::Gemini];

    fn credential_account(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::XAi => "xai",
            Self::Gemini => "gemini",
        }
    }

    pub(crate) fn provider(self) -> EngineProvider {
        match self {
            Self::OpenAi => EngineProvider::OpenAi,
            Self::XAi => EngineProvider::XAi,
            Self::Gemini => EngineProvider::Gemini,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProviderStatus {
    pub provider: CloudProviderId,
    pub provider_name: &'static str,
    pub engine_id: &'static str,
    pub model_name: &'static str,
    pub configured: bool,
    pub selected: bool,
    pub experimental: bool,
    pub description: &'static str,
    pub language_support: &'static str,
    pub cleanup_behavior: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CloudProviderSpec {
    pub id: CloudProviderId,
    pub provider_name: &'static str,
    pub engine_id: &'static str,
    pub model: &'static str,
    pub model_name: &'static str,
    pub experimental: bool,
    pub description: &'static str,
    pub language_support: &'static str,
    pub cleanup_behavior: &'static str,
}

impl CloudProviderSpec {
    pub(crate) fn engine_config(self) -> EngineConfig {
        EngineConfig {
            provider: self.id.provider(),
            model: self.model.into(),
            location: EngineLocation::Cloud,
        }
    }

    fn status(self, configured: bool, selected: bool) -> CloudProviderStatus {
        CloudProviderStatus {
            provider: self.id,
            provider_name: self.provider_name,
            engine_id: self.engine_id,
            model_name: self.model_name,
            configured,
            selected,
            experimental: self.experimental,
            description: self.description,
            language_support: self.language_support,
            cleanup_behavior: self.cleanup_behavior,
        }
    }
}

pub(crate) fn provider_spec(id: CloudProviderId) -> CloudProviderSpec {
    match id {
        CloudProviderId::OpenAi => CloudProviderSpec {
            id,
            provider_name: "OpenAI",
            engine_id: "openai-gpt-4o-transcribe",
            model: "gpt-4o-transcribe",
            model_name: "GPT-4o Transcribe",
            experimental: false,
            description: "Dedicated speech-to-text for completed recordings.",
            language_support: "Multilingual batch transcription",
            cleanup_behavior: "Spick cleanup runs after transcription",
        },
        CloudProviderId::XAi => CloudProviderSpec {
            id,
            provider_name: "xAI",
            engine_id: "xai-speech-to-text",
            // The xAI endpoint currently has no model request field. This is a
            // Spick registry token, not a provider model claim.
            model: "speech-to-text",
            model_name: "xAI Speech to Text",
            experimental: false,
            description: "Dedicated speech-to-text for completed recordings.",
            language_support: "Multilingual batch transcription",
            cleanup_behavior: "Filler handling follows your cleanup setting",
        },
        CloudProviderId::Gemini => CloudProviderSpec {
            id,
            provider_name: "Google",
            engine_id: "gemini-3-5-flash",
            model: "gemini-3.5-flash",
            model_name: "Gemini 3.5 Flash",
            experimental: true,
            description:
                "Experimental general audio understanding, not a dedicated speech-to-text endpoint.",
            language_support: "Model-dependent multilingual audio",
            cleanup_behavior: "General audio response; cleanup is experimental",
        },
    }
}

pub(crate) fn provider_for_engine(engine: &EngineConfig) -> Option<CloudProviderId> {
    CloudProviderId::ORDERED.into_iter().find(|provider| {
        let spec = provider_spec(*provider);
        engine == &spec.engine_config()
    })
}

pub(crate) trait CredentialStore: Send + Sync {
    fn get(&self, provider: CloudProviderId) -> Result<Option<Zeroizing<String>>, ()>;
    fn set(&self, provider: CloudProviderId, api_key: &str) -> Result<(), ()>;
    fn delete(&self, provider: CloudProviderId) -> Result<(), ()>;
}

#[derive(Default)]
struct OsCredentialStore;

impl OsCredentialStore {
    fn entry(provider: CloudProviderId) -> Result<keyring::Entry, ()> {
        keyring::Entry::new(CREDENTIAL_SERVICE, provider.credential_account()).map_err(|_| ())
    }
}

impl CredentialStore for OsCredentialStore {
    fn get(&self, provider: CloudProviderId) -> Result<Option<Zeroizing<String>>, ()> {
        match Self::entry(provider)?.get_password() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(()),
        }
    }

    fn set(&self, provider: CloudProviderId, api_key: &str) -> Result<(), ()> {
        Self::entry(provider)?.set_password(api_key).map_err(|_| ())
    }

    fn delete(&self, provider: CloudProviderId) -> Result<(), ()> {
        match Self::entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(()),
        }
    }
}

pub struct CloudRuntime {
    credentials: Arc<dyn CredentialStore>,
    /// Serializes credential mutation and provider activation. It is never
    /// acquired by shortcut-start or microphone callbacks.
    configuration: Mutex<()>,
}

impl Default for CloudRuntime {
    fn default() -> Self {
        Self {
            credentials: Arc::new(OsCredentialStore),
            configuration: Mutex::new(()),
        }
    }
}

impl CloudRuntime {
    #[cfg(test)]
    fn with_credentials(credentials: Arc<dyn CredentialStore>) -> Self {
        Self {
            credentials,
            configuration: Mutex::new(()),
        }
    }

    pub(crate) fn credential(
        &self,
        provider: CloudProviderId,
    ) -> Result<Option<Zeroizing<String>>, String> {
        self.credentials.get(provider).map_err(|()| {
            "The OS credential store could not be read. Unlock it and try again.".into()
        })
    }

    fn statuses(&self, settings: &AppSettings) -> Result<Vec<CloudProviderStatus>, String> {
        CloudProviderId::ORDERED
            .into_iter()
            .map(|provider| {
                let configured = self.credential(provider)?.is_some();
                let spec = provider_spec(provider);
                Ok(spec.status(
                    configured,
                    settings.transcription_engine == spec.engine_config(),
                ))
            })
            .collect()
    }

    fn status(
        &self,
        provider: CloudProviderId,
        settings: &AppSettings,
    ) -> Result<CloudProviderStatus, String> {
        let configured = self.credential(provider)?.is_some();
        let spec = provider_spec(provider);
        Ok(spec.status(
            configured,
            settings.transcription_engine == spec.engine_config(),
        ))
    }
}

fn require_main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("this command is only available from the Spick dashboard".into())
    }
}

#[tauri::command]
pub async fn list_cloud_providers(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<Vec<CloudProviderStatus>, String> {
    require_main_window(&window)?;
    drop(window);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _configuration = state
            .cloud
            .configuration
            .lock()
            .map_err(|_| "cloud configuration is unavailable".to_string())?;
        state.cloud.statuses(&state.settings_snapshot()?)
    })
    .await
    .map_err(|error| format!("cloud status worker failed: {error}"))?
}

#[tauri::command]
pub async fn set_cloud_api_key(
    window: WebviewWindow,
    app: AppHandle,
    provider: CloudProviderId,
    api_key: String,
) -> Result<CloudProviderStatus, String> {
    require_main_window(&window)?;
    drop(window);
    tauri::async_runtime::spawn_blocking(move || {
        let api_key = Zeroizing::new(api_key);
        let trimmed = api_key.trim();
        validate_api_key(trimmed)?;
        let state = app.state::<AppState>();
        let _configuration = state
            .cloud
            .configuration
            .lock()
            .map_err(|_| "cloud configuration is unavailable".to_string())?;
        state
            .cloud
            .credentials
            .set(provider, trimmed)
            .map_err(|()| {
                "The API key could not be saved in the OS credential store.".to_string()
            })?;
        state.cloud.status(provider, &state.settings_snapshot()?)
    })
    .await
    .map_err(|error| format!("credential-store worker failed: {error}"))?
}

#[tauri::command]
pub async fn delete_cloud_api_key(
    window: WebviewWindow,
    app: AppHandle,
    provider: CloudProviderId,
) -> Result<CloudProviderStatus, String> {
    require_main_window(&window)?;
    drop(window);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _configuration = state
            .cloud
            .configuration
            .lock()
            .map_err(|_| "cloud configuration is unavailable".to_string())?;
        let settings = state.settings_snapshot()?;
        if provider_for_engine(&settings.transcription_engine) == Some(provider) {
            return Err("Choose another transcription engine before removing this key.".into());
        }
        state.cloud.credentials.delete(provider).map_err(|()| {
            "The API key could not be removed from the OS credential store.".to_string()
        })?;
        state.cloud.status(provider, &settings)
    })
    .await
    .map_err(|error| format!("credential-store worker failed: {error}"))?
}

#[tauri::command]
pub async fn activate_cloud_provider(
    window: WebviewWindow,
    app: AppHandle,
    provider: CloudProviderId,
) -> Result<AppSettings, String> {
    require_main_window(&window)?;
    drop(window);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _configuration = state
            .cloud
            .configuration
            .lock()
            .map_err(|_| "cloud configuration is unavailable".to_string())?;
        let configured = state
            .cloud
            .credential(provider)?
            .is_some_and(|secret| !secret.trim().is_empty());
        if !configured {
            return Err("Save an API key for this provider before selecting it.".into());
        }

        let _settings_update = state
            .settings_update
            .lock()
            .map_err(|_| "settings update is unavailable".to_string())?;
        let _model_configuration = state
            .model_configuration
            .lock()
            .map_err(|_| "model configuration is unavailable".to_string())?;
        let mut current = state
            .settings
            .write()
            .map_err(|_| "settings lock is poisoned".to_string())?;
        validate_cloud_language_policy(&current.language_policy)?;
        let mut updated = current.clone();
        updated.transcription_engine = provider_spec(provider).engine_config();
        updated.validate()?;
        state.persist_settings(&updated)?;
        *current = updated.clone();
        Ok(updated)
    })
    .await
    .map_err(|error| format!("cloud activation worker failed: {error}"))?
}

fn validate_api_key(api_key: &str) -> Result<(), String> {
    if !(MIN_API_KEY_BYTES..=MAX_API_KEY_BYTES).contains(&api_key.len())
        || api_key.chars().any(char::is_control)
    {
        return Err("Enter a valid API key.".into());
    }
    Ok(())
}

pub(crate) fn validate_cloud_language_policy(policy: &LanguagePolicy) -> Result<(), String> {
    policy.validate()?;
    match policy {
        LanguagePolicy::Auto | LanguagePolicy::Fixed { .. } => Ok(()),
        LanguagePolicy::Preferred { .. }
        | LanguagePolicy::Mixed { .. }
        | LanguagePolicy::Translate { .. } => {
            Err("Cloud transcription currently supports Auto or one fixed language.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct MemoryCredentials {
        values: Mutex<HashMap<CloudProviderId, String>>,
        fail_reads: bool,
    }

    impl CredentialStore for MemoryCredentials {
        fn get(&self, provider: CloudProviderId) -> Result<Option<Zeroizing<String>>, ()> {
            if self.fail_reads {
                return Err(());
            }
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(&provider)
                .cloned()
                .map(Zeroizing::new))
        }

        fn set(&self, provider: CloudProviderId, api_key: &str) -> Result<(), ()> {
            self.values.lock().unwrap().insert(provider, api_key.into());
            Ok(())
        }

        fn delete(&self, provider: CloudProviderId) -> Result<(), ()> {
            self.values.lock().unwrap().remove(&provider);
            Ok(())
        }
    }

    #[test]
    fn provider_registry_has_stable_order_and_exact_engine_configs() {
        assert_eq!(
            CloudProviderId::ORDERED,
            [
                CloudProviderId::OpenAi,
                CloudProviderId::XAi,
                CloudProviderId::Gemini
            ]
        );
        for provider in CloudProviderId::ORDERED {
            let spec = provider_spec(provider);
            assert_eq!(provider_for_engine(&spec.engine_config()), Some(provider));
            assert_eq!(spec.engine_config().location, EngineLocation::Cloud);
        }
        assert_eq!(provider_spec(CloudProviderId::XAi).model, "speech-to-text");
    }

    #[test]
    fn status_never_contains_a_credential() {
        let credentials = Arc::new(MemoryCredentials::default());
        credentials
            .set(CloudProviderId::OpenAi, "sk-super-secret")
            .unwrap();
        let runtime = CloudRuntime::with_credentials(credentials);
        let statuses = runtime.statuses(&AppSettings::default()).unwrap();
        assert!(statuses[0].configured);
        let json = serde_json::to_string(&statuses).unwrap();
        assert!(!json.contains("sk-super-secret"));
        assert!(!json.contains("apiKey"));
    }

    #[test]
    fn a_store_failure_is_not_reported_as_a_missing_key() {
        let runtime = CloudRuntime::with_credentials(Arc::new(MemoryCredentials {
            fail_reads: true,
            ..MemoryCredentials::default()
        }));
        let error = runtime.statuses(&AppSettings::default()).unwrap_err();
        assert!(error.contains("could not be read"));
    }

    #[test]
    fn api_key_validation_is_bounded_and_rejects_control_characters() {
        assert!(validate_api_key("short").is_err());
        assert!(validate_api_key("abcdefgh").is_ok());
        assert!(validate_api_key("abc\ndefgh").is_err());
        assert!(validate_api_key(&"x".repeat(MAX_API_KEY_BYTES + 1)).is_err());
    }

    #[test]
    fn only_shipped_cloud_language_modes_are_accepted() {
        assert!(validate_cloud_language_policy(&LanguagePolicy::Auto).is_ok());
        assert!(validate_cloud_language_policy(&LanguagePolicy::Fixed {
            language: "hi-IN".into()
        })
        .is_ok());
        assert!(validate_cloud_language_policy(&LanguagePolicy::Mixed {
            languages: vec!["en".into(), "hi".into()]
        })
        .is_err());
    }
}
