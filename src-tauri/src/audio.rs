//! Ephemeral, session-bound microphone capture.
//!
//! CPAL streams stay on their owning thread because they are intentionally
//! `!Send` on some platforms. Starting and stopping only exchange small control
//! messages; device initialization, permission prompts, stream teardown, and
//! PCM finalization never block a shortcut handler. The real-time callback only
//! downmixes and attempts a bounded, non-blocking send.

use std::{
    mem,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SampleFormat, SizedSample, Stream,
};
use serde::Serialize;

pub const AUDIO_LEVEL_EVENT: &str = "dictation://audio-level";
pub const OUTPUT_SAMPLE_RATE: u32 = 16_000;
pub const AUDIO_LEVEL_INTERVAL_MS: u64 = 50;
pub const MAX_CAPTURE_DURATION_MS: u64 = 10 * 60 * 1_000;
/// Upper bound for native device discovery, permission resolution, stream
/// construction, and `Stream::play` before the owning session is recovered.
pub(crate) const MICROPHONE_START_READY_TIMEOUT: Duration = Duration::from_secs(8);

const AUDIO_QUEUE_CAPACITY: usize = 32;
const LEVEL_FLOOR_DB: f32 = -60.0;
const CAPTURE_OWNER_POLL: Duration = Duration::from_millis(5);
const FINALIZE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const DISCARD_RESPONSE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_OUTPUT_SAMPLES: usize =
    OUTPUT_SAMPLE_RATE as usize * (MAX_CAPTURE_DURATION_MS as usize / 1_000);
const DISCONTINUOUS_CAPTURE_ERROR: &str =
    "The microphone buffer fell behind and missed part of this recording. Nothing was transcribed or typed. Please try again.";
const MICROPHONE_START_TIMEOUT_ERROR: &str =
    "The microphone didn’t become ready in time. Check microphone access or choose another input, then try again.";

pub(crate) type LevelSink = Arc<dyn Fn(AudioLevelEvent) + Send + Sync>;
pub(crate) type ReadySink = Arc<dyn Fn(AudioCaptureReady) + Send + Sync>;
pub(crate) type ErrorSink = Arc<dyn Fn(AudioCaptureFailure) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioCaptureReady {
    pub session_id: String,
    /// Private monotonic marker captured immediately after `Stream::play`
    /// returned. It is converted to a relative duration before diagnostics
    /// cross the native boundary.
    pub stream_play_returned_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioCaptureFailure {
    pub session_id: String,
    pub message: String,
    /// Startup watchdog failures may only transition the matching session
    /// while it is still in `Starting`.
    pub requires_starting: bool,
}

/// Lightweight IPC payload for the HUD waveform. PCM never crosses the Tauri
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLevelEvent {
    /// Perceptual dBFS mapping in the inclusive range 0...1.
    pub level: f32,
    /// Linear absolute peak for the most recent chunk, clamped to 0...1.
    pub peak: f32,
    /// Duration processed by the capture worker.
    pub captured_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioCapturePhase {
    Idle,
    Starting,
    Capturing,
    Ready,
}

/// Read-only capture metadata. It intentionally contains no audio samples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCaptureStatus {
    pub phase: AudioCapturePhase,
    pub device_name: Option<String>,
    pub input_sample_rate: Option<u32>,
    pub input_channels: Option<u16>,
    pub output_sample_rate: u32,
    pub sample_count: usize,
    pub captured_ms: u64,
    pub dropped_chunks: u64,
}

/// A privacy-safe input-device choice for Settings. CPAL does not expose a
/// stable cross-platform device identifier, so the stored name is resolved
/// again immediately before capture and an unavailable choice fails clearly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInputDevice {
    pub name: String,
    pub is_default: bool,
}

pub fn list_input_devices() -> Result<Vec<AudioInputDevice>, String> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device_name(&device));
    let devices = host
        .input_devices()
        .map_err(|error| format!("could not list microphones: {error}"))?;
    let mut names = devices
        .filter_map(|device| device_name(&device))
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();

    Ok(names
        .into_iter()
        .map(|name| AudioInputDevice {
            is_default: default_name.as_deref() == Some(name.as_str()),
            name,
        })
        .collect())
}

impl AudioCaptureStatus {
    fn idle() -> Self {
        Self {
            phase: AudioCapturePhase::Idle,
            device_name: None,
            input_sample_rate: None,
            input_channels: None,
            output_sample_rate: OUTPUT_SAMPLE_RATE,
            sample_count: 0,
            captured_ms: 0,
            dropped_chunks: 0,
        }
    }
}

/// Sendable process state. The non-Send CPAL stream lives exclusively inside
/// the capture-owner thread referenced by `ActiveCapture`.
#[derive(Default)]
pub struct AudioCaptureController {
    active: Option<ActiveCapture>,
}

impl AudioCaptureController {
    /// Spawn capture initialization and return immediately. Device discovery,
    /// permission handling, and `Stream::play` happen on the owner thread.
    pub fn start(
        &mut self,
        session_id: String,
        input_device_name: Option<String>,
        level_sink: LevelSink,
        ready_sink: ReadySink,
        error_sink: ErrorSink,
    ) -> Result<AudioCaptureStatus, String> {
        if let Some(active) = &self.active {
            return Err(format!(
                "microphone capture is already assigned to session {}",
                active.session_id
            ));
        }

        let capture = ActiveCapture::spawn(
            session_id,
            input_device_name,
            level_sink,
            ready_sink,
            error_sink,
        )?;
        let status = capture.status();
        self.active = Some(capture);
        Ok(status)
    }

