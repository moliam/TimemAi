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
    // `queue_timeout` supersedes the legacy `max_wait` name; both accept the
    // `300s` / `300` / `-1` syntax. `resp_timeout` is new. `queue_timeout` is
    // legacy: mail is always delivered immediately, the value is persisted
    // with the envelope but no longer gates delivery.
    let queue_raw = str_field(action, "queue_timeout");
    let legacy_raw = str_field(action, "max_wait");
    let queue_timeout = parse_secs(if queue_raw.is_empty() {
        &legacy_raw
    } else {
        &queue_raw
    })
    .unwrap_or(crate::host::MAILBOX_DEFAULT_QUEUE_TIMEOUT_SECS);
    let resp_timeout = parse_secs(&str_field(action, "resp_timeout"))
        .unwrap_or(crate::host::MAILBOX_DEFAULT_RESP_TIMEOUT_SECS);
    let session_id = core.current_session_id().to_string();
    runtime.on_core_topic_events(&[crate::host::mailbox_send_topic_event(
        session_id,
        &to,
        &msg,
        queue_timeout,
        resp_timeout,
    )]);
    let resp_text = if resp_timeout < 0 {
        "no read-receipt deadline".to_string()
    } else {
        format!("you will be notified if it is not read within {resp_timeout}s")
    };
    ActionExecution::Completed(ActionOutcome::completed(format!(
        "Mail delivered to mailbox {to}: {resp_text}. A read receipt or a timeout notice will arrive as a [mailbox] message."
    )))
}

fn parse_secs(raw: &str) -> Option<i64> {
    raw.trim().trim_end_matches('s').trim().parse::<i64>().ok()
}

fn str_field(action: &ParsedAction, key: &str) -> String {
    action.input_str(key)
}

fn failed(error: &str) -> ActionExecution {
    ActionExecution::Completed(ActionOutcome::failed(format!(
        "Action result: mailbox\nerror: {error}"
    )))
}
