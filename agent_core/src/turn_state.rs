use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TURN_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TurnToken {
    pub session_id: String,
    pub turn_id: String,
    pub epoch: u64,
}

impl TurnToken {
    pub(crate) fn allocate(session_id: &str, wall_clock_millis: u128) -> Self {
        let epoch = NEXT_TURN_EPOCH.fetch_add(1, Ordering::Relaxed);
        Self {
            session_id: session_id.to_string(),
            turn_id: format!("turn_{wall_clock_millis}_{epoch}"),
            epoch,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnInputAdmission {
    Open,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnActivity {
    Running,
    WaitingModel { round: u32 },
    WaitingUser,
    RunningTools,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnProjectionOutcome {
    Completed,
    Cancelled,
    Failed { code: String },
    Interrupted { code: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveTurnProjection {
    pub token: TurnToken,
    pub stop_requested: bool,
    pub input_admission: TurnInputAdmission,
    pub activity: TurnActivity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinishedTurnProjection {
    pub token: TurnToken,
    pub outcome: TurnProjectionOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TurnProjection {
    Active(ActiveTurnProjection),
    Finished(FinishedTurnProjection),
}

/// Core-owned reducer for one accepted turn. It prevents activity updates from
/// reviving a finished turn and makes terminal publication idempotent.
pub(crate) struct TurnProjectionState {
    active: Option<ActiveTurnProjection>,
}

impl TurnProjectionState {
    pub(crate) fn start(token: TurnToken) -> (Self, TurnProjection) {
        let active = ActiveTurnProjection {
            token,
            stop_requested: false,
            input_admission: TurnInputAdmission::Open,
            activity: TurnActivity::Running,
        };
        (
            Self {
                active: Some(active.clone()),
            },
            TurnProjection::Active(active),
        )
    }

    pub(crate) fn set_activity(&mut self, activity: TurnActivity) -> Option<TurnProjection> {
        let active = self.active.as_mut()?;
        if active.activity == activity {
            return None;
        }
        active.activity = activity;
        Some(TurnProjection::Active(active.clone()))
    }

    pub(crate) fn request_stop(&mut self) -> Option<TurnProjection> {
        let active = self.active.as_mut()?;
        if active.stop_requested {
            return None;
        }
        active.stop_requested = true;
        Some(TurnProjection::Active(active.clone()))
    }

    pub(crate) fn close_input(&mut self) -> Option<TurnProjection> {
        let active = self.active.as_mut()?;
        if active.input_admission == TurnInputAdmission::Closed {
            return None;
        }
        active.input_admission = TurnInputAdmission::Closed;
        Some(TurnProjection::Active(active.clone()))
    }

    pub(crate) fn finish(&mut self, outcome: TurnProjectionOutcome) -> Option<TurnProjection> {
        let active = self.active.take()?;
        Some(TurnProjection::Finished(FinishedTurnProjection {
            token: active.token,
            outcome,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_projection_is_once_and_cannot_be_revived_by_activity() {
        let token = TurnToken::allocate("session", 1);
        let (mut state, started) = TurnProjectionState::start(token);
        assert!(matches!(started, TurnProjection::Active(_)));
        assert!(state.finish(TurnProjectionOutcome::Cancelled).is_some());
        assert!(state
            .set_activity(TurnActivity::WaitingModel { round: 2 })
            .is_none());
        assert!(state.request_stop().is_none());
        assert!(state.finish(TurnProjectionOutcome::Completed).is_none());
    }

    #[test]
    fn allocated_epochs_and_ids_are_monotonic_and_unique() {
        let first = TurnToken::allocate("session", 42);
        let second = TurnToken::allocate("session", 42);
        assert!(second.epoch > first.epoch);
        assert_ne!(first.turn_id, second.turn_id);
    }
}
