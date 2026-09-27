use crate::response_protocol::ParsedAction;
use crate::{ActionExecution, ActionOutcome, ActionRuntime, AgentCore};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let summary = action
        .raw_input
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if summary.is_empty() {
        return failed("summary_required");
    }
    core.record_turn_finished(summary.to_string());
    ActionExecution::Completed(ActionOutcome::completed(
        "Turn finished. The turn ends with this summary as the final answer.",
    ))
}

fn failed(error: &str) -> ActionExecution {
    ActionExecution::Completed(ActionOutcome::failed(format!(
        "Action result: turn_finished\nerror: {error}"
    )))
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_turn_finished_tests.rs"]
mod tests;
