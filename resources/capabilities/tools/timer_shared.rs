use crate::response_protocol::ParsedAction;
use crate::{ActionExecution, ActionOutcome, ActionRuntime, AgentCore, TimerSpec};

pub(crate) fn prompt_field(action: &ParsedAction, tool: &str) -> Result<String, ActionExecution> {
    let prompt = action.input_str("prompt");
    if prompt.is_empty() {
        return Err(failed(
            tool,
            format!("{tool} requires a non-empty prompt. That prompt is the text delivered when the timer fires."),
        ));
    }
    Ok(prompt)
}

pub(crate) fn name_field(action: &ParsedAction) -> String {
    action.input_str("name")
}

pub(crate) fn at_field(action: &ParsedAction) -> Option<String> {
    let at = action.input_str("at");
    if at.is_empty() {
        None
    } else {
        Some(at)
    }
}

pub(crate) fn create_timer(
    core: &mut AgentCore,
    runtime: &mut dyn ActionRuntime,
    tool: &str,
    name: String,
    prompt: String,
    mode: &str,
    after_ms: Option<u64>,
    at: Option<String>,
    interval_ms: Option<u64>,
    trigger: Option<String>,
    queue_timeout_minutes: u64,
) -> ActionExecution {
    let spec = TimerSpec {
        timer_id: crate::unique_id("timer"),
        name,
        prompt,
        mode: mode.to_string(),
        after_ms,
        at,
        interval_ms,
        trigger,
        queue_timeout_minutes,
        created_by: "model".to_string(),
        allow_agent_edit: true,
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
                Some(&added),
            )]);
            ActionExecution::Completed(ActionOutcome::completed(format!(
                "Timer created: [{}|{}|model] {}. The runtime will deliver this prompt when due; it survives compression and restart.{}. Active timers: {}.",
                added.name,
                added.mode,
                added.prompt,
                match added.trigger.as_deref() {
                    Some("session_idle") => " It stays armed and fires every time a turn ends through task_finished; the fired turn itself ends that way too, so the prompt must delete this timer once done or keep each firing bounded.".to_string(),
                    Some("after_compression") => " It stays armed and fires every time this session's context compression completes; the fired turn itself can compress, so the prompt must delete this timer once done or keep each firing bounded.".to_string(),
                    _ => {
                        if added.queue_timeout_minutes > 0 {
                            format!(
                                " Queue delivery: when the session is busy at fire time the prompt waits in the session queue for up to {} minute(s), then is delivered directly",
                                added.queue_timeout_minutes
                            )
                        } else {
                            String::new()
                        }
                    }
                },
                core.timers().len()
            )))
        }
        Err(error) => failed(tool, create_timer_error(&error)),
    }
}

pub(crate) fn delete_timer(
    core: &mut AgentCore,
    runtime: &mut dyn ActionRuntime,
    tool: &str,
    name: String,
) -> ActionExecution {
    if name.is_empty() {
        return failed(
            tool,
            "Cannot delete a timer without its name. Use the exact name shown in the active timer list."
                .to_string(),
        );
    }
    if let Some(timer) = core.timers().iter().find(|timer| timer.name == name) {
        if timer.created_by != "model" && !timer.allow_agent_edit {
            return failed(
                tool,
                format!(
                    "Cannot delete the timer \"{name}\": the user disabled agent changes for it. Ask the user to delete it, or to allow agent changes when recreating it."
                ),
            );
        }
    }
    match core.remove_timer(&name) {
        Ok(removed) => {
            let session_id = core.current_session_id().to_string();
            runtime.on_core_topic_events(&[crate::host::timer_topic_event(
                session_id,
                "deleted",
                core.timers(),
                Some(&removed),
            )]);
            ActionExecution::Completed(ActionOutcome::completed(format!(
                "Timer deleted: [{}|{}|model] {}. Active timers: {}.",
                removed.name,
                removed.mode,
                removed.prompt,
                core.timers().len()
            )))
        }
        Err(error) => failed(tool, delete_timer_error(&error)),
    }
}

pub(crate) fn failed(tool: &str, message: impl Into<String>) -> ActionExecution {
    ActionExecution::Completed(ActionOutcome::failed(format!(
        "Action result: {tool}\nerror: {}",
        message.into()
    )))
}

pub(crate) fn create_timer_error(error: &str) -> String {
    if let Some(name) = error.strip_prefix("name_exists:") {
        format!(
            "Cannot create the timer: a timer named \"{name}\" already exists. Delete it first with timer_cancel and name=\"{name}\", or choose a different name."
        )
    } else if error == "timer_limit" {
        "Cannot create the timer: this session already has the maximum number of timers. Delete an unused timer before creating another.".to_string()
    } else {
        format!("Cannot create the timer: {error}")
    }
}

pub(crate) fn delete_timer_error(error: &str) -> String {
    if let Some(name) = error.strip_prefix("not_found:") {
        format!(
            "Cannot delete the timer: no timer named \"{name}\" exists. Nothing was deleted; use the exact name from the active timer list."
        )
    } else {
        format!("Cannot delete the timer: {error}")
    }
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_timer_tests.rs"]
mod tests;
