use crate::response_protocol::ParsedAction;
use crate::timer_shared::{create_timer, failed, name_field, prompt_field};
use crate::{ActionExecution, ActionRuntime, AgentCore};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let prompt = match prompt_field(action, "timer_every") {
        Ok(prompt) => prompt,
        Err(error) => return error,
    };
    let Some(interval_ms) = action.input_u64("interval_ms") else {
        return failed(
            "timer_every",
            "interval_ms is required: give the refire period in milliseconds.",
        );
    };
    if interval_ms == 0 {
        return failed("timer_every", "interval_ms must be greater than zero.");
    }
    create_timer(
        core,
        runtime,
        "timer_every",
        name_field(action),
        prompt,
        "repeat",
        Some(interval_ms),
        None,
        Some(interval_ms),
        None,
        0,
    )
}
