use std::{
    os::raw::c_int,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField, KeyCode,
};

use super::{
    gesture::{GestureEvent, GestureInput},
    ChordQueueFlags, InputMonitoringAccess,
};
use crate::hud;

const LISTENER_START_TIMEOUT: Duration = Duration::from_secs(2);
const RUN_LOOP_POLL: Duration = Duration::from_millis(100);
const REBUILD_BACKOFF: Duration = Duration::from_millis(100);
const REBUILD_MAX_BACKOFF: Duration = Duration::from_secs(5);
const LISTENER_STOPPED: u8 = 0;
const LISTENER_ACTIVE: u8 = 1;
const LISTENER_RECOVERING: u8 = 2;
const KEY_WORDS: usize = 4;
const IO_HID_REQUEST_TYPE_LISTEN_EVENT: c_int = 1;
const IO_HID_ACCESS_TYPE_GRANTED: c_int = 0;
const IO_HID_ACCESS_TYPE_DENIED: c_int = 1;

extern "C" {
    fn IOHIDCheckAccess(request_type: c_int) -> c_int;
    fn IOHIDRequestAccess(request_type: c_int) -> bool;
}

pub struct ListenerHandle {
    stop: Arc<AtomicBool>,
    health: Arc<AtomicU8>,
    worker: Option<JoinHandle<()>>,
}

impl ListenerHandle {
    pub fn is_active(&self) -> bool {
        self.health.load(Ordering::Acquire) == LISTENER_ACTIVE
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.health.store(LISTENER_STOPPED, Ordering::Release);
    }
}

impl Drop for ListenerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn input_monitoring_access() -> InputMonitoringAccess {
    input_monitoring_access_from_raw(unsafe { IOHIDCheckAccess(IO_HID_REQUEST_TYPE_LISTEN_EVENT) })
}

pub fn request_input_monitoring_access() -> InputMonitoringAccess {
    // Unlike the CoreGraphics preflight/request pair, this explicitly enrolls
    // the process for the Input Monitoring privacy service.
    if unsafe { IOHIDRequestAccess(IO_HID_REQUEST_TYPE_LISTEN_EVENT) } {
        InputMonitoringAccess::Granted
    } else {
        input_monitoring_access()
    }
}

fn input_monitoring_access_from_raw(raw: c_int) -> InputMonitoringAccess {
    match raw {
        IO_HID_ACCESS_TYPE_GRANTED => InputMonitoringAccess::Granted,
        IO_HID_ACCESS_TYPE_DENIED => InputMonitoringAccess::Denied,
        _ => InputMonitoringAccess::Unknown,
    }
}

pub fn start_listener(
    sender: SyncSender<GestureEvent>,
    overflowed: Arc<AtomicBool>,
    chord_queue: Arc<ChordQueueFlags>,
) -> Result<ListenerHandle, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let health = Arc::new(AtomicU8::new(LISTENER_RECOVERING));
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let worker_stop = Arc::clone(&stop);
    let worker_health = Arc::clone(&health);
    let worker = thread::Builder::new()
        .name("spick-option-listener".into())
        .spawn(move || {
            run_listener(
                sender,
                overflowed,
                chord_queue,
                worker_stop,
                Arc::clone(&worker_health),
                ready_sender,
            );
            worker_health.store(LISTENER_STOPPED, Ordering::Release);
        })
        .map_err(|error| format!("could not start the Option-key listener: {error}"))?;

    let mut handle = ListenerHandle {
        stop,
        health,
        worker: Some(worker),
    };
    match ready_receiver.recv_timeout(LISTENER_START_TIMEOUT) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(error)) => {
            handle.stop();
            Err(error)
        }
        Err(_) => {
            handle.stop();
            Err("the Option-key listener did not start in time".into())
        }
    }
}

