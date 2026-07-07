use crate::response_protocol::ParsedAction;
use crate::{capmgr, memmgr, self_tool, shell_exec};
use crate::{ActionExecution, ActionRuntime, AgentCore};

pub(crate) const BUILTIN_TOOL_BINDINGS: &[&str] = &["memmgr", "capmgr", "run_bash", "self_tool"];

type BuiltinToolCallback =
    fn(&mut AgentCore, &ParsedAction, &mut dyn ActionRuntime) -> ActionExecution;

pub(crate) fn execute_builtin_tool(
    core: &mut AgentCore,
    binding_name: &str,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> Option<ActionExecution> {
    builtin_tool_callback(binding_name).map(|callback| callback(core, action, runtime))
}

fn builtin_tool_callback(binding_name: &str) -> Option<BuiltinToolCallback> {
    match binding_name {
        "capmgr" => Some(execute_capmgr),
        "memmgr" => Some(execute_memmgr),
        "self_tool" => Some(execute_self_tool),
        "run_bash" => Some(execute_run_bash),
        _ => None,
    }
}

fn execute_capmgr(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    ActionExecution::Completed(capmgr::execute_action(core, action))
}

fn execute_memmgr(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    ActionExecution::Completed(memmgr::execute(core, action))
}

fn execute_self_tool(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    ActionExecution::Completed(self_tool::execute_action(core, action))
}

fn execute_run_bash(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    shell_exec::execute_run_bash_action(core, action, runtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_lists_all_compiled_tool_callbacks() {
        for binding in BUILTIN_TOOL_BINDINGS {
            assert!(
                builtin_tool_callback(binding).is_some(),
                "missing builtin callback for {binding}"
            );
        }
        assert!(builtin_tool_callback("ghost_tool").is_none());
    }
}
