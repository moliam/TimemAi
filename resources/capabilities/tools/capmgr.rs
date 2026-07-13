use crate::capability::CapabilityRegistry;
use crate::response_protocol::ParsedAction;
use crate::AgentCore;

#[derive(Debug, Clone, Copy)]
pub struct CapmgrActionInput<'a> {
    pub op: &'a str,
    pub kind: &'a str,
    pub id: &'a str,
}

pub fn execute(registry: &CapabilityRegistry, input: CapmgrActionInput<'_>) -> String {
    if input.op.trim().is_empty() {
        return "Action result: capmgr\nerror: invalid_input\nmessage: Missing `op`. Use list, load, inspect, job_status, or job_cancel.".to_string();
    }
    match input.op {
        "list" => registry.list_text(input.kind.trim()),
        "load" | "inspect" => {
            if input.kind.trim().is_empty() {
                return format!(
                    "Action result: capmgr\nop: {}\nerror: invalid_input\nmessage: Missing `kind`. Use tool or skill.",
                    input.op
                );
            }
            if input.id.trim().is_empty() {
                return format!(
                    "Action result: capmgr\nop: {}\nkind: {}\nerror: invalid_input\nmessage: Missing `id`. Provide the capability id to load.",
                    input.op, input.kind
                );
            }
            registry.load_text(input.kind.trim(), input.id)
        }
        other => format!("Action result: capmgr\nop: {other}\nerror: unsupported_op"),
    }
}

pub(crate) fn execute_action(core: &mut AgentCore, action: &ParsedAction) -> String {
    let op = action.input_lower("op");
    if op == "job_status" {
        return core
            .tool_jobs
            .status(&action.input_str("job_id"), action.status_timeout_ms());
    }
    if op == "job_cancel" {
        return core.tool_jobs.cancel(&action.input_str("job_id"));
    }
    execute(
        &core.capabilities,
        CapmgrActionInput {
            op: &op,
            kind: &action.input_str("kind"),
            id: &action.input_str("id"),
        },
    )
}

#[cfg(test)]
#[path = "../../../agent_core/tests/unit/capability_tool_capmgr_tests.rs"]
mod tests;
