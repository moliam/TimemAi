use serde::{Serialize, Serializer};
pub use timem_ui_contract::message_fifo::{
    MessageQueueBlockReason, MessageQueueContinuation, MessageQueueItem, MessageQueueProjection,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionMessageQueueError {
    CapacityReached { capacity: usize },
    DuplicateCommandId { command_id: String },
    UnknownCommandId { command_id: String },
    DispatchInProgress { command_id: String },
    InvalidOrder,
}

/// Session-owned state machine for messages accepted for a future Core turn.
/// Interfaces consume its projection; transports must not recreate transitions.
#[derive(Debug, Clone)]
pub struct SessionMessageQueue<T> {
    projection: MessageQueueProjection<T>,
    capacity: usize,
    next_enqueue_seq: u64,
}

impl<T> SessionMessageQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            projection: MessageQueueProjection::default(),
            capacity,
            next_enqueue_seq: 0,
        }
    }

    pub fn restore(projection: MessageQueueProjection<T>, capacity: usize) -> Self {
        let next_enqueue_seq = projection
            .items
            .iter()
            .map(|item| item.enqueue_seq)
            .max()
            .map_or(0, |seq| seq.saturating_add(1));
        Self {
            projection,
            capacity,
            next_enqueue_seq,
        }
    }

    pub fn projection(&self) -> &MessageQueueProjection<T> {
        &self.projection
    }
    pub fn into_projection(self) -> MessageQueueProjection<T> {
        self.projection
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn is_empty(&self) -> bool {
        self.projection.items.is_empty()
    }
    pub fn len(&self) -> usize {
        self.projection.items.len()
    }
    pub fn item(&self, command_id: &str) -> Option<&MessageQueueItem<T>> {
        self.projection
            .items
            .iter()
            .find(|item| item.command_id == command_id)
    }

    pub fn enqueue(
        &mut self,
        command_id: impl Into<String>,
        payload: T,
    ) -> Result<(), SessionMessageQueueError> {
        let command_id = command_id.into();
        if self
            .projection
            .items
            .iter()
            .any(|item| item.command_id == command_id)
        {
            return Err(SessionMessageQueueError::DuplicateCommandId { command_id });
        }
        if self.projection.items.len() >= self.capacity {
            return Err(SessionMessageQueueError::CapacityReached {
                capacity: self.capacity,
            });
        }
        let enqueue_seq = self.next_enqueue_seq;
        self.next_enqueue_seq = self.next_enqueue_seq.saturating_add(1);
        self.projection.items.push(MessageQueueItem {
            command_id,
            enqueue_seq,
            payload,
        });
        self.bump_revision();
        Ok(())
    }

    pub fn update_payload(
        &mut self,
        command_id: &str,
        payload: T,
    ) -> Result<(), SessionMessageQueueError> {
        self.ensure_not_dispatching(command_id)?;
        let index = self.item_index(command_id)?;
        self.projection.items[index].payload = payload;
        self.bump_revision();
        Ok(())
    }

    pub fn remove(
        &mut self,
        command_id: &str,
    ) -> Result<MessageQueueItem<T>, SessionMessageQueueError> {
        self.ensure_not_dispatching(command_id)?;
        let index = self.item_index(command_id)?;
        let item = self.projection.items.remove(index);
        self.bump_revision();
        Ok(item)
    }

    /// The order must contain every queued command exactly once.
    pub fn reorder(&mut self, command_ids: &[String]) -> Result<(), SessionMessageQueueError> {
        if let Some(command_id) = &self.projection.dispatching_command_id {
            return Err(SessionMessageQueueError::DispatchInProgress {
                command_id: command_id.clone(),
            });
        }
        if command_ids.len() != self.projection.items.len() {
            return Err(SessionMessageQueueError::InvalidOrder);
        }
        let mut indexes = Vec::with_capacity(command_ids.len());
        for command_id in command_ids {
            let Some(index) = self
                .projection
                .items
                .iter()
                .position(|item| item.command_id == *command_id)
            else {
                return Err(SessionMessageQueueError::InvalidOrder);
            };
            if indexes.contains(&index) {
                return Err(SessionMessageQueueError::InvalidOrder);
            }
            indexes.push(index);
        }
        let mut old: Vec<Option<MessageQueueItem<T>>> = std::mem::take(&mut self.projection.items)
            .into_iter()
            .map(Some)
            .collect();
        self.projection.items = indexes
            .into_iter()
            .map(|index| old[index].take().expect("validated unique queue index"))
            .collect();
        self.bump_revision();
        Ok(())
    }

    /// Changes only the persistent user preference; enabling never grants dispatch.
    pub fn set_auto_send_enabled(&mut self, enabled: bool) {
        if self.projection.auto_send_enabled != enabled {
            self.projection.auto_send_enabled = enabled;
            self.bump_revision();
        }
    }

    /// The sole transition that grants one automatic continuation.
    pub fn grant_after_normal_completion(&mut self) {
        self.projection.continuation = MessageQueueContinuation::Granted;
        self.bump_revision();
    }

    pub fn block_continuation(&mut self, reason: MessageQueueBlockReason) {
        self.projection.continuation = MessageQueueContinuation::Blocked { reason };
        self.bump_revision();
    }

    /// Reserves the front item and consumes the grant. The item remains visible
    /// until Core authoritatively reports TurnStarted.
    pub fn begin_automatic_dispatch(&mut self) -> Option<&MessageQueueItem<T>> {
        if !self.projection.auto_send_enabled
            || self.projection.dispatching_command_id.is_some()
            || self.projection.continuation != MessageQueueContinuation::Granted
        {
            return None;
        }
        let command_id = self.projection.items.first()?.command_id.clone();
        self.projection.dispatching_command_id = Some(command_id);
        self.projection.continuation = MessageQueueContinuation::AwaitingNormalCompletion;
        self.bump_revision();
        self.projection.items.first()
    }

    /// A rejected command did not start a turn, so the grant may be retried.
    pub fn reject_dispatch(&mut self, command_id: &str) -> bool {
        if self.projection.dispatching_command_id.as_deref() != Some(command_id) {
            return false;
        }
        self.projection.dispatching_command_id = None;
        self.projection.continuation = MessageQueueContinuation::Granted;
        self.bump_revision();
        true
    }

    pub fn confirm_turn_started(&mut self, command_id: &str) -> Option<MessageQueueItem<T>> {
        if self.projection.dispatching_command_id.as_deref() != Some(command_id) {
            return None;
        }
        let index = self
            .projection
            .items
            .iter()
            .position(|item| item.command_id == command_id)?;
        self.projection.dispatching_command_id = None;
        let item = self.projection.items.remove(index);
        self.bump_revision();
        Some(item)
    }

    /// Explicit send-now reserves the selected item without requiring or
    /// consuming an automatic-continuation grant. It remains visible until
    /// authoritative TurnStarted, exactly like an automatic dispatch.
    pub fn begin_immediate_dispatch(
        &mut self,
        command_id: &str,
    ) -> Result<&MessageQueueItem<T>, SessionMessageQueueError> {
        if let Some(dispatching_command_id) = &self.projection.dispatching_command_id {
            return Err(SessionMessageQueueError::DispatchInProgress {
                command_id: dispatching_command_id.clone(),
            });
        }
        let index = self.item_index(command_id)?;
        self.projection.dispatching_command_id = Some(command_id.to_string());
        self.bump_revision();
        Ok(&self.projection.items[index])
    }

    fn item_index(&self, command_id: &str) -> Result<usize, SessionMessageQueueError> {
        self.projection
            .items
            .iter()
            .position(|item| item.command_id == command_id)
            .ok_or_else(|| SessionMessageQueueError::UnknownCommandId {
                command_id: command_id.to_string(),
            })
    }

    fn ensure_not_dispatching(&self, command_id: &str) -> Result<(), SessionMessageQueueError> {
        if self.projection.dispatching_command_id.as_deref() == Some(command_id) {
            return Err(SessionMessageQueueError::DispatchInProgress {
                command_id: command_id.to_string(),
            });
        }
        Ok(())
    }

    fn bump_revision(&mut self) {
        self.projection.revision = self.projection.revision.saturating_add(1);
    }
}

impl<T: Serialize> Serialize for SessionMessageQueue<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.projection.serialize(serializer)
    }
}

impl<T> Default for SessionMessageQueue<T> {
    fn default() -> Self {
        Self::new(usize::MAX)
    }
}
