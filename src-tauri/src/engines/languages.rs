use std::sync::Arc;

use crate::domain::LanguagePolicy;

use super::capabilities::PolicyCompatibilityError;
use super::models::{ModelLanguageSet, WhisperModelManifest};

/// Language tokens present in multilingual Whisper families before large-v3.
/// This mirrors whisper.cpp's language table except `yue`, whose token was
/// introduced with large-v3. Keep this list versioned with the linked model
/// catalog rather than assuming the runtime's newest table fits every model.
const LEGACY_MULTILINGUAL_CODES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
    "ha", "ba", "jw", "su",
];

pub fn whisper_language_codes(model: &WhisperModelManifest) -> Arc<[String]> {
    if model.languages == ModelLanguageSet::EnglishOnly {
        return Arc::from(vec![String::from("en")]);
    }

    let extra = usize::from(model.family.supports_cantonese());
    let mut languages = Vec::with_capacity(LEGACY_MULTILINGUAL_CODES.len() + extra);
    languages.extend(LEGACY_MULTILINGUAL_CODES.iter().map(|code| (*code).into()));
    if model.family.supports_cantonese() {
        languages.push("yue".into());
    }
    Arc::from(languages)
}

/// Converts a structurally valid BCP-47 tag into the short code expected by
/// whisper.cpp. Region/script subtags select UI locale behavior but Whisper
/// consumes only its language token, so `en-IN` becomes `en` and `zh-Hant`
/// becomes `zh`.
pub fn normalize_whisper_language_tag(
    language_tag: &str,
    model: &WhisperModelManifest,
) -> Result<String, PolicyCompatibilityError> {
    let trimmed = language_tag.trim();
    let mut subtags = trimmed.split('-');
    let primary_tag = subtags.next().unwrap_or_default();
    if !(2..=8).contains(&primary_tag.len())
        || !primary_tag.bytes().all(|byte| byte.is_ascii_alphabetic())
        || subtags.any(|subtag| {
            subtag.is_empty()
                || subtag.len() > 8
                || !subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        return Err(PolicyCompatibilityError::InvalidPolicy(format!(
            "invalid BCP-47 language tag: {trimmed}"
        )));
    }

    let primary = primary_tag.to_ascii_lowercase();

    // whisper.cpp follows two legacy identifiers. Accept their modern BCP-47
    // forms at the product boundary, but pass the decoder's identifiers over
    // FFI. The legacy forms remain accepted for imported settings.
    let whisper_code = match primary.as_str() {
        "jv" | "jw" => "jw",
        "fil" | "tl" => "tl",
        code => code,
    };

    if model_supports_code(model, whisper_code) {
        Ok(whisper_code.into())
    } else {
        Err(PolicyCompatibilityError::LanguageUnsupported(
            language_tag.into(),
        ))
    }
}

fn model_supports_code(model: &WhisperModelManifest, code: &str) -> bool {
    match model.languages {
        ModelLanguageSet::EnglishOnly => code == "en",
        ModelLanguageSet::Multilingual => {
            LEGACY_MULTILINGUAL_CODES.contains(&code)
                || (code == "yue" && model.family.supports_cantonese())
        }
    }
}

pub fn normalize_whisper_policy(
    policy: &LanguagePolicy,
    model: &WhisperModelManifest,
) -> Result<LanguagePolicy, PolicyCompatibilityError> {
    // This normalized copy is decoder-only. The caller must retain the
    // original output locale (for example en-IN) for a later formatting stage;
    // Whisper's English translation task does not guarantee regional style.
    // P2: `LanguagePolicy::Translate` also cannot express an auto-detected
    // source because its source list is non-empty. Model source selection as
    // Auto/Fixed/Preferred before exposing that choice in the UI.
    policy
        .validate()
        .map_err(PolicyCompatibilityError::InvalidPolicy)?;

    Ok(match policy {
        LanguagePolicy::Auto => LanguagePolicy::Auto,
        LanguagePolicy::Fixed { language } => LanguagePolicy::Fixed {
            language: normalize_whisper_language_tag(language, model)?,
        },
        LanguagePolicy::Preferred { languages } => LanguagePolicy::Preferred {
            languages: normalize_languages(languages, model)?,
        },
        LanguagePolicy::Mixed { languages } => LanguagePolicy::Mixed {
            languages: normalize_languages(languages, model)?,
        },
        LanguagePolicy::Translate {
            source_languages,
            output_language,
        } => LanguagePolicy::Translate {
            source_languages: normalize_languages(source_languages, model)?,
            output_language: normalize_whisper_language_tag(output_language, model)?,
        },
    })
}

fn normalize_languages(
    languages: &[String],
    model: &WhisperModelManifest,
) -> Result<Vec<String>, PolicyCompatibilityError> {
    languages
        .iter()
        .map(|language| normalize_whisper_language_tag(language, model))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::engines::models::curated_whisper_models;

    #[test]
    fn locale_and_script_tags_become_whisper_language_codes() {
        let small = curated_whisper_models()[2].as_ref();
        assert_eq!(
            normalize_whisper_language_tag("en-IN", small),
            Ok("en".into())
        );
        assert_eq!(
            normalize_whisper_language_tag("zh-Hant-TW", small),
            Ok("zh".into())
        );
        assert_eq!(
            normalize_whisper_language_tag("jv-ID", small),
            Ok("jw".into())
        );
    }

    #[test]
    fn legacy_language_table_is_complete_and_unique() {
        assert_eq!(LEGACY_MULTILINGUAL_CODES.len(), 99);
        assert_eq!(
            LEGACY_MULTILINGUAL_CODES
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len(),
            99
        );
    }

    #[test]
    fn unsupported_codes_are_rejected_before_ffi() {
        let small = curated_whisper_models()[2].as_ref();
        assert_eq!(
            normalize_whisper_language_tag("zu-ZA", small),
            Err(PolicyCompatibilityError::LanguageUnsupported(
                "zu-ZA".into()
            ))
        );
        assert!(matches!(
            normalize_whisper_language_tag("en--IN", small),
            Err(PolicyCompatibilityError::InvalidPolicy(_))
        ));
    }

    #[test]
    fn cantonese_is_only_exposed_for_large_v3_families() {
        let tiny = curated_whisper_models()[0].as_ref();
        let large_v3_turbo = curated_whisper_models()[3].as_ref();

        assert!(normalize_whisper_language_tag("yue-HK", tiny).is_err());
        assert_eq!(
            normalize_whisper_language_tag("yue-HK", large_v3_turbo),
            Ok("yue".into())
        );
        assert!(!whisper_language_codes(tiny)
            .iter()
            .any(|code| code == "yue"));
        assert!(whisper_language_codes(large_v3_turbo)
            .iter()
            .any(|code| code == "yue"));
    }

    #[test]
    fn english_only_models_reject_every_non_english_policy() {
        let english = curated_whisper_models()[1].as_ref();
        assert_eq!(
            normalize_whisper_policy(
                &LanguagePolicy::Fixed {
                    language: "en-GB".into()
                },
                english
            ),
            Ok(LanguagePolicy::Fixed {
                language: "en".into()
            })
        );
        assert!(normalize_whisper_policy(
            &LanguagePolicy::Fixed {
                language: "hi-IN".into()
            },
            english
        )
        .is_err());
    }
}
