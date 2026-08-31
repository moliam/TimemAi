use serde::{Deserialize, Serialize};
use serde_json::json;
use timem_ui_contract::message_fifo::{
    MessageQueueBlockReason, MessageQueueContinuation, MessageQueueItem, MessageQueueProjection,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Payload {
    text: String,
}

#[test]
fn serializes_complete_session_message_queue_projection() {
    let projection = MessageQueueProjection {
        revision: 7,
        items: vec![MessageQueueItem {
            command_id: "queued-1".to_string(),
            enqueue_seq: 3,
            payload: Payload {
                text: "next".to_string(),
            },
        }],
        auto_send_enabled: false,
        continuation: MessageQueueContinuation::Blocked {
            reason: MessageQueueBlockReason::SessionStopped,
        },
        dispatching_command_id: None,
    };
    let value = serde_json::to_value(&projection).unwrap();
    assert_eq!(
        value,
        json!({
            "revision": 7,
            "items": [{"command_id": "queued-1", "enqueue_seq": 3, "payload": {"text": "next"}}],
            "auto_send_enabled": false,
            "continuation": {"state": "blocked", "reason": "session_stopped"}
        })
    );
    assert_eq!(
        serde_json::from_value::<MessageQueueProjection<Payload>>(value).unwrap(),
        projection
    );
}

#[test]
fn defaults_enabled_without_continuation_permission() {
    let projection = MessageQueueProjection::<Payload>::default();
    assert!(projection.auto_send_enabled);
    assert_eq!(
        projection.continuation,
        MessageQueueContinuation::AwaitingNormalCompletion
    );
    assert!(projection.items.is_empty());
    assert!(projection.dispatching_command_id.is_none());
}
