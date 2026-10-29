use crate::response_protocol::ParsedAction;
use crate::{ActionExecution, ActionOutcome, ActionRuntime, AgentCore};

pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let to = str_field(action, "to");
    if to.is_empty() {
        return failed("to_required");
    }
    let msg = str_field(action, "msg");
    if msg.is_empty() {
        return failed("msg_required");
    }
    // The user-facing syntax is `max_wait=300s`; bare `300` and `-1` are
    // accepted too. Default 300; -1 waits indefinitely.
    let max_wait = parse_max_wait(&str_field(action, "max_wait"));
    let session_id = core.current_session_id().to_string();
    runtime.on_core_topic_events(&[crate::host::mailbox_send_topic_event(
        session_id, &to, &msg, max_wait,
    )]);
    let wait_text = if max_wait < 0 {
        "no forced-delivery timeout: it waits until the recipient consumes it".to_string()
    } else {
        format!("forced delivery after {max_wait}s if still unconsumed")
    };
    ActionExecution::Completed(ActionOutcome::completed(format!(
        "Mail queued for mailbox {to}: {wait_text}. The recipient's runtime will surface it in its next model request."
    )))
}

fn parse_max_wait(raw: &str) -> i64 {
    raw.trim()
        .trim_end_matches('s')
        .trim()
        .parse::<i64>()
        .unwrap_or(300)
}

fn str_field(action: &ParsedAction, key: &str) -> String {
    action.input_str(key)
}

fn failed(error: &str) -> ActionExecution {
    ActionExecution::Completed(ActionOutcome::failed(format!(
        "Action result: mailbox\nerror: {error}"
    )))
}
