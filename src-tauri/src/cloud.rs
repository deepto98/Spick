use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};
use zeroize::Zeroizing;

use crate::{
    domain::{AppSettings, EngineConfig, EngineLocation, EngineProvider, LanguagePolicy},
    engines::{AudioInput, EngineError, TranscriptResult, TranscriptionRequest},
    state::AppState,
};

const CREDENTIAL_SERVICE: &str = "app.spick.desktop";
const MAIN_WINDOW_LABEL: &str = "main";
const MIN_API_KEY_BYTES: usize = 8;
const MAX_API_KEY_BYTES: usize = 8 * 1024;
const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";
const XAI_ENDPOINT: &str = "https://api.x.ai/v1/stt";
const GEMINI_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1/interactions";
const CLOUD_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_OPENAI_MULTIPART_BYTES: usize = 24_000_000;
const MAX_XAI_MULTIPART_BYTES: usize = 500_000_000;
const MAX_GEMINI_JSON_BYTES: usize = 20_000_000;
const MAX_PROMPT_BYTES: usize = 8 * 1024;
const MAX_XAI_KEYTERMS: usize = 100;
const MAX_XAI_KEYTERM_CHARS: usize = 50;

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
    /// One process-lived client keeps each provider's idle HTTPS connection
    /// available for a following dictation. Constructing an Agent per request
    /// would discard ureq's connection pool and force avoidable DNS/TLS work.
    http: ureq::Agent,
    /// Serializes short credential/configuration transactions and tracks
    /// provider requests without holding a mutex across network I/O.
    configuration: Mutex<CloudConfigurationState>,
}

#[derive(Default)]
struct CloudConfigurationState {
    in_flight: [u32; 3],
}

impl CloudConfigurationState {
    fn request_count(&self, provider: CloudProviderId) -> u32 {
        self.in_flight[provider_index(provider)]
    }
}

struct CloudRequestLease<'a> {
    configuration: &'a Mutex<CloudConfigurationState>,
    provider: CloudProviderId,
}

impl Drop for CloudRequestLease<'_> {
    fn drop(&mut self) {
        let mut configuration = self
            .configuration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = &mut configuration.in_flight[provider_index(self.provider)];
        debug_assert!(*count > 0, "cloud request lease underflow");
        *count = count.saturating_sub(1);
    }
}

impl Default for CloudRuntime {
    fn default() -> Self {
        Self {
            credentials: Arc::new(OsCredentialStore),
            http: cloud_http_agent(),
            configuration: Mutex::new(CloudConfigurationState::default()),
        }
    }
}

