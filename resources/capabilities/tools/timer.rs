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
        _ => failed(
            "No timer action was performed: op must be \"new\" to create a timer or \"del\" \
             to delete one by name.",
        ),
    }
}

fn execute_new(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let prompt = str_field(action, "prompt");
    if prompt.is_empty() {
        return failed(
            "Cannot create the timer: op=\"new\" requires a non-empty prompt. That prompt is \
             the text delivered when the timer fires.",
        );
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
    let queue_timeout_minutes = action.input_u64("queue_timeout").unwrap_or(0);
    let trigger = str_field(action, "trigger");

    // The tool validates shape only; the host parses `at` with its clock.
    match mode.as_str() {
        "once" => {
            if !trigger.is_empty() {
                return failed(PERIODIC_OR_EVENT_HINT);
            }
            if interval_ms.is_some() {
                return failed(
                    "Cannot create the timer: interval_ms only applies to mode=\"repeat\". \
                     For a one-shot timer, omit interval_ms and use mode=\"once\" with exactly \
                     one of after_ms or at.",
                );
            }
            if !schedule_exactly_one(after_ms, &at) {
                return failed(SCHEDULE_ONE_HINT);
            }
            if after_ms == Some(0) {
                return failed(
                    "Cannot create the timer: after_ms must be greater than zero. Use a positive \
                     number of milliseconds, or use at for an absolute first-fire time.",
                );
            }
        }
        "repeat" => {
            if !trigger.is_empty() {
                return failed(PERIODIC_OR_EVENT_HINT);
            }
            if interval_ms.is_none() {
                return failed(
                    "Cannot create the timer: mode=\"repeat\" also requires interval_ms, the \
                     refire period in milliseconds, in addition to exactly one of after_ms or at.",
                );
            }
            if interval_ms == Some(0) {
                return failed(
                    "Cannot create the timer: interval_ms must be greater than zero. Give the \
                     refire period in milliseconds.",
                );
            }
            if !schedule_exactly_one(after_ms, &at) {
                return failed(SCHEDULE_ONE_HINT);
            }
            if after_ms == Some(0) {
                return failed(
                    "Cannot create the timer: after_ms must be greater than zero. Use a positive \
                     number of milliseconds, or use at for an absolute first-fire time.",
                );
            }
        }
        "trigger" => {
            if !matches!(trigger.as_str(), "session_idle" | "after_compression") {
                return failed(
                    "Cannot create the timer: mode=\"trigger\" needs trigger=\"session_idle\" or \
                     trigger=\"after_compression\". Those are the only supported events, so put \
                     one of those exact event names in the trigger field.",
                );
            }
            if after_ms.is_some() || !at.is_empty() || interval_ms.is_some() {
                return failed(
                    "Cannot create the timer: mode=\"trigger\" is event-driven, so after_ms, at, \
                     and interval_ms must all be omitted; the selected event decides when the \
                     prompt fires. Remove those schedule fields.",
                );
            }
        }
        _ => {
            return failed(
                "Cannot create the timer: mode must be \"once\", \"repeat\", or \"trigger\". \
                 \"once\" fires one time, \"repeat\" refires every interval_ms, and \"trigger\" \
                 fires on session_idle or after_compression.",
            )
        }
    }

    let spec = TimerSpec {
        timer_id: crate::unique_id("timer"),
        name,
        prompt,
        mode,
        after_ms,
        at: if at.is_empty() { None } else { Some(at) },
        interval_ms,
        trigger: if trigger.is_empty() {
            None
        } else {
            Some(trigger)
        },
        queue_timeout_minutes,
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
        Err(error) => failed(create_timer_error(&error)),
    }
}

fn execute_del(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let name = str_field(action, "name");
    if name.is_empty() {
        return failed(
            "Cannot delete a timer without its name: call op=\"del\" with the exact name shown \
             in the active timer list.",
        );
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
        Err(error) => failed(delete_timer_error(&error)),
    }
}

/// The first fire is described by exactly one of after_ms / at.
fn schedule_exactly_one(after_ms: Option<u64>, at: &str) -> bool {
    matches!((after_ms, at.is_empty()), (None, false) | (Some(_), true))
}

fn str_field(action: &ParsedAction, key: &str) -> String {
    action.input_str(key)
}

fn u64_field(action: &ParsedAction, key: &str) -> Option<u64> {
    // Delegate to the shared accessor: XML suites deliver bare text children
    // as strings when no integer schema applies, so numeric fields must
    // accept both JSON numbers and numeric strings uniformly.
    action.input_u64(key)
}

const PERIODIC_OR_EVENT_HINT: &str =
    "Cannot create the timer: trigger is only valid with mode=\"trigger\", but a time-based \
     mode was used here. For a periodic timer, use mode=\"repeat\" with exactly one of after_ms \
     or at plus interval_ms and omit trigger, for example \
     {\"op\":\"new\",\"mode\":\"repeat\",\"after_ms\":4800000,\"interval_ms\":4800000,\"prompt\":\"...\"}; \
     the timer refires on its own and needs no event. For an event-driven timer, use \
     mode=\"trigger\" with trigger=\"session_idle\" or \"after_compression\" and omit \
     after_ms, at, and interval_ms.";
const SCHEDULE_ONE_HINT: &str =
    "Cannot create the timer: exactly one of after_ms or at is required. Set after_ms for a \
     relative delay, or at for an absolute ISO-RFC3339 instant, but not both and not neither.";

fn create_timer_error(error: &str) -> String {
    if let Some(name) = error.strip_prefix("name_exists:") {
        format!(
            "Cannot create the timer: a timer named \"{name}\" already exists. Delete it first \
             with op=\"del\" and name=\"{name}\", or choose a different name."
        )
    } else if error == "timer_limit" {
        "Cannot create the timer: this session already has the maximum number of timers. \
         Delete an unused timer before creating another."
            .to_string()
    } else {
        format!("Cannot create the timer: {error}")
    }
}

fn delete_timer_error(error: &str) -> String {
    if let Some(name) = error.strip_prefix("not_found:") {
        format!(
            "Cannot delete the timer: no timer named \"{name}\" exists. Nothing was deleted; \
             use the exact name from the active timer list."
        )
    } else {
        format!("Cannot delete the timer: {error}")
    }
}

fn failed(message: impl Into<String>) -> ActionExecution {
    ActionExecution::Completed(ActionOutcome::failed(format!(
        "Action result: timer\nerror: {}",
        message.into()
    )))
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_timer_tests.rs"]
mod tests;