fn run_listener(
    sender: SyncSender<GestureEvent>,
    overflowed: Arc<AtomicBool>,
    chord_queue: Arc<ChordQueueFlags>,
    stop: Arc<AtomicBool>,
    health: Arc<AtomicU8>,
    ready_sender: SyncSender<Result<(), String>>,
) {
    let mut ready_sender = Some(ready_sender);
    let mut rebuild_failures = 0_u32;
    while !stop.load(Ordering::Acquire) {
        let rebuild = Arc::new(AtomicBool::new(false));
        let pressed_inputs = Arc::new(PressedInputs::default());
        let callback_sender = sender.clone();
        let callback_overflowed = Arc::clone(&overflowed);
        let callback_rebuild = Arc::clone(&rebuild);
        let callback_pressed_inputs = Arc::clone(&pressed_inputs);
        let callback_chord_queue = Arc::clone(&chord_queue);
        let callback = move |_proxy, event_type, event: &CGEvent| {
            if matches!(
                event_type,
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
            ) {
                callback_rebuild.store(true, Ordering::Release);
            }
            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
            let pointer_targets_hud = pointer_event_targets_hud(event_type, event);
            callback_pressed_inputs.observe(event_type, keycode, pointer_targets_hud);
            let mut input = classify_pointer_target(
                normalize_fields(event_type, keycode, event.get_flags()),
                pointer_targets_hud,
            );
            if input == Some(GestureInput::OptionDown) {
                input = callback_pressed_inputs.chord_input().or(input);
            }
            if let Some(input) = input {
                if !callback_chord_queue.claim(input) {
                    return CallbackResult::Keep;
                }
                let event = GestureEvent {
                    input,
                    occurred_at: Instant::now(),
                };
                match callback_sender.try_send(event) {
                    Ok(()) => {}
                    Err(TrySendError::Full(event)) => {
                        callback_chord_queue.release(event.input);
                        // Repeated HUD pointer input is deliberately lossy and
                        // coalesced. Every external pointer, keyboard, or Option
                        // transition is fail-closed because losing one could
                        // turn an ordinary chord into an accepted dictation.
                        if lost_input_requires_recovery(event.input) {
                            callback_overflowed.store(true, Ordering::Release);
                        }
                    }
                    Err(TrySendError::Disconnected(event)) => {
                        callback_chord_queue.release(event.input)
                    }
                }
            }
            CallbackResult::Keep
        };

        let event_tap = match CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::TailAppendEventTap,
            CGEventTapOptions::ListenOnly,
            vec![
                CGEventType::FlagsChanged,
                CGEventType::KeyDown,
                CGEventType::KeyUp,
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseUp,
                CGEventType::RightMouseDown,
                CGEventType::RightMouseUp,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseUp,
                CGEventType::ScrollWheel,
            ],
            callback,
        ) {
            Ok(event_tap) => event_tap,
            Err(()) => {
                health.store(LISTENER_RECOVERING, Ordering::Release);
                if let Some(ready) = ready_sender.take() {
                    let _ = ready.send(Err(
                        "macOS could not create the passive Option-key listener".into(),
                    ));
                    return;
                }
                sleep_with_stop(&stop, rebuild_backoff(rebuild_failures));
                rebuild_failures = rebuild_failures.saturating_add(1);
                continue;
            }
        };
        let source = match event_tap.mach_port().create_runloop_source(0) {
            Ok(source) => source,
            Err(()) => {
                health.store(LISTENER_RECOVERING, Ordering::Release);
                if let Some(ready) = ready_sender.take() {
                    let _ = ready.send(Err(
                        "macOS could not attach the passive Option-key listener".into(),
                    ));
                    return;
                }
                sleep_with_stop(&stop, rebuild_backoff(rebuild_failures));
                rebuild_failures = rebuild_failures.saturating_add(1);
                continue;
            }
        };

        let run_loop = CFRunLoop::get_current();
        let default_mode = unsafe { kCFRunLoopDefaultMode };
        run_loop.add_source(&source, default_mode);
        event_tap.enable();
        health.store(LISTENER_ACTIVE, Ordering::Release);
        rebuild_failures = 0;
        if let Some(ready) = ready_sender.take() {
            let _ = ready.send(Ok(()));
        }

        while !stop.load(Ordering::Acquire) && !rebuild.load(Ordering::Acquire) {
            CFRunLoop::run_in_mode(default_mode, RUN_LOOP_POLL, true);
        }

        run_loop.remove_source(&source, default_mode);
        health.store(LISTENER_RECOVERING, Ordering::Release);
        if rebuild.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
            sleep_with_stop(&stop, REBUILD_BACKOFF);
        }
    }
}

