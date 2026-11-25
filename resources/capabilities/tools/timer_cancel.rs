use crate::response_protocol::ParsedAction;
use crate::timer_shared::{delete_timer, name_field};
use crate::{ActionExecution, ActionRuntime, AgentCore};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    delete_timer(core, runtime, "timer_cancel", name_field(action))
}
