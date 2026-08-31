use crate::message_queue::{SessionMessageQueue, SessionMessageQueueError};
use timem_ui_contract::message_fifo::{
    MessageQueueBlockReason, MessageQueueContinuation, MessageQueueItem, MessageQueueProjection,
};

fn ids<'a>(queue: &'a SessionMessageQueue<&'static str>) -> Vec<&'a str> {
    queue
        .projection()
        .items
        .iter()
        .map(|item| item.command_id.as_str())
        .collect()
}

#[test]
fn owns_fifo_edit_delete_reorder_and_capacity() {
    let mut queue = SessionMessageQueue::new(3);
    queue.enqueue("a", "A").unwrap();
    queue.enqueue("b", "B").unwrap();
    queue.enqueue("c", "C").unwrap();
    assert_eq!(ids(&queue), vec!["a", "b", "c"]);
    queue.update_payload("b", "B2").unwrap();
    queue
        .reorder(&["c".into(), "b".into(), "a".into()])
        .unwrap();
    assert_eq!(ids(&queue), vec!["c", "b", "a"]);
    assert_eq!(queue.remove("b").unwrap().payload, "B2");
    queue.enqueue("d", "D").unwrap();
    assert_eq!(
        queue.enqueue("e", "E"),
        Err(SessionMessageQueueError::CapacityReached { capacity: 3 })
    );
    assert_eq!(
        queue.enqueue("a", "duplicate"),
        Err(SessionMessageQueueError::DuplicateCommandId {
            command_id: "a".into()
        })
    );
}

#[test]
fn invalid_reorder_is_atomic() {
    let mut queue = SessionMessageQueue::new(3);
    queue.enqueue("a", "A").unwrap();
    queue.enqueue("b", "B").unwrap();
    queue.enqueue("c", "C").unwrap();
    assert_eq!(
        queue.reorder(&["a".into(), "missing".into(), "c".into()]),
        Err(SessionMessageQueueError::InvalidOrder)
    );
    assert_eq!(ids(&queue), vec!["a", "b", "c"]);
}

#[test]
fn enabling_auto_send_does_not_create_permission() {
    let mut queue = SessionMessageQueue::new(4);
    queue.enqueue("a", "A").unwrap();
    queue.set_auto_send_enabled(false);
    assert!(queue.begin_automatic_dispatch().is_none());
    queue.set_auto_send_enabled(true);
    assert_eq!(
        queue.projection().continuation,
        MessageQueueContinuation::AwaitingNormalCompletion
    );
    assert!(queue.begin_automatic_dispatch().is_none());
}

#[test]
fn one_normal_completion_grants_exactly_one_started_turn() {
    let mut queue = SessionMessageQueue::new(4);
    queue.enqueue("a", "A").unwrap();
    queue.enqueue("b", "B").unwrap();
    queue.grant_after_normal_completion();
    assert_eq!(queue.begin_automatic_dispatch().unwrap().command_id, "a");
    assert!(queue.begin_automatic_dispatch().is_none());
    assert_eq!(ids(&queue), vec!["a", "b"]);
    assert_eq!(queue.confirm_turn_started("a").unwrap().payload, "A");
    assert_eq!(ids(&queue), vec!["b"]);
    assert!(queue.begin_automatic_dispatch().is_none());
}

#[test]
fn all_non_normal_outcomes_block_and_preserve_queue_and_switch() {
    for reason in [
        MessageQueueBlockReason::UserCancelled,
        MessageQueueBlockReason::TurnFailed,
        MessageQueueBlockReason::TurnInterrupted,
        MessageQueueBlockReason::SessionStopped,
    ] {
        let mut queue = SessionMessageQueue::new(4);
        queue.enqueue("a", "A").unwrap();
        queue.set_auto_send_enabled(false);
        queue.block_continuation(reason.clone());
        assert_eq!(ids(&queue), vec!["a"]);
        assert!(!queue.projection().auto_send_enabled);
        assert_eq!(
            queue.projection().continuation,
            MessageQueueContinuation::Blocked { reason }
        );
        assert!(queue.begin_automatic_dispatch().is_none());
    }
}

#[test]
fn dispatch_stays_visible_and_locked_until_authoritative_start() {
    let mut queue = SessionMessageQueue::new(4);
    queue.enqueue("a", "A").unwrap();
    queue.grant_after_normal_completion();
    queue.begin_automatic_dispatch().unwrap();
    assert_eq!(ids(&queue), vec!["a"]);
    assert_eq!(
        queue.projection().dispatching_command_id.as_deref(),
        Some("a")
    );
    assert!(matches!(
        queue.update_payload("a", "changed"),
        Err(SessionMessageQueueError::DispatchInProgress { .. })
    ));
    assert!(queue.reject_dispatch("a"));
    assert_eq!(
        queue.projection().continuation,
        MessageQueueContinuation::Granted
    );
    queue.begin_automatic_dispatch().unwrap();
    assert!(queue.confirm_turn_started("a").is_some());
    assert!(queue.projection().items.is_empty());
}

#[test]
fn immediate_dispatch_also_waits_for_authoritative_start() {
    let mut queue = SessionMessageQueue::new(4);
    queue.enqueue("a", "A").unwrap();
    queue.enqueue("b", "B").unwrap();
    assert_eq!(queue.begin_immediate_dispatch("b").unwrap().command_id, "b");
    assert_eq!(ids(&queue), vec!["a", "b"]);
    assert_eq!(
        queue.projection().dispatching_command_id.as_deref(),
        Some("b")
    );
    assert!(queue.confirm_turn_started("b").is_some());
    assert_eq!(ids(&queue), vec!["a"]);
}

#[test]
fn restore_keeps_projection_and_monotonic_sequence() {
    let projection = MessageQueueProjection {
        revision: 9,
        items: vec![MessageQueueItem {
            command_id: "old".into(),
            enqueue_seq: 41,
            payload: "old payload",
        }],
        auto_send_enabled: false,
        continuation: MessageQueueContinuation::Blocked {
            reason: MessageQueueBlockReason::SessionStopped,
        },
        dispatching_command_id: None,
    };
    let mut queue = SessionMessageQueue::restore(projection, 4);
    queue.enqueue("new", "new payload").unwrap();
    assert_eq!(queue.projection().items[1].enqueue_seq, 42);
    assert!(!queue.projection().auto_send_enabled);
}
