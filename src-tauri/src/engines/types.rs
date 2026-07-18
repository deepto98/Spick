use std::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use serde::Serialize;

use crate::domain::{DictationDelivery, LanguagePolicy};

use super::capabilities::PolicyCompatibilityError;

#[derive(Debug, Clone, Copy)]
pub struct AudioInput<'a> {
    /// Interleaved, normalized PCM samples.
    pub samples: &'a [f32],
    pub sample_rate_hz: u32,
    pub channels: u16,
}

impl AudioInput<'_> {
    pub fn validate(self) -> Result<(), EngineError> {
        if self.samples.is_empty() {
            return Err(EngineError::InvalidRequest("audio is empty".into()));
        }
        if self.sample_rate_hz == 0 {
            return Err(EngineError::InvalidRequest(
                "sample rate must be greater than zero".into(),
            ));
        }
        if self.channels == 0 {
            return Err(EngineError::InvalidRequest(
                "channel count must be greater than zero".into(),
            ));
        }
        if !self
            .samples
            .len()
            .is_multiple_of(usize::from(self.channels))
        {
            return Err(EngineError::InvalidRequest(
                "interleaved audio does not contain complete frames".into(),
            ));
        }
        if self.samples.iter().any(|sample| !sample.is_finite()) {
            return Err(EngineError::InvalidRequest(
                "audio contains a non-finite sample".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TranscriptionRequest<'a> {
    pub audio: AudioInput<'a>,
    pub language_policy: &'a LanguagePolicy,
    /// Product names and uncommon words used to bias recognition.
    pub vocabulary: &'a [&'a str],
    /// A session-owned flag checked before and during local inference.
    pub cancellation: Option<&'a AtomicBool>,
}

impl TranscriptionRequest<'_> {
    pub fn validate(self) -> Result<(), EngineError> {
        if self.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        self.audio.validate()?;
        if self.vocabulary.iter().any(|term| term.trim().is_empty()) {
            return Err(EngineError::InvalidRequest(
                "vocabulary terms cannot be empty".into(),
            ));
        }
        Ok(())
    }

    pub fn is_cancelled(self) -> bool {
        self.cancellation
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub language: Option<String>,
    /// Normalized to 0.0..=1.0 when the provider reports confidence.
    pub confidence: Option<f32>,
}

impl TranscriptSegment {
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.text.trim().is_empty() {
            return Err(EngineError::InvalidResult(
                "transcript segments cannot be empty".into(),
            ));
        }
        if self.end_ms < self.start_ms {
            return Err(EngineError::InvalidResult(
                "transcript segment ends before it starts".into(),
            ));
        }
        validate_optional_language(self.language.as_deref())?;
        validate_optional_confidence(self.confidence)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResult {
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
    pub detected_language: Option<String>,
    pub confidence: Option<f32>,
    pub is_final: bool,
}

/// Ephemeral, in-memory output for one completed transcription. It is safe to
/// expose to the dashboard for recovery, but is never persisted by this type.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationTranscript {
    pub session_id: String,
    pub engine_id: String,
    pub transcript: TranscriptResult,
    pub delivery: DictationDelivery,
}

impl TranscriptResult {
    pub fn final_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            segments: Vec::new(),
            detected_language: None,
            confidence: None,
            is_final: true,
        }
    }

    pub fn validate(&self) -> Result<(), EngineError> {
        validate_optional_language(self.detected_language.as_deref())?;
        validate_optional_confidence(self.confidence)?;

        let mut previous_end = 0;
        for (index, segment) in self.segments.iter().enumerate() {
            segment.validate()?;
            if index > 0 && segment.start_ms < previous_end {
                return Err(EngineError::InvalidResult(
                    "transcript segments overlap or are out of order".into(),
                ));
            }
            previous_end = segment.end_ms;
        }
        Ok(())
    }
}

fn validate_optional_language(language: Option<&str>) -> Result<(), EngineError> {
    if language.is_some_and(|language| language.trim().is_empty()) {
        Err(EngineError::InvalidResult(
            "reported language cannot be empty".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_optional_confidence(confidence: Option<f32>) -> Result<(), EngineError> {
    if confidence
        .is_some_and(|confidence| !confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
    {
        Err(EngineError::InvalidResult(
            "confidence must be between zero and one".into(),
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CleanupRequest<'a> {
    pub transcript: &'a TranscriptResult,
    /// The desired output tag, if cleanup is also responsible for language
    /// normalization. A plain cleanup engine may ignore this when it declares
    /// that it only preserves language.
    pub output_language: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupResult {
    pub text: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    InvalidRequest(String),
    UnsupportedPolicy(PolicyCompatibilityError),
    Backend(String),
    InvalidResult(String),
    Cancelled,
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(reason) => write!(formatter, "invalid engine request: {reason}"),
            Self::UnsupportedPolicy(reason) => write!(formatter, "unsupported policy: {reason}"),
            Self::Backend(reason) => write!(formatter, "engine backend failed: {reason}"),
            Self::InvalidResult(reason) => write!(formatter, "invalid engine result: {reason}"),
            Self::Cancelled => formatter.write_str("engine request was cancelled"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<PolicyCompatibilityError> for EngineError {
    fn from(value: PolicyCompatibilityError) -> Self {
        Self::UnsupportedPolicy(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_requires_complete_finite_frames() {
        let odd_stereo = AudioInput {
            samples: &[0.0, 0.1, 0.2],
            sample_rate_hz: 16_000,
            channels: 2,
        };
        assert!(odd_stereo.validate().is_err());

        let invalid = AudioInput {
            samples: &[f32::NAN],
            sample_rate_hz: 16_000,
            channels: 1,
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn transcript_segments_must_be_ordered_and_confidence_normalized() {
        let mut result = TranscriptResult {
            text: "hello world".into(),
            segments: vec![
                TranscriptSegment {
                    text: "hello".into(),
                    start_ms: 0,
                    end_ms: 500,
                    language: Some("en".into()),
                    confidence: Some(0.9),
                },
                TranscriptSegment {
                    text: "world".into(),
                    start_ms: 400,
                    end_ms: 700,
                    language: Some("en".into()),
                    confidence: Some(0.8),
                },
            ],
            detected_language: Some("en".into()),
            confidence: Some(0.85),
            is_final: true,
        };
        assert!(result.validate().is_err());

        result.segments[1].start_ms = 500;
        result.confidence = Some(1.1);
        assert!(result.validate().is_err());

        result.confidence = Some(1.0);
        assert_eq!(result.validate(), Ok(()));
    }

    #[test]
    fn silence_can_be_a_valid_empty_final_result() {
        assert_eq!(TranscriptResult::final_text("").validate(), Ok(()));
    }
}
