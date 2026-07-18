use std::fmt;
use std::sync::Arc;

use crate::domain::LanguagePolicy;

/// How many language hints an engine can honor for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageHintSupport {
    None,
    Single,
    Multiple,
}

impl LanguageHintSupport {
    fn accepts(self, count: usize) -> bool {
        match self {
            Self::None => count == 0,
            Self::Single => count <= 1,
            Self::Multiple => true,
        }
    }
}

/// The language set an adapter is prepared to accept.
///
/// `ProviderDefined` is intentionally distinct from "every language": it
/// means the provider owns the exact, potentially evolving list and Spick can
/// only validate the structural parts of the request locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageCoverage {
    None,
    ProviderDefined,
    Explicit(Arc<[String]>),
}

impl LanguageCoverage {
    pub fn explicit<I, S>(languages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Explicit(Arc::from(
            languages.into_iter().map(Into::into).collect::<Vec<_>>(),
        ))
    }

    pub fn supports(&self, requested: &str) -> bool {
        match self {
            Self::None => false,
            Self::ProviderDefined => true,
            Self::Explicit(supported) => supported
                .iter()
                .any(|candidate| language_tag_matches(candidate, requested)),
        }
    }
}

fn language_tag_matches(supported: &str, requested: &str) -> bool {
    if supported.eq_ignore_ascii_case(requested) {
        return true;
    }

    // Providers commonly advertise a base language ("en") while the user
    // selects a locale ("en-IN"). A locale never matches in the other
    // direction, so a provider declaring en-US does not implicitly claim all
    // English variants.
    requested
        .split_once('-')
        .is_some_and(|(base, _)| supported.eq_ignore_ascii_case(base))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VocabularySupport {
    None,
    /// The engine can bias recognition with an initial prompt or phrase list,
    /// but cannot guarantee exact spelling.
    PromptBiasing,
    /// The engine exposes a first-class custom vocabulary feature.
    Exact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionCapabilities {
    pub batch: bool,
    pub streaming: bool,
    pub language_detection: bool,
    pub language_hints: LanguageHintSupport,
    pub code_switching: bool,
    pub translation: bool,
    pub vocabulary: VocabularySupport,
    pub offline: bool,
    pub input_languages: LanguageCoverage,
    pub translation_targets: LanguageCoverage,
}

impl TranscriptionCapabilities {
    /// Detect contradictory declarations before an adapter is registered.
    pub fn validate_declaration(&self) -> Result<(), &'static str> {
        if !self.batch && !self.streaming {
            return Err("a transcription engine must support batch or streaming input");
        }
        if self.code_switching && !self.language_detection {
            return Err("code switching requires language detection");
        }
        // Code-switch preservation and candidate-language constraints are
        // independent features. Some providers can preserve mixed speech but
        // accept no language list. `Mixed { languages }` still requires
        // multiple hints below because that specific product policy asks the
        // engine to constrain detection to the listed candidates.
        if self.translation && self.translation_targets == LanguageCoverage::None {
            return Err("translation requires at least one translation target");
        }
        if !self.translation && self.translation_targets != LanguageCoverage::None {
            return Err("translation targets were declared without translation support");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyCompatibilityError {
    InvalidPolicy(String),
    LanguageDetectionUnsupported,
    LanguageHintsUnsupported {
        requested: usize,
        available: LanguageHintSupport,
    },
    CodeSwitchingUnsupported,
    TranslationUnsupported,
    VocabularyUnsupported,
    LanguageUnsupported(String),
    TranslationTargetUnsupported(String),
}

impl fmt::Display for PolicyCompatibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy(reason) => write!(formatter, "invalid language policy: {reason}"),
            Self::LanguageDetectionUnsupported => {
                formatter.write_str("the engine cannot automatically detect language")
            }
            Self::LanguageHintsUnsupported {
                requested,
                available,
            } => write!(
                formatter,
                "the policy needs {requested} language hint(s), but the engine supports {available:?}"
            ),
            Self::CodeSwitchingUnsupported => {
                formatter.write_str("the engine cannot preserve mixed-language speech")
            }
            Self::TranslationUnsupported => {
                formatter.write_str("the engine cannot translate while transcribing")
            }
            Self::VocabularyUnsupported => {
                formatter.write_str("the engine cannot use custom vocabulary")
            }
            Self::LanguageUnsupported(language) => {
                write!(formatter, "the engine does not support input language {language}")
            }
            Self::TranslationTargetUnsupported(language) => write!(
                formatter,
                "the engine cannot translate into output language {language}"
            ),
        }
    }
}

impl std::error::Error for PolicyCompatibilityError {}

/// Validates the product-level language policy against one adapter's declared
/// behavior. This is stricter than `LanguagePolicy::validate`, which only
/// checks that the policy itself is structurally sound.
pub fn validate_language_policy(
    policy: &LanguagePolicy,
    capabilities: &TranscriptionCapabilities,
) -> Result<(), PolicyCompatibilityError> {
    policy
        .validate()
        .map_err(PolicyCompatibilityError::InvalidPolicy)?;

    match policy {
        LanguagePolicy::Auto => require_detection(capabilities),
        LanguagePolicy::Fixed { language } => {
            require_hints(capabilities, 1)?;
            require_input_language(capabilities, language)
        }
        LanguagePolicy::Preferred { languages } => {
            require_detection(capabilities)?;
            require_hints(capabilities, languages.len())?;
            require_input_languages(capabilities, languages)
        }
        LanguagePolicy::Mixed { languages } => {
            if !capabilities.code_switching {
                return Err(PolicyCompatibilityError::CodeSwitchingUnsupported);
            }
            require_detection(capabilities)?;
            require_hints(capabilities, languages.len())?;
            require_input_languages(capabilities, languages)
        }
        LanguagePolicy::Translate {
            source_languages,
            output_language,
        } => {
            if !capabilities.translation {
                return Err(PolicyCompatibilityError::TranslationUnsupported);
            }
            if source_languages.len() > 1 {
                require_detection(capabilities)?;
            }
            require_hints(capabilities, source_languages.len())?;
            require_input_languages(capabilities, source_languages)?;
            if !capabilities.translation_targets.supports(output_language) {
                return Err(PolicyCompatibilityError::TranslationTargetUnsupported(
                    output_language.clone(),
                ));
            }
            Ok(())
        }
    }
}

pub fn validate_transcription_request(
    policy: &LanguagePolicy,
    vocabulary: &[&str],
    capabilities: &TranscriptionCapabilities,
) -> Result<(), PolicyCompatibilityError> {
    validate_language_policy(policy, capabilities)?;
    if !vocabulary.is_empty() && capabilities.vocabulary == VocabularySupport::None {
        return Err(PolicyCompatibilityError::VocabularyUnsupported);
    }
    Ok(())
}

fn require_detection(
    capabilities: &TranscriptionCapabilities,
) -> Result<(), PolicyCompatibilityError> {
    if capabilities.language_detection {
        Ok(())
    } else {
        Err(PolicyCompatibilityError::LanguageDetectionUnsupported)
    }
}

fn require_hints(
    capabilities: &TranscriptionCapabilities,
    count: usize,
) -> Result<(), PolicyCompatibilityError> {
    if capabilities.language_hints.accepts(count) {
        Ok(())
    } else {
        Err(PolicyCompatibilityError::LanguageHintsUnsupported {
            requested: count,
            available: capabilities.language_hints,
        })
    }
}

fn require_input_languages(
    capabilities: &TranscriptionCapabilities,
    languages: &[String],
) -> Result<(), PolicyCompatibilityError> {
    for language in languages {
        require_input_language(capabilities, language)?;
    }
    Ok(())
}

fn require_input_language(
    capabilities: &TranscriptionCapabilities,
    language: &str,
) -> Result<(), PolicyCompatibilityError> {
    if capabilities.input_languages.supports(language) {
        Ok(())
    } else {
        Err(PolicyCompatibilityError::LanguageUnsupported(
            language.to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn multilingual() -> TranscriptionCapabilities {
        TranscriptionCapabilities {
            batch: true,
            streaming: true,
            language_detection: true,
            language_hints: LanguageHintSupport::Multiple,
            code_switching: true,
            translation: true,
            vocabulary: VocabularySupport::PromptBiasing,
            offline: false,
            input_languages: LanguageCoverage::ProviderDefined,
            translation_targets: LanguageCoverage::explicit(["en", "fr"]),
        }
    }

    fn english_only() -> TranscriptionCapabilities {
        TranscriptionCapabilities {
            batch: true,
            streaming: false,
            language_detection: false,
            language_hints: LanguageHintSupport::Single,
            code_switching: false,
            translation: false,
            vocabulary: VocabularySupport::None,
            offline: true,
            input_languages: LanguageCoverage::explicit(["en"]),
            translation_targets: LanguageCoverage::None,
        }
    }

    #[test]
    fn base_language_capability_accepts_a_locale() {
        assert!(LanguageCoverage::explicit(["en"]).supports("en-IN"));
        assert!(!LanguageCoverage::explicit(["en-US"]).supports("en-IN"));
    }

    #[test]
    fn validates_every_language_policy_mode() {
        let policies = [
            LanguagePolicy::Auto,
            LanguagePolicy::Fixed {
                language: "de-DE".into(),
            },
            LanguagePolicy::Preferred {
                languages: vec!["en-IN".into(), "hi-IN".into()],
            },
            LanguagePolicy::Mixed {
                languages: vec!["en-IN".into(), "hi-IN".into()],
            },
            LanguagePolicy::Translate {
                source_languages: vec!["de-DE".into(), "fr-FR".into()],
                output_language: "en-US".into(),
            },
        ];

        let capabilities = multilingual();
        for policy in policies {
            assert_eq!(validate_language_policy(&policy, &capabilities), Ok(()));
        }
    }

    #[test]
    fn rejects_auto_and_non_english_for_an_english_only_model() {
        let capabilities = english_only();
        assert_eq!(
            validate_language_policy(&LanguagePolicy::Auto, &capabilities),
            Err(PolicyCompatibilityError::LanguageDetectionUnsupported)
        );
        assert_eq!(
            validate_language_policy(
                &LanguagePolicy::Fixed {
                    language: "es".into()
                },
                &capabilities
            ),
            Err(PolicyCompatibilityError::LanguageUnsupported("es".into()))
        );
        assert_eq!(
            validate_language_policy(
                &LanguagePolicy::Fixed {
                    language: "en-GB".into()
                },
                &capabilities
            ),
            Ok(())
        );
    }

    #[test]
    fn reports_specific_mixed_translation_and_vocabulary_failures() {
        let capabilities = english_only();
        assert_eq!(
            validate_language_policy(
                &LanguagePolicy::Mixed {
                    languages: vec!["en".into(), "hi".into()]
                },
                &capabilities
            ),
            Err(PolicyCompatibilityError::CodeSwitchingUnsupported)
        );
        assert_eq!(
            validate_language_policy(
                &LanguagePolicy::Translate {
                    source_languages: vec!["es".into()],
                    output_language: "en".into()
                },
                &capabilities
            ),
            Err(PolicyCompatibilityError::TranslationUnsupported)
        );
        assert_eq!(
            validate_transcription_request(
                &LanguagePolicy::Fixed {
                    language: "en".into()
                },
                &["Spick"],
                &capabilities
            ),
            Err(PolicyCompatibilityError::VocabularyUnsupported)
        );
    }

    #[test]
    fn declaration_validation_catches_contradictions() {
        let invalid = TranscriptionCapabilities {
            batch: false,
            streaming: false,
            ..multilingual()
        };
        assert!(invalid.validate_declaration().is_err());

        let invalid = TranscriptionCapabilities {
            code_switching: true,
            language_detection: false,
            ..multilingual()
        };
        assert!(invalid.validate_declaration().is_err());

        let unconstrained_mixed_speech = TranscriptionCapabilities {
            code_switching: true,
            language_hints: LanguageHintSupport::None,
            ..multilingual()
        };
        assert!(unconstrained_mixed_speech.validate_declaration().is_ok());
        assert!(validate_language_policy(
            &LanguagePolicy::Mixed {
                languages: vec!["en".into(), "hi".into()]
            },
            &unconstrained_mixed_speech
        )
        .is_err());
    }
}
