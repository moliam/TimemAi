use super::*;
use serde_json::json;

fn payload(turn: &str, attempt: u64, revision: u64, text: Option<&str>) -> Value {
    json!({
        "turn_id": turn,
        "attempt": attempt,
        "revision": revision,
        "interruption": null,
        "response": text.map(|text| json!({
            "attempt": attempt,
            "revision": revision,
            "text": text,
            "status": "streaming"
        }))
    })
}

#[test]
fn preview_is_a_revisioned_replacement_not_an_append_stream() {
    let mut state = StreamPreviewState::default();
    assert_eq!(
        state.apply_payload(&payload("turn-1", 1, 1, Some("hel"))),
        StreamPreviewUpdate::Changed
    );
    assert_eq!(state.visible_text(), Some("hel"));
    assert_eq!(
        state.apply_payload(&payload("turn-1", 1, 2, Some("hello"))),
        StreamPreviewUpdate::Changed
    );
    assert_eq!(state.visible_text(), Some("hello"));
    assert_eq!(
        state.apply_payload(&payload("turn-1", 1, 1, Some("stale"))),
        StreamPreviewUpdate::Ignored
    );
    assert_eq!(state.visible_text(), Some("hello"));
}

#[test]
fn retry_can_keep_prior_text_until_the_new_attempt_replaces_it() {
    let mut state = StreamPreviewState::default();
    state.apply_payload(&payload("turn-1", 1, 3, Some("prior attempt")));
    let interrupted = json!({
        "turn_id": "turn-1", "attempt": 2, "revision": 4,
        "interruption": "network_error",
        "response": {"attempt": 1, "revision": 3, "text": "prior attempt", "status": "intermediate"}
    });
    assert_eq!(
        state.apply_payload(&interrupted),
        StreamPreviewUpdate::Changed
    );
    assert_eq!(state.visible_text(), Some("prior attempt"));
    assert_eq!(state.snapshot().unwrap().attempt, 2);
    assert_eq!(
        state.snapshot().unwrap().interruption.as_deref(),
        Some("network_error")
    );
    state.apply_payload(&payload("turn-1", 2, 5, Some("replacement")));
    assert_eq!(state.visible_text(), Some("replacement"));
}

#[test]
fn null_response_retracts_without_implying_turn_completion() {
    let mut state = StreamPreviewState::default();
    state.apply_payload(&payload("turn-1", 1, 1, Some("temporary")));
    assert_eq!(
        state.apply_payload(&payload("turn-1", 1, 2, None)),
        StreamPreviewUpdate::Retracted
    );
    assert_eq!(state.visible_text(), None);
    assert!(
        state.snapshot().is_some(),
        "revision remains presentation state only"
    );
}

#[test]
fn snapshot_and_live_core_topic_share_the_same_projection_path() {
    let mut state = StreamPreviewState::default();
    let first = payload("turn-1", 1, 7, Some("snapshot"));
    state.apply_payload(&first);
    let event = json!({
        "topic": {"name": "core.model.preview"},
        "payload": payload("turn-1", 1, 8, Some("live"))
    });
    assert_eq!(state.apply_core_topic(&event), StreamPreviewUpdate::Changed);
    assert_eq!(state.visible_text(), Some("live"));
    assert_eq!(
        state.apply_topic(
            CORE_TOPIC_MODEL_PREVIEW,
            &payload("turn-1", 1, 9, Some("structured")),
        ),
        StreamPreviewUpdate::Changed
    );
    assert_eq!(state.visible_text(), Some("structured"));
    assert_eq!(
        state.apply_core_topic(&json!({"topic":{"name":"core.action"},"payload":{}})),
        StreamPreviewUpdate::Ignored
    );
}

