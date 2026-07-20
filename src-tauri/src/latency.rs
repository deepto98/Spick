//! Ephemeral, privacy-safe measurements for diagnosing perceived latency.
//!
//! A trace is owned by the processing worker and emitted once after that worker
//! owns a Completed or Failed terminal transition. It is never persisted and
//! deliberately excludes dictated content, audio samples, target metadata,
//! configuration identifiers, absolute timestamps, and error strings.

use std::time::{Duration, Instant};

use serde::Serialize;

pub const DICTATION_LATENCY_EVENT: &str = "dictation://latency";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationLatencyOutcome {
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationLatencyEvent {
    pub session_id: String,
    pub revision: u64,
    pub outcome: DictationLatencyOutcome,
    pub audio_duration_ms: Option<u64>,
    pub stop_to_processing_ms: u64,
    pub capture_finalize_ms: Option<u64>,
    pub transcription_ms: Option<u64>,
    pub delivery_ms: Option<u64>,
    pub stop_to_delivery_ms: Option<u64>,
    pub processing_total_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessingLatencyTrace {
    session_id: String,
    stop_requested_at: Instant,
    stop_to_processing_ms: u64,
    audio_duration_ms: Option<u64>,
    capture_finalize_ms: Option<u64>,
    transcription_ms: Option<u64>,
    delivery_ms: Option<u64>,
    stop_to_delivery_ms: Option<u64>,
}

impl ProcessingLatencyTrace {
    pub(crate) fn start(session_id: String, stop_requested_at: Instant) -> Self {
        Self {
            session_id,
            stop_requested_at,
            stop_to_processing_ms: 0,
            audio_duration_ms: None,
            capture_finalize_ms: None,
            transcription_ms: None,
            delivery_ms: None,
            stop_to_delivery_ms: None,
        }
    }

    pub(crate) fn mark_processing_emitted(&mut self) {
        self.mark_processing_emitted_at(Instant::now());
    }

    pub(crate) fn mark_capture_finalize(&mut self, started_at: Instant) {
        self.mark_capture_finalize_at(started_at, Instant::now());
    }

    pub(crate) fn set_audio_duration(&mut self, audio_duration_ms: u64) {
        self.audio_duration_ms = Some(audio_duration_ms);
    }

    pub(crate) fn mark_transcription(&mut self, started_at: Instant) {
        self.mark_transcription_at(started_at, Instant::now());
    }

    pub(crate) fn mark_delivery(&mut self, started_at: Instant) {
        self.mark_delivery_at(started_at, Instant::now());
    }

    pub(crate) fn finish(
        self,
        revision: u64,
        outcome: DictationLatencyOutcome,
    ) -> DictationLatencyEvent {
        self.finish_at(revision, outcome, Instant::now())
    }

    fn mark_processing_emitted_at(&mut self, finished_at: Instant) {
        self.stop_to_processing_ms = elapsed_ms(self.stop_requested_at, finished_at);
    }

    fn mark_capture_finalize_at(&mut self, started_at: Instant, finished_at: Instant) {
        self.capture_finalize_ms = Some(elapsed_ms(started_at, finished_at));
    }

    fn mark_transcription_at(&mut self, started_at: Instant, finished_at: Instant) {
        self.transcription_ms = Some(elapsed_ms(started_at, finished_at));
    }

    fn mark_delivery_at(&mut self, started_at: Instant, finished_at: Instant) {
        self.delivery_ms = Some(elapsed_ms(started_at, finished_at));
        self.stop_to_delivery_ms = Some(elapsed_ms(self.stop_requested_at, finished_at));
    }

    fn finish_at(
        self,
        revision: u64,
        outcome: DictationLatencyOutcome,
        finished_at: Instant,
    ) -> DictationLatencyEvent {
        DictationLatencyEvent {
            session_id: self.session_id,
            revision,
            outcome,
            audio_duration_ms: self.audio_duration_ms,
            stop_to_processing_ms: self.stop_to_processing_ms,
            capture_finalize_ms: self.capture_finalize_ms,
            transcription_ms: self.transcription_ms,
            delivery_ms: self.delivery_ms,
            stop_to_delivery_ms: self.stop_to_delivery_ms,
            processing_total_ms: elapsed_ms(self.stop_requested_at, finished_at),
        }
    }
}

fn elapsed_ms(started_at: Instant, finished_at: Instant) -> u64 {
    duration_ms(finished_at.saturating_duration_since(started_at))
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_trace_reports_exact_relative_stage_timings() {
        let base = Instant::now();
        let mut trace = ProcessingLatencyTrace::start("opaque-session".into(), base);
        trace.mark_processing_emitted_at(base + Duration::from_millis(3));
        trace.mark_capture_finalize_at(
            base + Duration::from_millis(4),
            base + Duration::from_millis(14),
        );
        trace.set_audio_duration(2_400);
        trace.mark_transcription_at(
            base + Duration::from_millis(14),
            base + Duration::from_millis(94),
        );
        trace.mark_delivery_at(
            base + Duration::from_millis(95),
            base + Duration::from_millis(101),
        );

        let event = trace.finish_at(
            7,
            DictationLatencyOutcome::Completed,
            base + Duration::from_millis(104),
        );

        assert_eq!(event.stop_to_processing_ms, 3);
        assert_eq!(event.capture_finalize_ms, Some(10));
        assert_eq!(event.audio_duration_ms, Some(2_400));
        assert_eq!(event.transcription_ms, Some(80));
        assert_eq!(event.delivery_ms, Some(6));
        assert_eq!(event.stop_to_delivery_ms, Some(101));
        assert_eq!(event.processing_total_ms, 104);
        let measured_stage_total = event.stop_to_processing_ms
            + event.capture_finalize_ms.unwrap_or_default()
            + event.transcription_ms.unwrap_or_default()
            + event.delivery_ms.unwrap_or_default();
        assert!(measured_stage_total <= event.processing_total_ms);
    }

    #[test]
    fn failed_trace_keeps_unreached_stages_explicitly_absent() {
        let base = Instant::now();
        let mut trace = ProcessingLatencyTrace::start("opaque-session".into(), base);
        trace.mark_processing_emitted_at(base + Duration::from_millis(2));
        trace.mark_capture_finalize_at(
            base + Duration::from_millis(3),
            base + Duration::from_millis(11),
        );

        let event = trace.finish_at(
            4,
            DictationLatencyOutcome::Failed,
            base + Duration::from_millis(13),
        );

        assert_eq!(event.capture_finalize_ms, Some(8));
        assert_eq!(event.transcription_ms, None);
        assert_eq!(event.delivery_ms, None);
        assert_eq!(event.stop_to_delivery_ms, None);
    }

    #[test]
    fn a_reversed_monotonic_boundary_saturates_to_zero() {
        let base = Instant::now();
        assert_eq!(elapsed_ms(base + Duration::from_millis(8), base), 0);
    }

    #[test]
    fn serialized_event_has_an_exact_privacy_safe_shape() {
        let event = DictationLatencyEvent {
            session_id: "opaque-session".into(),
            revision: 9,
            outcome: DictationLatencyOutcome::Completed,
            audio_duration_ms: Some(1_000),
            stop_to_processing_ms: 2,
            capture_finalize_ms: Some(10),
            transcription_ms: Some(30),
            delivery_ms: Some(6),
            stop_to_delivery_ms: Some(48),
            processing_total_ms: 50,
        };

        let value = serde_json::to_value(&event).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "audioDurationMs",
                "captureFinalizeMs",
                "deliveryMs",
                "outcome",
                "processingTotalMs",
                "revision",
                "sessionId",
                "stopToDeliveryMs",
                "stopToProcessingMs",
                "transcriptionMs",
            ]
        );
        let object = value.as_object().unwrap();
        for forbidden_key in [
            "text",
            "transcript",
            "segments",
            "samples",
            "targetApp",
            "language",
            "engine",
            "provider",
            "path",
            "error",
            "timestamp",
        ] {
            assert!(
                !object.contains_key(forbidden_key),
                "serialized diagnostics contained forbidden key: {forbidden_key}"
            );
        }
    }
}
