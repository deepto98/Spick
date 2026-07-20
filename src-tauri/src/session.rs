use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::domain::{
    DictationDelivery, DictationSession, DictationStateEvent, EngineConfig, LanguagePolicy,
    SessionState, SessionTrigger,
};
use uuid::Uuid;

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
        transcription_engine: EngineConfig,
        cleanup_engine: Option<EngineConfig>,
    ) -> Result<DictationStateEvent, SessionError> {
        self.start_at(
            trigger,
            language_policy,
            transcription_engine,
            cleanup_engine,
            now_ms(),
        )
    }

    pub fn stop(&mut self) -> Result<DictationStateEvent, SessionError> {
        self.stop_at(now_ms())
    }

    /// Mark the microphone stream ready. Until this transition succeeds, the
    /// session must not claim that Spick is listening.
    pub fn ready(&mut self) -> Result<DictationStateEvent, SessionError> {
        self.ready_at()
    }

    /// Atomically claim the native side effect. Cancellation is deliberately
    /// unavailable after this transition, so it can never report success while
    /// another worker is still able to mutate the target field.
    pub fn begin_insertion(&mut self) -> Result<DictationStateEvent, SessionError> {
        self.begin_insertion_at()
    }

    pub fn cancel(&mut self, reason: Option<String>) -> Result<DictationStateEvent, SessionError> {
        self.cancel_at(reason, now_ms())
    }

    /// Complete processing after a transcription backend has produced output.
    pub fn complete(
        &mut self,
        delivery: DictationDelivery,
    ) -> Result<DictationStateEvent, SessionError> {
        self.complete_at(delivery, now_ms())
    }

    /// Fail the active session while keeping the diagnostic available to the UI.
    pub fn fail(&mut self, error: String) -> Result<DictationStateEvent, SessionError> {
        self.fail_at(error, None, now_ms())
    }

    pub fn fail_with_delivery(
        &mut self,
        error: String,
        delivery: DictationDelivery,
    ) -> Result<DictationStateEvent, SessionError> {
        self.fail_at(error, Some(delivery), now_ms())
    }

    fn start_at(
        &mut self,
        trigger: SessionTrigger,
        language_policy: LanguagePolicy,
        transcription_engine: EngineConfig,
        cleanup_engine: Option<EngineConfig>,
        timestamp_ms: u64,
    ) -> Result<DictationStateEvent, SessionError> {
        if let Some(session) = &self.current {
            if matches!(
                session.state,
                SessionState::Starting
                    | SessionState::Listening
                    | SessionState::Processing
                    | SessionState::Inserting
            ) {
                return Err(SessionError::AlreadyActive(session.state));
            }
        }

        self.revision = self.revision.saturating_add(1);
        self.current = Some(DictationSession {
            // Receipts use this identifier as their exact-once key across app
            // launches. A random UUID avoids clock rollback and restart
            // collisions that a process-local counter cannot prevent.
            id: format!("dictation-{}", Uuid::new_v4()),
            state: SessionState::Starting,
            trigger,
            language_policy,
            transcription_engine,
            cleanup_engine,
            started_at_ms: timestamp_ms,
            ended_at_ms: None,
            cancel_reason: None,
            error: None,
            delivery: None,
        });

        Ok(self.snapshot())
    }

    fn ready_at(&mut self) -> Result<DictationStateEvent, SessionError> {
        {
            let session = self.current.as_mut().ok_or(SessionError::NoSession)?;
            if session.state != SessionState::Starting {
                return Err(SessionError::InvalidTransition {
                    from: session.state,
                    action: "mark ready",
                });
            }
            session.state = SessionState::Listening;
        }
        self.revision = self.revision.saturating_add(1);
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
                SessionState::Starting | SessionState::Listening | SessionState::Processing
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

    fn begin_insertion_at(&mut self) -> Result<DictationStateEvent, SessionError> {
        {
            let session = self.current.as_mut().ok_or(SessionError::NoSession)?;
            if session.state != SessionState::Processing {
                return Err(SessionError::InvalidTransition {
                    from: session.state,
                    action: "begin insertion for",
                });
            }
            session.state = SessionState::Inserting;
        }
        self.revision = self.revision.saturating_add(1);
        Ok(self.snapshot())
    }

    fn complete_at(
        &mut self,
        delivery: DictationDelivery,
        timestamp_ms: u64,
    ) -> Result<DictationStateEvent, SessionError> {
        {
            let session = self.current.as_mut().ok_or(SessionError::NoSession)?;
            if session.state != SessionState::Inserting {
                return Err(SessionError::InvalidTransition {
                    from: session.state,
                    action: "complete",
                });
            }

            session.state = SessionState::Completed;
            session.ended_at_ms = Some(timestamp_ms);
            session.delivery = Some(delivery);
        }
        self.revision = self.revision.saturating_add(1);
        Ok(self.snapshot())
    }

    fn fail_at(
        &mut self,
        error: String,
        delivery: Option<DictationDelivery>,
        timestamp_ms: u64,
    ) -> Result<DictationStateEvent, SessionError> {
        {
            let session = self.current.as_mut().ok_or(SessionError::NoSession)?;
            if !matches!(
                session.state,
                SessionState::Starting | SessionState::Listening | SessionState::Processing
            ) {
                return Err(SessionError::InvalidTransition {
                    from: session.state,
                    action: "fail",
                });
            }

            session.state = SessionState::Failed;
            session.ended_at_ms = Some(timestamp_ms);
            session.error = Some(error);
            session.delivery = delivery;
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
    use crate::domain::{
        AppSettings, DictationDeliveryStatus, EngineProvider, BUILTIN_READABLE_CLEANUP_MODEL,
    };

    fn engine() -> EngineConfig {
        AppSettings::default().transcription_engine
    }

    fn cleanup_engine() -> Option<EngineConfig> {
        Some(EngineConfig::local(
            EngineProvider::BuiltIn,
            BUILTIN_READABLE_CLEANUP_MODEL,
        ))
    }

    fn session(event: &DictationStateEvent) -> &DictationSession {
        event.session.as_ref().expect("expected a session")
    }

    fn delivery() -> DictationDelivery {
        DictationDelivery {
            status: DictationDeliveryStatus::Inserted,
            transcript_available: true,
            target_app: Some("Notes".into()),
            caret_repositioned: Some(true),
        }
    }

    #[test]
    fn push_to_talk_waits_for_microphone_readiness_before_listening() {
        let mut controller = SessionController::default();

        assert_eq!(controller.snapshot(), DictationStateEvent::idle());
        let starting = controller
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                cleanup_engine(),
                100,
            )
            .unwrap();
        assert_eq!(starting.state, SessionState::Starting);
        assert_eq!(starting.revision, 1);
        assert_eq!(session(&starting).started_at_ms, 100);

        let listening = controller.ready_at().unwrap();
        assert_eq!(listening.state, SessionState::Listening);
        assert_eq!(listening.revision, 2);

        let processing = controller.stop_at(200).unwrap();
        assert_eq!(processing.state, SessionState::Processing);
        assert_eq!(processing.revision, 3);
        assert_eq!(session(&processing).ended_at_ms, None);
    }

    #[test]
    fn session_keeps_its_engine_choices_when_recording_started() {
        let mut controller = SessionController::default();
        let selected =
            EngineConfig::local(EngineProvider::WhisperCpp, "whisper-tiny-multilingual-f16");
        let selected_cleanup = cleanup_engine();
        let starting = controller
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                selected.clone(),
                selected_cleanup.clone(),
                100,
            )
            .unwrap();

        assert_eq!(session(&starting).transcription_engine, selected);
        assert_eq!(session(&starting).cleanup_engine, selected_cleanup);
    }

    #[test]
    fn active_sessions_cannot_overlap() {
        let mut controller = SessionController::default();
        controller
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                cleanup_engine(),
                100,
            )
            .unwrap();

        assert_eq!(
            controller.start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                cleanup_engine(),
                101
            ),
            Err(SessionError::AlreadyActive(SessionState::Starting))
        );
    }

    #[test]
    fn processing_can_complete_and_a_new_session_can_then_start() {
        let mut controller = SessionController::default();
        controller
            .start_at(
                SessionTrigger::UserInterface,
                LanguagePolicy::Auto,
                engine(),
                cleanup_engine(),
                100,
            )
            .unwrap();
        controller.ready_at().unwrap();
        controller.stop_at(150).unwrap();
        let inserting = controller.begin_insertion_at().unwrap();
        assert_eq!(inserting.state, SessionState::Inserting);

        let completed = controller.complete_at(delivery(), 300).unwrap();
        assert_eq!(completed.state, SessionState::Completed);
        assert_eq!(completed.revision, 5);
        assert_eq!(session(&completed).ended_at_ms, Some(300));
        assert_eq!(session(&completed).delivery.as_ref(), Some(&delivery()));

        let next = controller
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                cleanup_engine(),
                400,
            )
            .unwrap();
        assert_eq!(next.state, SessionState::Starting);
        assert_eq!(next.revision, 6);
        assert_ne!(session(&completed).id, session(&next).id);
    }

    #[test]
    fn receipt_identity_does_not_repeat_across_controllers_at_the_same_time() {
        let mut first = SessionController::default();
        let mut restarted = SessionController::default();
        let first = first
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                None,
                100,
            )
            .unwrap();
        let restarted = restarted
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                None,
                100,
            )
            .unwrap();

        assert_ne!(session(&first).id, session(&restarted).id);
        for event in [&first, &restarted] {
            let uuid = session(event).id.strip_prefix("dictation-").unwrap();
            assert!(Uuid::parse_str(uuid).is_ok());
        }
    }

    #[test]
    fn cancel_is_terminal_and_records_a_reason() {
        let mut controller = SessionController::default();
        controller
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                cleanup_engine(),
                100,
            )
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
            controller.fail_at("microphone unavailable".into(), None, 100),
            Err(SessionError::NoSession)
        );

        controller
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                cleanup_engine(),
                110,
            )
            .unwrap();
        let failed = controller
            .fail_at("microphone unavailable".into(), None, 120)
            .unwrap();
        assert_eq!(failed.state, SessionState::Failed);
        assert_eq!(
            session(&failed).error.as_deref(),
            Some("microphone unavailable")
        );
    }

    #[test]
    fn cancellation_cannot_claim_success_after_insertion_is_claimed() {
        let mut controller = SessionController::default();
        controller
            .start_at(
                SessionTrigger::Shortcut,
                LanguagePolicy::Auto,
                engine(),
                cleanup_engine(),
                100,
            )
            .unwrap();
        controller.ready_at().unwrap();
        controller.stop_at(150).unwrap();
        controller.begin_insertion_at().unwrap();

        assert_eq!(
            controller.cancel_at(Some("too late".into()), 160),
            Err(SessionError::InvalidTransition {
                from: SessionState::Inserting,
                action: "cancel",
            })
        );
    }
}
