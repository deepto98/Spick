use std::sync::Arc;

use crate::domain::{EngineLocation, EngineProvider, LanguagePolicy};

use super::capabilities::{
    validate_language_policy, validate_transcription_request, LanguageCoverage,
    LanguageHintSupport, TranscriptionCapabilities, VocabularySupport,
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

pub(super) mod private {
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

/// The narrow seam the whisper.cpp runtime must satisfy.
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
    pub cancellation: Option<&'a std::sync::atomic::AtomicBool>,
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

        let capabilities = whisper_model_capabilities(&model);
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

fn whisper_model_capabilities(model: &WhisperModelManifest) -> TranscriptionCapabilities {
    let multilingual = model.languages == ModelLanguageSet::Multilingual;
    TranscriptionCapabilities {
        batch: true,
        streaming: false,
        language_detection: multilingual,
        language_hints: LanguageHintSupport::Single,
        // Base Whisper chooses one language token for a decoding task. It may
        // incidentally emit borrowed words, but does not promise segment-level
        // language preservation.
        code_switching: false,
        translation: multilingual,
        vocabulary: VocabularySupport::PromptBiasing,
        offline: true,
        input_languages: LanguageCoverage::Explicit(whisper_language_codes(model)),
        translation_targets: if multilingual {
            LanguageCoverage::explicit(["en"])
        } else {
            LanguageCoverage::None
        },
    }
}

/// Checks a saved language policy before a model becomes active. Keeping this
/// at the adapter boundary prevents the model manager and decoder from
/// disagreeing about English-only or multilingual behavior.
pub fn validate_whisper_model_policy(
    policy: &LanguagePolicy,
    model: &WhisperModelManifest,
) -> Result<(), EngineError> {
    model
        .validate()
        .map_err(|reason| EngineError::InvalidRequest(reason.into()))?;
    let normalized = normalize_whisper_policy(policy, model)?;
    validate_language_policy(&normalized, &whisper_model_capabilities(model))
        .map_err(EngineError::UnsupportedPolicy)
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
            cancellation: request.cancellation,
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

/// Fast, private cleanup used as the no-model baseline. It removes a small,
/// language-specific set of standalone hesitation sounds outside quoted or
/// explicitly referenced text, then normalizes whitespace. Languages without
/// a reviewed policy pass through unchanged.
pub struct RuleBasedCleanupEngine {
    english_fillers: Vec<String>,
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
            english_fillers: fillers
                .into_iter()
                .map(Into::into)
                .map(|filler| filler.trim().to_lowercase())
                .filter(|filler| !filler.is_empty())
                .collect(),
            descriptor: EngineDescriptor::builtin_cleanup(),
        }
    }

    fn is_filler(&self, language: CleanupLanguage, token: &str) -> bool {
        let trimmed = token.trim_matches(is_spoken_filler_punctuation);
        if trimmed.is_empty() {
            return false;
        }
        let normalized = trimmed.to_lowercase();
        match language {
            CleanupLanguage::English => self
                .english_fillers
                .iter()
                .any(|filler| filler == &normalized),
            _ => language.fillers().contains(&normalized.as_str()),
        }
    }

    fn is_explicit_reference(
        &self,
        language: CleanupLanguage,
        tokens: &[&str],
        index: usize,
    ) -> bool {
        let token = tokens[index];
        let normalized = normalized_token(token);
        let letters = token
            .chars()
            .filter(|character| character.is_alphabetic())
            .collect::<String>();
        if letters.chars().count() > 1
            && letters.chars().any(char::is_uppercase)
            && letters.chars().all(|character| !character.is_lowercase())
        {
            return true;
        }

        if token.eq_ignore_ascii_case(&normalized)
            && tokens
                .get(index + 1)
                .is_some_and(|next| matches!(*next, "=" | ":=" | "=>" | "==" | "(" | "["))
        {
            return true;
        }

        if token.eq_ignore_ascii_case(&normalized)
            && tokens.get(index + 1).is_some_and(|next| {
                matches!(
                    normalized_token(next).as_str(),
                    "is" | "was" | "means" | "meant" | "refers" | "stands"
                )
            })
        {
            return true;
        }

        let Some(previous) = index.checked_sub(1).map(|previous| tokens[previous]) else {
            return false;
        };
        let previous = normalized_token(previous);
        if matches!(
            previous.as_str(),
            "say"
                | "says"
                | "said"
                | "word"
                | "term"
                | "phrase"
                | "called"
                | "means"
                | "write"
                | "writes"
                | "wrote"
                | "type"
                | "typed"
                | "spell"
                | "spelled"
                | "variable"
                | "identifier"
                | "symbol"
                | "parameter"
                | "argument"
                | "function"
                | "method"
                | "class"
                | "struct"
                | "enum"
                | "constant"
                | "field"
                | "property"
                | "key"
                | "column"
                | "label"
                | "command"
                | "option"
                | "flag"
                | "module"
                | "package"
                | "namespace"
                | "let"
                | "var"
                | "const"
                | "call"
                | "named"
                | "name"
                | "set"
                | "assign"
                | "rename"
                | "use"
                | "declare"
        ) {
            return true;
        }

        if language.reference_words().contains(&previous.as_str()) {
            return true;
        }

        // A bare filler after a determiner is likely being used as a noun
        // ("an um"), while punctuation such as "a, um," still marks a pause.
        matches!(previous.as_str(), "a" | "an" | "the") && token.eq_ignore_ascii_case(&normalized)
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
        let Some(language) = request
            .transcript
            .detected_language
            .as_deref()
            .and_then(CleanupLanguage::from_detected_tag)
        else {
            return Ok(CleanupResult {
                text: original.into(),
                changed: false,
            });
        };

        let tokens = original.split_whitespace().collect::<Vec<_>>();
        let mut quotes = QuoteState::default();
        let mut cleaned_tokens = Vec::with_capacity(tokens.len());
        for (index, token) in tokens.iter().enumerate() {
            let quoted = quotes.observe(token);
            if self.is_filler(language, token)
                && !quoted
                && !self.is_explicit_reference(language, &tokens, index)
            {
                if comma_repair_is_safe(&cleaned_tokens) {
                    repair_punctuation_after_removed_filler(&mut cleaned_tokens, token);
                } else {
                    cleaned_tokens.push((*token).to_owned());
                }
            } else {
                cleaned_tokens.push((*token).to_owned());
            }
        }
        let cleaned = cleaned_tokens.join(" ");

        Ok(CleanupResult {
            changed: cleaned != original,
            text: cleaned,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupLanguage {
    English,
    Spanish,
    French,
    German,
    Hindi,
    Italian,
    Russian,
    Japanese,
    Chinese,
}

impl CleanupLanguage {
    fn from_detected_tag(tag: &str) -> Option<Self> {
        let primary = tag.trim().split('-').next()?;
        if primary.eq_ignore_ascii_case("en") {
            Some(Self::English)
        } else if primary.eq_ignore_ascii_case("es") {
            Some(Self::Spanish)
        } else if primary.eq_ignore_ascii_case("fr") {
            Some(Self::French)
        } else if primary.eq_ignore_ascii_case("de") {
            Some(Self::German)
        } else if primary.eq_ignore_ascii_case("hi") {
            Some(Self::Hindi)
        } else if primary.eq_ignore_ascii_case("it") {
            Some(Self::Italian)
        } else if primary.eq_ignore_ascii_case("ru") {
            Some(Self::Russian)
        } else if primary.eq_ignore_ascii_case("ja") {
            Some(Self::Japanese)
        } else if primary.eq_ignore_ascii_case("zh") {
            Some(Self::Chinese)
        } else {
            None
        }
    }

    fn fillers(self) -> &'static [&'static str] {
        match self {
            Self::English => &[],
            Self::Spanish => &["eh"],
            Self::French => &["euh"],
            Self::German => &["äh", "ähm"],
            Self::Hindi => &["उम्", "उम्म"],
            Self::Italian => &["ehm"],
            Self::Russian => &["эээ"],
            Self::Japanese => &["えー"],
            Self::Chinese => &["呃"],
        }
    }

    fn reference_words(self) -> &'static [&'static str] {
        match self {
            Self::English => &[],
            Self::Spanish => &["palabra", "término", "frase", "decir", "escribe", "nombre"],
            Self::French => &["mot", "terme", "phrase", "dire", "écrire", "nom"],
            Self::German => &["wort", "begriff", "phrase", "sagen", "schreiben", "name"],
            Self::Hindi => &["शब्द", "वाक्यांश", "कहना", "लिखना", "नाम"],
            Self::Italian => &["parola", "termine", "frase", "dire", "scrivere", "nome"],
            Self::Russian => &["слово", "термин", "фраза", "сказать", "написать", "имя"],
            Self::Japanese => &["言葉", "単語", "用語", "言う", "書く", "名前"],
            Self::Chinese => &["词", "术语", "说", "写", "名字"],
        }
    }
}

fn is_spoken_filler_punctuation(character: char) -> bool {
    matches!(
        character,
        ',' | '.'
            | '!'
            | '?'
            | ';'
            | ':'
            | '…'
            | '，'
            | '。'
            | '！'
            | '？'
            | '；'
            | '：'
            | '、'
            | '।'
            | '¿'
            | '¡'
    )
}

fn comma_repair_is_safe(cleaned_tokens: &[String]) -> bool {
    let Some(previous) = cleaned_tokens.last() else {
        return true;
    };
    if !previous.ends_with(',') {
        return true;
    }
    if introductory_comma_belongs_to_previous(cleaned_tokens) {
        return true;
    }

    matches!(
        normalized_token(previous).as_str(),
        "am" | "are"
            | "is"
            | "was"
            | "were"
            | "be"
            | "been"
            | "being"
            | "i"
            | "you"
            | "he"
            | "she"
            | "we"
            | "they"
            | "it"
            | "and"
            | "but"
            | "or"
    )
}

fn repair_punctuation_after_removed_filler(cleaned_tokens: &mut Vec<String>, removed: &str) {
    let preserve_introductory_comma = introductory_comma_belongs_to_previous(cleaned_tokens);

    if !preserve_introductory_comma
        && cleaned_tokens
            .last()
            .is_some_and(|previous| previous.ends_with(','))
    {
        let previous = cleaned_tokens
            .last_mut()
            .expect("a trailing comma requires a previous token");
        previous.pop();
        if previous.is_empty() {
            cleaned_tokens.pop();
        }
    }

    let terminal = removed
        .chars()
        .rev()
        .find(|character| !matches!(character, ',' | ';' | ':' | '，' | '；' | '：' | '、'))
        .filter(|character| matches!(character, '.' | '!' | '?' | '…' | '。' | '！' | '？' | '।'));
    if let (Some(terminal), Some(previous)) = (terminal, cleaned_tokens.last_mut()) {
        // When the filler ended a sentence, its terminal mark supersedes even
        // a comma that was otherwise valid before an introductory filler.
        // Appending would produce malformed output such as `Well,.`.
        if previous.ends_with(',') {
            previous.pop();
        }
        if !previous.ends_with(['.', '!', '?', '…', '。', '！', '？', '।']) {
            previous.push(terminal);
        }
    }
}

fn introductory_comma_belongs_to_previous(cleaned_tokens: &[String]) -> bool {
    let starts_sentence = cleaned_tokens.len() == 1
        || cleaned_tokens
            .get(cleaned_tokens.len().saturating_sub(2))
            .is_some_and(|before| before.ends_with(['.', '!', '?', '…', '。', '！', '？', '।']));
    starts_sentence
        && cleaned_tokens.last().is_some_and(|previous| {
            matches!(
                normalized_token(previous).as_str(),
                "well"
                    | "yes"
                    | "no"
                    | "okay"
                    | "ok"
                    | "so"
                    | "now"
                    | "then"
                    | "however"
                    | "meanwhile"
                    | "moreover"
                    | "nevertheless"
                    | "finally"
            )
        })
}

fn normalized_token(token: &str) -> String {
    token
        .trim_matches(|character: char| {
            character.is_ascii_punctuation() || is_spoken_filler_punctuation(character)
        })
        .to_lowercase()
}

#[derive(Default)]
struct QuoteState {
    double: bool,
    single: bool,
    backtick: bool,
}

impl QuoteState {
    fn observe(&mut self, token: &str) -> bool {
        let started_quoted = self.any();
        let mut contains_quote = false;
        let characters = token.chars().collect::<Vec<_>>();

        for (index, character) in characters.iter().enumerate() {
            match character {
                '"' => {
                    self.double = !self.double;
                    contains_quote = true;
                }
                '“' => {
                    self.double = true;
                    contains_quote = true;
                }
                '”' if self.double => {
                    self.double = false;
                    contains_quote = true;
                }
                '‘' => {
                    self.single = true;
                    contains_quote = true;
                }
                '’' if self.single => {
                    self.single = false;
                    contains_quote = true;
                }
                '\'' if index == 0 || (self.single && index + 1 == characters.len()) => {
                    self.single = !self.single;
                    contains_quote = true;
                }
                '`' => {
                    self.backtick = !self.backtick;
                    contains_quote = true;
                }
                _ => {}
            }
        }

        started_quoted || contains_quote
    }

    fn any(&self) -> bool {
        self.double || self.single || self.backtick
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
            cancellation: None,
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
    fn model_policy_validation_rejects_auto_for_english_only_weights() {
        let english_only = &curated_whisper_models()[1];
        assert!(matches!(
            validate_whisper_model_policy(&LanguagePolicy::Auto, english_only),
            Err(EngineError::UnsupportedPolicy(_))
        ));
        assert!(validate_whisper_model_policy(
            &LanguagePolicy::Fixed {
                language: "en-IN".into(),
            },
            english_only,
        )
        .is_ok());
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
            cancellation: None,
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
            cancellation: None,
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
    fn rule_cleanup_removes_pause_punctuated_fillers_and_normalizes_whitespace() {
        let engine = RuleBasedCleanupEngine::default();
        let transcript = english_transcript("Um,  I need the umbrella uh, today.");
        let result = engine
            .cleanup(CleanupRequest {
                transcript: &transcript,
                output_language: None,
            })
            .unwrap();

        assert_eq!(result.text, "I need the umbrella today.");
        assert!(result.changed);
        assert_eq!(engine.descriptor().role(), EngineRole::Cleanup);
    }

    #[test]
    fn rule_cleanup_preserves_quoted_and_explicitly_referenced_fillers() {
        let engine = RuleBasedCleanupEngine::default();
        let transcript =
            english_transcript("Say um, then \"uh and erm\", but, uh, keep the term um and UM.");

        let result = engine
            .cleanup(CleanupRequest {
                transcript: &transcript,
                output_language: None,
            })
            .unwrap();

        assert_eq!(
            result.text,
            "Say um, then \"uh and erm\", but keep the term um and UM."
        );
        assert!(result.changed);
    }

    #[test]
    fn rule_cleanup_repairs_pause_punctuation() {
        let engine = RuleBasedCleanupEngine::default();
        let transcript = english_transcript(
            "This is, uh, ready. However, um, we should go. I, erm, agree. I think uh.",
        );

        let result = engine
            .cleanup(CleanupRequest {
                transcript: &transcript,
                output_language: None,
            })
            .unwrap();

        assert_eq!(
            result.text,
            "This is ready. However, we should go. I agree. I think."
        );
        assert!(result.changed);
    }

    #[test]
    fn rule_cleanup_replaces_an_introductory_comma_with_the_terminal_mark() {
        let engine = RuleBasedCleanupEngine::default();

        for (original, expected) in [("Well, um.", "Well."), ("Well, um…", "Well…")] {
            let transcript = english_transcript(original);
            let result = engine
                .cleanup(CleanupRequest {
                    transcript: &transcript,
                    output_language: None,
                })
                .unwrap();

            assert_eq!(result.text, expected);
            assert!(result.changed);
        }
    }

    #[test]
    fn rule_cleanup_preserves_identifier_and_code_uses() {
        let engine = RuleBasedCleanupEngine::default();
        let original = "um = 2. Set um to three. Rename um next. Use uh here. Declare erm now. Keep the name um. Then let uh = 4 and call erm(). Keep --um and $uh. Leave um” alone.";
        let transcript = english_transcript(original);

        let result = engine
            .cleanup(CleanupRequest {
                transcript: &transcript,
                output_language: None,
            })
            .unwrap();

        assert_eq!(result.text, original);
        assert!(!result.changed);
    }

    #[test]
    fn rule_cleanup_removes_standalone_bare_fillers() {
        let engine = RuleBasedCleanupEngine::default();
        let original = "I um think uh might mean erm plus one.";
        let transcript = english_transcript(original);

        let result = engine
            .cleanup(CleanupRequest {
                transcript: &transcript,
                output_language: None,
            })
            .unwrap();

        assert_eq!(result.text, "I think might mean plus one.");
        assert!(result.changed);
    }

    #[test]
    fn rule_cleanup_uses_reviewed_fillers_for_each_detected_language() {
        let engine = RuleBasedCleanupEngine::default();
        for (language, original, expected) in [
            ("es", "Necesito eh, revisarlo.", "Necesito revisarlo."),
            ("fr-FR", "Je dois euh, vérifier.", "Je dois vérifier."),
            ("de", "Ich muss Ähm, prüfen.", "Ich muss prüfen."),
            ("hi", "मुझे उम्म, यह देखना है।", "मुझे यह देखना है।"),
            ("it", "Devo ehm, controllare.", "Devo controllare."),
            ("ru", "Нужно эээ, проверить.", "Нужно проверить."),
            (
                "ja",
                "確認します えー、 もう一度。",
                "確認します もう一度。",
            ),
            ("zh-Hans", "我想 呃， 再看看。", "我想 再看看。"),
        ] {
            let transcript = transcript_in(language, original);
            let result = engine
                .cleanup(CleanupRequest {
                    transcript: &transcript,
                    output_language: None,
                })
                .unwrap();

            assert_eq!(result.text, expected, "language {language}");
            assert!(result.changed, "language {language}");
        }
    }

    #[test]
    fn rule_cleanup_keeps_multilingual_quotes_and_references() {
        let engine = RuleBasedCleanupEngine::default();
        let original = "Di \"eh,\", conserva la palabra eh, y deja eh sin puntuación.";
        let transcript = transcript_in("es", original);

        let result = engine
            .cleanup(CleanupRequest {
                transcript: &transcript,
                output_language: None,
            })
            .unwrap();

        assert_eq!(
            result.text,
            "Di \"eh,\", conserva la palabra eh, y deja sin puntuación."
        );
        assert!(result.changed);
    }

    #[test]
    fn rule_cleanup_keeps_bare_fillers_used_as_subjects_or_nouns() {
        let engine = RuleBasedCleanupEngine::default();
        let original = "Um is the label. An uh can be meaningful. The term erm means a pause.";
        let transcript = english_transcript(original);

        let result = engine
            .cleanup(CleanupRequest {
                transcript: &transcript,
                output_language: None,
            })
            .unwrap();

        assert_eq!(result.text, original);
        assert!(!result.changed);
    }

    #[test]
    fn rule_cleanup_is_language_tagged_instead_of_using_a_global_word_list() {
        let engine = RuleBasedCleanupEngine::default();
        let spanish = transcript_in("es", "Keep um, exactly.");
        let english = transcript_in("en", "Keep euh, exactly.");

        for transcript in [&spanish, &english] {
            let result = engine
                .cleanup(CleanupRequest {
                    transcript,
                    output_language: None,
                })
                .unwrap();
            assert_eq!(result.text, transcript.text);
            assert!(!result.changed);
        }
    }

    #[test]
    fn rule_cleanup_leaves_unknown_and_unreviewed_languages_untouched() {
        let engine = RuleBasedCleanupEngine::default();
        let unknown = TranscriptResult::final_text("Um,  leave this exactly.");
        let swahili = transcript_in("sw", "Um,  acha hivi.");

        for transcript in [&unknown, &swahili] {
            let result = engine
                .cleanup(CleanupRequest {
                    transcript,
                    output_language: None,
                })
                .unwrap();
            assert_eq!(result.text, transcript.text);
            assert!(!result.changed);
        }
    }

    fn english_transcript(text: &str) -> TranscriptResult {
        transcript_in("en", text)
    }

    fn transcript_in(language: &str, text: &str) -> TranscriptResult {
        let mut transcript = TranscriptResult::final_text(text);
        transcript.detected_language = Some(language.into());
        transcript
    }
}