    /// Atomically detach the capture belonging to `session_id`. The caller can
    /// finalize it on a native worker without holding the global audio lock.
    pub(crate) fn take_for_session(
        &mut self,
        session_id: &str,
    ) -> Result<CaptureFinalizer, String> {
        match self.active.as_ref() {
            Some(active) if active.session_id == session_id => {}
            Some(active) => {
                return Err(format!(
                    "microphone capture belongs to session {}, not {session_id}",
                    active.session_id
                ))
            }
            None => return Err(format!("session {session_id} has no microphone capture")),
        }

        let active = self.active.take().expect("active capture was checked");
        active.mark_finalizing();
        Ok(CaptureFinalizer { active })
    }

    /// Detach only an exact session match. Stale error/cancel callbacks can
    /// never remove a newer session's microphone.
    pub(crate) fn take_matching(&mut self, session_id: &str) -> Option<CaptureFinalizer> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.session_id == session_id)
        {
            let active = self.active.take().expect("active capture was checked");
            active.mark_finalizing();
            Some(CaptureFinalizer { active })
        } else {
            None
        }
    }

    pub fn status(&self) -> AudioCaptureStatus {
        self.active
            .as_ref()
            .map_or_else(AudioCaptureStatus::idle, ActiveCapture::status)
    }
}

/// Detached capture ownership used by finalization and cancellation workers.
pub(crate) struct CaptureFinalizer {
    active: ActiveCapture,
}

impl CaptureFinalizer {
    pub(crate) fn session_id(&self) -> &str {
        &self.active.session_id
    }

    pub(crate) fn finalize(self) -> Result<FinalizedCapture, String> {
        let capture = self
            .active
            .request_finalization(true, FINALIZE_RESPONSE_TIMEOUT)?
            .ok_or_else(|| "the microphone worker did not return captured audio".to_string())?;
        require_continuous_capture(capture)
    }

    pub(crate) fn discard(self) -> Result<(), String> {
        self.active
            .request_finalization(false, DISCARD_RESPONSE_TIMEOUT)
            .map(|_| ())
    }
}

struct ActiveCapture {
    session_id: String,
    command_sender: mpsc::Sender<CaptureCommand>,
    owner: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    metadata: Arc<Mutex<Option<CaptureMetadata>>>,
    captured_frames: Arc<AtomicU64>,
    observed_frames: Arc<AtomicU64>,
    dropped_chunks: Arc<AtomicU64>,
}

impl ActiveCapture {
    fn mark_finalizing(&self) -> bool {
        let mut current = self.phase.load(Ordering::Acquire);
        loop {
            match InternalCapturePhase::from_raw(current) {
                InternalCapturePhase::Starting => {
                    match self.phase.compare_exchange(
                        current,
                        InternalCapturePhase::Finalizing as u8,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            // There is no usable capture to retain yet. Wake a
                            // late platform open/play and let discard finish
                            // without waiting for an owner response.
                            self.shutdown.store(true, Ordering::Release);
                            return true;
                        }
                        Err(next) => current = next,
                    }
                }
                InternalCapturePhase::Capturing => {
                    match self.phase.compare_exchange(
                        current,
                        InternalCapturePhase::Finalizing as u8,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => return true,
                        Err(next) => current = next,
                    }
                }
                InternalCapturePhase::Finalizing => return true,
                InternalCapturePhase::Failed | InternalCapturePhase::Stopped => {
                    self.shutdown.store(true, Ordering::Release);
                    return false;
                }
            }
        }
    }

    fn spawn(
        session_id: String,
        input_device_name: Option<String>,
        level_sink: LevelSink,
        ready_sink: ReadySink,
        error_sink: ErrorSink,
    ) -> Result<Self, String> {
        let captured_frames = Arc::new(AtomicU64::new(0));
        let observed_frames = Arc::new(AtomicU64::new(0));
        let dropped_chunks = Arc::new(AtomicU64::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let phase = Arc::new(AtomicU8::new(InternalCapturePhase::Starting as u8));
        let metadata = Arc::new(Mutex::new(None));
        let (data_sender, data_receiver) = mpsc::sync_channel(AUDIO_QUEUE_CAPACITY);
        let (command_sender, command_receiver) = mpsc::channel();
        let (lifecycle_sender, lifecycle_receiver) = mpsc::channel();

        let notifier_session_id = session_id.clone();
        let notifier_shutdown = Arc::clone(&shutdown);
        let notifier_phase = Arc::clone(&phase);
        thread::Builder::new()
            .name("spick-audio-notifier".into())
            .spawn(move || {
                run_capture_notifier(
                    notifier_session_id,
                    lifecycle_receiver,
                    notifier_shutdown,
                    notifier_phase,
                    ready_sink,
                    error_sink,
                );
            })
            .map_err(|error| format!("could not start the microphone notifier: {error}"))?;

        let owner_session_id = session_id.clone();
        let owner_captured_frames = Arc::clone(&captured_frames);
        let owner_observed_frames = Arc::clone(&observed_frames);
        let owner_dropped_chunks = Arc::clone(&dropped_chunks);
        let owner_shutdown = Arc::clone(&shutdown);
        let owner_phase = Arc::clone(&phase);
        let owner_metadata = Arc::clone(&metadata);
        let stream_error_sender = command_sender.clone();
        let owner = thread::Builder::new()
            .name("spick-audio-capture".into())
            .spawn(move || {
                run_capture_owner(
                    owner_session_id,
                    input_device_name,
                    data_sender,
                    data_receiver,
                    command_receiver,
                    stream_error_sender,
                    lifecycle_sender,
                    owner_captured_frames,
                    owner_observed_frames,
                    owner_dropped_chunks,
                    owner_shutdown,
                    owner_phase,
                    owner_metadata,
                    level_sink,
                );
            })
            .map_err(|error| format!("could not start the microphone worker: {error}"))?;

        Ok(Self {
            session_id,
            command_sender,
            owner: Some(owner),
            shutdown,
            phase,
            metadata,
            captured_frames,
            observed_frames,
            dropped_chunks,
        })
    }

    fn status(&self) -> AudioCaptureStatus {
        let metadata = self.metadata.lock().ok().and_then(|value| value.clone());
        let captured_frames = self.captured_frames.load(Ordering::Relaxed);
        let observed_frames = self.observed_frames.load(Ordering::Relaxed);
        let (device_name, input_sample_rate, input_channels, captured_ms, sample_count) = metadata
            .map_or((None, None, None, 0, 0), |metadata| {
                let captured_ms = duration_ms(observed_frames, metadata.input_sample_rate)
                    .min(MAX_CAPTURE_DURATION_MS);
                let sample_count = output_sample_count(captured_frames, metadata.input_sample_rate)
                    .min(MAX_OUTPUT_SAMPLES);
                (
                    Some(metadata.device_name),
                    Some(metadata.input_sample_rate),
                    Some(metadata.input_channels),
                    captured_ms,
                    sample_count,
                )
            });

        AudioCaptureStatus {
            phase: InternalCapturePhase::load(&self.phase).public_phase(),
            device_name,
            input_sample_rate,
            input_channels,
            output_sample_rate: OUTPUT_SAMPLE_RATE,
            sample_count,
            captured_ms,
            dropped_chunks: self.dropped_chunks.load(Ordering::Relaxed),
        }
    }

    fn request_finalization(
        mut self,
        retain_audio: bool,
        timeout: Duration,
    ) -> Result<Option<FinalizedCapture>, String> {
        if !self.mark_finalizing() || (!retain_audio && self.shutdown.load(Ordering::Acquire)) {
            return Ok(None);
        }
        let (response_sender, response_receiver) = mpsc::channel();
        self.command_sender
            .send(CaptureCommand::Finalize {
                retain_audio,
                response_sender,
            })
            .map_err(|_| "the microphone worker is no longer available".to_string())?;

        let result = response_receiver
            .recv_timeout(timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    format!(
                        "microphone worker did not respond within {} ms",
                        timeout.as_millis()
                    )
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    "the microphone worker stopped before finalizing".to_string()
                }
            })??;

        self.shutdown.store(true, Ordering::Relaxed);
        if self.owner.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(owner) = self.owner.take() {
                owner
                    .join()
                    .map_err(|_| "the microphone worker stopped unexpectedly".to_string())?;
            }
        }
        Ok(result)
    }
}

