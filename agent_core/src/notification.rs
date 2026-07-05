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
        report_job_progress: String,
        final_answer: String,
        continue_work: bool,
    },
    Action {
        intent: Option<String>,
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
    matches!(action.action.as_str(), "run_bash" | "shell_job_status")
}

fn memmgr_memory_activity(action: &ParsedAction) -> CoreMemoryActivity {
    let mem_type = action.input_str("type");
    let op = action.input_str("op");
    match (mem_type.as_str(), op.as_str()) {
        ("durable", "query" | "schema" | "sql") => CoreMemoryActivity::Read,
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
        report_job_progress: if envelope.continue_work {
            envelope.report_job_progress.trim().to_string()
        } else {
            String::new()
        },
        final_answer: envelope.final_answer.trim().to_string(),
        continue_work: envelope.continue_work,
    });
    events.extend(envelope.next_actions.iter().map(notification_from_action));
    events
}

pub fn notification_from_action(action: &ParsedAction) -> CoreNotification {
    let intent = (!action.intent.trim().is_empty()).then(|| action.intent.trim().to_string());
    CoreNotification::Action {
        intent,
        action: action.action.clone(),
        input: action.raw_input.clone(),
        kind: action_kind(action),
        active: action_active(action),
        memory_activity: action_memory_activity(action),
    }
}

fn action_kind(action: &ParsedAction) -> CoreActionKind {
    match action.action.as_str() {
        "run_bash" => CoreActionKind::Bash {
            command: action.input_str("command"),
        },
        "shell_job_status" => CoreActionKind::ShellJob {
            job_id: action.input_str("job_id"),
        },
        "memmgr" => CoreActionKind::Memory {
            surface: action.input_str("type"),
            operation: action.input_str("op"),
        },
        "capmgr" => CoreActionKind::Capability {
            op: action.input_str("op"),
            kind: action.input_str("kind"),
            id: action.input_str("id"),
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
            r#"{"status":"working","free_talk":"先说明一下我的判断。","report_job_progress":"正在检查。","next_actions":[{"action":"memmgr","intent":"查询项目记忆","args":{"type":"durable","op":"query","query":"project"}},{"action":"run_bash","intent":"查看文件","args":{"command":"pwd"}},{"action":"self_tool","intent":"读取运行时信息","args":{"type":"about_me","op":"read"}}]}"#,
            &crate::capability::CapabilityRegistry::builtin(),
        );
        let events = notifications_from_envelope(&envelope);
        assert_eq!(
            events,
            vec![
                CoreNotification::ModelResponse {
                    status: "working".to_string(),
                    free_talk: "先说明一下我的判断。".to_string(),
                    report_job_progress: "正在检查。".to_string(),
                    final_answer: String::new(),
                    continue_work: true,
                },
                CoreNotification::Action {
                    intent: Some("查询项目记忆".to_string()),
                    action: "memmgr".to_string(),
                    input: serde_json::json!({
                        "type": "durable",
                        "op": "query",
                        "query": "project"
                    }),
                    kind: CoreActionKind::Memory {
                        surface: "durable".to_string(),
                        operation: "query".to_string(),
                    },
                    active: false,
                    memory_activity: CoreMemoryActivity::Read,
                },
                CoreNotification::Action {
                    intent: Some("查看文件".to_string()),
                    action: "run_bash".to_string(),
                    input: serde_json::json!({
                        "command": "pwd"
                    }),
                    kind: CoreActionKind::Bash {
                        command: "pwd".to_string(),
                    },
                    active: true,
                    memory_activity: CoreMemoryActivity::None,
                },
                CoreNotification::Action {
                    intent: Some("读取运行时信息".to_string()),
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
}
