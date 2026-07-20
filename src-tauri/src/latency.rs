//! Ephemeral, privacy-safe measurements for diagnosing perceived latency.
//!
//! One trace follows an exact dictation session from startup to its terminal
//! transition. It is never persisted and deliberately excludes dictated
//! content, audio samples, target metadata, configuration identifiers,
//! absolute timestamps, and error strings.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::Serialize;

pub const DICTATION_LATENCY_EVENT: &str = "dictation://latency";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationLatencyOutcome {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationLatencyEvent {
    pub session_id: String,
    pub revision: u64,
    pub outcome: DictationLatencyOutcome,
    /// Time spent in the external text-target capture call. UI-triggered
    /// transcript tests do not capture an external target, so this is absent.
    pub target_capture_ms: Option<u64>,
    /// Cumulative startup milestones measured from `start_session` entry.
    pub start_to_target_capture_return_ms: Option<u64>,
    pub start_to_audio_owner_spawn_ms: Option<u64>,
    pub start_to_starting_emitted_ms: Option<u64>,
    /// Native HUD presentation returned successfully; this does not claim
    /// that WebView content painted a visible frame.
    pub start_to_hud_show_return_ms: Option<u64>,
    /// The native input stream's `play` call returned successfully.
    pub start_to_microphone_ready_ms: Option<u64>,
    pub start_to_listening_emitted_ms: Option<u64>,
    pub audio_duration_ms: Option<u64>,
    pub stop_to_processing_ms: Option<u64>,
    pub capture_finalize_ms: Option<u64>,
    pub transcription_ms: Option<u64>,
    pub delivery_ms: Option<u64>,
    pub stop_to_delivery_ms: Option<u64>,
    pub processing_total_ms: Option<u64>,
}

#[derive(Debug, Default)]
struct StartupLatencyStages {
    session_id: Option<String>,
    target_capture_started_at: Option<Instant>,
    target_capture_ms: Option<u64>,
    start_to_target_capture_return_ms: Option<u64>,
    start_to_audio_owner_spawn_ms: Option<u64>,
    start_to_starting_emitted_ms: Option<u64>,
    start_to_hud_show_return_ms: Option<u64>,
    start_to_microphone_ready_ms: Option<u64>,
    start_to_listening_emitted_ms: Option<u64>,
}

#[derive(Debug)]
struct StartupLatencyInner {
    started_at: Instant,
    stages: Mutex<StartupLatencyStages>,
    finished: AtomicBool,
}

/// Cloneable only so the successful start path and the exact-session state
/// registry can share one trace. Terminal publication consumes it logically
/// with `finished`, even if a stale callback still owns an `Arc` clone.
#[derive(Debug, Clone)]
pub(crate) struct StartupLatencyTrace {
    inner: Arc<StartupLatencyInner>,
}

impl StartupLatencyTrace {
    pub(crate) fn start() -> Self {
        Self::start_at(Instant::now())
    }

    fn start_at(started_at: Instant) -> Self {
        Self {
            inner: Arc::new(StartupLatencyInner {
                started_at,
                stages: Mutex::new(StartupLatencyStages::default()),
                finished: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn bind_session(&self, session_id: &str) -> bool {
        if self.inner.finished.load(Ordering::Acquire) {
            return false;
        }
        let mut stages = self
            .inner
            .stages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.inner.finished.load(Ordering::Acquire) {
            return false;
        }
        match stages.session_id.as_deref() {
            Some(current) => current == session_id,
            None => {
                stages.session_id = Some(session_id.to_owned());
                true
            }
        }
    }

    pub(crate) fn session_id(&self) -> Option<String> {
        self.inner
            .stages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .session_id
            .clone()
    }

    pub(crate) fn mark_target_capture_started(&self) {
        self.mark_target_capture_started_at(Instant::now());
    }

    pub(crate) fn mark_target_capture_returned(&self) {
        self.mark_target_capture_returned_at(Instant::now());
    }

    pub(crate) fn mark_audio_owner_spawned(&self) {
        self.mark_audio_owner_spawned_at(Instant::now());
    }

    pub(crate) fn mark_starting_emitted(&self) {
        self.mark_starting_emitted_at(Instant::now());
    }

    pub(crate) fn mark_hud_show_returned(&self) {
        self.mark_hud_show_returned_at(Instant::now());
    }

    pub(crate) fn mark_microphone_ready_at(&self, ready_at: Instant) {
        self.mark_relative(|stages| &mut stages.start_to_microphone_ready_ms, ready_at);
    }

    pub(crate) fn mark_listening_emitted(&self) {
        self.mark_listening_emitted_at(Instant::now());
    }

    fn mark_target_capture_started_at(&self, started_at: Instant) {
        if self.inner.finished.load(Ordering::Acquire) {
            return;
        }
        let mut stages = self
            .inner
            .stages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.inner.finished.load(Ordering::Acquire) {
            return;
        }
        stages.target_capture_started_at.get_or_insert(started_at);
    }

    fn mark_target_capture_returned_at(&self, returned_at: Instant) {
        if self.inner.finished.load(Ordering::Acquire) {
            return;
        }
        let mut stages = self
            .inner
            .stages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.inner.finished.load(Ordering::Acquire) {
            return;
        }
        if stages.target_capture_ms.is_none() {
            stages.target_capture_ms = stages
                .target_capture_started_at
                .map(|started_at| elapsed_ms(started_at, returned_at));
        }
        stages
            .start_to_target_capture_return_ms
            .get_or_insert_with(|| elapsed_ms(self.inner.started_at, returned_at));
    }

    fn mark_audio_owner_spawned_at(&self, marked_at: Instant) {
        self.mark_relative(
            |stages| &mut stages.start_to_audio_owner_spawn_ms,
            marked_at,
        );
    }

    fn mark_starting_emitted_at(&self, marked_at: Instant) {
        self.mark_relative(|stages| &mut stages.start_to_starting_emitted_ms, marked_at);
    }

    fn mark_hud_show_returned_at(&self, marked_at: Instant) {
        self.mark_relative(|stages| &mut stages.start_to_hud_show_return_ms, marked_at);
    }

    fn mark_listening_emitted_at(&self, marked_at: Instant) {
        self.mark_relative(
            |stages| &mut stages.start_to_listening_emitted_ms,
            marked_at,
        );
    }

    fn mark_relative(
        &self,
        field: impl FnOnce(&mut StartupLatencyStages) -> &mut Option<u64>,
        marked_at: Instant,
    ) {
        if self.inner.finished.load(Ordering::Acquire) {
            return;
        }
        let mut stages = self
            .inner
            .stages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.inner.finished.load(Ordering::Acquire) {
            return;
        }
        field(&mut stages).get_or_insert_with(|| elapsed_ms(self.inner.started_at, marked_at));
    }

    pub(crate) fn finish(
        &self,
        revision: u64,
        outcome: DictationLatencyOutcome,
        processing: Option<ProcessingLatencyMeasurements>,
    ) -> Option<DictationLatencyEvent> {
        let stages = self
            .inner
            .stages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let session_id = stages.session_id.clone()?;
        if processing
            .as_ref()
            .is_some_and(|processing| processing.session_id != session_id)
        {
            return None;
        }
        if self
            .inner
            .finished
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let processing = processing.unwrap_or_default();
        Some(DictationLatencyEvent {
            session_id,
            revision,
            outcome,
            target_capture_ms: stages.target_capture_ms,
            start_to_target_capture_return_ms: stages.start_to_target_capture_return_ms,
            start_to_audio_owner_spawn_ms: stages.start_to_audio_owner_spawn_ms,
            start_to_starting_emitted_ms: stages.start_to_starting_emitted_ms,
            start_to_hud_show_return_ms: stages.start_to_hud_show_return_ms,
            start_to_microphone_ready_ms: stages.start_to_microphone_ready_ms,
            start_to_listening_emitted_ms: stages.start_to_listening_emitted_ms,
            audio_duration_ms: processing.audio_duration_ms,
            stop_to_processing_ms: processing.stop_to_processing_ms,
            capture_finalize_ms: processing.capture_finalize_ms,
            transcription_ms: processing.transcription_ms,
            delivery_ms: processing.delivery_ms,
            stop_to_delivery_ms: processing.stop_to_delivery_ms,
            processing_total_ms: processing.processing_total_ms,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProcessingLatencyMeasurements {
    session_id: String,
    audio_duration_ms: Option<u64>,
    stop_to_processing_ms: Option<u64>,
    capture_finalize_ms: Option<u64>,
    transcription_ms: Option<u64>,
    delivery_ms: Option<u64>,
    stop_to_delivery_ms: Option<u64>,
    processing_total_ms: Option<u64>,
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

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
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

    pub(crate) fn finish(self) -> ProcessingLatencyMeasurements {
        self.finish_at(Instant::now())
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

    fn finish_at(self, finished_at: Instant) -> ProcessingLatencyMeasurements {
        ProcessingLatencyMeasurements {
            session_id: self.session_id,
            audio_duration_ms: self.audio_duration_ms,
            stop_to_processing_ms: Some(self.stop_to_processing_ms),
            capture_finalize_ms: self.capture_finalize_ms,
            transcription_ms: self.transcription_ms,
            delivery_ms: self.delivery_ms,
            stop_to_delivery_ms: self.stop_to_delivery_ms,
            processing_total_ms: Some(elapsed_ms(self.stop_requested_at, finished_at)),
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
    fn completed_trace_reports_exact_startup_and_processing_timings() {
        let base = Instant::now();
        let startup = StartupLatencyTrace::start_at(base);
        assert!(startup.bind_session("opaque-session"));
        startup.mark_target_capture_started_at(base + Duration::from_millis(2));
        startup.mark_target_capture_returned_at(base + Duration::from_millis(12));
        startup.mark_audio_owner_spawned_at(base + Duration::from_millis(18));
        startup.mark_starting_emitted_at(base + Duration::from_millis(19));
        startup.mark_hud_show_returned_at(base + Duration::from_millis(27));
        startup.mark_microphone_ready_at(base + Duration::from_millis(24));
        startup.mark_listening_emitted_at(base + Duration::from_millis(29));

        let stop = base + Duration::from_millis(2_000);
        let mut processing = ProcessingLatencyTrace::start("opaque-session".into(), stop);
        processing.mark_processing_emitted_at(stop + Duration::from_millis(3));
        processing.mark_capture_finalize_at(
            stop + Duration::from_millis(4),
            stop + Duration::from_millis(14),
        );
        processing.set_audio_duration(2_400);
        processing.mark_transcription_at(
            stop + Duration::from_millis(14),
            stop + Duration::from_millis(94),
        );
        processing.mark_delivery_at(
            stop + Duration::from_millis(95),
            stop + Duration::from_millis(101),
        );

        let event = startup
            .finish(
                7,
                DictationLatencyOutcome::Completed,
                Some(processing.finish_at(stop + Duration::from_millis(104))),
            )
            .unwrap();

        assert_eq!(event.target_capture_ms, Some(10));
        assert_eq!(event.start_to_target_capture_return_ms, Some(12));
        assert_eq!(event.start_to_audio_owner_spawn_ms, Some(18));
        assert_eq!(event.start_to_starting_emitted_ms, Some(19));
        assert_eq!(event.start_to_hud_show_return_ms, Some(27));
        assert_eq!(event.start_to_microphone_ready_ms, Some(24));
        assert_eq!(event.start_to_listening_emitted_ms, Some(29));
        assert_eq!(event.stop_to_processing_ms, Some(3));
        assert_eq!(event.capture_finalize_ms, Some(10));
        assert_eq!(event.audio_duration_ms, Some(2_400));
        assert_eq!(event.transcription_ms, Some(80));
        assert_eq!(event.delivery_ms, Some(6));
        assert_eq!(event.stop_to_delivery_ms, Some(101));
        assert_eq!(event.processing_total_ms, Some(104));
    }

    #[test]
    fn cancelled_startup_keeps_unreached_stages_absent_and_finishes_once() {
        let base = Instant::now();
        let startup = StartupLatencyTrace::start_at(base);
        assert!(startup.bind_session("opaque-session"));
        startup.mark_target_capture_started_at(base + Duration::from_millis(2));
        startup.mark_target_capture_returned_at(base + Duration::from_millis(11));
        startup.mark_audio_owner_spawned_at(base + Duration::from_millis(15));
        startup.mark_starting_emitted_at(base + Duration::from_millis(16));

        let event = startup
            .finish(4, DictationLatencyOutcome::Cancelled, None)
            .unwrap();
        assert_eq!(event.target_capture_ms, Some(9));
        assert_eq!(event.start_to_audio_owner_spawn_ms, Some(15));
        assert_eq!(event.start_to_hud_show_return_ms, None);
        assert_eq!(event.start_to_microphone_ready_ms, None);
        assert_eq!(event.start_to_listening_emitted_ms, None);
        assert_eq!(event.stop_to_processing_ms, None);
        assert_eq!(event.processing_total_ms, None);
        assert_eq!(
            startup.finish(5, DictationLatencyOutcome::Failed, None),
            None
        );
    }

    #[test]
    fn mismatched_processing_cannot_consume_or_mutate_the_bound_trace() {
        let base = Instant::now();
        let startup = StartupLatencyTrace::start_at(base);
        assert!(startup.bind_session("session-current"));
        let stale = ProcessingLatencyTrace::start("session-stale".into(), base).finish_at(base);

        assert_eq!(
            startup.finish(3, DictationLatencyOutcome::Failed, Some(stale)),
            None
        );
        startup.mark_starting_emitted_at(base + Duration::from_millis(4));
        let event = startup
            .finish(4, DictationLatencyOutcome::Cancelled, None)
            .unwrap();
        assert_eq!(event.session_id, "session-current");
        assert_eq!(event.start_to_starting_emitted_ms, Some(4));

        startup.mark_hud_show_returned_at(base + Duration::from_millis(9));
        assert_eq!(
            startup
                .inner
                .stages
                .lock()
                .unwrap()
                .start_to_hud_show_return_ms,
            None
        );
    }

    #[test]
    fn a_reversed_monotonic_boundary_saturates_to_zero() {
        let base = Instant::now();
        assert_eq!(elapsed_ms(base + Duration::from_millis(8), base), 0);
    }

    #[test]
    fn serialized_event_has_an_exact_privacy_safe_shape() {
        let startup = StartupLatencyTrace::start();
        assert!(startup.bind_session("opaque-session"));
        let event = startup
            .finish(9, DictationLatencyOutcome::Failed, None)
            .unwrap();

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
                "startToAudioOwnerSpawnMs",
                "startToHudShowReturnMs",
                "startToListeningEmittedMs",
                "startToMicrophoneReadyMs",
                "startToStartingEmittedMs",
                "startToTargetCaptureReturnMs",
                "stopToDeliveryMs",
                "stopToProcessingMs",
                "targetCaptureMs",
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
            "device",
            "model",
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
