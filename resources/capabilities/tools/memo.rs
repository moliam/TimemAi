use crate::response_protocol::ParsedAction;
use crate::{ActionExecution, ActionOutcome, ActionRuntime, AgentCore};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let op = action
        .raw_input
        .get("op")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let text = action
        .raw_input
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let outcome_text = match op {
        "create" | "update" => {
            if text.is_empty() {
                return failed("text_required");
            }
            core.set_active_memo(text.clone());
            // Publish the authoritative memo state so interfaces can surface
            // the active long-task reminder (WebUI memo indicator).
            let session_id = core.current_session_id().to_string();
            runtime.on_core_topic_events(&[crate::host::memo_topic_event_with_op(
                session_id,
                Some(&text),
                "created",
            )]);
            "Memo created.  You should work continuously without disturbing user to achieve the memo goal if steps are clear. Don't `finish` in the middle unless user asks.".to_string()
        }
        "delete" => {
            core.clear_active_memo();
            let session_id = core.current_session_id().to_string();
            runtime.on_core_topic_events(&[crate::host::memo_topic_event_with_op(
                session_id, None, "deleted",
            )]);
            "Memo deleted. No active memo. Reminder: make sure that all user's demands/goal are met. If next step is clear, you should automatically continue without disturbing user. Don't `finish` in the middle unless user asks.".to_string()
        }
        _ => return failed("op_invalid"),
    };
    ActionExecution::Completed(ActionOutcome::completed(outcome_text))
}

fn failed(error: &str) -> ActionExecution {
    ActionExecution::Completed(ActionOutcome::failed(format!(
        "Action result: memo\nerror: {error}"
    )))
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_memo_tests.rs"]
mod tests;
