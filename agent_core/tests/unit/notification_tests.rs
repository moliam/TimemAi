use super::*;
use crate::response_protocol::ResponseProtocolKind;

#[test]
fn notification_events_are_protocol_independent_core_data() {
    let suite = ResponseProtocolKind::Json.suite();
    let envelope = suite.parse(
            r#"{"status":"working","free_talk":"先说明一下我的判断。","next_actions":[{"memmgr":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}},{"run_bash":{"cmd":"pwd"}},{"self_tool":{"type":"about_me","op":"read"}}]}"#,
            &crate::capability::CapabilityRegistry::builtin(),
        );
    let events = notifications_from_envelope(&envelope);
    assert_eq!(
        events,
        vec![
            CoreNotification::ModelResponse {
                status: "working".to_string(),
                free_talk: "先说明一下我的判断。".to_string(),
                final_answer: String::new(),
                continue_work: true,
            },
            CoreNotification::Action {
                action: "memmgr".to_string(),
                input: serde_json::json!({
                    "type": "durable",
                    "op": "sql",
                    "sql": "SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5",
                    "params": ["%project%"],
                    "limit": 5
                }),
                kind: CoreActionKind::Memory {
                    surface: "durable".to_string(),
                    operation: "sql".to_string(),
                },
                active: false,
                memory_activity: CoreMemoryActivity::Read,
            },
            CoreNotification::Action {
                action: "run_bash".to_string(),
                input: serde_json::json!({
                    "cmd": "pwd"
                }),
                kind: CoreActionKind::Bash {
                    command: "pwd".to_string(),
                    mode: "normal".to_string(),
                    interval_ms: None,
                    timeout_ms: None,
                    loop_timeout_ms: None,
                    once_timeout_ms: None,
                },
                active: true,
                memory_activity: CoreMemoryActivity::None,
            },
            CoreNotification::Action {
                action: "self_tool".to_string(),
                input: serde_json::json!({
                    "type": "about_me",
                    "op": "read"
                }),
                kind: CoreActionKind::SelfTool {
                    self_type: "about_me".to_string(),
                    op: "read".to_string(),
                },
                active: false,
                memory_activity: CoreMemoryActivity::None,
            },
        ]
    );
}

#[test]
fn capmgr_job_status_notification_uses_job_id_as_capability_id() {
    let suite = ResponseProtocolKind::Json.suite();
    let envelope = suite.parse(
            r#"{"free_talk":"检查后台工具任务。","next_actions":[{"capmgr":{"op":"job_status","job_id":"tool_job_42","timeout_ms":1000}}]}"#,
            &crate::capability::CapabilityRegistry::builtin(),
        );
    let events = notifications_from_envelope(&envelope);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CoreNotification::Action {
                kind: CoreActionKind::Capability { op, id, .. },
                active: true,
                ..
            } if op == "job_status" && id == "tool_job_42"
        )
    }));
}

#[test]
fn grouped_actions_emit_each_action_without_intent_metadata() {
    let suite = ResponseProtocolKind::Json.suite();
    let envelope = suite.parse(
            r#"{"free_talk":"checking","next_actions":[[{"run_bash":{"cmd":"printf a","timeout_ms":5000}},{"run_bash":{"cmd":"printf b","timeout_ms":5000}}]]}"#,
            &crate::capability::CapabilityRegistry::builtin(),
        );
    let events = notifications_from_envelope(&envelope);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CoreNotification::Action {
                kind: CoreActionKind::Bash { command, .. },
                ..
            } if command == "printf a"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CoreNotification::Action {
                kind: CoreActionKind::Bash { command, .. },
                ..
            } if command == "printf b"
        )
    }));
}
