use std::sync::Arc;

use crate::domain::{EngineLocation, EngineProvider, LanguagePolicy};

use super::capabilities::{
    validate_transcription_request, LanguageCoverage, LanguageHintSupport,
    TranscriptionCapabilities, VocabularySupport,
};
use super::languages::{normalize_whisper_policy, whisper_language_codes};
use super::models::{ModelLanguageSet, WhisperModelManifest};
use super::types::{
    AudioInput, CleanupRequest, CleanupResult, EngineError, TranscriptResult, TranscriptionRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineRole {
    Transcription,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionBoundary {
    LocalProcess,
    Network,
}

/// Trusted identity attached by an adapter constructor.
///
/// Fields are private and location is derived from a non-public execution
/// boundary. A caller can therefore route descriptors returned by registered
/// adapters, but cannot construct a descriptor that labels arbitrary network
/// work as local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineDescriptor {
    id: Arc<str>,
    display_name: Arc<str>,
    provider: EngineProvider,
    boundary: ExecutionBoundary,
    role: EngineRole,
}

impl EngineDescriptor {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn provider(&self) -> EngineProvider {
        self.provider
    }

    pub fn location(&self) -> EngineLocation {
        match self.boundary {
            ExecutionBoundary::LocalProcess => EngineLocation::Local,
            ExecutionBoundary::Network => EngineLocation::Cloud,
        }
    }

    pub fn role(&self) -> EngineRole {
        self.role
    }

    fn local_whisper(model: &WhisperModelManifest) -> Result<Self, EngineError> {
        Self::validated(
            model.id.clone(),
            model.display_name.clone(),
            EngineProvider::WhisperCpp,
            ExecutionBoundary::LocalProcess,
            EngineRole::Transcription,
        )
    }

    fn cloud_speech(
        id: impl Into<String>,
        display_name: impl Into<String>,
        provider: EngineProvider,
    ) -> Result<Self, EngineError> {
        if !matches!(
            provider,
            EngineProvider::OpenAi | EngineProvider::Gemini | EngineProvider::XAi
        ) {
            return Err(EngineError::InvalidRequest(
                "cloud speech adapters require a cloud provider".into(),
            ));
        }
        Self::validated(
            id.into(),
            display_name.into(),
            provider,
            ExecutionBoundary::Network,
            EngineRole::Transcription,
        )
    }

    fn builtin_cleanup() -> Self {
        // Static built-in metadata is known-valid. Keeping construction here
        // still preserves the same trusted-boundary invariant.
        Self::validated(
            "builtin-readable-v1".into(),
            "Readable cleanup".into(),
            EngineProvider::BuiltIn,
            ExecutionBoundary::LocalProcess,
            EngineRole::Cleanup,
        )
        .expect("built-in cleanup descriptor must be valid")
    }

    fn validated(
        id: String,
        display_name: String,
        provider: EngineProvider,
        boundary: ExecutionBoundary,
        role: EngineRole,
    ) -> Result<Self, EngineError> {
        if id.trim().is_empty() || display_name.trim().is_empty() {
            return Err(EngineError::InvalidRequest(
                "engine id and display name cannot be empty".into(),
            ));
        }
        if id.chars().any(char::is_whitespace) {
            return Err(EngineError::InvalidRequest(
                "engine id cannot contain whitespace".into(),
            ));
        }
        Ok(Self {
            id: Arc::from(id),
            display_name: Arc::from(display_name),
            provider,
            boundary,
            role,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_local(id: &str, role: EngineRole) -> Self {
        Self::validated(
            id.into(),
            id.into(),
            EngineProvider::WhisperCpp,
            ExecutionBoundary::LocalProcess,
            role,
        )
        .unwrap()
    }

    #[cfg(test)]
    pub(crate) fn test_cloud(id: &str, role: EngineRole) -> Self {
        Self::validated(
            id.into(),
            id.into(),
            EngineProvider::OpenAi,
            ExecutionBoundary::Network,
            role,
        )
        .unwrap()
    }
}

mod private {
    pub trait Sealed {}
}

/// Batch transcription contract. Although an engine may also advertise a
/// streaming surface, this method must return a final result.
pub trait TranscriptionEngine: private::Sealed + Send + Sync {
    fn descriptor(&self) -> &EngineDescriptor;
    fn capabilities(&self) -> &TranscriptionCapabilities;
    fn transcribe(
        &self,
        request: TranscriptionRequest<'_>,
    ) -> Result<TranscriptResult, EngineError>;
}

pub trait CleanupEngine: private::Sealed + Send + Sync {
    fn descriptor(&self) -> &EngineDescriptor;
    fn cleanup(&self, request: CleanupRequest<'_>) -> Result<CleanupResult, EngineError>;
}

/// The narrow seam the future whisper.cpp FFI implementation must satisfy.
/// It is sealed so only trusted application code may implement the decoder and
/// receive a local execution identity.
pub trait WhisperCppDecoder: private::Sealed + Send + Sync {
    fn decode(&self, request: WhisperDecodeRequest<'_>) -> Result<TranscriptResult, EngineError>;
}

#[derive(Debug, Clone, Copy)]
pub struct WhisperDecodeRequest<'a> {
    /// Always mono 16 kHz normalized PCM. The adapter validates this before
    /// calling a decoder, so the FFI layer may rely on the invariant.
    pub audio: AudioInput<'a>,
    /// A normalized whisper.cpp language token such as `en`, never a locale.
    pub language_hint: Option<&'a str>,
    pub translate_to_english: bool,
    pub prompt_vocabulary: &'a [&'a str],
}

pub struct WhisperCppAdapter<D> {
    descriptor: EngineDescriptor,
    capabilities: TranscriptionCapabilities,
    model: Arc<WhisperModelManifest>,
    decoder: D,
}

impl<D> private::Sealed for WhisperCppAdapter<D> where D: WhisperCppDecoder {}

impl<D> WhisperCppAdapter<D>
where
    D: WhisperCppDecoder,
{
    /// `WhisperCppDecoder` is sealed, so this public constructor accepts only
    /// reviewed in-crate decoder implementations and cannot be used to label an
    /// arbitrary external network backend as local.
    pub fn new(model: Arc<WhisperModelManifest>, decoder: D) -> Result<Self, EngineError> {
        model
            .validate()
            .map_err(|reason| EngineError::InvalidRequest(reason.into()))?;

        let multilingual = model.languages == ModelLanguageSet::Multilingual;
        let capabilities = TranscriptionCapabilities {
            batch: true,
            streaming: false,
            language_detection: multilingual,
            language_hints: LanguageHintSupport::Single,
            // Base Whisper chooses one language token for a decoding task. It
            // may incidentally emit borrowed words, but does not promise
            // segment-level language preservation.
            code_switching: false,
            translation: multilingual,
            vocabulary: VocabularySupport::PromptBiasing,
            offline: true,
            input_languages: LanguageCoverage::Explicit(whisper_language_codes(&model)),
            translation_targets: if multilingual {
                LanguageCoverage::explicit(["en"])
            } else {
                LanguageCoverage::None
            },
        };
        capabilities
            .validate_declaration()
            .map_err(|reason| EngineError::InvalidRequest(reason.into()))?;

        Ok(Self {
            descriptor: EngineDescriptor::local_whisper(&model)?,
            capabilities,
            model,
            decoder,
        })
    }

    pub fn model(&self) -> &Arc<WhisperModelManifest> {
        &self.model
    }
}

impl<D> TranscriptionEngine for WhisperCppAdapter<D>
where
    D: WhisperCppDecoder,
{
    fn descriptor(&self) -> &EngineDescriptor {
        &self.descriptor
    }

    fn capabilities(&self) -> &TranscriptionCapabilities {
        &self.capabilities
    }

    fn transcribe(
        &self,
        request: TranscriptionRequest<'_>,
    ) -> Result<TranscriptResult, EngineError> {
        request.validate()?;
        validate_whisper_audio(request.audio)?;

        let normalized_policy = normalize_whisper_policy(request.language_policy, &self.model)?;
        validate_transcription_request(&normalized_policy, request.vocabulary, &self.capabilities)?;

        let (language_hint, translate_to_english) = map_whisper_policy(&normalized_policy);
        let result = self.decoder.decode(WhisperDecodeRequest {
            audio: request.audio,
            language_hint,
            translate_to_english,
            prompt_vocabulary: request.vocabulary,
        })?;
        validate_batch_result(&result)?;
        Ok(result)
    }
}

fn validate_whisper_audio(audio: AudioInput<'_>) -> Result<(), EngineError> {
    if audio.sample_rate_hz != 16_000 {
        return Err(EngineError::InvalidRequest(
            "whisper.cpp requires 16 kHz PCM; resample before transcription".into(),
        ));
    }
    if audio.channels != 1 {
        return Err(EngineError::InvalidRequest(
            "whisper.cpp requires mono PCM; downmix before transcription".into(),
        ));
    }
    if audio
        .samples
        .iter()
        .any(|sample| !(-1.0..=1.0).contains(sample))
    {
        return Err(EngineError::InvalidRequest(
            "whisper.cpp PCM samples must be normalized to -1.0..=1.0".into(),
        ));
    }
    Ok(())
}

fn validate_batch_result(result: &TranscriptResult) -> Result<(), EngineError> {
    result.validate()?;
    if !result.is_final {
        return Err(EngineError::InvalidResult(
            "batch transcription returned a non-final result".into(),
        ));
    }
    Ok(())
}

fn map_whisper_policy(policy: &LanguagePolicy) -> (Option<&str>, bool) {
    match policy {
        LanguagePolicy::Auto | LanguagePolicy::Mixed { .. } => (None, false),
        LanguagePolicy::Fixed { language } => (Some(language.as_str()), false),
        LanguagePolicy::Preferred { languages } => (languages.first().map(String::as_str), false),
        LanguagePolicy::Translate {
            source_languages, ..
        } => (source_languages.first().map(String::as_str), true),
    }
}

/// Provider SDK/HTTP implementations adapt to this transport-neutral seam.
/// It is synchronous by design; callers execute network adapters away from the
/// UI thread. A separate streaming adapter will own partial-result semantics.
///
/// P2: capabilities are still supplied by the in-crate provider adapter. Before
/// settings expose arbitrary cloud model IDs, replace that argument with a
/// versioned provider/model capability registry verified against each API.
pub trait CloudSpeechClient: Send + Sync {
    fn transcribe(
        &self,
        request: TranscriptionRequest<'_>,
    ) -> Result<TranscriptResult, EngineError>;
}

pub struct CloudSpeechAdapter<C> {
    descriptor: EngineDescriptor,
    capabilities: TranscriptionCapabilities,
    client: C,
}

impl<C> private::Sealed for CloudSpeechAdapter<C> where C: CloudSpeechClient {}

impl<C> CloudSpeechAdapter<C>
where
    C: CloudSpeechClient,
{
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        provider: EngineProvider,
        capabilities: TranscriptionCapabilities,
        client: C,
    ) -> Result<Self, EngineError> {
        if capabilities.offline {
            return Err(EngineError::InvalidRequest(
                "a cloud adapter cannot claim offline execution".into(),
            ));
        }
        if !capabilities.batch {
            return Err(EngineError::InvalidRequest(
                "the synchronous cloud adapter requires batch support".into(),
            ));
        }
        capabilities
            .validate_declaration()
            .map_err(|reason| EngineError::InvalidRequest(reason.into()))?;

        Ok(Self {
            descriptor: EngineDescriptor::cloud_speech(id, display_name, provider)?,
            capabilities,
            client,
        })
    }
}

impl<C> TranscriptionEngine for CloudSpeechAdapter<C>
where
    C: CloudSpeechClient,
{
    fn descriptor(&self) -> &EngineDescriptor {
        &self.descriptor
    }

    fn capabilities(&self) -> &TranscriptionCapabilities {
        &self.capabilities
    }

    fn transcribe(
        &self,
        request: TranscriptionRequest<'_>,
    ) -> Result<TranscriptResult, EngineError> {
        request.validate()?;
        validate_transcription_request(
            request.language_policy,
            request.vocabulary,
            &self.capabilities,
        )?;
        let result = self.client.transcribe(request)?;
        validate_batch_result(&result)?;
        Ok(result)
    }
}

/// Fast, private cleanup used as the no-model baseline. It only removes exact
/// filler tokens and normalizes whitespace; it never sends text off-device or
/// rewrites the user's meaning.
///
/// P2: this baseline is English/ASCII-oriented and has no cleanup-language
/// capability contract yet. Register it only for English sessions until
/// Unicode tokenization and language-specific filler policies are modeled.
pub struct RuleBasedCleanupEngine {
    fillers: Vec<String>,
    descriptor: EngineDescriptor,
}

impl private::Sealed for RuleBasedCleanupEngine {}

impl Default for RuleBasedCleanupEngine {
    fn default() -> Self {
        Self::new(["um", "uh", "erm"])
    }
}

impl RuleBasedCleanupEngine {
    pub fn new<I, S>(fillers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            fillers: fillers
                .into_iter()
                .map(Into::into)
                .map(|filler| filler.trim().to_ascii_lowercase())
                .filter(|filler| !filler.is_empty())
                .collect(),
            descriptor: EngineDescriptor::builtin_cleanup(),
        }
    }

    fn is_filler(&self, token: &str) -> bool {
        let normalized = token
            .trim_matches(|character: char| character.is_ascii_punctuation())
            .to_ascii_lowercase();
        self.fillers.iter().any(|filler| filler == &normalized)
    }
}

impl CleanupEngine for RuleBasedCleanupEngine {
    fn descriptor(&self) -> &EngineDescriptor {
        &self.descriptor
    }

    fn cleanup(&self, request: CleanupRequest<'_>) -> Result<CleanupResult, EngineError> {
        request.transcript.validate()?;
        if request.output_language.is_some() {
            return Err(EngineError::InvalidRequest(
                "rule-based cleanup preserves the input language".into(),
            ));
        }

        let original = request.transcript.text.as_str();
        let cleaned = original
            .split_whitespace()
            .filter(|token| !self.is_filler(token))
            .collect::<Vec<_>>()
            .join(" ");

        Ok(CleanupResult {
            changed: cleaned != original,
            text: cleaned,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::engines::models::curated_whisper_models;

    type WhisperInvocation = Option<(Option<String>, bool, Vec<String>)>;

    #[derive(Default)]
    struct RecordingWhisperDecoder {
        invocation: Mutex<WhisperInvocation>,
        partial: bool,
    }

    impl private::Sealed for RecordingWhisperDecoder {}

    impl WhisperCppDecoder for RecordingWhisperDecoder {
        fn decode(
            &self,
            request: WhisperDecodeRequest<'_>,
        ) -> Result<TranscriptResult, EngineError> {
            *self.invocation.lock().unwrap() = Some((
                request.language_hint.map(str::to_owned),
                request.translate_to_english,
                request
                    .prompt_vocabulary
                    .iter()
                    .map(|term| (*term).to_owned())
                    .collect(),
            ));
            let mut result = TranscriptResult::final_text("hello Spick");
            result.is_final = !self.partial;
            Ok(result)
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
        }
    }

    #[test]
    fn whisper_adapter_normalizes_locale_before_decoder() {
        let decoder = RecordingWhisperDecoder::default();
        let adapter =
            WhisperCppAdapter::new(Arc::clone(&curated_whisper_models()[2]), decoder).unwrap();
        let policy = LanguagePolicy::Fixed {
            language: "en-IN".into(),
        };
        let result = adapter
            .transcribe(request(&[0.0, 0.2, -0.1], &policy, &["Spick"]))
            .unwrap();

        assert_eq!(result.text, "hello Spick");
        assert_eq!(adapter.descriptor().location(), EngineLocation::Local);
        assert!(adapter.capabilities().offline);
        assert_eq!(
            *adapter.decoder.invocation.lock().unwrap(),
            Some((Some("en".into()), false, vec!["Spick".into()]))
        );
    }

    #[test]
    fn unsupported_language_never_reaches_whisper_decoder() {
        let adapter = WhisperCppAdapter::new(
            Arc::clone(&curated_whisper_models()[2]),
            RecordingWhisperDecoder::default(),
        )
        .unwrap();
        let policy = LanguagePolicy::Fixed {
            language: "zu-ZA".into(),
        };

        assert!(adapter.transcribe(request(&[0.0], &policy, &[])).is_err());
        assert!(adapter.decoder.invocation.lock().unwrap().is_none());
    }

    #[test]
    fn cantonese_requires_a_large_v3_model() {
        let policy = LanguagePolicy::Fixed {
            language: "yue-HK".into(),
        };
        let small = WhisperCppAdapter::new(
            Arc::clone(&curated_whisper_models()[2]),
            RecordingWhisperDecoder::default(),
        )
        .unwrap();
        assert!(small.transcribe(request(&[0.0], &policy, &[])).is_err());

        let large = WhisperCppAdapter::new(
            Arc::clone(&curated_whisper_models()[3]),
            RecordingWhisperDecoder::default(),
        )
        .unwrap();
        assert!(large.transcribe(request(&[0.0], &policy, &[])).is_ok());
        assert_eq!(
            large.decoder.invocation.lock().unwrap().as_ref().unwrap().0,
            Some("yue".into())
        );
    }

    #[test]
    fn mixed_language_mode_is_not_claimed_by_base_whisper() {
        let adapter = WhisperCppAdapter::new(
            Arc::clone(&curated_whisper_models()[3]),
            RecordingWhisperDecoder::default(),
        )
        .unwrap();
        let policy = LanguagePolicy::Mixed {
            languages: vec!["en-IN".into(), "hi-IN".into()],
        };
        assert!(!adapter.capabilities().code_switching);
        assert!(adapter.transcribe(request(&[0.0], &policy, &[])).is_err());
        assert!(adapter.decoder.invocation.lock().unwrap().is_none());
    }

    #[test]
    fn whisper_boundary_requires_normalized_16khz_mono_pcm() {
        let adapter = WhisperCppAdapter::new(
            Arc::clone(&curated_whisper_models()[2]),
            RecordingWhisperDecoder::default(),
        )
        .unwrap();
        let policy = LanguagePolicy::Fixed {
            language: "en".into(),
        };

        let wrong_rate = TranscriptionRequest {
            audio: AudioInput {
                samples: &[0.0],
                sample_rate_hz: 48_000,
                channels: 1,
            },
            language_policy: &policy,
            vocabulary: &[],
        };
        assert!(adapter.transcribe(wrong_rate).is_err());

        let stereo = TranscriptionRequest {
            audio: AudioInput {
                samples: &[0.0, 0.0],
                sample_rate_hz: 16_000,
                channels: 2,
            },
            language_policy: &policy,
            vocabulary: &[],
        };
        assert!(adapter.transcribe(stereo).is_err());
        assert!(adapter
            .transcribe(request(&[0.0, 1.01], &policy, &[]))
            .is_err());
        assert!(adapter.decoder.invocation.lock().unwrap().is_none());
    }

    #[test]
    fn batch_whisper_rejects_partial_results() {
        let adapter = WhisperCppAdapter::new(
            Arc::clone(&curated_whisper_models()[2]),
            RecordingWhisperDecoder {
                partial: true,
                ..Default::default()
            },
        )
        .unwrap();
        let policy = LanguagePolicy::Fixed {
            language: "en".into(),
        };
        assert_eq!(
            adapter.transcribe(request(&[0.0], &policy, &[])),
            Err(EngineError::InvalidResult(
                "batch transcription returned a non-final result".into()
            ))
        );
    }

    struct FixedCloudClient {
        partial: bool,
    }

    impl CloudSpeechClient for FixedCloudClient {
        fn transcribe(
            &self,
            _request: TranscriptionRequest<'_>,
        ) -> Result<TranscriptResult, EngineError> {
            let mut result = TranscriptResult::final_text("cloud result");
            result.is_final = !self.partial;
            Ok(result)
        }
    }

    fn cloud_capabilities() -> TranscriptionCapabilities {
        TranscriptionCapabilities {
            batch: true,
            streaming: false,
            language_detection: true,
            language_hints: LanguageHintSupport::Multiple,
            code_switching: false,
            translation: true,
            vocabulary: VocabularySupport::Exact,
            offline: false,
            input_languages: LanguageCoverage::ProviderDefined,
            translation_targets: LanguageCoverage::ProviderDefined,
        }
    }

    #[test]
    fn cloud_adapter_is_always_network_labeled_and_batch_final() {
        let adapter = CloudSpeechAdapter::new(
            "sample-cloud-speech",
            "Sample cloud speech",
            EngineProvider::OpenAi,
            cloud_capabilities(),
            FixedCloudClient { partial: false },
        )
        .unwrap();
        let policy = LanguagePolicy::Auto;

        let result = adapter.transcribe(request(&[0.0], &policy, &[])).unwrap();
        assert_eq!(result.text, "cloud result");
        assert_eq!(adapter.descriptor().location(), EngineLocation::Cloud);

        assert!(CloudSpeechAdapter::new(
            "fake-local",
            "Fake local",
            EngineProvider::WhisperCpp,
            cloud_capabilities(),
            FixedCloudClient { partial: false }
        )
        .is_err());

        let partial = CloudSpeechAdapter::new(
            "partial-cloud",
            "Partial cloud",
            EngineProvider::Gemini,
            cloud_capabilities(),
            FixedCloudClient { partial: true },
        )
        .unwrap();
        assert!(partial.transcribe(request(&[0.0], &policy, &[])).is_err());
    }

    #[test]
    fn rule_cleanup_removes_only_exact_fillers_and_normalizes_whitespace() {
        let engine = RuleBasedCleanupEngine::default();
        let transcript = TranscriptResult::final_text("Um,  I need the umbrella, uh, today.");
        let result = engine
            .cleanup(CleanupRequest {
                transcript: &transcript,
                output_language: None,
            })
            .unwrap();

        assert_eq!(result.text, "I need the umbrella, today.");
        assert!(result.changed);
        assert_eq!(engine.descriptor().role(), EngineRole::Cleanup);
    }
}