impl CloudRuntime {
    #[cfg(test)]
    fn with_credentials(credentials: Arc<dyn CredentialStore>) -> Self {
        Self {
            credentials,
            http: cloud_http_agent(),
            configuration: Mutex::new(CloudConfigurationState::default()),
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

    pub(crate) fn first_configured_compatible(
        &self,
        language_policy: &LanguagePolicy,
    ) -> Result<Option<CloudProviderId>, EngineError> {
        let _configuration = self
            .configuration
            .lock()
            .map_err(|_| EngineError::Backend("Cloud credential access is unavailable.".into()))?;
        for provider in CloudProviderId::ORDERED {
            if validate_provider_language_policy(provider, language_policy).is_err() {
                continue;
            }
            let secret = self.credential(provider).map_err(EngineError::Backend)?;
            if is_configured_secret(secret.as_ref()) {
                return Ok(Some(provider));
            }
        }
        Ok(None)
    }

    fn begin_request(
        &self,
        provider: CloudProviderId,
    ) -> Result<(Zeroizing<String>, CloudRequestLease<'_>), EngineError> {
        let mut configuration = self
            .configuration
            .lock()
            .map_err(|_| EngineError::Backend("Cloud credential access is unavailable.".into()))?;
        let credential = self.credential(provider).map_err(EngineError::Backend)?;
        if !is_configured_secret(credential.as_ref()) {
            return Err(EngineError::Backend(format!(
                "No API key is saved for {}.",
                provider_spec(provider).provider_name
            )));
        }
        let credential = credential.expect("configured credentials are present");
        let count = &mut configuration.in_flight[provider_index(provider)];
        *count = count.checked_add(1).ok_or_else(|| {
            EngineError::Backend("Too many cloud transcription requests are active.".into())
        })?;
        drop(configuration);
        Ok((
            credential,
            CloudRequestLease {
                configuration: &self.configuration,
                provider,
            },
        ))
    }

    pub(crate) fn transcribe<F>(
        &self,
        provider: CloudProviderId,
        request: TranscriptionRequest<'_>,
        clean: bool,
        claim_upload: F,
    ) -> Result<CloudTranscription, EngineError>
    where
        F: FnOnce() -> Result<bool, String>,
    {
        request.validate()?;
        validate_provider_language_policy(provider, request.language_policy)
            .map_err(EngineError::InvalidRequest)?;
        check_cancelled(request.cancellation)?;
        let wav = Zeroizing::new(pcm16_wav(request.audio)?);
        let prepared = prepare_request(provider, wav.as_slice(), &request, clean)?;
        check_cancelled(request.cancellation)?;

        // Credential mutation is refused while this short-lived lease exists,
        // but unrelated provider/status work remains responsive during I/O.
        let (credential, request_lease) = self.begin_request(provider)?;
        check_cancelled(request.cancellation)?;
        if !claim_upload().map_err(EngineError::Backend)? {
            return Err(EngineError::Cancelled);
        }
        let response = send_request(&self.http, &prepared, credential.as_str())?;
        drop(request_lease);
        drop(credential);
        drop(prepared);
        drop(wav);
        // This cannot recall an upload already accepted by the provider, but
        // it prevents a cancelled session from parsing or retaining its text.
        check_cancelled(request.cancellation)?;
        let transcript = parse_response(provider, response.as_str(), request.language_policy)?;
        Ok(CloudTranscription {
            engine_id: provider_spec(provider).engine_id,
            transcript,
            cleanup_applied: matches!(provider, CloudProviderId::XAi | CloudProviderId::Gemini)
                && clean,
        })
    }

    fn statuses(&self, settings: &AppSettings) -> Result<Vec<CloudProviderStatus>, String> {
        let _configuration = self
            .configuration
            .lock()
            .map_err(|_| "cloud configuration is unavailable".to_string())?;
        CloudProviderId::ORDERED
            .into_iter()
            .map(|provider| self.status_unlocked(provider, settings))
            .collect()
    }

    fn status(
        &self,
        provider: CloudProviderId,
        settings: &AppSettings,
    ) -> Result<CloudProviderStatus, String> {
        let _configuration = self
            .configuration
            .lock()
            .map_err(|_| "cloud configuration is unavailable".to_string())?;
        self.status_unlocked(provider, settings)
    }

    fn status_unlocked(
        &self,
        provider: CloudProviderId,
        settings: &AppSettings,
    ) -> Result<CloudProviderStatus, String> {
        let secret = self.credential(provider)?;
        let configured = is_configured_secret(secret.as_ref());
        let spec = provider_spec(provider);
        Ok(spec.status(
            configured,
            settings.transcription_engine == spec.engine_config(),
        ))
    }
}

fn provider_index(provider: CloudProviderId) -> usize {
    match provider {
        CloudProviderId::OpenAi => 0,
        CloudProviderId::XAi => 1,
        CloudProviderId::Gemini => 2,
    }
}

fn is_configured_secret(secret: Option<&Zeroizing<String>>) -> bool {
    secret.is_some_and(|secret| !secret.trim().is_empty())
}

fn ensure_credential_mutable(
    configuration: &CloudConfigurationState,
    provider: CloudProviderId,
) -> Result<(), String> {
    if configuration.request_count(provider) == 0 {
        Ok(())
    } else {
        Err(format!(
            "Wait for the current {} transcription to finish before changing its API key.",
            provider_spec(provider).provider_name
        ))
    }
}

pub(crate) struct CloudTranscription {
    pub engine_id: &'static str,
    pub transcript: TranscriptResult,
    /// xAI can omit filler words as part of recognition. Other providers
    /// return a transcript that still needs Spick's optional local cleanup.
    pub cleanup_applied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authentication {
    Bearer,
    GoogleApiKey,
}

struct PreparedRequest {
    endpoint: &'static str,
    content_type: String,
    authentication: Authentication,
    /// Contains a complete copy of the captured audio. Zero it as soon as the
    /// synchronous request finishes or fails.
    body: Zeroizing<Vec<u8>>,
}

fn check_cancelled(cancellation: Option<&AtomicBool>) -> Result<(), EngineError> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        Err(EngineError::Cancelled)
    } else {
        Ok(())
    }
}

fn prepare_request(
    provider: CloudProviderId,
    wav: &[u8],
    request: &TranscriptionRequest<'_>,
    clean: bool,
) -> Result<PreparedRequest, EngineError> {
    match provider {
        CloudProviderId::OpenAi => prepare_openai_request(wav, request),
        CloudProviderId::XAi => prepare_xai_request(wav, request, clean),
        CloudProviderId::Gemini => prepare_gemini_request(wav, request, clean),
    }
}

fn prepare_openai_request(
    wav: &[u8],
    request: &TranscriptionRequest<'_>,
) -> Result<PreparedRequest, EngineError> {
    let mut fields = vec![("model", Zeroizing::new("gpt-4o-transcribe".to_string()))];
    if let Some(language) = fixed_language(request.language_policy) {
        fields.push(("language", Zeroizing::new(provider_language_code(language))));
    }
    if let Some(prompt) = vocabulary_prompt(request.vocabulary, MAX_PROMPT_BYTES) {
        fields.push(("prompt", prompt));
    }
    let (content_type, body) = multipart_body(&fields, wav)?;
    if body.len() > MAX_OPENAI_MULTIPART_BYTES {
        return Err(EngineError::InvalidRequest(
            "This recording is too long for OpenAI transcription.".into(),
        ));
    }
    Ok(PreparedRequest {
        endpoint: OPENAI_ENDPOINT,
        content_type,
        authentication: Authentication::Bearer,
        body,
    })
}

fn prepare_xai_request(
    wav: &[u8],
    request: &TranscriptionRequest<'_>,
    clean: bool,
) -> Result<PreparedRequest, EngineError> {
    let mut fields = Vec::new();
    if let Some(language) = fixed_language(request.language_policy)
        .map(base_language)
        .filter(|language| xai_formatting_language(language))
    {
        fields.push(("format", Zeroizing::new("true".to_string())));
        fields.push(("language", Zeroizing::new(provider_language_code(language))));
    }
    fields.push((
        "filler_words",
        Zeroizing::new(if clean { "false" } else { "true" }.to_string()),
    ));
    fields.extend(
        request
            .vocabulary
            .iter()
            .filter_map(|term| bounded_term(term, MAX_XAI_KEYTERM_CHARS))
            .take(MAX_XAI_KEYTERMS)
            .map(|term| ("keyterm", term)),
    );
    // multipart_body always appends the file after every text field. xAI
    // explicitly requires that ordering.
    let (content_type, body) = multipart_body(&fields, wav)?;
    if body.len() > MAX_XAI_MULTIPART_BYTES {
        return Err(EngineError::InvalidRequest(
            "This recording is too long for xAI transcription.".into(),
        ));
    }
    Ok(PreparedRequest {
        endpoint: XAI_ENDPOINT,
        content_type,
        authentication: Authentication::Bearer,
        body,
    })
}

fn prepare_gemini_request(
    wav: &[u8],
    request: &TranscriptionRequest<'_>,
    clean: bool,
) -> Result<PreparedRequest, EngineError> {
    let prompt = gemini_prompt(request.language_policy, request.vocabulary, clean);
    let encoded_audio = Zeroizing::new(BASE64.encode(wav));
    // Interactions stores requests by default. `store: false` is a deliberate
    // privacy boundary, not an optional optimization.
    let mut body = Zeroizing::new(Vec::new());
    serde_json::to_writer(
        &mut *body,
        &GeminiRequestBody {
            model: "gemini-3.5-flash",
            store: false,
            input: [
                GeminiInput::Text {
                    text: prompt.as_str(),
                },
                GeminiInput::Audio {
                    data: encoded_audio.as_str(),
                    mime_type: "audio/wav",
                },
            ],
        },
    )
    .map_err(|_| EngineError::Backend("Could not prepare the Gemini request.".into()))?;
    // Google's 20 MB limit covers the complete encoded request, including
    // base64 expansion and prompt JSON.
    validate_gemini_body_size(body.len())?;
    Ok(PreparedRequest {
        endpoint: GEMINI_ENDPOINT,
        content_type: "application/json".into(),
        authentication: Authentication::GoogleApiKey,
        body,
    })
}

#[derive(Serialize)]
struct GeminiRequestBody<'a> {
    model: &'static str,
    store: bool,
    input: [GeminiInput<'a>; 2],
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GeminiInput<'a> {
    Text {
        text: &'a str,
    },
    Audio {
        data: &'a str,
        mime_type: &'static str,
    },
}

fn validate_gemini_body_size(body_bytes: usize) -> Result<(), EngineError> {
    if body_bytes >= MAX_GEMINI_JSON_BYTES {
        Err(EngineError::InvalidRequest(
            "This recording is too long for Gemini inline audio.".into(),
        ))
    } else {
        Ok(())
    }
}

fn send_request(
    agent: &ureq::Agent,
    request: &PreparedRequest,
    api_key: &str,
) -> Result<Zeroizing<String>, EngineError> {
    let response = match request.authentication {
        Authentication::Bearer => {
            let authorization = Zeroizing::new(format!("Bearer {api_key}"));
            agent
                .post(request.endpoint)
                .header("Content-Type", &request.content_type)
                .header("Authorization", authorization.as_str())
                .send(request.body.as_slice())
        }
        Authentication::GoogleApiKey => agent
            .post(request.endpoint)
            .header("Content-Type", &request.content_type)
            .header("x-goog-api-key", api_key)
            .send(request.body.as_slice()),
    }
    .map_err(|error| sanitized_http_error(request.endpoint, error))?;

    response
        .into_body()
        .into_with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map(Zeroizing::new)
        .map_err(|_| EngineError::Backend("The cloud response could not be read safely.".into()))
}

fn cloud_http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        // Provider credentials use headers, including Google's custom API-key
        // header. Never allow either credential style to cross a redirect.
        .max_redirects(0)
        .timeout_global(Some(CLOUD_REQUEST_TIMEOUT))
        .build()
        .new_agent()
}

