use crate::response_protocol::ParsedAction;
use crate::{capmgr, memmgr, readfile, self_tool, shell_exec, sub_answer, toolgen};
use crate::{ActionExecution, ActionRuntime, AgentCore};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub(crate) const BUILTIN_TOOL_BINDINGS: &[&str] = &[
    "memmgr",
    "capmgr",
    "context_compact",
    "readfile",
    "run_bash",
    "run_powershell",
    "self_tool",
    "sub_answer",
    "toolgen",
];

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
        "context_compact" => Some(execute_context_compact),
        "memmgr" => Some(execute_memmgr),
        "readfile" => Some(execute_readfile),
        "self_tool" => Some(execute_self_tool),
        "sub_answer" => Some(execute_sub_answer),
        "run_bash" | "run_powershell" => Some(execute_local_shell),
        "toolgen" => Some(execute_toolgen),
        _ => None,
    }
}

fn execute_context_compact(
    _core: &mut AgentCore,
    _action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    // AgentCore promotes this intrinsic action before ordinary dispatch so it
    // can atomically rewrite prompt state. This callback is a defensive guard
    // that also keeps manifest and compiled binding registries paired.
    ActionExecution::Completed(crate::ActionOutcome::failed(
        "Action result: context_compact\nerror: intrinsic_dispatch_required",
    ))
}

fn execute_toolgen(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    toolgen::execute_action(core, action)
}

fn execute_capmgr(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    ActionExecution::Completed(capmgr::execute_action_outcome(core, action))
}

fn execute_memmgr(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    ActionExecution::Completed(memmgr::execute_outcome(core, action))
}

fn execute_self_tool(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    ActionExecution::Completed(self_tool::execute_action_outcome(core, action))
}

fn execute_sub_answer(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    sub_answer::execute_action(core, action, runtime)
}

fn execute_readfile(
    core: &mut AgentCore,
    action: &ParsedAction,
    _runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    ActionExecution::Completed(readfile::execute_action_outcome(core, action))
}

fn execute_local_shell(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    shell_exec::execute_run_bash_action(core, action, runtime)
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_registry_tests.rs"]
mod tests;
