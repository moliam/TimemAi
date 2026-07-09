use crate::response_protocol::{ParsedAction, ParsedEnvelope};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreMemoryActivity {
    None,
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreActionKind {
    Bash {
        command: String,
        mode: String,
        interval_ms: Option<u64>,
        timeout_ms: Option<i64>,
        loop_timeout_ms: Option<i64>,
        once_timeout_ms: Option<u64>,
    },
    ShellJob {
        job_id: String,
    },
    Memory {
        surface: String,
        operation: String,
    },
    Capability {
        op: String,
        #[serde(rename = "capability_kind")]
        kind: String,
        id: String,
    },
    SelfTool {
        self_type: String,
        op: String,
    },
    ChatHistory {
        operation: String,
    },
    Other {
        action: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreNotification {
    ModelResponse {
        status: String,
        free_talk: String,
        final_answer: String,
        continue_work: bool,
    },
    Action {
        action: String,
        input: Value,
        kind: CoreActionKind,
        active: bool,
        memory_activity: CoreMemoryActivity,
    },
}

fn action_memory_activity(action: &ParsedAction) -> CoreMemoryActivity {
    match action.action.as_str() {
        "memmgr" => memmgr_memory_activity(action),
        _ => CoreMemoryActivity::None,
    }
}

fn action_active(action: &ParsedAction) -> bool {
    action.action == "run_bash"
        || (action.action == "capmgr" && action.input_lower("op") == "job_status")
}

fn memmgr_memory_activity(action: &ParsedAction) -> CoreMemoryActivity {
    let mem_type = action.input_str("type");
    let op = action.input_str("op");
    match (mem_type.as_str(), op.as_str()) {
        ("durable", "schema" | "sql") => CoreMemoryActivity::Read,
        ("durable", _) => CoreMemoryActivity::Write,
        _ => CoreMemoryActivity::None,
    }
}

pub fn notifications_from_envelope(envelope: &ParsedEnvelope) -> Vec<CoreNotification> {
    let mut events = Vec::new();
    events.push(CoreNotification::ModelResponse {
        status: if envelope.continue_work {
            "working".to_string()
        } else {
            "finished".to_string()
        },
        free_talk: envelope.thought.trim().to_string(),
        final_answer: envelope.final_answer.trim().to_string(),
        continue_work: envelope.continue_work,
    });
    events.extend(envelope.next_actions.iter().map(notification_from_action));
    events
}

pub fn notification_from_action(action: &ParsedAction) -> CoreNotification {
    CoreNotification::Action {
        action: action.action.clone(),
        input: action.raw_input.clone(),
        kind: action_kind(action),
        active: action_active(action),
        memory_activity: action_memory_activity(action),
    }
}

fn action_kind(action: &ParsedAction) -> CoreActionKind {
    match action.action.as_str() {
        "run_bash" => {
            let interval_ms = action.input_u64("interval_ms");
            let loop_command = action.input_str("loop_cmd");
            let command = if loop_command.is_empty() {
                action.input_str("cmd")
            } else {
                loop_command
            };
            CoreActionKind::Bash {
                command,
                mode: if interval_ms.is_some() {
                    "poll".to_string()
                } else if action.background() {
                    "background".to_string()
                } else {
                    "normal".to_string()
                },
                interval_ms,
                timeout_ms: if interval_ms.is_some() {
                    None
                } else {
                    action.input_i64("timeout_ms")
                },
                loop_timeout_ms: interval_ms.and_then(|_| action.input_i64("loop_timeout_ms")),
                once_timeout_ms: interval_ms.and_then(|_| action.input_u64("once_timeout_ms")),
            }
        }
        "memmgr" => CoreActionKind::Memory {
            surface: action.input_str("type"),
            operation: action.input_str("op"),
        },
        "capmgr" => CoreActionKind::Capability {
            op: action.input_str("op"),
            kind: action.input_str("kind"),
            id: if matches!(
                action.input_lower("op").as_str(),
                "job_status" | "job_cancel"
            ) {
                action.input_str("job_id")
            } else {
                action.input_str("id")
            },
        },
        "self_tool" => CoreActionKind::SelfTool {
            self_type: action.input_str("type"),
            op: action.input_str("op"),
        },
        "chat_history_query" => CoreActionKind::ChatHistory {
            operation: "query".to_string(),
        },
        other => CoreActionKind::Other {
            action: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response_protocol::ResponseProtocolKind;

    #[test]
    fn notification_events_are_protocol_independent_core_data() {
        let suite = ResponseProtocolKind::Json.suite();
        let envelope = suite.parse(
            r#"{"status":"working","free_talk":"先说明一下我的判断。","next_actions":[{"action":"memmgr","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}},{"action":"run_bash","args":{"cmd":"pwd"}},{"action":"self_tool","args":{"type":"about_me","op":"read"}}]}"#,
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
            r#"{"free_talk":"检查后台工具任务。","next_actions":[{"action":"capmgr","args":{"op":"job_status","job_id":"tool_job_42","timeout_ms":1000}}]}"#,
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
            r#"{"free_talk":"checking","next_actions":[{"order":"parallel","actions":[{"action":"run_bash","args":{"cmd":"printf a","timeout_ms":5000}},{"action":"run_bash","args":{"cmd":"printf b","timeout_ms":5000}}]}]}"#,
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
}