fn rebuild_backoff(failures: u32) -> Duration {
    let multiplier = 1_u32 << failures.min(16);
    REBUILD_BACKOFF
        .saturating_mul(multiplier)
        .min(REBUILD_MAX_BACKOFF)
}

fn sleep_with_stop(stop: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !stop.load(Ordering::Acquire) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(remaining.min(RUN_LOOP_POLL));
    }
}

struct PressedInputs {
    keys: [AtomicU64; KEY_WORDS],
    mouse_buttons: AtomicU8,
    hud_mouse_buttons: AtomicU8,
}

impl Default for PressedInputs {
    fn default() -> Self {
        Self {
            keys: std::array::from_fn(|_| AtomicU64::new(0)),
            mouse_buttons: AtomicU8::new(0),
            hud_mouse_buttons: AtomicU8::new(0),
        }
    }
}

impl PressedInputs {
    fn observe(&self, event_type: CGEventType, keycode: i64, pointer_targets_hud: bool) {
        match event_type {
            CGEventType::KeyDown => self.set_key(keycode, true),
            CGEventType::KeyUp => self.set_key(keycode, false),
            CGEventType::LeftMouseDown => self.set_mouse(0, true, pointer_targets_hud),
            CGEventType::LeftMouseUp => self.set_mouse(0, false, false),
            CGEventType::RightMouseDown => self.set_mouse(1, true, pointer_targets_hud),
            CGEventType::RightMouseUp => self.set_mouse(1, false, false),
            CGEventType::OtherMouseDown => self.set_mouse(2, true, pointer_targets_hud),
            CGEventType::OtherMouseUp => self.set_mouse(2, false, false),
            _ => {}
        }
    }

    fn chord_input(&self) -> Option<GestureInput> {
        if self
            .keys
            .iter()
            .any(|word| word.load(Ordering::Relaxed) != 0)
        {
            Some(GestureInput::KeyboardChord)
        } else {
            let mouse_buttons = self.mouse_buttons.load(Ordering::Relaxed);
            let hud_mouse_buttons = self.hud_mouse_buttons.load(Ordering::Relaxed) & mouse_buttons;
            if mouse_buttons & !hud_mouse_buttons != 0 {
                Some(GestureInput::PointerChord)
            } else if hud_mouse_buttons != 0 {
                Some(GestureInput::HudPointerChord)
            } else {
                None
            }
        }
    }

    fn set_key(&self, keycode: i64, down: bool) {
        let Ok(keycode) = usize::try_from(keycode) else {
            return;
        };
        let word = keycode / u64::BITS as usize;
        let Some(slot) = self.keys.get(word) else {
            return;
        };
        let bit = 1_u64 << (keycode % u64::BITS as usize);
        if down {
            slot.fetch_or(bit, Ordering::Relaxed);
        } else {
            slot.fetch_and(!bit, Ordering::Relaxed);
        }
    }

