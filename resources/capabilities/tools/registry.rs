use crate::response_protocol::ParsedAction;
use crate::{capmgr, memmgr, self_tool, shell_exec};
use crate::{ActionExecution, ActionRuntime, AgentCore};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub(crate) const BUILTIN_TOOL_BINDINGS: &[&str] = &["memmgr", "capmgr", "run_bash", "self_tool"];

type BuiltinToolCallback =
    fn(&mut AgentCore, &ParsedAction, &mut dyn ActionRuntime) -> ActionExecution;

pub(crate) fn execute_builtin_tool(
    core: &mut AgentCore,
    binding_name: &str,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> Result<Option<ActionExecution>, BuiltinToolFailure> {
    let Some(callback) = builtin_tool_callback(binding_name) else {
        return Ok(None);
    };
    catch_builtin_execution(|| callback(core, action, runtime)).map(Some)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BuiltinToolFailure;

fn catch_builtin_execution<F>(execute: F) -> Result<ActionExecution, BuiltinToolFailure>
where
    F: FnOnce() -> ActionExecution,
{
    catch_unwind(AssertUnwindSafe(execute)).map_err(|_| BuiltinToolFailure)
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
#[path = "../../../agent_core/tests/unit/capability_tool_registry_tests.rs"]
mod tests;