fn sanitized_http_error(endpoint: &str, error: ureq::Error) -> EngineError {
    let provider = if endpoint == OPENAI_ENDPOINT {
        "OpenAI"
    } else if endpoint == XAI_ENDPOINT {
        "xAI"
    } else {
        "Gemini"
    };
    let message = match error {
        ureq::Error::StatusCode(status) => {
            format!("{provider} rejected the transcription request (HTTP {status}).")
        }
        ureq::Error::Timeout(_) => format!("{provider} transcription timed out."),
        _ => format!("{provider} could not be reached."),
    };
    EngineError::Backend(message)
}

fn parse_response(
    provider: CloudProviderId,
    response: &str,
    policy: &LanguagePolicy,
) -> Result<TranscriptResult, EngineError> {
    match provider {
        CloudProviderId::OpenAi => parse_openai_response(response, policy),
        CloudProviderId::XAi => parse_xai_response(response, policy),
        CloudProviderId::Gemini => parse_gemini_response(response, policy),
    }
}

fn parse_openai_response(
    response: &str,
    policy: &LanguagePolicy,
) -> Result<TranscriptResult, EngineError> {
    let value: serde_json::Value = serde_json::from_str(response)
        .map_err(|_| EngineError::InvalidResult("OpenAI returned invalid JSON.".into()))?;
    let text = required_text(value.get("text"), "OpenAI")?;
    let language = value
        .get("language")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_detected_language)
        .or_else(|| {
            fixed_language(policy)
                .map(base_language)
                .map(str::to_string)
        });
    Ok(final_transcript(text, language))
}

