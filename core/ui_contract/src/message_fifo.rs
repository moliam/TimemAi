use serde::{Deserialize, Serialize};

/// A future user message accepted by the Host but not yet started as a Core Turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageQueueItem<T> {
    pub command_id: String,
    pub enqueue_seq: u64,
    pub payload: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageQueueBlockReason {
    UserCancelled,
    TurnFailed,
    TurnInterrupted,
    SessionStopped,
}

/// One-shot automatic continuation gate. It is separate from the persistent
/// user switch and from Session/Worker lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MessageQueueContinuation {
    AwaitingNormalCompletion,
    Granted,
    Blocked { reason: MessageQueueBlockReason },
}

/// Complete Session-level projection for future-message behavior.
/// This crate defines the data shape only; Session/Core orchestration owns transitions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageQueueProjection<T> {
    pub revision: u64,
    pub items: Vec<MessageQueueItem<T>>,
    pub auto_send_enabled: bool,
    pub continuation: MessageQueueContinuation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatching_command_id: Option<String>,
}

impl<T> Default for MessageQueueProjection<T> {
    fn default() -> Self {
        Self {
            revision: 0,
            items: Vec::new(),
            auto_send_enabled: true,
            continuation: MessageQueueContinuation::AwaitingNormalCompletion,
            dispatching_command_id: None,
        }
    }
}