#[test]
fn different_turn_resets_revision_scope_and_render_is_provisional() {
    let mut state = StreamPreviewState::default();
    state.apply_payload(&payload("turn-old", 3, 99, Some("old")));
    assert_eq!(
        state.apply_payload(&payload("turn-new", 1, 1, Some("new"))),
        StreamPreviewUpdate::Changed
    );
    let snapshot = state.snapshot().unwrap();
    assert_eq!(snapshot.turn_id, "turn-new");
    assert_eq!(render_stream_preview(snapshot), "new");
}

fn tool_action(action_id: &str, event: &str, status: &str, active: bool) -> StreamToolAction {
    StreamToolAction {
        action: "run_bash".to_string(),
        action_id: action_id.to_string(),
        input: json!({"cmd":"cargo test"}),
        event: event.to_string(),
        status: status.to_string(),
        active,
        pid: None,
    }
}

#[test]
fn active_tools_stay_visible_then_fold_into_success_counts() {
    let mut state = StreamToolFoldState::default();
    assert_eq!(
        state.apply(&tool_action("a-1", "execution_start", "running", true)),
        StreamToolFoldUpdate::Changed
    );
    let running = render_stream_tool_fold(&state);
    assert!(running.contains("Run command"));
    assert!(running.contains("cargo test"));
    assert!(!running.contains("Tools folded"));

    let mut background = tool_action("a-1", "finish", "background_running", true);
    background.pid = Some(42);
    assert_eq!(state.apply(&background), StreamToolFoldUpdate::Changed);
    assert!(render_stream_tool_fold(&state).contains("pid 42"));

    assert_eq!(
        state.apply(&tool_action("a-1", "finish", "completed", false)),
        StreamToolFoldUpdate::Changed
    );
    assert_eq!(state.active_count(), 0);
    assert_eq!(state.completed_counts(), (1, 0));
    let folded = render_stream_tool_fold(&state);
    assert!(folded.contains("Tools folded"));
    assert!(folded.contains("✓ 1"));
    assert!(!folded.contains("Run command"));
}

#[test]
fn terminal_tools_count_once_and_classify_failures() {
    let mut state = StreamToolFoldState::default();
    let failed = tool_action("a-2", "finish", "timeout", false);
    assert_eq!(state.apply(&failed), StreamToolFoldUpdate::Changed);
    assert_eq!(state.apply(&failed), StreamToolFoldUpdate::Ignored);
    assert_eq!(state.completed_counts(), (0, 1));
    assert!(render_stream_tool_fold(&state).contains("× 1"));
    assert_eq!(
        state.apply(&tool_action("a-2", "start", "running", true)),
        StreamToolFoldUpdate::Ignored,
        "an authoritative terminal id cannot reopen"
    );
}

#[test]
fn missing_ids_correlate_lifecycle_but_allow_a_new_start_after_terminal() {
    let mut state = StreamToolFoldState::default();
    let running = tool_action("", "execution_start", "running", true);
    let completed = tool_action("", "finish", "completed", false);
    assert_eq!(state.apply(&running), StreamToolFoldUpdate::Changed);
    assert_eq!(state.apply(&completed), StreamToolFoldUpdate::Changed);
    assert_eq!(state.apply(&completed), StreamToolFoldUpdate::Ignored);
    assert_eq!(state.completed_counts(), (1, 0));
    assert_eq!(state.apply(&running), StreamToolFoldUpdate::Changed);
    assert_eq!(state.active_count(), 1);
}

#[test]
fn non_action_and_turn_terminal_markers_do_not_enter_tool_projection() {
    let mut state = StreamToolFoldState::default();
    assert_eq!(
        state.apply_json_topic(&json!({
            "topic":{"name":"core.model.response"},
            "payload":{"action":"run_bash"}
        })),
        StreamToolFoldUpdate::Ignored
    );
    assert_eq!(
        state.apply_json_topic(&json!({
            "topic":{"name":"core.action"},
            "payload":{"action":"task_finished","status":"completed"}
        })),
        StreamToolFoldUpdate::Ignored
    );
    assert!(state.is_empty());
}
