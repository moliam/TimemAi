use crate::response_protocol::ParsedAction;
use crate::{host, ActionExecution, ActionOutcome, ActionRuntime, AgentCore};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let task = action
        .raw_input
        .get("task")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let answer = action
        .raw_input
        .get("answer")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if task.is_empty() {
        return failed("task_required");
    }
    if answer.is_empty() {
        return failed("answer_required");
    }
    if !core.sub_answer_enabled() {
        return failed("primary_worker_required");
    }
    let ordinal = core.record_sub_answer();
    runtime.on_core_topic_events(&[host::sub_answer_topic_event(
        core.current_session_id(),
        crate::unique_id("sub_answer"),
        ordinal,
        task,
        answer,
    )]);
    ActionExecution::Completed(ActionOutcome::completed("Shown to user successfully."))
}

fn failed(error: &str) -> ActionExecution {
    ActionExecution::Completed(ActionOutcome::failed(format!(
        "Action result: sub_answer\nerror: {error}"
    )))
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_sub_answer_tests.rs"]
mod tests;