fn parse_xai_response(
    response: &str,
    policy: &LanguagePolicy,
) -> Result<TranscriptResult, EngineError> {
    let value: serde_json::Value = serde_json::from_str(response)
        .map_err(|_| EngineError::InvalidResult("xAI returned invalid JSON.".into()))?;
    let text = required_text(value.get("text"), "xAI")?;
    let language = value
        .get("language")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_detected_language)
        .or_else(|| {
            fixed_language(policy)
                .map(base_language)
                .map(str::to_string)
        });
    Ok(final_transcript(text, language))
}

fn parse_gemini_response(
    response: &str,
    policy: &LanguagePolicy,
) -> Result<TranscriptResult, EngineError> {
    let value: serde_json::Value = serde_json::from_str(response)
        .map_err(|_| EngineError::InvalidResult("Gemini returned invalid JSON.".into()))?;
    if value.get("status").and_then(serde_json::Value::as_str) != Some("completed") {
        return Err(EngineError::InvalidResult(
            "Gemini did not complete the transcription.".into(),
        ));
    }
    let text = value
        .get("steps")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|step| step.get("type").and_then(serde_json::Value::as_str) == Some("model_output"))
        .filter_map(|step| step.get("content").and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|content| content.get("type").and_then(serde_json::Value::as_str) == Some("text"))
        .filter_map(|content| content.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    let text = text.trim();
    if text.is_empty() {
        return Err(EngineError::InvalidResult(
            "Gemini returned no transcript text.".into(),
        ));
    }
    let language = fixed_language(policy)
        .map(base_language)
        .map(str::to_string);
    Ok(final_transcript(text.to_string(), language))
}

