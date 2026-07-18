use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::domain::{
    DictationSession, DictationStateEvent, LanguagePolicy, SessionState, SessionTrigger,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    AlreadyActive(SessionState),
    NoSession,
    InvalidTransition {
        from: SessionState,
        action: &'static str,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive(state) => {
                write!(formatter, "a dictation session is already {state:?}")
            }
            Self::NoSession => formatter.write_str("there is no dictation session"),
            Self::InvalidTransition { from, action } => {
                write!(formatter, "cannot {action} a {from:?} session")
            }
        }
    }
}

/// Pure state machine for one-at-a-time dictation sessions.
///
/// Audio capture and transcription will be plugged in around these transitions;
/// they do not belong in the lifecycle model itself.
#[derive(Debug, Default)]
pub struct SessionController {
    current: Option<DictationSession>,
    sequence: u64,
    revision: u64,
}

impl SessionController {
    pub fn snapshot(&self) -> DictationStateEvent {
        match &self.current {
            Some(session) => DictationStateEvent {
                revision: self.revision,
                state: session.state,
                session: Some(session.clone()),
            },
            None => DictationStateEvent {
                revision: self.revision,
                ..DictationStateEvent::idle()
            },
        }
    }

    pub fn start(
        &mut self,
        trigger: SessionTrigger,
        language_policy: LanguagePolicy,
    ) -> Result<DictationStateEvent, SessionError> {
        self.start_at(trigger, language_policy, now_ms())
    }

    pub fn stop(&mut self) -> Result<DictationStateEvent, SessionError> {
        self.stop_at(now_ms())
    }

    pub fn cancel(&mut self, reason: Option<String>) -> Result<DictationStateEvent, SessionError> {
        self.cancel_at(reason, now_ms())
    }

    /// Complete processing after a transcription backend has produced output.
    pub fn complete(&mut self) -> Result<DictationStateEvent, SessionError> {
        self.complete_at(now_ms())
    }

    /// Fail the active session while keeping the diagnostic available to the UI.
    pub fn fail(&mut self, error: String) -> Result<DictationStateEvent, SessionError> {
        self.fail_at(error, now_ms())
    }

    fn start_at(
        &mut self,
        trigger: SessionTrigger,
        language_policy: LanguagePolicy,
        timestamp_ms: u64,
    ) -> Result<DictationStateEvent, SessionError> {
        if let Some(session) = &self.current {
            if matches!(
                session.state,
                SessionState::Listening | SessionState::Processing
            ) {
                return Err(SessionError::AlreadyActive(session.state));
            }
        }

        self.sequence = self.sequence.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
        self.current = Some(DictationSession {
            id: format!("dictation-{timestamp_ms}-{}", self.sequence),
            state: SessionState::Listening,
            trigger,
            language_policy,
            started_at_ms: timestamp_ms,
            ended_at_ms: None,
            cancel_reason: None,
            error: None,
        });

        Ok(self.snapshot())
    }

    fn stop_at(&mut self, _timestamp_ms: u64) -> Result<DictationStateEvent, SessionError> {
        {
            let session = self.current.as_mut().ok_or(SessionError::NoSession)?;
            if session.state != SessionState::Listening {
                return Err(SessionError::InvalidTransition {
                    from: session.state,
                    action: "stop",
                });
            }

            // Releasing push-to-talk ends capture, but the session remains active
            // until the transcription/cleanup pipeline completes or fails.
            session.state = SessionState::Processing;
        }
        self.revision = self.revision.saturating_add(1);
        Ok(self.snapshot())
    }

    fn cancel_at(
        &mut self,
        reason: Option<String>,
        timestamp_ms: u64,
    ) -> Result<DictationStateEvent, SessionError> {
        {
            let session = self.current.as_mut().ok_or(SessionError::NoSession)?;
            if !matches!(
                session.state,
                SessionState::Listening | SessionState::Processing
            ) {
                return Err(SessionError::InvalidTransition {
                    from: session.state,
                    action: "cancel",
                });
            }

            session.state = SessionState::Cancelled;
            session.ended_at_ms = Some(timestamp_ms);
            session.cancel_reason = reason.filter(|value| !value.trim().is_empty());
        }
        self.revision = self.revision.saturating_add(1);
        Ok(self.snapshot())
    }

