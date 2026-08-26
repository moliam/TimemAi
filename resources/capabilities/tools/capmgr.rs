use crate::capability::CapabilityRegistry;
use crate::response_protocol::ParsedAction;
use crate::{ActionOutcome, AgentCore};

#[derive(Debug, Clone, Copy)]
pub struct CapmgrActionInput<'a> {
    pub op: &'a str,
    pub kind: &'a str,
    pub id: &'a str,
}

pub fn execute(registry: &CapabilityRegistry, input: CapmgrActionInput<'_>) -> String {
    if input.op.trim().is_empty() {
        return "Action result: capmgr\nerror: invalid_input\nmessage: Missing `op`. Use list, load, job_status, or job_cancel.".to_string();
    }
    match input.op {
        "list" => registry.list_text(input.kind.trim()),
        "load" => {
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

pub(crate) fn execute_action_outcome(core: &mut AgentCore, action: &ParsedAction) -> ActionOutcome {
    let op = action.input_lower("op");
    if op == "job_status" {
        return core
            .tool_jobs
            .status_outcome(&action.input_str("job_id"), action.status_timeout_ms());
    }
    if op == "job_cancel" {
        return core.tool_jobs.cancel_outcome(&action.input_str("job_id"));
    }

    let kind = action.input_str("kind");
    let id = action.input_str("id");
    let text = execute(
        &core.capabilities,
        CapmgrActionInput {
            op: &op,
            kind: &kind,
            id: &id,
        },
    );

    let succeeded = match op.as_str() {
        "list" => matches!(kind.trim(), "" | "all" | "tool" | "skill"),
        "load" => {
            !kind.trim().is_empty()
                && !id.trim().is_empty()
                && match kind.trim() {
                    "tool" => core.capabilities.contains_tool(id.trim()),
                    "skill" => core.capabilities.contains_skill(id.trim()),
                    _ => false,
                }
        }
        _ => false,
    };
    if succeeded {
        ActionOutcome::completed(text)
    } else {
        ActionOutcome::failed(text)
    }
}

#[cfg(test)]
#[path = "../../../agent_core/tests/unit/capability_tool_capmgr_tests.rs"]
mod tests;