fn required_text(value: Option<&serde_json::Value>, provider: &str) -> Result<String, EngineError> {
    let text = value.and_then(serde_json::Value::as_str).map(str::trim);
    match text {
        Some(text) if !text.is_empty() => Ok(text.to_string()),
        _ => Err(EngineError::InvalidResult(format!(
            "{provider} returned no transcript text."
        ))),
    }
}

fn final_transcript(text: String, detected_language: Option<String>) -> TranscriptResult {
    TranscriptResult {
        text,
        segments: Vec::new(),
        detected_language,
        confidence: None,
        is_final: true,
    }
}

fn pcm16_wav(audio: AudioInput<'_>) -> Result<Vec<u8>, EngineError> {
    audio.validate()?;
    if audio.sample_rate_hz != 16_000 || audio.channels != 1 {
        return Err(EngineError::InvalidRequest(
            "Cloud transcription requires mono 16 kHz audio.".into(),
        ));
    }
    if audio
        .samples
        .iter()
        .any(|sample| !(-1.0..=1.0).contains(sample))
    {
        return Err(EngineError::InvalidRequest(
            "Cloud PCM samples must be normalized to -1.0..=1.0.".into(),
        ));
    }
    let data_size = audio
        .samples
        .len()
        .checked_mul(2)
        .and_then(|size| u32::try_from(size).ok())
        .ok_or_else(|| EngineError::InvalidRequest("The recording is too large.".into()))?;
    let riff_size = data_size
        .checked_add(36)
        .ok_or_else(|| EngineError::InvalidRequest("The recording is too large.".into()))?;
    let mut wav = Vec::with_capacity(44 + data_size as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&16_000_u32.to_le_bytes());
    wav.extend_from_slice(&32_000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for sample in audio.samples {
        let scaled = if *sample < 0.0 {
            *sample * 32_768.0
        } else {
            *sample * 32_767.0
        };
        wav.extend_from_slice(&(scaled.round() as i16).to_le_bytes());
    }
    Ok(wav)
}

fn multipart_body(
    fields: &[(&'static str, Zeroizing<String>)],
    file: &[u8],
) -> Result<(String, Zeroizing<Vec<u8>>), EngineError> {
    let boundary = safe_multipart_boundary(fields, file);
    let mut body = Zeroizing::new(Vec::new());
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"spick.wav\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok((format!("multipart/form-data; boundary={boundary}"), body))
}

fn safe_multipart_boundary(fields: &[(&'static str, Zeroizing<String>)], file: &[u8]) -> String {
    loop {
        let boundary = format!("spick-{}", uuid::Uuid::new_v4().simple());
        let bytes = boundary.as_bytes();
        if !contains_bytes(file, bytes)
            && fields
                .iter()
                .all(|(_, value)| !contains_bytes(value.as_bytes(), bytes))
        {
            return boundary;
        }
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn fixed_language(policy: &LanguagePolicy) -> Option<&str> {
    match policy {
        LanguagePolicy::Fixed { language } => Some(language),
        _ => None,
    }
}

fn base_language(language: &str) -> &str {
    language.split_once('-').map_or(language, |(base, _)| base)
}

fn provider_language_code(language: &str) -> String {
    base_language(language).to_ascii_lowercase()
}

fn vocabulary_prompt(vocabulary: &[&str], max_bytes: usize) -> Option<Zeroizing<String>> {
    let mut prompt = Zeroizing::new(String::from("Vocabulary: "));
    let initial = prompt.len();
    for term in vocabulary
        .iter()
        .map(|term| term.trim())
        .filter(|term| !term.is_empty())
    {
        let separator = if prompt.len() == initial { "" } else { ", " };
        if prompt.len() + separator.len() + term.len() > max_bytes {
            break;
        }
        prompt.push_str(separator);
        prompt.push_str(term);
    }
    (prompt.len() > initial).then_some(prompt)
}

fn bounded_term(term: &str, max_chars: usize) -> Option<Zeroizing<String>> {
    let term = term.trim();
    if term.is_empty() {
        return None;
    }
    Some(Zeroizing::new(term.chars().take(max_chars).collect()))
}

fn gemini_prompt(policy: &LanguagePolicy, vocabulary: &[&str], clean: bool) -> Zeroizing<String> {
    let style = if clean {
        "Remove hesitation fillers and repair obvious punctuation without changing meaning, language, or script."
    } else {
        "Preserve the speaker's language, script, punctuation, and filler words."
    };
    let mut prompt = Zeroizing::new(String::from("Transcribe this recording faithfully. "));
    prompt.push_str(style);
    prompt.push_str(" Return only the transcript, with no commentary.");
    if let Some(language) = fixed_language(policy) {
        prompt.push_str(" The spoken language is ");
        prompt.push_str(language);
        prompt.push('.');
    }
    if let Some(vocabulary) = vocabulary_prompt(vocabulary, MAX_PROMPT_BYTES) {
        prompt.push_str(" Use these spelling hints when they are spoken: ");
        prompt.push_str(vocabulary.as_str());
        prompt.push('.');
    }
    prompt
}

fn xai_formatting_language(language: &str) -> bool {
    matches!(
        language.to_ascii_lowercase().as_str(),
        "ar" | "cs"
            | "da"
            | "nl"
            | "en"
            | "fil"
            | "fr"
            | "de"
            | "hi"
            | "id"
            | "it"
            | "ja"
            | "ko"
            | "mk"
            | "ms"
            | "fa"
            | "pl"
            | "pt"
            | "ro"
            | "ru"
            | "es"
            | "sv"
            | "th"
            | "tr"
            | "vi"
    )
}

fn normalize_detected_language(language: &str) -> Option<String> {
    let language = language.trim();
    if language.is_empty() {
        return None;
    }
    let lowercase = language.to_ascii_lowercase();
    let normalized = match lowercase.as_str() {
        "english" => "en",
        "french" => "fr",
        "german" => "de",
        "hindi" => "hi",
        "spanish" => "es",
        "bengali" => "bn",
        "japanese" => "ja",
        "korean" => "ko",
        "portuguese" => "pt",
        "russian" => "ru",
        "italian" => "it",
        "arabic" => "ar",
        other
            if other.len() <= 35
                && other
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') =>
        {
            other
        }
        _ => return None,
    };
    Some(normalized.to_string())
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
        let configuration = state
            .cloud
            .configuration
            .lock()
            .map_err(|_| "cloud configuration is unavailable".to_string())?;
        ensure_credential_mutable(&configuration, provider)?;
        state
            .cloud
            .credentials
            .set(provider, trimmed)
            .map_err(|()| {
                "The API key could not be saved in the OS credential store.".to_string()
            })?;
        drop(configuration);
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
        let configuration = state
            .cloud
            .configuration
            .lock()
            .map_err(|_| "cloud configuration is unavailable".to_string())?;
        ensure_credential_mutable(&configuration, provider)?;
        let settings = state.settings_snapshot()?;
        if provider_for_engine(&settings.transcription_engine) == Some(provider) {
            return Err("Choose another transcription engine before removing this key.".into());
        }
        state.cloud.credentials.delete(provider).map_err(|()| {
            "The API key could not be removed from the OS credential store.".to_string()
        })?;
        drop(configuration);
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
        let secret = state.cloud.credential(provider)?;
        let configured = is_configured_secret(secret.as_ref());
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
        validate_provider_language_policy(provider, &current.language_policy)?;
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

pub(crate) fn validate_provider_language_policy(
    provider: CloudProviderId,
    policy: &LanguagePolicy,
) -> Result<(), String> {
    validate_cloud_language_policy(policy)?;
    if provider == CloudProviderId::XAi
        && fixed_language(policy)
            .map(base_language)
            .is_some_and(|language| !xai_formatting_language(language))
    {
        return Err(
            "xAI cannot apply a fixed formatting language for this selection. Use Auto or choose a language supported by xAI formatting."
                .into(),
        );
    }
    Ok(())
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

    fn request<'a>(
        samples: &'a [f32],
        policy: &'a LanguagePolicy,
        vocabulary: &'a [&'a str],
    ) -> TranscriptionRequest<'a> {
        TranscriptionRequest {
            audio: AudioInput {
                samples,
                sample_rate_hz: 16_000,
                channels: 1,
            },
            language_policy: policy,
            vocabulary,
            cancellation: None,
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
    fn empty_credentials_are_never_reported_or_selected_as_configured() {
        let credentials = Arc::new(MemoryCredentials::default());
        credentials.set(CloudProviderId::OpenAi, "   ").unwrap();
        let runtime = CloudRuntime::with_credentials(credentials);

        assert!(!runtime.statuses(&AppSettings::default()).unwrap()[0].configured);
        assert_eq!(
            runtime
                .first_configured_compatible(&LanguagePolicy::Auto)
                .unwrap(),
            None
        );
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
        assert!(validate_provider_language_policy(
            CloudProviderId::XAi,
            &LanguagePolicy::Fixed {
                language: "en-US".into()
            }
        )
        .is_ok());
        assert!(validate_provider_language_policy(
            CloudProviderId::XAi,
            &LanguagePolicy::Fixed {
                language: "bn-IN".into()
            }
        )
        .is_err());
        assert!(validate_provider_language_policy(
            CloudProviderId::OpenAi,
            &LanguagePolicy::Fixed {
                language: "bn-IN".into()
            }
        )
        .is_ok());
    }

    #[test]
    fn credential_changes_fail_fast_only_for_the_provider_in_flight() {
        let credentials = Arc::new(MemoryCredentials::default());
        credentials
            .set(CloudProviderId::OpenAi, "openai-secret")
            .unwrap();
        let runtime = CloudRuntime::with_credentials(credentials);
        let (_credential, lease) = runtime.begin_request(CloudProviderId::OpenAi).unwrap();

        {
            let configuration = runtime.configuration.lock().unwrap();
            assert!(ensure_credential_mutable(&configuration, CloudProviderId::OpenAi).is_err());
            assert!(ensure_credential_mutable(&configuration, CloudProviderId::Gemini).is_ok());
        }
        drop(lease);
        let configuration = runtime.configuration.lock().unwrap();
        assert!(ensure_credential_mutable(&configuration, CloudProviderId::OpenAi).is_ok());
    }

    #[test]
    fn pcm_is_encoded_as_checked_mono_16khz_wav() {
        let wav = pcm16_wav(AudioInput {
            samples: &[-1.0, 0.0, 1.0],
            sample_rate_hz: 16_000,
            channels: 1,
        })
        .unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
        assert_eq!(
            i16::from_le_bytes(wav[44..46].try_into().unwrap()),
            i16::MIN
        );
        assert_eq!(i16::from_le_bytes(wav[46..48].try_into().unwrap()), 0);
        assert_eq!(
            i16::from_le_bytes(wav[48..50].try_into().unwrap()),
            i16::MAX
        );
    }

    #[test]
    fn openai_request_uses_fixed_endpoint_model_language_and_prompt() {
        let samples = [0.0_f32, 0.25];
        let policy = LanguagePolicy::Fixed {
            language: "HI-IN".into(),
        };
        let vocabulary = ["Spick", "Deepto"];
        let wav = pcm16_wav(request(&samples, &policy, &vocabulary).audio).unwrap();
        let prepared =
            prepare_openai_request(&wav, &request(&samples, &policy, &vocabulary)).unwrap();
        let body = String::from_utf8_lossy(&prepared.body);
        assert_eq!(prepared.endpoint, OPENAI_ENDPOINT);
        assert_eq!(prepared.authentication, Authentication::Bearer);
        assert!(body.contains("name=\"model\"\r\n\r\ngpt-4o-transcribe"));
        assert!(body.contains("name=\"language\"\r\n\r\nhi"));
        assert!(body.contains("Vocabulary: Spick, Deepto"));
        assert!(body.contains("name=\"file\"; filename=\"spick.wav\""));
    }

    #[test]
    fn xai_request_has_no_model_field_and_puts_file_last() {
        let samples = [0.0_f32, -0.5, 0.5];
        let policy = LanguagePolicy::Fixed {
            language: "EN-US".into(),
        };
        let long_term = "a".repeat(MAX_XAI_KEYTERM_CHARS + 10);
        let vocabulary = [long_term.as_str(), "Spick"];
        let wav = pcm16_wav(request(&samples, &policy, &vocabulary).audio).unwrap();
        let prepared =
            prepare_xai_request(&wav, &request(&samples, &policy, &vocabulary), true).unwrap();
        let body = String::from_utf8_lossy(&prepared.body);
        assert_eq!(prepared.endpoint, XAI_ENDPOINT);
        assert!(!body.contains("name=\"model\""));
        assert!(body.contains("name=\"format\"\r\n\r\ntrue"));
        assert!(body.contains("name=\"language\"\r\n\r\nen"));
        assert!(body.contains("name=\"filler_words\"\r\n\r\nfalse"));
        assert!(body.contains(&"a".repeat(MAX_XAI_KEYTERM_CHARS)));
        assert!(!body.contains(&"a".repeat(MAX_XAI_KEYTERM_CHARS + 1)));
        let file = body.find("name=\"file\"").unwrap();
        let last_keyterm = body.rfind("name=\"keyterm\"").unwrap();
        assert!(file > last_keyterm, "xAI requires the file field last");

        let verbatim =
            prepare_xai_request(&wav, &request(&samples, &policy, &vocabulary), false).unwrap();
        assert!(
            String::from_utf8_lossy(&verbatim.body).contains("name=\"filler_words\"\r\n\r\ntrue")
        );
    }

    #[test]
    fn gemini_request_is_stable_stateless_and_caps_the_encoded_json() {
        let samples = [0.0_f32, 0.5];
        let policy = LanguagePolicy::Auto;
        let vocabulary = ["Spick"];
        let wav = pcm16_wav(request(&samples, &policy, &vocabulary).audio).unwrap();
        let prepared =
            prepare_gemini_request(&wav, &request(&samples, &policy, &vocabulary), false).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&prepared.body).unwrap();
        assert_eq!(prepared.endpoint, GEMINI_ENDPOINT);
        assert_eq!(prepared.authentication, Authentication::GoogleApiKey);
        assert_eq!(value["model"], "gemini-3.5-flash");
        assert_eq!(value["store"], false);
        assert_eq!(value["input"][1]["type"], "audio");
        assert!(value["input"][1]["data"].as_str().unwrap().len() > wav.len());
        assert!(prepared.body.len() < MAX_GEMINI_JSON_BYTES);
        assert!(validate_gemini_body_size(MAX_GEMINI_JSON_BYTES - 1).is_ok());
        assert!(validate_gemini_body_size(MAX_GEMINI_JSON_BYTES).is_err());
    }

    #[test]
    fn current_provider_responses_are_parsed_without_legacy_gemini_outputs() {
        let auto = LanguagePolicy::Auto;
        let openai = parse_openai_response(r#"{"text":" hello "}"#, &auto).unwrap();
        assert_eq!(openai.text, "hello");
        let xai = parse_xai_response(
            r#"{"text":"नमस्ते","language":"Hindi","duration":1.2}"#,
            &auto,
        )
        .unwrap();
        assert_eq!(xai.detected_language.as_deref(), Some("hi"));
        let gemini = parse_gemini_response(
            r#"{"status":"completed","steps":[{"type":"model_output","content":[{"type":"text","text":"hello"}]}]}"#,
            &auto,
        )
        .unwrap();
        assert_eq!(gemini.text, "hello");
        assert!(parse_gemini_response(
            r#"{"status":"completed","outputs":[{"type":"text","text":"legacy"}]}"#,
            &auto
        )
        .is_err());
        assert!(
            parse_gemini_response(
                r#"{"status":"failed","steps":[{"type":"model_output","content":[{"type":"text","text":"partial"}]}]}"#,
                &auto
            )
            .is_err()
        );
    }

    #[test]
    fn cancellation_and_http_failures_are_safe_and_sanitized() {
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            check_cancelled(Some(&cancelled)),
            Err(EngineError::Cancelled)
        );
        let error = sanitized_http_error(OPENAI_ENDPOINT, ureq::Error::StatusCode(401));
        let message = error.to_string();
        assert!(message.contains("OpenAI"));
        assert!(message.contains("401"));
        assert!(!message.contains("secret"));
    }

    #[test]
    fn runtime_owns_one_hardened_shared_http_agent() {
        let runtime = CloudRuntime::with_credentials(Arc::new(MemoryCredentials::default()));
        let shared = runtime.http.clone();

        assert!(runtime.http.config().https_only());
        assert_eq!(runtime.http.config().max_redirects(), 0);
        assert_eq!(
            runtime.http.config().timeouts().global,
            Some(CLOUD_REQUEST_TIMEOUT)
        );
        assert!(
            std::ptr::eq(runtime.http.config(), shared.config()),
            "ureq Agent clones must retain the runtime's shared client state"
        );
    }

    #[test]
    fn fallback_chooses_only_the_first_configured_compatible_provider() {
        let credentials = Arc::new(MemoryCredentials::default());
        credentials
            .set(CloudProviderId::Gemini, "gemini-secret")
            .unwrap();
        credentials.set(CloudProviderId::XAi, "xai-secret").unwrap();
        let runtime = CloudRuntime::with_credentials(credentials);
        assert_eq!(
            runtime
                .first_configured_compatible(&LanguagePolicy::Auto)
                .unwrap(),
            Some(CloudProviderId::XAi)
        );
        assert_eq!(
            runtime
                .first_configured_compatible(&LanguagePolicy::Fixed {
                    language: "bn-IN".into(),
                })
                .unwrap(),
            Some(CloudProviderId::Gemini),
            "an incompatible configured provider must be skipped before any upload"
        );
    }
}
