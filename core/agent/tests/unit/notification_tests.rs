use super::*;
use crate::response_protocol::ResponseProtocolKind;

#[test]
fn notification_events_are_protocol_independent_core_data() {
    let suite = ResponseProtocolKind::Json.suite();
    let envelope = suite.parse(
            r#"{"status":"working","free_talk":"先说明一下我的判断。","working_still_action":[{"memmgr":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}},{"run_bash":{"cmd":"pwd"}},{"self_tool":{"type":"params"}}]}"#,
            &crate::capability::CapabilityRegistry::builtin_for_host(crate::capability::CapabilityHostProfile::with_local_command_execution()),
        );
    let events = notifications_from_envelope(&envelope);
    let action_ids = events
        .iter()
        .filter_map(|event| match event {
            CoreNotification::Action { action_id, .. } => Some(action_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(action_ids.len(), 3);
    assert!(action_ids.iter().all(|action_id| !action_id.is_empty()));
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
                action_id: action_ids[0].clone(),
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
                action_id: action_ids[1].clone(),
                input: serde_json::json!({
                    "cmd": "pwd"
                }),
                kind: CoreActionKind::Bash {
                    command: "pwd".to_string(),
                    mode: "normal".to_string(),
                    interval_ms: None,
                    timeout_ms: Some(5000),
                    loop_timeout_ms: None,
                    once_timeout_ms: None,
                },
                active: true,
                memory_activity: CoreMemoryActivity::None,
            },
            CoreNotification::Action {
                action: "self_tool".to_string(),
                action_id: action_ids[2].clone(),
                input: serde_json::json!({"type": "params"}),
                kind: CoreActionKind::SelfTool {
                    self_type: "params".to_string(),
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
            r#"{"free_talk":"检查后台工具任务。","working_still_action":[{"capmgr":{"op":"job_status","job_id":"tool_job_42","timeout_ms":1000}}]}"#,
            &crate::capability::CapabilityRegistry::builtin_for_host(crate::capability::CapabilityHostProfile::with_local_command_execution()),
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
            r#"{"free_talk":"checking","working_still_action":[[{"run_bash":{"cmd":"printf a","timeout_ms":5000}},{"run_bash":{"cmd":"printf b","timeout_ms":5000}}]]}"#,
            &crate::capability::CapabilityRegistry::builtin_for_host(crate::capability::CapabilityHostProfile::with_local_command_execution()),
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

#[test]
fn run_bash_notifications_publish_effective_wait_budgets() {
    let suite = ResponseProtocolKind::Json.suite();
    let envelope = suite.parse(
        r#"{"working_still_action":[{"run_bash":{"cmd":"sleep 10"}},{"run_bash":{"loop_cmd":"test -f done","interval_ms":1000}},{"run_bash":{"cmd":"long-task","background":true}}]}"#,
        &crate::capability::CapabilityRegistry::builtin_for_host(
            crate::capability::CapabilityHostProfile::with_local_command_execution(),
        ),
    );
    let events = notifications_from_envelope(&envelope);
    let bash_kinds = events
        .iter()
        .filter_map(|event| match event {
            CoreNotification::Action {
                kind:
                    CoreActionKind::Bash {
                        mode,
                        timeout_ms,
                        loop_timeout_ms,
                        once_timeout_ms,
                        ..
                    },
                ..
            } => Some((
                mode.as_str(),
                *timeout_ms,
                *loop_timeout_ms,
                *once_timeout_ms,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        bash_kinds,
        vec![
            ("normal", Some(5000), None, None),
            ("poll", None, Some(600_000), Some(5000)),
            ("background", None, None, None),
        ]
    );
}
