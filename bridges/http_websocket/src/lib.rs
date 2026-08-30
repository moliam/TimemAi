//! HTTP/WebSocket delivery state for authoritative Core projections and commands.
//!
//! This crate owns only communication concerns such as monotonic revisions,
//! duplicate/stale delivery rejection, bounded queues, and reconnect snapshots.
//! It does not own or reinterpret Turn lifecycle semantics.

use serde::{Deserialize, Serialize, Serializer};
use timem_ui_contract::projections::{TurnProjection, TurnToken};

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
    use timem_ui_contract::projections::{
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
    fn newer_core_epoch_replaces_finished_projection_without_bridge_lifecycle() {
        let mut cache = TurnProjectionCache::default();
        cache.apply(finished("a", 1));
        assert_eq!(
            cache.apply(active("b", 2, false)),
            ProjectionApplyResult::Applied { revision: 2 }
        );
        assert_eq!(cache.current().unwrap().projection, active("b", 2, false));
    }

    #[test]
    fn next_turn_intents_are_bounded_fifo_deduplicated_and_revisioned() {
        let mut queue = NextTurnIntentQueue::new(2);
        assert_eq!(
            queue.enqueue("command-a".to_string(), "A".to_string()),
            NextTurnEnqueueResult::Enqueued {
                enqueue_seq: 1,
                revision: 1
            }
        );
        assert_eq!(
            queue.enqueue("command-a".to_string(), "duplicate".to_string()),
            NextTurnEnqueueResult::Duplicate {
                enqueue_seq: 1,
                revision: 1
            }
        );
        assert_eq!(
            queue.enqueue("command-b".to_string(), "B".to_string()),
            NextTurnEnqueueResult::Enqueued {
                enqueue_seq: 2,
                revision: 2
            }
        );
        assert_eq!(
            queue.enqueue("command-c".to_string(), "C".to_string()),
            NextTurnEnqueueResult::Full {
                capacity: 2,
                revision: 2
            }
        );
        let (first, revision) = queue.pop_front().unwrap();
        assert_eq!(
            (first.command_id.as_str(), first.payload.as_str()),
            ("command-a", "A")
        );
        assert_eq!(revision, 3);
        assert_eq!(queue.front().unwrap().command_id, "command-b");
    }

    #[test]
    fn next_turn_queue_round_trip_preserves_order_and_monotonic_sequence() {
        let mut queue = NextTurnIntentQueue::new(3);
        queue.enqueue("command-a".to_string(), 10u32);
        queue.enqueue("command-b".to_string(), 20u32);
        let encoded = serde_json::to_vec(&queue).unwrap();
        let mut restored: NextTurnIntentQueue<u32> = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.snapshot(), queue.snapshot());
        restored.pop_front();
        assert_eq!(
            restored.enqueue("command-c".to_string(), 30),
            NextTurnEnqueueResult::Enqueued {
                enqueue_seq: 3,
                revision: 4
            }
        );
    }

    #[test]
    fn removing_a_queued_intent_advances_revision_and_is_idempotent() {
        let mut queue = NextTurnIntentQueue::new(2);
        queue.enqueue("command-a".to_string(), "A".to_string());
        assert!(matches!(
            queue.remove("command-a"),
            NextTurnRemoveResult::Removed { revision: 2, .. }
        ));
        assert!(matches!(
            queue.remove("command-a"),
            NextTurnRemoveResult::Missing { revision: 2 }
        ));
    }
}

/// Input accepted for delivery to a future Core Turn. This is communication state,
/// not a Turn lifecycle state, and therefore carries no Turn token or epoch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NextTurnIntent<T> {
    pub command_id: String,
    pub enqueue_seq: u64,
    pub payload: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionedNextTurnIntents<T> {
    pub revision: u64,
    pub items: Vec<NextTurnIntent<T>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextTurnEnqueueResult {
    Enqueued { enqueue_seq: u64, revision: u64 },
    Duplicate { enqueue_seq: u64, revision: u64 },
    Full { capacity: usize, revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextTurnRemoveResult<T> {
    Removed {
        intent: NextTurnIntent<T>,
        revision: u64,
    },
    Missing {
        revision: u64,
    },
}

/// Bounded FIFO HTTP/WebSocket delivery storage for future-turn commands.
/// It never dispatches by itself and cannot change Core lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NextTurnIntentQueue<T> {
    capacity: usize,
    revision: u64,
    next_enqueue_seq: u64,
    items: Vec<NextTurnIntent<T>>,
}

impl<T> NextTurnIntentQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            revision: 0,
            next_enqueue_seq: 1,
            items: Vec::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn front(&self) -> Option<&NextTurnIntent<T>> {
        self.items.first()
    }

    pub fn snapshot(&self) -> VersionedNextTurnIntents<T>
    where
        T: Clone,
    {
        VersionedNextTurnIntents {
            revision: self.revision,
            items: self.items.clone(),
        }
    }

    pub fn enqueue(&mut self, command_id: String, payload: T) -> NextTurnEnqueueResult {
        if let Some(existing) = self
            .items
            .iter()
            .find(|intent| intent.command_id == command_id)
        {
            return NextTurnEnqueueResult::Duplicate {
                enqueue_seq: existing.enqueue_seq,
                revision: self.revision,
            };
        }
        if self.items.len() >= self.capacity {
            return NextTurnEnqueueResult::Full {
                capacity: self.capacity,
                revision: self.revision,
            };
        }
        let enqueue_seq = self.next_enqueue_seq;
        self.next_enqueue_seq = self.next_enqueue_seq.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
        self.items.push(NextTurnIntent {
            command_id,
            enqueue_seq,
            payload,
        });
        NextTurnEnqueueResult::Enqueued {
            enqueue_seq,
            revision: self.revision,
        }
    }

    pub fn pop_front(&mut self) -> Option<(NextTurnIntent<T>, u64)> {
        if self.items.is_empty() {
            return None;
        }
        let intent = self.items.remove(0);
        self.revision = self.revision.saturating_add(1);
        Some((intent, self.revision))
    }

    pub fn remove(&mut self, command_id: &str) -> NextTurnRemoveResult<T> {
        let Some(index) = self
            .items
            .iter()
            .position(|intent| intent.command_id == command_id)
        else {
            return NextTurnRemoveResult::Missing {
                revision: self.revision,
            };
        };
        let intent = self.items.remove(index);
        self.revision = self.revision.saturating_add(1);
        NextTurnRemoveResult::Removed {
            intent,
            revision: self.revision,
        }
    }
}