    fn complete_at(&mut self, timestamp_ms: u64) -> Result<DictationStateEvent, SessionError> {
        {
            let session = self.current.as_mut().ok_or(SessionError::NoSession)?;
            if session.state != SessionState::Processing {
                return Err(SessionError::InvalidTransition {
                    from: session.state,
                    action: "complete",
                });
            }

            session.state = SessionState::Completed;
            session.ended_at_ms = Some(timestamp_ms);
        }
        self.revision = self.revision.saturating_add(1);
        Ok(self.snapshot())
    }

    fn fail_at(
        &mut self,
        error: String,
        timestamp_ms: u64,
    ) -> Result<DictationStateEvent, SessionError> {
        {
            let session = self.current.as_mut().ok_or(SessionError::NoSession)?;
            if !matches!(
                session.state,
                SessionState::Listening | SessionState::Processing
            ) {
                return Err(SessionError::InvalidTransition {
                    from: session.state,
                    action: "fail",
                });
            }

            session.state = SessionState::Failed;
            session.ended_at_ms = Some(timestamp_ms);
            session.error = Some(error);
        }
        self.revision = self.revision.saturating_add(1);
        Ok(self.snapshot())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(event: &DictationStateEvent) -> &DictationSession {
        event.session.as_ref().expect("expected a session")
    }

    #[test]
    fn push_to_talk_moves_from_idle_to_listening_to_processing() {
        let mut controller = SessionController::default();

        assert_eq!(controller.snapshot(), DictationStateEvent::idle());
        let listening = controller
            .start_at(SessionTrigger::Shortcut, LanguagePolicy::Auto, 100)
            .unwrap();
        assert_eq!(listening.state, SessionState::Listening);
        assert_eq!(listening.revision, 1);
        assert_eq!(session(&listening).started_at_ms, 100);

        let processing = controller.stop_at(200).unwrap();
        assert_eq!(processing.state, SessionState::Processing);
        assert_eq!(processing.revision, 2);
        assert_eq!(session(&processing).ended_at_ms, None);
    }

    #[test]
    fn active_sessions_cannot_overlap() {
        let mut controller = SessionController::default();
        controller
            .start_at(SessionTrigger::Shortcut, LanguagePolicy::Auto, 100)
            .unwrap();

        assert_eq!(
            controller.start_at(SessionTrigger::Shortcut, LanguagePolicy::Auto, 101),
            Err(SessionError::AlreadyActive(SessionState::Listening))
        );
    }

    #[test]
    fn processing_can_complete_and_a_new_session_can_then_start() {
        let mut controller = SessionController::default();
        controller
            .start_at(SessionTrigger::UserInterface, LanguagePolicy::Auto, 100)
            .unwrap();
        controller.stop_at(150).unwrap();

        let completed = controller.complete_at(300).unwrap();
        assert_eq!(completed.state, SessionState::Completed);
        assert_eq!(completed.revision, 3);
        assert_eq!(session(&completed).ended_at_ms, Some(300));

        let next = controller
            .start_at(SessionTrigger::Shortcut, LanguagePolicy::Auto, 400)
            .unwrap();
        assert_eq!(next.state, SessionState::Listening);
        assert_eq!(next.revision, 4);
        assert_ne!(session(&completed).id, session(&next).id);
    }

    #[test]
    fn cancel_is_terminal_and_records_a_reason() {
        let mut controller = SessionController::default();
        controller
            .start_at(SessionTrigger::Shortcut, LanguagePolicy::Auto, 100)
            .unwrap();

        let cancelled = controller
            .cancel_at(Some("focus changed".into()), 125)
            .unwrap();
        assert_eq!(cancelled.state, SessionState::Cancelled);
        assert_eq!(session(&cancelled).ended_at_ms, Some(125));
        assert_eq!(
            session(&cancelled).cancel_reason.as_deref(),
            Some("focus changed")
        );
        assert!(controller.stop_at(130).is_err());
    }

    #[test]
    fn only_active_sessions_can_fail() {
        let mut controller = SessionController::default();
        assert_eq!(
            controller.fail_at("microphone unavailable".into(), 100),
            Err(SessionError::NoSession)
        );

        controller
            .start_at(SessionTrigger::Shortcut, LanguagePolicy::Auto, 110)
            .unwrap();
        let failed = controller
            .fail_at("microphone unavailable".into(), 120)
            .unwrap();
        assert_eq!(failed.state, SessionState::Failed);
        assert_eq!(
            session(&failed).error.as_deref(),
            Some("microphone unavailable")
        );
    }
}