impl Drop for ActiveCapture {
    fn drop(&mut self) {
        // On timeout the owner may still be inside a platform permission call.
        // It is detached rather than joined, and this flag prevents it from
        // starting a stream when that call eventually returns.
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

#[repr(u8)]
#[derive(Clone, Copy)]
enum InternalCapturePhase {
    Starting = 0,
    Capturing = 1,
    Finalizing = 2,
    Failed = 3,
    Stopped = 4,
}

impl InternalCapturePhase {
    fn load(value: &AtomicU8) -> Self {
        Self::from_raw(value.load(Ordering::Acquire))
    }

    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Starting,
            1 => Self::Capturing,
            2 => Self::Finalizing,
            3 => Self::Failed,
            _ => Self::Stopped,
        }
    }

    fn public_phase(self) -> AudioCapturePhase {
        match self {
            Self::Starting => AudioCapturePhase::Starting,
            Self::Capturing | Self::Finalizing => AudioCapturePhase::Capturing,
            Self::Failed | Self::Stopped => AudioCapturePhase::Idle,
        }
    }
}

#[derive(Clone)]
struct CaptureMetadata {
    device_name: String,
    input_sample_rate: u32,
    input_channels: u16,
}

enum CaptureCommand {
    Finalize {
        retain_audio: bool,
        response_sender: mpsc::Sender<Result<Option<FinalizedCapture>, String>>,
    },
    StreamError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AudioCaptureLifecycleEvent {
    Ready(AudioCaptureReady),
    Failed(AudioCaptureFailure),
}

fn run_capture_notifier(
    session_id: String,
    receiver: Receiver<AudioCaptureLifecycleEvent>,
    shutdown: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    ready_sink: ReadySink,
    error_sink: ErrorSink,
) {
    let initial = match receiver.recv_timeout(MICROPHONE_START_READY_TIMEOUT) {
        Ok(event) => Some(event),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if mark_start_timed_out(&phase, &shutdown) {
                error_sink(AudioCaptureFailure {
                    session_id,
                    message: MICROPHONE_START_TIMEOUT_ERROR.into(),
                    requires_starting: true,
                });
                return;
            }
            // Ready may have won its phase CAS immediately before publishing
            // the event. Only that state can still owe the notifier an event;
            // cancellation/finalization and failure are already terminal here.
            if matches!(
                InternalCapturePhase::load(&phase),
                InternalCapturePhase::Capturing
            ) {
                receiver.recv().ok()
            } else {
                None
            }
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            if mark_capture_failed(&phase) {
                shutdown.store(true, Ordering::Release);
                error_sink(AudioCaptureFailure {
                    session_id,
                    message: "The microphone stopped before it became ready. Please try again."
                        .into(),
                    requires_starting: true,
                });
            }
            return;
        }
    };

    if let Some(event) = initial {
        if !dispatch_capture_lifecycle(event, &ready_sink, &error_sink) {
            return;
        }
    } else {
        return;
    }

    while let Ok(event) = receiver.recv() {
        if !dispatch_capture_lifecycle(event, &ready_sink, &error_sink) {
            return;
        }
    }
}

fn dispatch_capture_lifecycle(
    event: AudioCaptureLifecycleEvent,
    ready_sink: &ReadySink,
    error_sink: &ErrorSink,
) -> bool {
    match event {
        AudioCaptureLifecycleEvent::Ready(ready) => {
            ready_sink(ready);
            true
        }
        AudioCaptureLifecycleEvent::Failed(failure) => {
            error_sink(failure);
            false
        }
    }
}

