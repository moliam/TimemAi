use crate::response_protocol::ParsedAction;
use crate::AgentCore;

pub(crate) fn execute_action(core: &AgentCore, action: &ParsedAction) -> String {
    if action.input_lower("op") == "cancel" {
        core.shell_jobs.cancel(&action.input_str("job_id"))
    } else {
        core.shell_jobs
            .status(&action.input_str("job_id"), action.status_timeout_ms())
    }
}
