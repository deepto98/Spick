use std::time::{Duration, Instant};

pub const HOLD_THRESHOLD: Duration = Duration::from_millis(280);
const QUARANTINE_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureInput {
    OptionDown,
    OptionUp,
    Chord,
    ListenerDisabled,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureAction {
    Start,
    Stop,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GestureState {
    Idle,
    Armed { deadline: Instant },
    DirtyIdle,
    Holding,
    ToggleListening,
    ToggleStopArmed,
    DirtyToggle,
    Quarantined { deadline: Instant },
}

pub struct GestureMachine {
    state: GestureState,
}

impl Default for GestureMachine {
    fn default() -> Self {
        Self {
            state: GestureState::Idle,
        }
    }
}

impl GestureMachine {
    pub fn handle(&mut self, input: GestureInput, now: Instant) -> Option<GestureAction> {
        match (self.state, input) {
            (_, GestureInput::Shutdown) => {
                let cancel = self.is_recording();
                self.state = GestureState::Idle;
                cancel.then_some(GestureAction::Cancel)
            }
            (GestureState::Idle, GestureInput::OptionDown) => {
                self.state = GestureState::Armed {
                    deadline: now + HOLD_THRESHOLD,
                };
                None
            }
            (GestureState::Armed { deadline }, GestureInput::OptionUp) if now < deadline => {
                self.state = GestureState::ToggleListening;
                Some(GestureAction::Start)
            }
            (GestureState::Armed { .. }, GestureInput::Chord | GestureInput::OptionDown) => {
                self.state = GestureState::DirtyIdle;
                None
            }
            (GestureState::Armed { .. }, GestureInput::ListenerDisabled) => {
                self.state = GestureState::Idle;
                None
            }
            (GestureState::Armed { .. }, GestureInput::OptionUp) => {
                // The timer normally transitions first. If scheduling delayed
                // both events, do not emit Stop without a preceding Start.
                self.state = GestureState::Idle;
                None
            }
            (GestureState::DirtyIdle, GestureInput::OptionUp) => {
                self.state = GestureState::Idle;
                None
            }
            (GestureState::DirtyIdle, GestureInput::ListenerDisabled) => {
                self.state = GestureState::Idle;
                None
            }
            (GestureState::Holding, GestureInput::OptionUp) => {
                self.state = GestureState::Idle;
                Some(GestureAction::Stop)
            }
            (GestureState::Holding, GestureInput::Chord | GestureInput::OptionDown) => {
                self.state = GestureState::DirtyIdle;
                Some(GestureAction::Cancel)
            }
            (GestureState::Holding, GestureInput::ListenerDisabled) => {
                self.state = GestureState::Idle;
                Some(GestureAction::Cancel)
            }
            (GestureState::ToggleListening, GestureInput::OptionDown) => {
                self.state = GestureState::ToggleStopArmed;
                None
            }
            (GestureState::ToggleStopArmed, GestureInput::OptionUp) => {
                self.state = GestureState::Idle;
                Some(GestureAction::Stop)
            }
            (GestureState::ToggleStopArmed, GestureInput::Chord | GestureInput::OptionDown) => {
                self.state = GestureState::DirtyToggle;
                None
            }
            (GestureState::DirtyToggle, GestureInput::OptionUp) => {
                self.state = GestureState::ToggleListening;
                None
            }
            (
                GestureState::ToggleListening
                | GestureState::ToggleStopArmed
                | GestureState::DirtyToggle,
                GestureInput::ListenerDisabled,
            ) => {
                self.state = GestureState::Idle;
                Some(GestureAction::Cancel)
            }
            (GestureState::Quarantined { .. }, GestureInput::OptionUp) => {
                self.state = GestureState::Idle;
                None
            }
            _ => None,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            GestureState::Armed { deadline } | GestureState::Quarantined { deadline } => {
                Some(deadline)
            }
            _ => None,
        }
    }

    pub fn handle_timeout(&mut self, now: Instant) -> Option<GestureAction> {
        match self.state {
            GestureState::Armed { deadline } if now >= deadline => {
                self.state = GestureState::Holding;
                Some(GestureAction::Start)
            }
            GestureState::Quarantined { deadline } if now >= deadline => {
                self.state = GestureState::Idle;
                None
            }
            _ => None,
        }
    }

    pub fn reconcile(&mut self, listening: bool) {
        if self.is_recording() && !listening {
            self.state = GestureState::Idle;
        }
    }

    pub fn quarantine(&mut self, now: Instant) {
        self.state = GestureState::Quarantined {
            deadline: now + QUARANTINE_DURATION,
        };
    }

    pub fn reset(&mut self) {
        self.state = GestureState::Idle;
    }

    fn is_recording(&self) -> bool {
        matches!(
            self.state,
            GestureState::Holding
                | GestureState::ToggleListening
                | GestureState::ToggleStopArmed
                | GestureState::DirtyToggle
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_taps_start_and_stop_toggle_recording() {
        let start = Instant::now();
        let mut machine = GestureMachine::default();
        assert_eq!(machine.handle(GestureInput::OptionDown, start), None);
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(80)),
            Some(GestureAction::Start)
        );
        assert_eq!(
            machine.handle(GestureInput::OptionDown, start + Duration::from_millis(500)),
            None
        );
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(560)),
            Some(GestureAction::Stop)
        );
    }

    #[test]
    fn a_hold_starts_at_the_threshold_and_stops_on_release() {
        let start = Instant::now();
        let mut machine = GestureMachine::default();
        machine.handle(GestureInput::OptionDown, start);
        assert_eq!(
            machine.handle_timeout(start + HOLD_THRESHOLD),
            Some(GestureAction::Start)
        );
        assert_eq!(
            machine.handle(
                GestureInput::OptionUp,
                start + HOLD_THRESHOLD + Duration::from_millis(100)
            ),
            Some(GestureAction::Stop)
        );
    }

    #[test]
    fn option_chords_never_start_and_cancel_an_active_hold() {
        let start = Instant::now();
        let mut machine = GestureMachine::default();
        machine.handle(GestureInput::OptionDown, start);
        assert_eq!(
            machine.handle(GestureInput::Chord, start + Duration::from_millis(40)),
            None
        );
        assert_eq!(machine.handle_timeout(start + HOLD_THRESHOLD), None);
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(60)),
            None
        );

        machine.handle(GestureInput::OptionDown, start);
        assert_eq!(
            machine.handle_timeout(start + HOLD_THRESHOLD),
            Some(GestureAction::Start)
        );
        assert_eq!(
            machine.handle(
                GestureInput::Chord,
                start + HOLD_THRESHOLD + Duration::from_millis(10)
            ),
            Some(GestureAction::Cancel)
        );
    }

    #[test]
    fn an_option_chord_does_not_stop_toggle_recording() {
        let start = Instant::now();
        let mut machine = toggled_machine(start);
        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(100));
        assert_eq!(
            machine.handle(GestureInput::Chord, start + Duration::from_millis(110)),
            None
        );
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(120)),
            None
        );
        assert_eq!(
            machine.handle(GestureInput::OptionDown, start + Duration::from_millis(200)),
            None
        );
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(220)),
            Some(GestureAction::Stop)
        );
    }

    #[test]
    fn option_release_after_modifier_chord_clears_dirty_state() {
        let start = Instant::now();
        let mut machine = GestureMachine::default();

        machine.handle(GestureInput::OptionDown, start);
        machine.handle(GestureInput::Chord, start + Duration::from_millis(10));
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(20)),
            None
        );

        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(100));
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(120)),
            Some(GestureAction::Start)
        );

        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(200));
        machine.handle(GestureInput::Chord, start + Duration::from_millis(210));
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(220)),
            None
        );

        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(300));
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(320)),
            Some(GestureAction::Stop)
        );
    }

    #[test]
    fn pressing_both_option_keys_fails_closed() {
        let start = Instant::now();
        let mut machine = GestureMachine::default();
        machine.handle(GestureInput::OptionDown, start);
        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(10));
        // Releasing the first physical Option still has the aggregate Option
        // flag set, so the native normalizer reports another down event.
        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(20));
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(30)),
            None
        );
        assert_eq!(machine.handle_timeout(start + HOLD_THRESHOLD), None);

        assert_eq!(
            machine.handle(GestureInput::OptionDown, start + Duration::from_secs(1)),
            None
        );
        assert_eq!(
            machine.handle(
                GestureInput::OptionUp,
                start + Duration::from_secs(1) + Duration::from_millis(20)
            ),
            Some(GestureAction::Start)
        );
    }

    #[test]
    fn two_option_keys_do_not_stop_toggle_recording() {
        let start = Instant::now();
        let mut machine = toggled_machine(start);
        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(100));
        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(110));
        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(120));
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(130)),
            None
        );
        assert_eq!(
            machine.handle(GestureInput::OptionDown, start + Duration::from_millis(200)),
            None
        );
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(220)),
            Some(GestureAction::Stop)
        );
    }

    #[test]
    fn listener_loss_cancels_only_an_active_recording() {
        let start = Instant::now();
        let mut machine = GestureMachine::default();
        assert_eq!(machine.handle(GestureInput::ListenerDisabled, start), None);
        machine = toggled_machine(start);
        assert_eq!(
            machine.handle(GestureInput::ListenerDisabled, start),
            Some(GestureAction::Cancel)
        );
    }

    #[test]
    fn terminal_session_reconciliation_makes_the_next_tap_a_start() {
        let start = Instant::now();
        let mut machine = toggled_machine(start);
        machine.reconcile(false);
        machine.handle(GestureInput::OptionDown, start + Duration::from_millis(100));
        assert_eq!(
            machine.handle(GestureInput::OptionUp, start + Duration::from_millis(120)),
            Some(GestureAction::Start)
        );
    }

    #[test]
    fn quarantine_ignores_stale_events_and_recovers() {
        let start = Instant::now();
        let mut machine = GestureMachine::default();
        machine.quarantine(start);
        assert_eq!(machine.handle(GestureInput::OptionDown, start), None);
        assert_eq!(machine.handle(GestureInput::Chord, start), None);
        assert_eq!(machine.handle_timeout(start + QUARANTINE_DURATION), None);
        machine.handle(GestureInput::OptionDown, start + Duration::from_secs(1));
        assert_eq!(
            machine.handle(
                GestureInput::OptionUp,
                start + Duration::from_secs(1) + Duration::from_millis(20)
            ),
            Some(GestureAction::Start)
        );
    }

    #[test]
    fn shutdown_cancels_active_recording() {
        let start = Instant::now();
        let mut machine = toggled_machine(start);
        assert_eq!(
            machine.handle(GestureInput::Shutdown, start),
            Some(GestureAction::Cancel)
        );
    }

    #[test]
    fn a_delayed_release_never_emits_stop_without_start() {
        let start = Instant::now();
        let mut machine = GestureMachine::default();
        machine.handle(GestureInput::OptionDown, start);
        assert_eq!(
            machine.handle(
                GestureInput::OptionUp,
                start + HOLD_THRESHOLD + Duration::from_millis(10)
            ),
            None
        );
    }

    fn toggled_machine(start: Instant) -> GestureMachine {
        let mut machine = GestureMachine::default();
        machine.handle(GestureInput::OptionDown, start);
        machine.handle(GestureInput::OptionUp, start + Duration::from_millis(20));
        machine
    }
}