fn mark_start_timed_out(phase: &AtomicU8, shutdown: &AtomicBool) -> bool {
    let won = phase
        .compare_exchange(
            InternalCapturePhase::Starting as u8,
            InternalCapturePhase::Failed as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok();
    if won {
        shutdown.store(true, Ordering::Release);
    }
    won
}

fn mark_capture_ready(phase: &AtomicU8) -> bool {
    phase
        .compare_exchange(
            InternalCapturePhase::Starting as u8,
            InternalCapturePhase::Capturing as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

fn mark_capture_failed(phase: &AtomicU8) -> bool {
    let mut current = phase.load(Ordering::Acquire);
    loop {
        if current != InternalCapturePhase::Starting as u8
            && current != InternalCapturePhase::Capturing as u8
        {
            return false;
        }
        match phase.compare_exchange(
            current,
            InternalCapturePhase::Failed as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(next) => current = next,
        }
    }
}

fn mark_capture_stopped_if_active(phase: &AtomicU8) {
    let mut current = phase.load(Ordering::Acquire);
    loop {
        if current != InternalCapturePhase::Starting as u8
            && current != InternalCapturePhase::Capturing as u8
        {
            return;
        }
        match phase.compare_exchange(
            current,
            InternalCapturePhase::Stopped as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

/// A short-lived handoff to the transcription pipeline. PCM is borrowed by the
/// decoder, overwritten on a best-effort basis, and released by `Drop` on
/// success, error, cancellation, or unwinding.
pub(crate) struct FinalizedCapture {
    samples_16khz: Vec<f32>,
    device_name: String,
    input_sample_rate: u32,
    input_channels: u16,
    captured_ms: u64,
    dropped_chunks: u64,
}

impl Drop for FinalizedCapture {
    fn drop(&mut self) {
        wipe_pcm(&mut self.samples_16khz);
    }
}

impl FinalizedCapture {
    pub(crate) fn status(&self) -> AudioCaptureStatus {
        AudioCaptureStatus {
            phase: AudioCapturePhase::Ready,
            device_name: Some(self.device_name.clone()),
            input_sample_rate: Some(self.input_sample_rate),
            input_channels: Some(self.input_channels),
            output_sample_rate: OUTPUT_SAMPLE_RATE,
            sample_count: self.samples_16khz.len(),
            captured_ms: self.captured_ms,
            dropped_chunks: self.dropped_chunks,
        }
    }

    pub(crate) fn pcm_16khz(&self) -> &[f32] {
        &self.samples_16khz
    }
}

fn require_continuous_capture(capture: FinalizedCapture) -> Result<FinalizedCapture, String> {
    if capture.dropped_chunks == 0 {
        Ok(capture)
    } else {
        Err(DISCONTINUOUS_CAPTURE_ERROR.into())
    }
}

#[allow(clippy::too_many_arguments)]
fn run_capture_owner(
    session_id: String,
    input_device_name: Option<String>,
    data_sender: SyncSender<Vec<f32>>,
    data_receiver: Receiver<Vec<f32>>,
    command_receiver: Receiver<CaptureCommand>,
    stream_error_sender: mpsc::Sender<CaptureCommand>,
    lifecycle_sender: mpsc::Sender<AudioCaptureLifecycleEvent>,
    captured_frames: Arc<AtomicU64>,
    observed_frames: Arc<AtomicU64>,
    dropped_chunks: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    shared_metadata: Arc<Mutex<Option<CaptureMetadata>>>,
    level_sink: LevelSink,
) {
    let setup = open_microphone_stream(
        input_device_name.as_deref(),
        data_sender,
        Arc::clone(&captured_frames),
        Arc::clone(&observed_frames),
        Arc::clone(&dropped_chunks),
        stream_error_sender,
    );
    let (stream, metadata) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            notify_failure(&session_id, error, &phase, &lifecycle_sender);
            return;
        }
    };

    match play_stream_if_running(&shutdown, || {
        stream
            .play()
            .map_err(|error| format!("could not start the microphone: {error}"))
    }) {
        Ok(true) => {}
        Ok(false) => {
            mark_capture_stopped_if_active(&phase);
            return;
        }
        Err(error) => {
            notify_failure(&session_id, error, &phase, &lifecycle_sender);
            return;
        }
    }
    let stream_play_returned_at = Instant::now();

    if let Ok(mut shared) = shared_metadata.lock() {
        *shared = Some(metadata.clone());
    }
    if !notify_ready(
        &session_id,
        stream_play_returned_at,
        &phase,
        &lifecycle_sender,
    ) {
        drop(stream);
        return;
    }

    let mut stream = Some(stream);
    let mut samples_16khz = SensitivePcm::default();
    let mut resampler =
        StreamingLinearResampler::new(metadata.input_sample_rate, OUTPUT_SAMPLE_RATE);
    let mut native_frames = 0_u64;
    let maximum_native_frames =
        u64::from(metadata.input_sample_rate).saturating_mul(MAX_CAPTURE_DURATION_MS / 1_000);
    let mut last_level_emit = Instant::now();

    loop {
        if shutdown.load(Ordering::Relaxed) {
            mark_capture_stopped_if_active(&phase);
            return;
        }

        match command_receiver.try_recv() {
            Ok(CaptureCommand::Finalize {
                retain_audio,
                response_sender,
            }) => {
                drop(stream.take());
                let mut finalization_error = None;
                while let Ok(chunk) = data_receiver.try_recv() {
                    if let Err(error) = append_bounded_chunk(
                        &mut resampler,
                        &mut samples_16khz.0,
                        &mut native_frames,
                        maximum_native_frames,
                        &chunk,
                    ) {
                        finalization_error = Some(error);
                        break;
                    }
                }

                let result = if let Some(error) = finalization_error {
                    Err(error)
                } else if retain_audio {
                    resampler.finish(&mut samples_16khz.0);
                    debug_assert!(samples_16khz.0.len() <= MAX_OUTPUT_SAMPLES);
                    Ok(Some(FinalizedCapture {
                        samples_16khz: mem::take(&mut samples_16khz.0),
                        device_name: metadata.device_name,
                        input_sample_rate: metadata.input_sample_rate,
                        input_channels: metadata.input_channels,
                        captured_ms: duration_ms(
                            observed_frames.load(Ordering::Relaxed),
                            metadata.input_sample_rate,
                        )
                        .min(MAX_CAPTURE_DURATION_MS),
                        dropped_chunks: dropped_chunks.load(Ordering::Relaxed),
                    }))
                } else {
                    Ok(None)
                };

                phase.store(InternalCapturePhase::Stopped as u8, Ordering::Relaxed);
                let _ = response_sender.send(result);
                return;
            }
            Ok(CaptureCommand::StreamError(error)) => {
                drop(stream.take());
                notify_failure(&session_id, error, &phase, &lifecycle_sender);
                return;
            }
            Err(TryRecvError::Disconnected) => {
                mark_capture_stopped_if_active(&phase);
                return;
            }
            Err(TryRecvError::Empty) => {}
        }

        match data_receiver.recv_timeout(CAPTURE_OWNER_POLL) {
            Ok(chunk) => {
                if let Err(error) = append_bounded_chunk(
                    &mut resampler,
                    &mut samples_16khz.0,
                    &mut native_frames,
                    maximum_native_frames,
                    &chunk,
                ) {
                    drop(stream.take());
                    notify_failure(&session_id, error, &phase, &lifecycle_sender);
                    return;
                }

                if last_level_emit.elapsed() >= Duration::from_millis(AUDIO_LEVEL_INTERVAL_MS) {
                    let (level, peak) = measure_level(&chunk);
                    level_sink(AudioLevelEvent {
                        level,
                        peak,
                        captured_ms: duration_ms(native_frames, metadata.input_sample_rate),
                    });
                    last_level_emit = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                mark_capture_stopped_if_active(&phase);
                return;
            }
        }
    }
}

fn play_stream_if_running(
    shutdown: &AtomicBool,
    play: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    if shutdown.load(Ordering::Relaxed) {
        return Ok(false);
    }
    play()?;
    Ok(!shutdown.load(Ordering::Relaxed))
}

#[derive(Default)]
struct SensitivePcm(Vec<f32>);

impl Drop for SensitivePcm {
    fn drop(&mut self) {
        wipe_pcm(&mut self.0);
    }
}

fn wipe_pcm(samples: &mut Vec<f32>) {
    samples.fill(0.0);
    samples.clear();
}

fn append_bounded_chunk(
    resampler: &mut StreamingLinearResampler,
    output: &mut Vec<f32>,
    native_frames: &mut u64,
    maximum_native_frames: u64,
    chunk: &[f32],
) -> Result<(), String> {
    let chunk_frames = chunk.len() as u64;
    if chunk_frames > maximum_native_frames.saturating_sub(*native_frames) {
        return Err(format!(
            "microphone capture exceeded the {} minute limit",
            MAX_CAPTURE_DURATION_MS / 60_000
        ));
    }

    resampler.push(chunk, output);
    *native_frames = native_frames.saturating_add(chunk_frames);
    if output.len() > MAX_OUTPUT_SAMPLES {
        return Err("microphone capture exceeded its in-memory PCM limit".into());
    }
    Ok(())
}

fn notify_failure(
    session_id: &str,
    message: String,
    phase: &AtomicU8,
    lifecycle_sender: &mpsc::Sender<AudioCaptureLifecycleEvent>,
) {
    if !mark_capture_failed(phase) {
        return;
    }
    let _ = lifecycle_sender.send(AudioCaptureLifecycleEvent::Failed(AudioCaptureFailure {
        session_id: session_id.to_string(),
        message,
        requires_starting: false,
    }));
}

fn notify_ready(
    session_id: &str,
    stream_play_returned_at: Instant,
    phase: &AtomicU8,
    lifecycle_sender: &mpsc::Sender<AudioCaptureLifecycleEvent>,
) -> bool {
    if !mark_capture_ready(phase) {
        return false;
    }
    let _ = lifecycle_sender.send(AudioCaptureLifecycleEvent::Ready(AudioCaptureReady {
        session_id: session_id.to_string(),
        stream_play_returned_at,
    }));
    true
}

fn open_microphone_stream(
    input_device_name: Option<&str>,
    sender: SyncSender<Vec<f32>>,
    captured_frames: Arc<AtomicU64>,
    observed_frames: Arc<AtomicU64>,
    dropped_chunks: Arc<AtomicU64>,
    stream_error_sender: mpsc::Sender<CaptureCommand>,
) -> Result<(Stream, CaptureMetadata), String> {
    let host = cpal::default_host();
    let device = match input_device_name {
        Some(expected) => host
            .input_devices()
            .map_err(|error| format!("could not list microphones: {error}"))?
            .find(|device| device_name(device).as_deref() == Some(expected))
            .ok_or_else(|| {
                format!(
                    "the selected microphone ‘{expected}’ is unavailable; choose another microphone in Settings"
                )
            })?,
        None => host
            .default_input_device()
            .ok_or_else(|| "no default microphone is available".to_string())?,
    };
    let device_name = device_name(&device).unwrap_or_else(|| "System default microphone".into());
    let supported_config = device
        .default_input_config()
        .map_err(|error| format!("could not read the microphone configuration: {error}"))?;
    let sample_format = supported_config.sample_format();
    let stream_config: cpal::StreamConfig = supported_config.into();
    let metadata = CaptureMetadata {
        device_name,
        input_sample_rate: stream_config.sample_rate,
        input_channels: stream_config.channels,
    };

    if metadata.input_sample_rate == 0 || metadata.input_channels == 0 {
        return Err("the selected microphone reported an invalid configuration".into());
    }

    let stream = build_input_stream(
        &device,
        stream_config,
        sample_format,
        sender,
        captured_frames,
        observed_frames,
        dropped_chunks,
        stream_error_sender,
    )?;
    Ok((stream, metadata))
}

fn device_name(device: &cpal::Device) -> Option<String> {
    let name = device.description().ok()?.name().trim().to_string();
    if name.is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
        None
    } else {
        Some(name)
    }
}

#[allow(clippy::too_many_arguments)]
fn build_input_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sample_format: SampleFormat,
    sender: SyncSender<Vec<f32>>,
    captured_frames: Arc<AtomicU64>,
    observed_frames: Arc<AtomicU64>,
    dropped_chunks: Arc<AtomicU64>,
    stream_error_sender: mpsc::Sender<CaptureCommand>,
) -> Result<Stream, String> {
    macro_rules! build {
        ($sample:ty) => {
            build_typed_input_stream::<$sample>(
                device,
                config,
                sender,
                captured_frames,
                observed_frames,
                dropped_chunks,
                stream_error_sender,
            )
        };
    }

    match sample_format {
        SampleFormat::I8 => build!(i8),
        SampleFormat::I16 => build!(i16),
        SampleFormat::I32 => build!(i32),
        SampleFormat::I64 => build!(i64),
        SampleFormat::U8 => build!(u8),
        SampleFormat::U16 => build!(u16),
        SampleFormat::U32 => build!(u32),
        SampleFormat::U64 => build!(u64),
        SampleFormat::F32 => build!(f32),
        SampleFormat::F64 => build!(f64),
        other => Err(format!("unsupported microphone sample format: {other}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_typed_input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sender: SyncSender<Vec<f32>>,
    captured_frames: Arc<AtomicU64>,
    observed_frames: Arc<AtomicU64>,
    dropped_chunks: Arc<AtomicU64>,
    stream_error_sender: mpsc::Sender<CaptureCommand>,
) -> Result<Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let error_callback = move |error: cpal::Error| {
        let _ = stream_error_sender.send(CaptureCommand::StreamError(format!(
            "microphone stream failed: {error}"
        )));
    };

    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mono = downmix_to_mono(data, channels);
                if mono.is_empty() {
                    return;
                }
                let frame_count = mono.len() as u64;
                observed_frames.fetch_add(frame_count, Ordering::Relaxed);
                match sender.try_send(mono) {
                    Ok(()) => {
                        captured_frames.fetch_add(frame_count, Ordering::Relaxed);
                    }
                    Err(TrySendError::Full(_)) => {
                        dropped_chunks.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(TrySendError::Disconnected(_)) => {}
                }
            },
            error_callback,
            None,
        )
        .map_err(|error| format!("could not open the microphone: {error}"))
}

fn downmix_to_mono<T>(input: &[T], channels: usize) -> Vec<f32>
where
    T: Sample,
    f32: FromSample<T>,
{
    if channels == 0 {
        return Vec::new();
    }

    input
        .chunks_exact(channels)
        .map(|frame| {
            let sum = frame
                .iter()
                .map(|sample| f32::from_sample(*sample))
                .sum::<f32>();
            let sample = sum / channels as f32;
            if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        })
        .collect()
}

fn measure_level(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }

    let mut square_sum = 0.0_f32;
    let mut peak = 0.0_f32;
    for &sample in samples {
        let sample = if sample.is_finite() { sample } else { 0.0 };
        square_sum += sample * sample;
        peak = peak.max(sample.abs());
    }

    let rms = (square_sum / samples.len() as f32).sqrt();
    let level = if rms <= f32::EPSILON {
        0.0
    } else {
        let db = 20.0 * rms.log10();
        ((db - LEVEL_FLOOR_DB) / -LEVEL_FLOOR_DB).clamp(0.0, 1.0)
    };
    (level, peak.clamp(0.0, 1.0))
}

/// Chunk-invariant linear resampling. Keeping only the previous input sample
/// lets the owner normalize to 16 kHz as audio arrives, capping ten minutes of
/// PCM at 9.6 million mono `f32` samples instead of retaining native-rate audio.
struct StreamingLinearResampler {
    input_rate: u32,
    output_rate: u32,
    next_output_position: f64,
    input_frames: u64,
    previous: Option<(u64, f32)>,
}

impl StreamingLinearResampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            input_rate,
            output_rate,
            next_output_position: 0.0,
            input_frames: 0,
            previous: None,
        }
    }

    fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
        if self.input_rate == 0 || self.output_rate == 0 {
            return;
        }
        let step = f64::from(self.input_rate) / f64::from(self.output_rate);

        for &sample in input {
            let index = self.input_frames;
            if let Some((previous_index, previous_sample)) = self.previous {
                while self.next_output_position <= index as f64 {
                    let fraction = (self.next_output_position - previous_index as f64) as f32;
                    output.push(previous_sample + (sample - previous_sample) * fraction);
                    self.next_output_position += step;
                }
            } else if self.next_output_position == 0.0 {
                output.push(sample);
                self.next_output_position += step;
            }

            self.previous = Some((index, sample));
            self.input_frames = self.input_frames.saturating_add(1);
        }
    }

    fn finish(&self, output: &mut Vec<f32>) {
        let target =
            output_sample_count_rounded(self.input_frames, self.input_rate, self.output_rate);
        output.truncate(target);
        if let Some((_, last_sample)) = self.previous {
            output.resize(target, last_sample);
        }
    }
}

