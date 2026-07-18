//! Provider-neutral speech engine contracts.
//!
//! Transcription and cleanup deliberately have separate traits: a speech
//! recognizer turns audio into a structured transcript, while a cleanup engine
//! edits text. Keeping those roles independent lets Spick run whisper.cpp
//! locally and still choose a local rule-based or cloud cleanup engine without
//! coupling their lifecycles.

mod capabilities;
mod languages;
mod models;
mod providers;
mod routing;
mod types;
mod whisper_cpp;

pub use capabilities::{
    validate_language_policy, validate_transcription_request, LanguageCoverage,
    LanguageHintSupport, PolicyCompatibilityError, TranscriptionCapabilities, VocabularySupport,
};
pub use languages::{normalize_whisper_language_tag, whisper_language_codes};
pub use models::{
    curated_whisper_models, resolve_curated_whisper_model, ModelLanguageSet, ModelLicense,
    WhisperModelFamily, WhisperModelManifest, WhisperQuantization,
};
pub use providers::{
    validate_whisper_model_policy, CleanupEngine, CloudSpeechAdapter, CloudSpeechClient,
    EngineDescriptor, EngineRole, RuleBasedCleanupEngine, TranscriptionEngine, WhisperCppAdapter,
    WhisperCppDecoder, WhisperDecodeRequest,
};
pub use routing::{PrivacyMode, RoutePlan, RoutingError};
pub use types::{
    AudioInput, CleanupRequest, CleanupResult, DictationTranscript, EngineError, TranscriptResult,
    TranscriptSegment, TranscriptionRequest,
};
pub use whisper_cpp::WhisperCppRuntime;
