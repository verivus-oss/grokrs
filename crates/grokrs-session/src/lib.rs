use grokrs_cap::TrustLevel;
use std::marker::PhantomData;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Created,
    Ready,
    RunningTurn,
    WaitingApproval,
    Closed,
    Failed(String),
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Ready => write!(f, "ready"),
            Self::RunningTurn => write!(f, "running_turn"),
            Self::WaitingApproval => write!(f, "waiting_approval"),
            Self::Closed => write!(f, "closed"),
            Self::Failed(reason) => write!(f, "failed:{reason}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session<T: TrustLevel> {
    id: String,
    state: SessionState,
    /// Count of state transitions for telemetry.
    state_transitions: u32,
    /// Count of turns (transitions to RunningTurn) for telemetry.
    total_turns: u32,
    _trust: PhantomData<T>,
}

impl<T: TrustLevel> Session<T> {
    pub fn new(id: impl Into<String>) -> Self {
        let id_str: String = id.into();

        #[cfg(feature = "otel")]
        {
            // Emit a session creation event. We cannot hold a span guard in
            // the Session struct (it is Clone), so we log the event instead.
            // The caller (CLI) is responsible for entering a session-scoped
            // span that wraps agent iterations and tool calls.
            tracing::info!(
                session.id = %id_str,
                session.trust_level = T::trust_rank(),
                "session created"
            );
        }

        Self {
            id: id_str,
            state: SessionState::Created,
            state_transitions: 0,
            total_turns: 0,
            _trust: PhantomData,
        }
    }

    #[must_use]
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn transition(&mut self, next: SessionState) {
        if matches!(&next, SessionState::RunningTurn) {
            self.total_turns += 1;
        }
        self.state_transitions += 1;
        self.state = next;

        #[cfg(feature = "otel")]
        {
            tracing::info!(
                session.id = %self.id,
                session.state = %self.state,
                session.state_transitions = self.state_transitions,
                session.total_turns = self.total_turns,
                "session state transition"
            );
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Return the total number of state transitions.
    #[must_use]
    pub fn state_transitions(&self) -> u32 {
        self.state_transitions
    }

    /// Return the total number of turns (transitions to RunningTurn).
    #[must_use]
    pub fn total_turns(&self) -> u32 {
        self.total_turns
    }
}

#[cfg(test)]
mod tests {
    use super::{Session, SessionState};
    use grokrs_cap::Untrusted;

    #[test]
    fn session_transitions_state() {
        let mut session = Session::<Untrusted>::new("s1");
        assert_eq!(session.state(), &SessionState::Created);
        session.transition(SessionState::Ready);
        assert_eq!(session.state(), &SessionState::Ready);
    }

    #[test]
    fn session_counts_state_transitions() {
        let mut session = Session::<Untrusted>::new("s2");
        assert_eq!(session.state_transitions(), 0);
        assert_eq!(session.total_turns(), 0);

        session.transition(SessionState::Ready);
        assert_eq!(session.state_transitions(), 1);
        assert_eq!(session.total_turns(), 0);

        session.transition(SessionState::RunningTurn);
        assert_eq!(session.state_transitions(), 2);
        assert_eq!(session.total_turns(), 1);

        session.transition(SessionState::Ready);
        assert_eq!(session.state_transitions(), 3);
        assert_eq!(session.total_turns(), 1);

        session.transition(SessionState::RunningTurn);
        assert_eq!(session.state_transitions(), 4);
        assert_eq!(session.total_turns(), 2);

        session.transition(SessionState::Closed);
        assert_eq!(session.state_transitions(), 5);
        assert_eq!(session.total_turns(), 2);
    }

    #[test]
    fn session_state_display() {
        assert_eq!(format!("{}", SessionState::Created), "created");
        assert_eq!(format!("{}", SessionState::Ready), "ready");
        assert_eq!(format!("{}", SessionState::RunningTurn), "running_turn");
        assert_eq!(
            format!("{}", SessionState::WaitingApproval),
            "waiting_approval"
        );
        assert_eq!(format!("{}", SessionState::Closed), "closed");
        assert_eq!(
            format!("{}", SessionState::Failed("oops".into())),
            "failed:oops"
        );
    }
}