#[cfg(test)]
fn resample_mono(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    let mut resampler = StreamingLinearResampler::new(input_rate, output_rate);
    let mut output = Vec::new();
    resampler.push(input, &mut output);
    resampler.finish(&mut output);
    output
}

fn duration_ms(sample_count: u64, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    sample_count.saturating_mul(1_000) / u64::from(sample_rate)
}

fn output_sample_count(input_frames: u64, input_sample_rate: u32) -> usize {
    output_sample_count_rounded(input_frames, input_sample_rate, OUTPUT_SAMPLE_RATE)
}

fn output_sample_count_rounded(input_frames: u64, input_rate: u32, output_rate: u32) -> usize {
    if input_rate == 0 || output_rate == 0 {
        return 0;
    }
    let count = (u128::from(input_frames) * u128::from(output_rate) + u128::from(input_rate) / 2)
        / u128::from(input_rate);
    usize::try_from(count).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inactive_capture(session_id: &str) -> (ActiveCapture, Arc<AtomicBool>) {
        let (command_sender, _command_receiver) = mpsc::channel::<CaptureCommand>();
        let shutdown = Arc::new(AtomicBool::new(false));
        (
            ActiveCapture {
                session_id: session_id.into(),
                command_sender,
                owner: None,
                shutdown: Arc::clone(&shutdown),
                phase: Arc::new(AtomicU8::new(InternalCapturePhase::Starting as u8)),
                metadata: Arc::new(Mutex::new(None)),
                captured_frames: Arc::new(AtomicU64::new(0)),
                observed_frames: Arc::new(AtomicU64::new(0)),
                dropped_chunks: Arc::new(AtomicU64::new(0)),
            },
            shutdown,
        )
    }

    #[test]
    fn timeout_bound_is_explicit_and_human_scale() {
        assert_eq!(MICROPHONE_START_READY_TIMEOUT, Duration::from_secs(8));
    }

    #[test]
    fn ready_wins_the_startup_race_exactly_once() {
        let phase = AtomicU8::new(InternalCapturePhase::Starting as u8);
        let shutdown = AtomicBool::new(false);

        assert!(mark_capture_ready(&phase));
        assert!(!mark_start_timed_out(&phase, &shutdown));
        assert!(!mark_capture_ready(&phase));
        assert!(!shutdown.load(Ordering::Acquire));
        assert!(matches!(
            InternalCapturePhase::load(&phase),
            InternalCapturePhase::Capturing
        ));
    }

    #[test]
    fn timeout_wins_the_startup_race_and_suppresses_late_callbacks() {
        let phase = AtomicU8::new(InternalCapturePhase::Starting as u8);
        let shutdown = AtomicBool::new(false);
        let (sender, receiver) = mpsc::channel();

        assert!(mark_start_timed_out(&phase, &shutdown));
        assert!(shutdown.load(Ordering::Acquire));
        mark_capture_stopped_if_active(&phase);
        assert!(!notify_ready("session-a", Instant::now(), &phase, &sender));
        notify_failure("session-a", "late failure".into(), &phase, &sender);
        assert!(receiver.try_recv().is_err());
        assert!(matches!(
            InternalCapturePhase::load(&phase),
            InternalCapturePhase::Failed
        ));
    }

    #[test]
    fn finalization_beats_both_ready_and_timeout() {
        let (capture, shutdown) = inactive_capture("session-a");
        capture.mark_finalizing();
        let (sender, receiver) = mpsc::channel();

        assert!(!mark_start_timed_out(&capture.phase, &shutdown));
        mark_capture_stopped_if_active(&capture.phase);
        assert!(!notify_ready(
            "session-a",
            Instant::now(),
            &capture.phase,
            &sender,
        ));
        notify_failure("session-a", "late failure".into(), &capture.phase, &sender);
        assert!(receiver.try_recv().is_err());
        assert!(matches!(
            InternalCapturePhase::load(&capture.phase),
            InternalCapturePhase::Finalizing
        ));
    }

    #[test]
    fn matching_detach_never_takes_another_sessions_audio_owner() {
        let (capture, shutdown) = inactive_capture("session-a");
        let mut controller = AudioCaptureController {
            active: Some(capture),
        };

        assert!(controller.take_matching("session-b").is_none());
        assert!(!shutdown.load(Ordering::Relaxed));

        let finalizer = controller.take_matching("session-a").unwrap();
        assert_eq!(finalizer.session_id(), "session-a");
        assert!(controller.active.is_none());
        drop(finalizer);
        assert!(shutdown.load(Ordering::Relaxed));
    }

    #[test]
    fn downmixes_interleaved_stereo_and_ignores_incomplete_frames() {
        let input = [1.0_f32, 0.0, -1.0, -0.5, 0.8];
        assert_eq!(downmix_to_mono(&input, 2), vec![0.5, -0.75]);
    }

    #[test]
    fn integer_samples_are_normalized_during_downmix() {
        let input = [i16::MAX, i16::MAX, i16::MIN, i16::MIN];
        let mono = downmix_to_mono(&input, 2);
        assert!((mono[0] - 1.0).abs() < 0.0001);
        assert!((mono[1] + 1.0).abs() < 0.0001);
    }

    #[test]
    fn level_is_bounded_and_silence_is_zero() {
        assert_eq!(measure_level(&[0.0; 32]), (0.0, 0.0));

        let (level, peak) = measure_level(&[0.1; 32]);
        assert!((level - (2.0 / 3.0)).abs() < 0.001);
        assert!((peak - 0.1).abs() < f32::EPSILON);

        assert_eq!(measure_level(&[2.0, -2.0]), (1.0, 1.0));
    }

    #[test]
    fn streaming_resampling_is_chunk_invariant_and_preserves_duration() {
        let input = vec![0.25; 48_000];
        let one_shot = resample_mono(&input, 48_000, 16_000);

        let mut streaming = StreamingLinearResampler::new(48_000, 16_000);
        let mut chunked = Vec::new();
        for chunk in input.chunks(511) {
            streaming.push(chunk, &mut chunked);
        }
        streaming.finish(&mut chunked);

        assert_eq!(one_shot.len(), 16_000);
        assert!(one_shot.iter().all(|sample| *sample == 0.25));
        assert_eq!(chunked, one_shot);
    }

    #[test]
    fn resampling_at_the_target_rate_is_exact() {
        let input = vec![-1.0, -0.5, 0.0, 0.5, 1.0];
        assert_eq!(resample_mono(&input, 16_000, 16_000), input);
    }

    #[test]
    fn capture_limit_bounds_duration_and_normalized_pcm_memory() {
        assert_eq!(MAX_CAPTURE_DURATION_MS, 600_000);
        assert_eq!(MAX_OUTPUT_SAMPLES, 9_600_000);

        let mut resampler = StreamingLinearResampler::new(4, 2);
        let mut output = Vec::new();
        let mut frames = 0;
        append_bounded_chunk(&mut resampler, &mut output, &mut frames, 4, &[0.0; 4]).unwrap();
        let error =
            append_bounded_chunk(&mut resampler, &mut output, &mut frames, 4, &[0.0]).unwrap_err();
        assert!(error.contains("10 minute limit"));
    }

    #[test]
    fn finalized_pcm_is_borrowed_by_the_transcription_pipeline() {
        let capture = FinalizedCapture {
            samples_16khz: vec![0.1, 0.2],
            device_name: "Test".into(),
            input_sample_rate: 48_000,
            input_channels: 2,
            captured_ms: 1,
            dropped_chunks: 0,
        };

        assert_eq!(capture.pcm_16khz(), &[0.1, 0.2]);
        assert_eq!(capture.status().captured_ms, 1);
        assert_eq!(capture.samples_16khz, vec![0.1, 0.2]);
    }

    #[test]
    fn discontinuous_capture_is_rejected_before_transcription() {
        let capture = FinalizedCapture {
            samples_16khz: vec![0.1, 0.2],
            device_name: "Test".into(),
            input_sample_rate: 48_000,
            input_channels: 1,
            captured_ms: 1,
            dropped_chunks: 2,
        };

        assert_eq!(
            require_continuous_capture(capture).err().unwrap(),
            DISCONTINUOUS_CAPTURE_ERROR
        );
    }

    #[test]
    fn complete_capture_remains_available_for_transcription() {
        let capture = FinalizedCapture {
            samples_16khz: vec![0.1, 0.2],
            device_name: "Test".into(),
            input_sample_rate: 48_000,
            input_channels: 1,
            captured_ms: 1,
            dropped_chunks: 0,
        };

        let capture = require_continuous_capture(capture).unwrap();
        assert_eq!(capture.pcm_16khz(), &[0.1, 0.2]);
    }

    #[test]
    fn stream_play_failure_and_post_play_shutdown_never_become_ready() {
        let shutdown = AtomicBool::new(false);
        assert_eq!(
            play_stream_if_running(&shutdown, || Err("permission denied".into())),
            Err("permission denied".into())
        );

        assert_eq!(
            play_stream_if_running(&shutdown, || {
                shutdown.store(true, Ordering::Relaxed);
                Ok(())
            }),
            Ok(false)
        );

        let called = AtomicBool::new(false);
        assert_eq!(
            play_stream_if_running(&shutdown, || {
                called.store(true, Ordering::Relaxed);
                Ok(())
            }),
            Ok(false)
        );
        assert!(!called.load(Ordering::Relaxed));
    }

    #[test]
    fn failure_keeps_its_originating_session_id() {
        let (sender, receiver) = mpsc::channel();
        let phase = AtomicU8::new(InternalCapturePhase::Capturing as u8);
        notify_failure("session-a", "device disappeared".into(), &phase, &sender);

        assert_eq!(
            receiver.recv().unwrap(),
            AudioCaptureLifecycleEvent::Failed(AudioCaptureFailure {
                session_id: "session-a".into(),
                message: "device disappeared".into(),
                requires_starting: false,
            })
        );
        assert!(matches!(
            InternalCapturePhase::load(&phase),
            InternalCapturePhase::Failed
        ));
    }

    #[test]
    fn readiness_is_session_bound_and_marks_capture_before_notification() {
        let (sender, receiver) = mpsc::channel();
        let phase = AtomicU8::new(InternalCapturePhase::Starting as u8);
        let stream_play_returned_at = Instant::now();

        assert!(notify_ready(
            "session-a",
            stream_play_returned_at,
            &phase,
            &sender,
        ));

        assert!(matches!(
            InternalCapturePhase::load(&phase),
            InternalCapturePhase::Capturing
        ));
        assert_eq!(
            receiver.recv().unwrap(),
            AudioCaptureLifecycleEvent::Ready(AudioCaptureReady {
                session_id: "session-a".into(),
                stream_play_returned_at,
            })
        );
    }

    #[test]
    fn level_event_serializes_only_animation_metadata() {
        let value = serde_json::to_value(AudioLevelEvent {
            level: 0.5,
            peak: 0.75,
            captured_ms: 125,
        })
        .unwrap();
        let object = value.as_object().unwrap();

        assert_eq!(object.len(), 3);
        assert_eq!(object["capturedMs"], 125);
        assert!(object.contains_key("level"));
        assert!(object.contains_key("peak"));
        assert!(!object.contains_key("samples"));
    }

    #[test]
    fn capture_status_never_serializes_audio_samples() {
        let value = serde_json::to_value(AudioCaptureStatus::idle()).unwrap();
        let object = value.as_object().unwrap();

        assert_eq!(object["phase"], "idle");
        assert_eq!(object["outputSampleRate"], OUTPUT_SAMPLE_RATE);
        assert!(!object.contains_key("samples"));
        assert!(!object.contains_key("samples16khz"));
    }
}
