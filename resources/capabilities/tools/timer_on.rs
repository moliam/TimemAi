use crate::response_protocol::ParsedAction;
use crate::timer_shared::{create_timer, failed, name_field, prompt_field};
use crate::{ActionExecution, ActionRuntime, AgentCore};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let prompt = match prompt_field(action, "timer_on") {
        Ok(prompt) => prompt,
        Err(error) => return error,
    };
    let event = action.input_str("event");
    if !matches!(event.as_str(), "session_idle" | "after_compression") {
        return failed(
            "timer_on",
            "event must be \"session_idle\" or \"after_compression\".",
        );
    }
    create_timer(
        core,
        runtime,
        "timer_on",
        name_field(action),
        prompt,
        "trigger",
        None,
        None,
        None,
        Some(event),
        0,
    )
}
