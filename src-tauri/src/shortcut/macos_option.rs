use std::{
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
    ChordQueueFlags,
};

const LISTENER_START_TIMEOUT: Duration = Duration::from_secs(2);
const RUN_LOOP_POLL: Duration = Duration::from_millis(100);
const REBUILD_BACKOFF: Duration = Duration::from_millis(100);
const LISTENER_STOPPED: u8 = 0;
const LISTENER_ACTIVE: u8 = 1;
const LISTENER_RECOVERING: u8 = 2;
const KEY_WORDS: usize = 4;

extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
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

pub fn listen_access_granted() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

pub fn request_listen_access() -> bool {
    unsafe { CGRequestListenEventAccess() }
}

pub fn start_listener(
    sender: SyncSender<GestureEvent>,
    overflowed: Arc<AtomicBool>,
    chord_queue: Arc<ChordQueueFlags>,
) -> Result<ListenerHandle, String> {
    if !listen_access_granted() {
        return Err("Allow Input Monitoring for Spick in System Settings".into());
    }

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
            callback_pressed_inputs.observe(event_type, keycode);
            let mut input = normalize_fields(event_type, keycode, event.get_flags());
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
                        // Pointer/scroll events are deliberately lossy and
                        // coalesced; dropping one must not masquerade as event
                        // tap failure. A lost keyboard or Option transition is
                        // still fatal so keyboard chords remain fail-closed.
                        if event.input != GestureInput::PointerChord {
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
                thread::sleep(REBUILD_BACKOFF);
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
                thread::sleep(REBUILD_BACKOFF);
                continue;
            }
        };

        let run_loop = CFRunLoop::get_current();
        let default_mode = unsafe { kCFRunLoopDefaultMode };
        run_loop.add_source(&source, default_mode);
        event_tap.enable();
        health.store(LISTENER_ACTIVE, Ordering::Release);
        if let Some(ready) = ready_sender.take() {
            let _ = ready.send(Ok(()));
        }

        while !stop.load(Ordering::Acquire) && !rebuild.load(Ordering::Acquire) {
            CFRunLoop::run_in_mode(default_mode, RUN_LOOP_POLL, true);
        }

        run_loop.remove_source(&source, default_mode);
        health.store(LISTENER_RECOVERING, Ordering::Release);
        if rebuild.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
            thread::sleep(REBUILD_BACKOFF);
        }
    }
}

struct PressedInputs {
    keys: [AtomicU64; KEY_WORDS],
    mouse_buttons: AtomicU8,
}

impl Default for PressedInputs {
    fn default() -> Self {
        Self {
            keys: std::array::from_fn(|_| AtomicU64::new(0)),
            mouse_buttons: AtomicU8::new(0),
        }
    }
}

impl PressedInputs {
    fn observe(&self, event_type: CGEventType, keycode: i64) {
        match event_type {
            CGEventType::KeyDown => self.set_key(keycode, true),
            CGEventType::KeyUp => self.set_key(keycode, false),
            CGEventType::LeftMouseDown => self.set_mouse(0, true),
            CGEventType::LeftMouseUp => self.set_mouse(0, false),
            CGEventType::RightMouseDown => self.set_mouse(1, true),
            CGEventType::RightMouseUp => self.set_mouse(1, false),
            CGEventType::OtherMouseDown => self.set_mouse(2, true),
            CGEventType::OtherMouseUp => self.set_mouse(2, false),
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
        } else if self.mouse_buttons.load(Ordering::Relaxed) != 0 {
            Some(GestureInput::PointerChord)
        } else {
            None
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

    fn set_mouse(&self, button: u8, down: bool) {
        let bit = 1_u8 << button;
        if down {
            self.mouse_buttons.fetch_or(bit, Ordering::Relaxed);
        } else {
            self.mouse_buttons.fetch_and(!bit, Ordering::Relaxed);
        }
    }
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
        pressed.observe(CGEventType::KeyDown, 12);
        assert_eq!(pressed.chord_input(), Some(GestureInput::KeyboardChord));
        pressed.observe(CGEventType::KeyUp, 12);
        assert_eq!(pressed.chord_input(), None);

        pressed.observe(CGEventType::LeftMouseDown, 0);
        assert_eq!(pressed.chord_input(), Some(GestureInput::PointerChord));
        pressed.observe(CGEventType::LeftMouseUp, 0);
        assert_eq!(pressed.chord_input(), None);
    }
}
