use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperModelFamily {
    Tiny,
    Base,
    Small,
    Medium,
    LargeV1,
    LargeV2,
    LargeV3,
    LargeV3Turbo,
}

impl WhisperModelFamily {
    /// Cantonese (`yue`) was added to the tokenizer used by large-v3. Earlier
    /// families must not be offered that language even though a newer
    /// whisper.cpp runtime recognizes the code globally.
    pub fn supports_cantonese(self) -> bool {
        matches!(self, Self::LargeV3 | Self::LargeV3Turbo)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhisperQuantization {
    F16,
    F32,
    Q4_0,
    Q4_1,
    Q5_0,
    Q5_1,
    Q8_0,
    /// Preserves metadata for a future quantizer without requiring an app
    /// release merely to deserialize its name.
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelLanguageSet {
    Multilingual,
    EnglishOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelLicense {
    pub spdx_id: String,
    pub name: String,
    pub url: String,
    pub attribution: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperModelManifest {
    pub id: String,
    pub display_name: String,
    pub file_name: String,
    pub family: WhisperModelFamily,
    pub languages: ModelLanguageSet,
    pub quantization: WhisperQuantization,
    pub download_bytes: u64,
    pub sha256: String,
    pub source_url: String,
    pub license: ModelLicense,
}

// P2 manifest hardening before arbitrary community catalogs are enabled:
// persist the GGML/GGUF format revision, estimated peak RAM, compatible
// whisper.cpp revision range, and bundled weight/runtime license notices.
// The current curated entries are integrity-pinned, but those operational
// fields must become mandatory before installation is opened to unreviewed
// manifests.

impl WhisperModelManifest {
    pub fn is_multilingual(&self) -> bool {
        self.languages == ModelLanguageSet::Multilingual
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.trim().is_empty()
            || self.display_name.trim().is_empty()
            || self.file_name.trim().is_empty()
        {
            return Err("model identity fields cannot be empty");
        }
        if !self.file_name.ends_with(".bin") {
            return Err("whisper.cpp model files must use the .bin extension");
        }
        if self.download_bytes == 0 {
            return Err("model download size must be greater than zero");
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("model SHA-256 must be 64 lowercase hexadecimal characters");
        }
        if !self.source_url.starts_with("https://") {
            return Err("model source must use HTTPS");
        }
        if self.license.spdx_id.trim().is_empty()
            || self.license.name.trim().is_empty()
            || !self.license.url.starts_with("https://")
            || self.license.attribution.trim().is_empty()
        {
            return Err("model license metadata is incomplete");
        }

        let dot_en = self.file_name.contains(".en.") || self.file_name.contains(".en-");
        if dot_en != (self.languages == ModelLanguageSet::EnglishOnly) {
            return Err("model filename and language-set metadata disagree");
        }
        Ok(())
    }
}

const SOURCE_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

fn whisper_license() -> ModelLicense {
    ModelLicense {
        spdx_id: "MIT".into(),
        name: "MIT License".into(),
        url: "https://github.com/openai/whisper/blob/main/LICENSE".into(),
        attribution: "OpenAI Whisper weights converted to GGML format by the whisper.cpp project."
            .into(),
    }
}

struct CuratedModelSpec {
    id: &'static str,
    display_name: &'static str,
    file_name: &'static str,
    family: WhisperModelFamily,
    languages: ModelLanguageSet,
    quantization: WhisperQuantization,
    download_bytes: u64,
    sha256: &'static str,
}

fn model(spec: CuratedModelSpec) -> Arc<WhisperModelManifest> {
    Arc::new(WhisperModelManifest {
        id: spec.id.into(),
        display_name: spec.display_name.into(),
        file_name: spec.file_name.into(),
        family: spec.family,
        languages: spec.languages,
        quantization: spec.quantization,
        download_bytes: spec.download_bytes,
        sha256: spec.sha256.into(),
        source_url: format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/{SOURCE_REVISION}/{}",
            spec.file_name
        ),
        license: whisper_license(),
    })
}

/// A deliberately small, curated catalog. The catalog owns all strings and
/// returns reference-counted manifests so a downloaded/custom manifest can use
/// the exact same runtime path without leaking strings or requiring `'static`
/// input. URLs are revision-pinned and downloads must pass SHA-256 before use.
pub fn curated_whisper_models() -> &'static [Arc<WhisperModelManifest>] {
    static MODELS: OnceLock<Vec<Arc<WhisperModelManifest>>> = OnceLock::new();
    MODELS.get_or_init(|| {
        vec![
            model(CuratedModelSpec {
                id: "whisper-tiny-multilingual-f16",
                display_name: "Whisper Tiny (multilingual)",
                file_name: "ggml-tiny.bin",
                family: WhisperModelFamily::Tiny,
                languages: ModelLanguageSet::Multilingual,
                quantization: WhisperQuantization::F16,
                download_bytes: 77_691_713,
                sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            }),
            model(CuratedModelSpec {
                id: "whisper-base-english-q5-1",
                display_name: "Whisper Base English (Q5_1)",
                file_name: "ggml-base.en-q5_1.bin",
                family: WhisperModelFamily::Base,
                languages: ModelLanguageSet::EnglishOnly,
                quantization: WhisperQuantization::Q5_1,
                download_bytes: 59_721_011,
                sha256: "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
            }),
            model(CuratedModelSpec {
                id: "whisper-small-multilingual-q5-1",
                display_name: "Whisper Small (multilingual, Q5_1)",
                file_name: "ggml-small-q5_1.bin",
                family: WhisperModelFamily::Small,
                languages: ModelLanguageSet::Multilingual,
                quantization: WhisperQuantization::Q5_1,
                download_bytes: 190_085_487,
                sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
            }),
            model(CuratedModelSpec {
                id: "whisper-large-v3-turbo-multilingual-q5-0",
                display_name: "Whisper Large v3 Turbo (multilingual, Q5_0)",
                file_name: "ggml-large-v3-turbo-q5_0.bin",
                family: WhisperModelFamily::LargeV3Turbo,
                languages: ModelLanguageSet::Multilingual,
                quantization: WhisperQuantization::Q5_0,
                download_bytes: 574_041_195,
                sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
            }),
        ]
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn curated_catalog_is_valid_unique_and_integrity_checked() {
        let mut ids = HashSet::new();
        let mut files = HashSet::new();

        for model in curated_whisper_models() {
            assert_eq!(model.validate(), Ok(()), "{}", model.id);
            assert!(ids.insert(&model.id), "duplicate model id {}", model.id);
            assert!(
                files.insert(&model.file_name),
                "duplicate model file {}",
                model.file_name
            );
            assert!(model.source_url.contains(SOURCE_REVISION));
        }
    }

    #[test]
    fn catalog_does_not_confuse_dot_en_with_multilingual_models() {
        for model in curated_whisper_models() {
            assert_eq!(
                model.file_name.contains(".en"),
                model.languages == ModelLanguageSet::EnglishOnly,
                "{} has inconsistent language metadata",
                model.file_name
            );
        }
        assert!(curated_whisper_models()
            .iter()
            .any(|model| model.is_multilingual()));
    }

    #[test]
    fn custom_manifest_owns_runtime_metadata() {
        let mut custom = (*curated_whisper_models()[0]).clone();
        custom.id = String::from("downloaded-at-runtime");
        custom.display_name = String::from("My downloaded model");

        let shared = Arc::new(custom);
        let second_owner = Arc::clone(&shared);
        assert_eq!(second_owner.id, "downloaded-at-runtime");
        assert_eq!(shared.validate(), Ok(()));
    }
}
