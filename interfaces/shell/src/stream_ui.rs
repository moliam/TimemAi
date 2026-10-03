use serde::Deserialize;
use serde_json::Value;

pub const CORE_TOPIC_MODEL_PREVIEW: &str = "core.model.preview";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamPreviewStatus {
    Streaming,
    Intermediate,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StreamResponsePreview {
    pub attempt: u64,
    pub revision: u64,
    pub text: String,
    pub status: StreamPreviewStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamPreviewSnapshot {
    pub turn_id: String,
    pub attempt: u64,
    pub revision: u64,
    pub interruption: Option<String>,
    pub response: Option<StreamResponsePreview>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPreviewUpdate {
    Ignored,
    Changed,
    Retracted,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StreamPreviewState {
    snapshot: Option<StreamPreviewSnapshot>,
}

#[derive(Debug, Deserialize)]
struct PreviewPayload {
    turn_id: String,
    attempt: u64,
    revision: u64,
    interruption: Option<String>,
    response: Option<StreamResponsePreview>,
}

impl StreamPreviewState {
    pub fn snapshot(&self) -> Option<&StreamPreviewSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn visible_text(&self) -> Option<&str> {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.response.as_ref())
            .map(|response| response.text.as_str())
            .filter(|text| !text.is_empty())
    }

    pub fn clear(&mut self) {
        self.snapshot = None;
    }

    pub fn apply_topic(&mut self, topic_name: &str, payload: &Value) -> StreamPreviewUpdate {
        if topic_name != CORE_TOPIC_MODEL_PREVIEW {
            return StreamPreviewUpdate::Ignored;
        }
        self.apply_payload(payload)
    }

    pub fn apply_core_topic(&mut self, event: &Value) -> StreamPreviewUpdate {
        let Some(topic_name) = event
            .get("topic")
            .and_then(|topic| topic.get("name"))
            .and_then(Value::as_str)
        else {
            return StreamPreviewUpdate::Ignored;
        };
        let Some(payload) = event.get("payload") else {
            return StreamPreviewUpdate::Ignored;
        };
        self.apply_topic(topic_name, payload)
    }

    pub fn apply_payload(&mut self, payload: &Value) -> StreamPreviewUpdate {
        let Ok(payload) = serde_json::from_value::<PreviewPayload>(payload.clone()) else {
            return StreamPreviewUpdate::Ignored;
        };
        if payload.turn_id.trim().is_empty() {
            return StreamPreviewUpdate::Ignored;
        }
        if self.snapshot.as_ref().is_some_and(|current| {
            current.turn_id == payload.turn_id && current.revision >= payload.revision
        }) {
            return StreamPreviewUpdate::Ignored;
        }
        let had_response = self
            .snapshot
            .as_ref()
            .filter(|current| current.turn_id == payload.turn_id)
            .and_then(|current| current.response.as_ref())
            .is_some();
        let has_response = payload.response.is_some();
        self.snapshot = Some(StreamPreviewSnapshot {
            turn_id: payload.turn_id,
            attempt: payload.attempt,
            revision: payload.revision,
            interruption: payload.interruption,
            response: payload.response,
        });
        if had_response && !has_response {
            StreamPreviewUpdate::Retracted
        } else {
            StreamPreviewUpdate::Changed
        }
    }
}

pub fn render_stream_preview(snapshot: &StreamPreviewSnapshot) -> String {
    let text = snapshot
        .response
        .as_ref()
        .map(|response| response.text.trim_end())
        .unwrap_or_default();
    let interruption = snapshot
        .interruption
        .as_deref()
        .filter(|reason| !reason.is_empty())
        .map(|reason| format!("\n\x1b[2m[preview interrupted: {reason}]\x1b[0m"))
        .unwrap_or_default();
    format!("{text}{interruption}")
}

#[cfg(test)]
#[path = "../tests/unit/stream_ui_tests.rs"]
mod tests;

pub const CORE_TOPIC_ACTION: &str = "core.action";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamToolAction {
    pub action: String,
    pub action_id: String,
    pub input: Value,
    pub event: String,
    pub status: String,
    pub active: bool,
    pub pid: Option<u32>,
}

impl StreamToolAction {
    pub fn from_core(action: &timem_in_process::agent_api::CoreActionTopic) -> Self {
        Self {
            action: action.action.clone(),
            action_id: action.action_id.clone(),
            input: action.input.clone(),
            event: action.event.clone(),
            status: action.status.clone(),
            active: action.active,
            pid: action.pid,
        }
    }

    pub fn from_payload(payload: &Value) -> Option<Self> {
        let action = payload.get("action")?.as_str()?.trim().to_string();
        if action.is_empty() || matches!(action.as_str(), "task_finished" | "turn_finished") {
            return None;
        }
        Some(Self {
            action,
            action_id: payload
                .get("action_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            input: payload.get("input").cloned().unwrap_or(Value::Null),
            event: payload
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or("start")
                .to_string(),
            status: payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("running")
                .to_string(),
            active: payload
                .get("active")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            pid: payload
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|pid| u32::try_from(pid).ok()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamToolFoldUpdate {
    Ignored,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamActiveTool {
    key: String,
    action: String,
    detail: String,
    status: String,
    pid: Option<u32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StreamToolFoldState {
    active: Vec<StreamActiveTool>,
    completed_success: usize,
    completed_failed: usize,
    completed_keys: std::collections::HashSet<String>,
    fallback_active: std::collections::HashMap<String, String>,
    fallback_completed: std::collections::HashSet<String>,
    next_fallback_sequence: u64,
}

impl StreamToolFoldState {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty() && self.completed_success == 0 && self.completed_failed == 0
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn completed_counts(&self) -> (usize, usize) {
        (self.completed_success, self.completed_failed)
    }

    pub fn apply_core_topic(
        &mut self,
        event: &timem_in_process::agent_api::CoreTopicEvent,
    ) -> StreamToolFoldUpdate {
        let Some(action) = event.as_action() else {
            return StreamToolFoldUpdate::Ignored;
        };
        self.apply(&StreamToolAction::from_core(&action))
    }

    pub fn apply_json_topic(&mut self, event: &Value) -> StreamToolFoldUpdate {
        let Some(topic_name) = event
            .get("topic")
            .and_then(|topic| topic.get("name"))
            .and_then(Value::as_str)
        else {
            return StreamToolFoldUpdate::Ignored;
        };
        if topic_name != CORE_TOPIC_ACTION {
            return StreamToolFoldUpdate::Ignored;
        }
        let Some(payload) = event.get("payload") else {
            return StreamToolFoldUpdate::Ignored;
        };
        let Some(action) = StreamToolAction::from_payload(payload) else {
            return StreamToolFoldUpdate::Ignored;
        };
        self.apply(&action)
    }

    pub fn apply(&mut self, action: &StreamToolAction) -> StreamToolFoldUpdate {
        if matches!(action.action.as_str(), "task_finished" | "turn_finished") {
            return StreamToolFoldUpdate::Ignored;
        }
        let terminal = stream_tool_terminal_kind(&action.event, &action.status, action.active);
        let signature = stream_tool_fallback_signature(action);
        let explicit = !action.action_id.trim().is_empty();
        let key = if explicit {
            format!("id:{}", action.action_id.trim())
        } else if let Some(key) = self.fallback_active.get(&signature) {
            key.clone()
        } else {
            if terminal.is_some() && self.fallback_completed.contains(&signature) {
                return StreamToolFoldUpdate::Ignored;
            }
            self.next_fallback_sequence = self.next_fallback_sequence.saturating_add(1);
            let key = format!("fallback:{}:{}", self.next_fallback_sequence, signature);
            self.fallback_active.insert(signature.clone(), key.clone());
            self.fallback_completed.remove(&signature);
            key
        };

        if let Some(failed) = terminal {
            if self.completed_keys.contains(&key) {
                return StreamToolFoldUpdate::Ignored;
            }
            self.active.retain(|entry| entry.key != key);
            self.completed_keys.insert(key.clone());
            if failed {
                self.completed_failed = self.completed_failed.saturating_add(1);
            } else {
                self.completed_success = self.completed_success.saturating_add(1);
            }
            if !explicit {
                self.fallback_active.remove(&signature);
                self.fallback_completed.insert(signature);
            }
            return StreamToolFoldUpdate::Changed;
        }

        if explicit && self.completed_keys.contains(&key) {
            // A stable id must not reopen after an authoritative terminal event.
            return StreamToolFoldUpdate::Ignored;
        }
        let detail = stream_tool_detail(&action.action, &action.input);
        let status = stream_tool_status_label(&action.status, action.pid);
        if let Some(existing) = self.active.iter_mut().find(|entry| entry.key == key) {
            let replacement = StreamActiveTool {
                key,
                action: stream_tool_action_name(&action.action),
                detail,
                status,
                pid: action.pid,
            };
            if *existing == replacement {
                return StreamToolFoldUpdate::Ignored;
            }
            *existing = replacement;
        } else {
            self.active.push(StreamActiveTool {
                key,
                action: stream_tool_action_name(&action.action),
                detail,
                status,
                pid: action.pid,
            });
        }
        StreamToolFoldUpdate::Changed
    }
}

fn stream_tool_terminal_kind(event: &str, status: &str, active: bool) -> Option<bool> {
    let status = status.trim().to_ascii_lowercase();
    if matches!(
        status.as_str(),
        "running" | "background_running" | "pending"
    ) || active
    {
        return None;
    }
    let terminal = event == "finish"
        || matches!(
            status.as_str(),
            "completed"
                | "background_finished"
                | "failed"
                | "error"
                | "timeout"
                | "cancelled"
                | "cancelled_by_user"
                | "serialization_failed"
        );
    if !terminal {
        return None;
    }
    Some(matches!(
        status.as_str(),
        "failed" | "error" | "timeout" | "cancelled" | "cancelled_by_user" | "serialization_failed"
    ))
}

fn stream_tool_fallback_signature(action: &StreamToolAction) -> String {
    format!(
        "{}:{}",
        action.action,
        serde_json::to_string(&action.input).unwrap_or_default()
    )
}

fn stream_tool_action_name(action: &str) -> String {
    match action {
        "run_bash" => "Run command".to_string(),
        "run_powershell" => "Run PowerShell".to_string(),
        "readfile" => "Read file".to_string(),
        "memmgr" => "Memory".to_string(),
        "memo" => "Work memo".to_string(),
        "self_tool" => "Runtime info".to_string(),
        "context_compress" => "Compact context".to_string(),
        other => other
            .split(['_', '.'])
            .filter(|part| !part.is_empty())
            .enumerate()
            .map(|(index, part)| {
                if index == 0 {
                    let mut chars = part.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default()
                } else {
                    part.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn stream_tool_detail(action: &str, input: &Value) -> String {
    let value = match action {
        "run_bash" | "run_powershell" => input
            .get("cmd")
            .or_else(|| input.get("loop_cmd"))
            .and_then(Value::as_str),
        "readfile" => input.get("path").and_then(Value::as_str),
        "memmgr" | "memo" => input.get("op").and_then(Value::as_str),
        "self_tool" => input.get("type").and_then(Value::as_str),
        _ => None,
    }
    .unwrap_or_default()
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(120).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn stream_tool_status_label(status: &str, pid: Option<u32>) -> String {
    match status {
        "background_running" => pid
            .map(|pid| format!("running in background · pid {pid}"))
            .unwrap_or_else(|| "running in background".to_string()),
        "pending" => "pending".to_string(),
        _ => "running".to_string(),
    }
}

pub fn render_stream_tool_fold(state: &StreamToolFoldState) -> String {
    if state.is_empty() {
        return String::new();
    }
    let mut output = String::new();
    if state.completed_success > 0 || state.completed_failed > 0 {
        output.push_str("\x1b[2mTools folded");
        if state.completed_success > 0 {
            output.push_str(&format!(" · ✓ {}", state.completed_success));
        }
        if state.completed_failed > 0 {
            output.push_str(&format!(" · × {}", state.completed_failed));
        }
        output.push_str("\x1b[0m\n");
    }
    for tool in &state.active {
        output.push_str("\x1b[96m›\x1b[0m \x1b[1m");
        output.push_str(&tool.action);
        output.push_str("\x1b[0m");
        if !tool.detail.is_empty() {
            output.push_str(" · ");
            output.push_str(&tool.detail);
        }
        output.push_str(" · \x1b[2m");
        output.push_str(&tool.status);
        output.push_str("\x1b[0m\n");
    }
    output
}