    fn set_mouse(&self, button: u8, down: bool, targets_hud: bool) {
        let bit = 1_u8 << button;
        if down {
            self.mouse_buttons.fetch_or(bit, Ordering::Relaxed);
            if targets_hud {
                self.hud_mouse_buttons.fetch_or(bit, Ordering::Relaxed);
            } else {
                self.hud_mouse_buttons.fetch_and(!bit, Ordering::Relaxed);
            }
        } else {
            self.mouse_buttons.fetch_and(!bit, Ordering::Relaxed);
            self.hud_mouse_buttons.fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

fn pointer_event_targets_hud(event_type: CGEventType, event: &CGEvent) -> bool {
    if !matches!(
        event_type,
        CGEventType::LeftMouseDown
            | CGEventType::RightMouseDown
            | CGEventType::OtherMouseDown
            | CGEventType::ScrollWheel
    ) {
        return false;
    }
    hud::owns_native_window_number(
        event.get_integer_value_field(EventField::MOUSE_EVENT_WINDOW_UNDER_MOUSE_POINTER),
    )
}

fn classify_pointer_target(
    input: Option<GestureInput>,
    pointer_targets_hud: bool,
) -> Option<GestureInput> {
    if input == Some(GestureInput::PointerChord) && pointer_targets_hud {
        Some(GestureInput::HudPointerChord)
    } else {
        input
    }
}

fn lost_input_requires_recovery(input: GestureInput) -> bool {
    input != GestureInput::HudPointerChord
}

fn normalize_fields(
    event_type: CGEventType,
    keycode: i64,
    flags: CGEventFlags,
) -> Option<GestureInput> {
    match event_type {
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
            Some(GestureInput::ListenerDisabled)
        }
        CGEventType::FlagsChanged => {
            if keycode == i64::from(KeyCode::OPTION) || keycode == i64::from(KeyCode::RIGHT_OPTION)
            {
                if !flags.contains(CGEventFlags::CGEventFlagAlternate) {
                    Some(GestureInput::OptionUp)
                } else if has_disallowed_modifier(flags) {
                    Some(GestureInput::KeyboardChord)
                } else {
                    Some(GestureInput::OptionDown)
                }
            } else if flags.contains(CGEventFlags::CGEventFlagAlternate) {
                Some(GestureInput::KeyboardChord)
            } else {
                None
            }
        }
        CGEventType::KeyDown if flags.contains(CGEventFlags::CGEventFlagAlternate) => {
            Some(GestureInput::KeyboardChord)
        }
        CGEventType::LeftMouseDown
        | CGEventType::RightMouseDown
        | CGEventType::OtherMouseDown
        | CGEventType::ScrollWheel
            if flags.contains(CGEventFlags::CGEventFlagAlternate) =>
        {
            Some(GestureInput::PointerChord)
        }
        _ => None,
    }
}

fn has_disallowed_modifier(flags: CGEventFlags) -> bool {
    flags.intersects(
        CGEventFlags::CGEventFlagCommand
            | CGEventFlags::CGEventFlagControl
            | CGEventFlags::CGEventFlagShift
            | CGEventFlags::CGEventFlagSecondaryFn,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iohid_access_values_are_mapped_without_guessing_unknown_values() {
        assert_eq!(
            input_monitoring_access_from_raw(IO_HID_ACCESS_TYPE_GRANTED),
            InputMonitoringAccess::Granted
        );
        assert_eq!(
            input_monitoring_access_from_raw(IO_HID_ACCESS_TYPE_DENIED),
            InputMonitoringAccess::Denied
        );
        assert_eq!(
            input_monitoring_access_from_raw(2),
            InputMonitoringAccess::Unknown
        );
        assert_eq!(
            input_monitoring_access_from_raw(99),
            InputMonitoringAccess::Unknown
        );
    }

    #[test]
    fn event_tap_rebuild_backoff_is_capped() {
        assert_eq!(rebuild_backoff(0), REBUILD_BACKOFF);
        assert_eq!(rebuild_backoff(1), REBUILD_BACKOFF * 2);
        assert_eq!(rebuild_backoff(32), REBUILD_MAX_BACKOFF);
    }

    #[test]
    fn option_flag_changes_map_to_down_and_up() {
        assert_eq!(
            normalize_fields(
                CGEventType::FlagsChanged,
                i64::from(KeyCode::OPTION),
                CGEventFlags::CGEventFlagAlternate,
            ),
            Some(GestureInput::OptionDown)
        );
        assert_eq!(
            normalize_fields(
                CGEventType::FlagsChanged,
                i64::from(KeyCode::RIGHT_OPTION),
                CGEventFlags::empty(),
            ),
            Some(GestureInput::OptionUp)
        );
    }

    #[test]
    fn modifier_and_key_chords_fail_closed() {
        assert_eq!(
            normalize_fields(
                CGEventType::FlagsChanged,
                i64::from(KeyCode::OPTION),
                CGEventFlags::CGEventFlagAlternate | CGEventFlags::CGEventFlagShift,
            ),
            Some(GestureInput::KeyboardChord)
        );
        assert_eq!(
            normalize_fields(CGEventType::KeyDown, 0, CGEventFlags::CGEventFlagAlternate,),
            Some(GestureInput::KeyboardChord)
        );
    }

    #[test]
    fn pointer_and_scroll_input_are_classified_separately() {
        for event_type in [
            CGEventType::LeftMouseDown,
            CGEventType::RightMouseDown,
            CGEventType::OtherMouseDown,
            CGEventType::ScrollWheel,
        ] {
            assert_eq!(
                normalize_fields(event_type, 0, CGEventFlags::CGEventFlagAlternate),
                Some(GestureInput::PointerChord)
            );
        }
    }

    #[test]
    fn only_pointer_chords_targeting_the_hud_receive_the_drag_exemption() {
        assert_eq!(
            classify_pointer_target(Some(GestureInput::PointerChord), true),
            Some(GestureInput::HudPointerChord)
        );
        assert_eq!(
            classify_pointer_target(Some(GestureInput::PointerChord), false),
            Some(GestureInput::PointerChord)
        );
        assert_eq!(
            classify_pointer_target(Some(GestureInput::OptionDown), true),
            Some(GestureInput::OptionDown)
        );
    }

    #[test]
    fn losing_an_external_pointer_chord_fails_closed() {
        assert!(lost_input_requires_recovery(GestureInput::PointerChord));
        assert!(lost_input_requires_recovery(GestureInput::KeyboardChord));
        assert!(!lost_input_requires_recovery(GestureInput::HudPointerChord));
    }

    #[test]
    fn option_release_wins_over_remaining_disallowed_modifiers() {
        for flags in [
            CGEventFlags::CGEventFlagShift,
            CGEventFlags::CGEventFlagCommand,
            CGEventFlags::CGEventFlagControl,
        ] {
            assert_eq!(
                normalize_fields(CGEventType::FlagsChanged, i64::from(KeyCode::OPTION), flags,),
                Some(GestureInput::OptionUp)
            );
        }
    }

    #[test]
    fn disabled_taps_request_recovery() {
        assert_eq!(
            normalize_fields(
                CGEventType::TapDisabledByUserInput,
                0,
                CGEventFlags::empty(),
            ),
            Some(GestureInput::ListenerDisabled)
        );
    }

    #[test]
    fn inputs_held_before_option_are_tracked_until_release() {
        let pressed = PressedInputs::default();
        pressed.observe(CGEventType::KeyDown, 12, false);
        assert_eq!(pressed.chord_input(), Some(GestureInput::KeyboardChord));
        pressed.observe(CGEventType::KeyUp, 12, false);
        assert_eq!(pressed.chord_input(), None);

        pressed.observe(CGEventType::LeftMouseDown, 0, false);
        assert_eq!(pressed.chord_input(), Some(GestureInput::PointerChord));
        pressed.observe(CGEventType::LeftMouseUp, 0, false);
        assert_eq!(pressed.chord_input(), None);
    }

    #[test]
    fn mouse_origin_distinguishes_the_hud_from_external_pointer_chords() {
        let pressed = PressedInputs::default();
        pressed.observe(CGEventType::LeftMouseDown, 0, true);
        assert_eq!(pressed.chord_input(), Some(GestureInput::HudPointerChord));

        // An external second button wins so a HUD drag cannot mask other input.
        pressed.observe(CGEventType::RightMouseDown, 0, false);
        assert_eq!(pressed.chord_input(), Some(GestureInput::PointerChord));
        pressed.observe(CGEventType::RightMouseUp, 0, false);
        assert_eq!(pressed.chord_input(), Some(GestureInput::HudPointerChord));
        pressed.observe(CGEventType::LeftMouseUp, 0, false);
        assert_eq!(pressed.chord_input(), None);
    }
}
