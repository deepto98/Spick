use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Once,
    },
};

use whisper_rs::{
    get_lang_str, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use super::{
    models::{ModelLanguageSet, WhisperModelManifest},
    providers::{
        private, TranscriptionEngine, WhisperCppAdapter, WhisperCppDecoder, WhisperDecodeRequest,
    },
    types::{EngineError, TranscriptResult, TranscriptSegment, TranscriptionRequest},
};

static INSTALL_LOG_HOOKS: Once = Once::new();
const MAX_PROMPT_TOKENS: usize = 224;

/// Owns the in-process whisper.cpp context cache.
///
/// Model loading is much more expensive than creating a decoder state, so the
/// active model stays resident between dictations. A model switch replaces the
/// cache only after the new context has loaded successfully.
#[derive(Default)]
pub struct WhisperCppRuntime {
    loaded: Mutex<Option<LoadedContext>>,
}

struct LoadedContext {
    model_id: String,
    model_path: PathBuf,
    context: Arc<WhisperContext>,
}

impl WhisperCppRuntime {
    pub fn load(&self, model: &WhisperModelManifest, model_path: &Path) -> Result<(), EngineError> {
        self.context_for(model, model_path).map(|_| ())
    }

    pub fn transcribe(
        &self,
        model: Arc<WhisperModelManifest>,
        model_path: &Path,
        request: TranscriptionRequest<'_>,
    ) -> Result<TranscriptResult, EngineError> {
        if request.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        let context = self.context_for(&model, model_path)?;
        if request.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        let adapter = WhisperCppAdapter::new(model, NativeWhisperDecoder { context })?;
        adapter.transcribe(request)
    }

    pub fn unload(&self, model_id: &str) {
        if let Ok(mut loaded) = self.loaded.lock() {
            if loaded
                .as_ref()
                .is_some_and(|loaded| loaded.model_id == model_id)
            {
                *loaded = None;
            }
        }
    }

    fn context_for(
        &self,
        model: &WhisperModelManifest,
        model_path: &Path,
    ) -> Result<Arc<WhisperContext>, EngineError> {
        let mut loaded = self
            .loaded
            .lock()
            .map_err(|_| EngineError::Backend("whisper.cpp model cache is unavailable".into()))?;

        if let Some(cached) = loaded.as_ref() {
            if cached.model_id == model.id && cached.model_path == model_path {
                return Ok(Arc::clone(&cached.context));
            }
        }

        if !model_path.is_file() {
            return Err(EngineError::Backend(format!(
                "{} is not installed",
                model.display_name
            )));
        }

        INSTALL_LOG_HOOKS.call_once(whisper_rs::install_logging_hooks);
        let context =
            WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
                .map_err(|error| {
                    EngineError::Backend(format!("could not load {}: {error}", model.display_name))
                })?;

        let expects_multilingual = model.languages == ModelLanguageSet::Multilingual;
        if context.is_multilingual() != expects_multilingual {
            return Err(EngineError::Backend(format!(
                "{} does not match its language metadata",
                model.display_name
            )));
        }

        let context = Arc::new(context);
        *loaded = Some(LoadedContext {
            model_id: model.id.clone(),
            model_path: model_path.to_path_buf(),
            context: Arc::clone(&context),
        });
        Ok(context)
    }
}

struct NativeWhisperDecoder {
    context: Arc<WhisperContext>,
}

impl private::Sealed for NativeWhisperDecoder {}

impl WhisperCppDecoder for NativeWhisperDecoder {
    fn decode(&self, request: WhisperDecodeRequest<'_>) -> Result<TranscriptResult, EngineError> {
        if is_cancelled(request.cancellation) {
            return Err(EngineError::Cancelled);
        }
        let prompt = vocabulary_prompt(request.prompt_vocabulary)?;
        let prompt_tokens = prompt
            .as_deref()
            .map(|prompt| {
                self.context
                    .tokenize(prompt, MAX_PROMPT_TOKENS)
                    .map_err(|error| {
                        EngineError::Backend(format!(
                            "whisper.cpp could not prepare vocabulary hints: {error}"
                        ))
                    })
            })
            .transpose()?;

        let mut parameters = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        parameters.set_n_threads(decoder_thread_count());
        parameters.set_translate(request.translate_to_english);
        parameters.set_language(request.language_hint);
        parameters.set_no_context(true);
        parameters.set_print_progress(false);
        parameters.set_print_realtime(false);
        parameters.set_print_special(false);
        parameters.set_print_timestamps(false);
        if let Some(tokens) = prompt_tokens.as_deref() {
            parameters.set_tokens(tokens);
        }
        if let Some(cancellation) = request.cancellation {
            // `state.full` is synchronous, so the borrowed AtomicBool and its
            // pointer remain valid for every callback invocation. The callback
            // only performs an atomic load and never touches whisper state.
            unsafe {
                parameters.set_abort_callback(Some(abort_when_cancelled));
                parameters.set_abort_callback_user_data(
                    std::ptr::from_ref(cancellation).cast_mut().cast::<c_void>(),
                );
            }
        }

        let mut state = self.context.create_state().map_err(|error| {
            EngineError::Backend(format!("whisper.cpp could not create a decoder: {error}"))
        })?;
        if let Err(error) = state.full(parameters, request.audio.samples) {
            if is_cancelled(request.cancellation) {
                return Err(EngineError::Cancelled);
            }
            return Err(EngineError::Backend(format!(
                "whisper.cpp decoding failed: {error}"
            )));
        }
        if is_cancelled(request.cancellation) {
            return Err(EngineError::Cancelled);
        }

        let detected_language = detected_language(&state);
        let mut transcript_text = String::new();
        let mut segments = Vec::new();

        for segment in state.as_iter() {
            let raw_text = segment.to_str().map_err(|error| {
                EngineError::InvalidResult(format!(
                    "whisper.cpp returned invalid transcript text: {error}"
                ))
            })?;
            let text = raw_text.trim();
            if text.is_empty() {
                continue;
            }

            transcript_text.push_str(raw_text);
            segments.push(TranscriptSegment {
                text: text.to_owned(),
                start_ms: timestamp_ms(segment.start_timestamp())?,
                end_ms: timestamp_ms(segment.end_timestamp())?,
                language: detected_language.clone(),
                confidence: None,
            });
        }

        Ok(TranscriptResult {
            text: transcript_text.trim().to_owned(),
            segments,
            detected_language,
            confidence: None,
            is_final: true,
        })
    }
}

unsafe extern "C" fn abort_when_cancelled(user_data: *mut c_void) -> bool {
    if user_data.is_null() {
        return false;
    }
    // SAFETY: `decode` passes a valid `&AtomicBool` that outlives the synchronous
    // whisper call. The callback does not retain the pointer.
    unsafe { &*user_data.cast::<AtomicBool>() }.load(Ordering::Relaxed)
}

fn is_cancelled(cancellation: Option<&AtomicBool>) -> bool {
    cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

fn detected_language(state: &whisper_rs::WhisperState) -> Option<String> {
    let language_id = state.full_lang_id_from_state();
    (language_id >= 0)
        .then(|| get_lang_str(language_id))
        .flatten()
        .map(str::to_owned)
}

fn decoder_thread_count() -> i32 {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4)
        .clamp(1, 8) as i32
}

fn timestamp_ms(centiseconds: i64) -> Result<u64, EngineError> {
    u64::try_from(centiseconds)
        .ok()
        .and_then(|value| value.checked_mul(10))
        .ok_or_else(|| {
            EngineError::InvalidResult("whisper.cpp returned an invalid timestamp".into())
        })
}

fn vocabulary_prompt(vocabulary: &[&str]) -> Result<Option<String>, EngineError> {
    if vocabulary.is_empty() {
        return Ok(None);
    }
    if vocabulary.iter().any(|term| term.contains('\0')) {
        return Err(EngineError::InvalidRequest(
            "vocabulary terms cannot contain null bytes".into(),
        ));
    }

    Ok(Some(vocabulary.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::LanguagePolicy,
        engines::{curated_whisper_models, AudioInput},
    };

    #[test]
    fn vocabulary_is_a_single_prompt_without_invented_instructions() {
        assert_eq!(vocabulary_prompt(&[]).unwrap(), None);
        assert_eq!(
            vocabulary_prompt(&["Spick", "whisper.cpp"]).unwrap(),
            Some("Spick, whisper.cpp".into())
        );
        assert!(vocabulary_prompt(&["bad\0term"]).is_err());
    }

    #[test]
    fn whisper_centiseconds_are_checked_before_conversion() {
        assert_eq!(timestamp_ms(123).unwrap(), 1_230);
        assert!(timestamp_ms(-1).is_err());
        assert!(timestamp_ms(i64::MAX).is_err());
    }

    #[test]
    fn decoder_threads_are_bounded() {
        assert!((1..=8).contains(&decoder_thread_count()));
    }

    #[test]
    #[ignore = "requires SPICK_WHISPER_MODEL_PATH and SPICK_WHISPER_WAV_PATH"]
    fn real_model_transcribes_a_wav_fixture() {
        let model_path = std::env::var("SPICK_WHISPER_MODEL_PATH").unwrap();
        let wav_path = std::env::var("SPICK_WHISPER_WAV_PATH").unwrap();
        let samples = read_pcm16_mono_wav(Path::new(&wav_path));
        let runtime = WhisperCppRuntime::default();
        let result = runtime
            .transcribe(
                Arc::clone(&curated_whisper_models()[0]),
                Path::new(&model_path),
                TranscriptionRequest {
                    audio: AudioInput {
                        samples: &samples,
                        sample_rate_hz: 16_000,
                        channels: 1,
                    },
                    language_policy: &LanguagePolicy::Auto,
                    vocabulary: &[],
                    cancellation: None,
                },
            )
            .unwrap();

        assert!(result.is_final);
        assert!(
            result.text.to_ascii_lowercase().contains("country"),
            "unexpected transcript: {}",
            result.text
        );
        assert_eq!(result.detected_language.as_deref(), Some("en"));
    }

    fn read_pcm16_mono_wav(path: &Path) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");

        let mut cursor = 12;
        let mut format = None;
        let mut samples = None;
        while cursor + 8 <= bytes.len() {
            let id = &bytes[cursor..cursor + 4];
            let size =
                u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
            let start = cursor + 8;
            let end = start + size;
            assert!(end <= bytes.len());

            if id == b"fmt " {
                format = Some((
                    u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap()),
                    u16::from_le_bytes(bytes[start + 2..start + 4].try_into().unwrap()),
                    u32::from_le_bytes(bytes[start + 4..start + 8].try_into().unwrap()),
                    u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap()),
                ));
            } else if id == b"data" {
                samples = Some(
                    bytes[start..end]
                        .chunks_exact(2)
                        .map(|sample| {
                            f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32_768.0
                        })
                        .collect(),
                );
            }
            cursor = end + (size % 2);
        }

        assert_eq!(format, Some((1, 1, 16_000, 16)));
        samples.expect("WAV data chunk")
    }
}
