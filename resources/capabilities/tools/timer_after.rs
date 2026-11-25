use crate::response_protocol::ParsedAction;
use crate::timer_shared::{at_field, create_timer, failed, name_field, prompt_field};
use crate::{ActionExecution, ActionRuntime, AgentCore};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let prompt = match prompt_field(action, "timer_after") {
        Ok(prompt) => prompt,
        Err(error) => return error,
    };
    let delay_ms = action.input_u64("delay_ms");
    let at = at_field(action);
    match (delay_ms, at) {
        (Some(_), Some(_)) => failed(
            "timer_after",
            "Provide exactly one of delay_ms or at, not both.",
        ),
        (None, None) => failed(
            "timer_after",
            "Provide exactly one of delay_ms or at: delay_ms for a relative delay, or at for an absolute ISO-RFC3339 UTC instant.",
        ),
        (Some(0), None) => failed(
            "timer_after",
            "delay_ms must be greater than zero.",
        ),
        (Some(delay_ms), None) => create_timer(
            core,
            runtime,
            "timer_after",
            name_field(action),
            prompt,
            "once",
            Some(delay_ms),
            None,
            None,
            None,
            0,
        ),
        (None, Some(at)) => create_timer(
            core,
            runtime,
            "timer_after",
            name_field(action),
            prompt,
            "once",
            None,
            Some(at),
            None,
            None,
            0,
        ),
    }
}
