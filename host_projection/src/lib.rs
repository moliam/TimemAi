//! UI- and transport-neutral caching for authoritative Agent Core projections.
//!
//! This crate does not own a Turn lifecycle. It stores the latest exact Core
//! projection and adds only Host delivery metadata such as a monotonic revision.

use agent_core::{TurnProjection, TurnToken};
use serde::{Deserialize, Serialize, Serializer};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionedTurnProjection {
    pub revision: u64,
    pub projection: TurnProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionApplyResult {
    Applied { revision: u64 },
    Duplicate { revision: u64 },
    IgnoredStale { current_revision: u64 },
}

#[derive(Debug, Clone, Default)]
pub struct TurnProjectionCache {
    revision: u64,
    current: Option<TurnProjection>,
}

impl Serialize for TurnProjectionCache {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.current().serialize(serializer)
    }
}

impl TurnProjectionCache {
    pub fn current(&self) -> Option<VersionedTurnProjection> {
        self.current
            .clone()
            .map(|projection| VersionedTurnProjection {
                revision: self.revision,
                projection,
            })
    }

    pub fn apply(&mut self, projection: TurnProjection) -> ProjectionApplyResult {
        if self.current.as_ref() == Some(&projection) {
            return ProjectionApplyResult::Duplicate {
                revision: self.revision,
            };
        }
        if self.is_stale(&projection) {
            return ProjectionApplyResult::IgnoredStale {
                current_revision: self.revision,
            };
        }
        self.revision = self.revision.saturating_add(1);
        self.current = Some(projection);
        ProjectionApplyResult::Applied {
            revision: self.revision,
        }
    }

    fn is_stale(&self, incoming: &TurnProjection) -> bool {
        let Some(current) = self.current.as_ref() else {
            return false;
        };
        let current_token = projection_token(current);
        let incoming_token = projection_token(incoming);
        if incoming_token.session_id != current_token.session_id {
            return true;
        }
        if incoming_token.epoch < current_token.epoch {
            return true;
        }
        if incoming_token.epoch > current_token.epoch {
            return false;
        }
        if incoming_token.turn_id != current_token.turn_id {
            return true;
        }
        matches!(current, TurnProjection::Finished(_))
    }
}

fn projection_token(projection: &TurnProjection) -> &TurnToken {
    match projection {
        TurnProjection::Active(active) => &active.token,
        TurnProjection::Finished(finished) => &finished.token,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{
        ActiveTurnProjection, FinishedTurnProjection, TurnActivity, TurnInputAdmission,
        TurnProjectionOutcome,
    };

    fn token(turn_id: &str, epoch: u64) -> TurnToken {
        TurnToken {
            session_id: "session".to_string(),
            turn_id: turn_id.to_string(),
            epoch,
        }
    }

    fn active(turn_id: &str, epoch: u64, stop_requested: bool) -> TurnProjection {
        TurnProjection::Active(ActiveTurnProjection {
            token: token(turn_id, epoch),
            stop_requested,
            input_admission: TurnInputAdmission::Open,
            activity: TurnActivity::Running,
        })
    }

    fn finished(turn_id: &str, epoch: u64) -> TurnProjection {
        TurnProjection::Finished(FinishedTurnProjection {
            token: token(turn_id, epoch),
            outcome: TurnProjectionOutcome::Completed,
        })
    }

    #[test]
    fn caches_exact_core_projection_and_adds_only_revision() {
        let mut cache = TurnProjectionCache::default();
        assert_eq!(
            cache.apply(active("a", 1, false)),
            ProjectionApplyResult::Applied { revision: 1 }
        );
        assert_eq!(
            cache.apply(active("a", 1, true)),
            ProjectionApplyResult::Applied { revision: 2 }
        );
        let current = cache.current().unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(current.projection, active("a", 1, true));
    }

    #[test]
    fn duplicate_and_stale_delivery_cannot_advance_or_revive_projection() {
        let mut cache = TurnProjectionCache::default();
        cache.apply(active("a", 1, false));
        assert_eq!(
            cache.apply(active("a", 1, false)),
            ProjectionApplyResult::Duplicate { revision: 1 }
        );
        cache.apply(finished("a", 1));
        assert_eq!(
            cache.apply(active("a", 1, true)),
            ProjectionApplyResult::IgnoredStale {
                current_revision: 2
            }
        );
        assert_eq!(
            cache.apply(active("older", 0, false)),
            ProjectionApplyResult::IgnoredStale {
                current_revision: 2
            }
        );
        assert_eq!(cache.current().unwrap().projection, finished("a", 1));
    }

    #[test]
    fn newer_core_epoch_replaces_finished_projection_without_host_lifecycle() {
        let mut cache = TurnProjectionCache::default();
        cache.apply(finished("a", 1));
        assert_eq!(
            cache.apply(active("b", 2, false)),
            ProjectionApplyResult::Applied { revision: 2 }
        );
        assert_eq!(cache.current().unwrap().projection, active("b", 2, false));
    }
}
