use crate::response_protocol::ParsedAction;
use crate::{ActionExecution, ActionOutcome, ActionRuntime, AgentCore, TimerSpec};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let op = str_field(action, "op");
    match op.as_str() {
        "new" => execute_new(core, action, runtime),
        "del" => execute_del(core, action, runtime),
        _ => failed("op_invalid"),
    }
}

fn execute_new(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let prompt = str_field(action, "prompt");
    if prompt.is_empty() {
        return failed("prompt_required");
    }
    let name = str_field(action, "name");
    let mode = {
        let mode = str_field(action, "mode");
        if mode.is_empty() {
            "once".to_string()
        } else {
            mode
        }
    };
    let after_ms = u64_field(action, "after_ms");
    let at = str_field(action, "at");
    let interval_ms = u64_field(action, "interval_ms");

    // The tool validates shape only; the host parses `at` with its clock.
    match mode.as_str() {
        "once" => {
            if interval_ms.is_some() {
                return failed("interval_requires_repeat");
            }
            if !schedule_exactly_one(after_ms, &at) {
                return failed("schedule_invalid");
            }
            if after_ms == Some(0) {
                return failed("after_ms_invalid");
            }
        }
        "repeat" => {
            if interval_ms.is_none() {
                return failed("interval_required");
            }
            if interval_ms == Some(0) {
                return failed("interval_invalid");
            }
            if !schedule_exactly_one(after_ms, &at) {
                return failed("schedule_invalid");
            }
            if after_ms == Some(0) {
                return failed("after_ms_invalid");
            }
        }
        _ => return failed("mode_invalid"),
    }

    let spec = TimerSpec {
        timer_id: crate::unique_id("timer"),
        name,
        prompt,
        mode,
        after_ms,
        at: if at.is_empty() { None } else { Some(at) },
        interval_ms,
        created_by: "model".to_string(),
    };
    match core.add_timer(spec) {
        Ok(added) => {
            // Publish the authoritative timer set so interfaces can surface
            // pending scheduled prompts (WebUI timer panel). The op label
            // distinguishes creating from deleting.
            let session_id = core.current_session_id().to_string();
            runtime.on_core_topic_events(&[crate::host::timer_topic_event(
                session_id,
                "created",
                core.timers(),
            )]);
            ActionExecution::Completed(ActionOutcome::completed(format!(
                "Timer created: [{}|{}|model] {}. The runtime will deliver this prompt when due; it survives compression and restart. Active timers: {}.",
                added.name,
                added.mode,
                added.prompt,
                core.timers().len()
            )))
        }
        Err(error) => failed(&error),
    }
}

fn execute_del(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let name = str_field(action, "name");
    if name.is_empty() {
        return failed("name_required");
    }
    match core.remove_timer(&name) {
        Ok(removed) => {
            let session_id = core.current_session_id().to_string();
            runtime.on_core_topic_events(&[crate::host::timer_topic_event(
                session_id,
                "deleted",
                core.timers(),
            )]);
            ActionExecution::Completed(ActionOutcome::completed(format!(
                "Timer deleted: [{}|{}|model] {}. Active timers: {}.",
                removed.name,
                removed.mode,
                removed.prompt,
                core.timers().len()
            )))
        }
        Err(error) => failed(&error),
    }
}

/// The first fire is described by exactly one of after_ms / at.
fn schedule_exactly_one(after_ms: Option<u64>, at: &str) -> bool {
    match (after_ms, at.is_empty()) {
        (None, true) => false,
        (Some(_), false) => false,
        _ => true,
    }
}

fn str_field(action: &ParsedAction, key: &str) -> String {
    action
        .raw_input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn u64_field(action: &ParsedAction, key: &str) -> Option<u64> {
    let value = action.raw_input.get(key)?;
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
}

fn failed(error: &str) -> ActionExecution {
    ActionExecution::Completed(ActionOutcome::failed(format!(
        "Action result: timer\nerror: {error}"
    )))
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_timer_tests.rs"]
mod tests;
