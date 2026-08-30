use std::sync::atomic::{AtomicU64, Ordering};

pub use timem_ui_contract::projections::{
    ActiveTurnProjection, FinishedTurnProjection, TurnActivity, TurnInputAdmission, TurnProjection,
    TurnProjectionOutcome, TurnToken,
};

static NEXT_TURN_EPOCH: AtomicU64 = AtomicU64::new(1);

pub(crate) fn allocate_turn_token(session_id: &str, wall_clock_millis: u128) -> TurnToken {
    let epoch = NEXT_TURN_EPOCH.fetch_add(1, Ordering::Relaxed);
    TurnToken {
        session_id: session_id.to_string(),
        turn_id: format!("turn_{wall_clock_millis}_{epoch}"),
        epoch,
    }
}

/// Agent-owned reducer for one accepted turn. It prevents activity updates from
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
        let token = allocate_turn_token("session", 1);
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
        let first = allocate_turn_token("session", 42);
        let second = allocate_turn_token("session", 42);
        assert!(second.epoch > first.epoch);
        assert_ne!(first.turn_id, second.turn_id);
    }
}
