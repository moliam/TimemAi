use crate::response_protocol::ParsedAction;
use crate::{ActionExecution, ActionOutcome, ActionRuntime, AgentCore};

/// Default and maximum entry count surfaced by `list`/`search`.
const MAILBOX_LIST_DEFAULT_COUNT: u64 = 20;
const MAILBOX_LIST_MAX_COUNT: u64 = 100;
/// Body preview length (characters) for `list` entries.
const MAILBOX_PREVIEW_CHARS: usize = 160;
/// Words kept around the first match in `search` previews.
const MAILBOX_SEARCH_WINDOW_WORDS: usize = 128;

/// `action` selects the operation: `send` (default when omitted), `list`,
/// `search`, or `read`. The read-side actions inspect the session's
/// host-owned mailbox.json directly (the same data plane the Web Host
/// persists), mirroring how memmgr lets the agent read its memory domain.
pub(crate) fn execute_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    match action.input_lower("action").as_str() {
        "" | "send" => execute_send(core, action, runtime),
        "list" => ActionExecution::Completed(execute_list(core, action)),
        "search" => ActionExecution::Completed(execute_search(core, action)),
        "read" => ActionExecution::Completed(execute_read(core, action)),
        other => failed(&format!("unknown_action:{other}")),
    }
}

fn execute_send(
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
    let session_id = core.current_session_id();
    // The action result and every later receipt/timeout notice must name the
    // same mail id. The host consumes this id when it persists the ledger and
    // spool envelope, so the model can correlate the follow-up [mailbox] notice
    // with the mail it just sent.
    let mail_id = generate_mail_id();
    runtime.on_core_topic_events(&[crate::host::mailbox_send_topic_event(
        session_id,
        &to,
        &msg,
        queue_timeout,
        resp_timeout,
        &mail_id,
    )]);
    let resp_text = if resp_timeout < 0 {
        "no read-receipt deadline".to_string()
    } else {
        format!("you will be notified if it is not read within {resp_timeout}s")
    };
    ActionExecution::Completed(ActionOutcome::completed(format!(
        "Mail sent to mailbox {to} (mail id {mail_id}): {resp_text}. A read receipt, timeout, or failure notice will arrive as a [mailbox] message naming this id."
    )))
}

/// 48-bit random id in the host-compatible `mail_` namespace. The tool owns id
/// generation because topic delivery is asynchronous: the Host persists the
/// envelope after this action returns, so only an id chosen here can be named
/// in the action result and matched by later receipts.
fn generate_mail_id() -> String {
    let mut random = [0u8; 6];
    if crate::os::fill_secure_random(&mut random).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or_default();
        let mixed =
            (nanos ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)).to_le_bytes();
        random.copy_from_slice(&mixed[..6]);
    }
    let mut id = String::from("mail_");
    for byte in random {
        use std::fmt::Write;
        let _ = write!(id, "{byte:02x}");
    }
    id
}

fn execute_list(core: &AgentCore, action: &ParsedAction) -> ActionOutcome {
    let count = list_count(action);
    let rows = load_mail_rows(core, None, Some(count));
    if rows.is_empty() {
        return ActionOutcome::completed("Action result: mailbox\nmailbox is empty: no mail yet");
    }
    let total = rows.len();
    let shown = total.min(count);
    let mut text = format!(
        "Action result: mailbox\nMost recent {shown} of {total} mails (in + out), newest first:"
    );
    for row in rows.iter().take(shown) {
        text.push('\n');
        text.push_str(&format_mail_line(row));
        let preview = preview_chars(&row.msg, MAILBOX_PREVIEW_CHARS);
        if !preview.is_empty() {
            text.push_str("\n  ");
            text.push_str(&preview);
        }
    }
    ActionOutcome::completed(text)
}

fn execute_search(core: &AgentCore, action: &ParsedAction) -> ActionOutcome {
    let query = action.input_str("query");
    if query.is_empty() {
        return ActionOutcome::failed("Action result: mailbox\nerror: query_required");
    }
    let rows = load_mail_rows(core, None, None);
    let query_lower = query.to_lowercase();
    let matches: Vec<&MailRow> = rows
        .iter()
        .filter(|row| row.msg.to_lowercase().contains(&query_lower))
        .collect();
    if matches.is_empty() {
        return ActionOutcome::completed(format!(
            "Action result: mailbox\nno mail matching \"{query}\""
        ));
    }
    let count = list_count(action);
    let shown = matches.len().min(count);
    let mut text = format!(
        "Action result: mailbox\nFound {} mails matching \"{query}\", showing {shown}:",
        matches.len()
    );
    for row in matches.into_iter().take(shown) {
        text.push('\n');
        text.push_str(&format_mail_line(row));
        let preview = preview_words(&row.msg, &query_lower, MAILBOX_SEARCH_WINDOW_WORDS);
        if !preview.is_empty() {
            text.push_str("\n  ");
            text.push_str(&preview);
        }
    }
    ActionOutcome::completed(text)
}

fn execute_read(core: &AgentCore, action: &ParsedAction) -> ActionOutcome {
    let mail_id = action.input_str("mail_id");
    if mail_id.is_empty() {
        return ActionOutcome::failed("Action result: mailbox\nerror: mail_id_required");
    }
    let rows = load_mail_rows(core, Some(&mail_id), Some(1));
    match rows.iter().find(|row| row.id == mail_id) {
        Some(row) => ActionOutcome::completed(format!(
            "Action result: mailbox\n{}\n{}",
            format_mail_line(row),
            row.msg
        )),
        None => {
            ActionOutcome::completed(format!("Action result: mailbox\nno mail with id {mail_id}"))
        }
    }
}

/// One merged mailbox entry: received ("in") or sent ("out") mail.
struct MailRow {
    id: String,
    direction: &'static str,
    peer: String,
    state: String,
    msg: String,
    created_at_ms: u128,
}

/// The session's host-owned mailbox.json lives beside the session metadata,
/// exactly where the Web Host persists mail state.
fn mailbox_json_path(core: &AgentCore) -> Option<std::path::PathBuf> {
    let memory_dir = core.memory_file().parent()?.to_path_buf();
    Some(
        memory_dir
            .join("sessions")
            .join(core.current_session_id())
            .join("mailbox.json"),
    )
}

/// Load the body-free session index and assemble bodies from the mem-wide
/// mail ledger, newest first. Missing/malformed files behave as an empty box.
fn load_mail_rows(
    core: &AgentCore,
    only_id: Option<&str>,
    body_limit: Option<usize>,
) -> Vec<MailRow> {
    let mut rows: Vec<MailRow> = Vec::new();
    let memory_file = core.memory_file();
    let Some(memory_dir) = memory_file.parent() else {
        return rows;
    };
    let Some(path) = mailbox_json_path(core) else {
        return rows;
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return rows;
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return rows;
    };
    for entry in state
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = str_value(entry, "id");
        let direction = str_value(entry, "dir");
        if id.is_empty()
            || !matches!(direction.as_str(), "in" | "out")
            || only_id.is_some_and(|wanted| wanted != id)
        {
            continue;
        }
        let state = str_value(entry, "state");
        rows.push(MailRow {
            direction: if direction == "in" { "in" } else { "out" },
            peer: str_value(entry, "peer"),
            state: if direction == "in" && state == "queued" {
                "unread".to_string()
            } else {
                state
            },
            msg: String::new(),
            id,
            created_at_ms: u128_value(entry, "created_at_ms"),
        });
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.created_at_ms));
    let body_count = body_limit.unwrap_or(rows.len()).min(rows.len());
    for row in rows.iter_mut().take(body_count) {
        let body_path = memory_dir
            .join("mail")
            .join(format!("mail_{}.json", row.id));
        row.msg = std::fs::read(body_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .map(|mail| str_value(&mail, "msg"))
            .unwrap_or_default();
    }
    rows
}

fn format_mail_line(row: &MailRow) -> String {
    format!(
        "[{}] {} {} {} {}",
        row.id,
        row.direction,
        row.peer,
        format_ms(row.created_at_ms),
        row.state
    )
}

fn preview_chars(msg: &str, max_chars: usize) -> String {
    if msg.chars().count() <= max_chars {
        return msg.to_string();
    }
    format!("{}...", msg.chars().take(max_chars).collect::<String>())
}

/// One case-insensitive body match, previewed as up to `window` words before
/// and after the first matched word, with ellipses marking truncation.
fn preview_words(msg: &str, query_lower: &str, window: usize) -> String {
    let words = split_words(msg);
    if words.is_empty() {
        return String::new();
    }
    let lower = msg.to_lowercase();
    let Some(byte_pos) = lower.find(query_lower) else {
        return preview_chars(msg, MAILBOX_PREVIEW_CHARS);
    };
    let match_char = lower[..byte_pos].chars().count();
    let matched = words
        .partition_point(|(_, end, _)| *end <= match_char)
        .min(words.len() - 1);
    let from = matched.saturating_sub(window);
    let to = (matched + window + 1).min(words.len());
    let mut out = String::new();
    if from > 0 {
        out.push_str("... ");
    }
    for (index, (_, _, word)) in words[from..to].iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(word);
    }
    if to < words.len() {
        out.push_str(" ...");
    }
    out
}

/// Whitespace-delimited words as `(char_start, char_end, text)` triples.
fn split_words(msg: &str) -> Vec<(usize, usize, &str)> {
    let mut words: Vec<(usize, usize, &str)> = Vec::new();
    let mut word_start: Option<(usize, usize)> = None; // (char index, byte index)
    let mut char_count = 0;
    for (byte_idx, ch) in msg.char_indices() {
        if ch.is_whitespace() {
            if let Some((start_char, start_byte)) = word_start.take() {
                words.push((start_char, char_count, &msg[start_byte..byte_idx]));
            }
        } else if word_start.is_none() {
            word_start = Some((char_count, byte_idx));
        }
        char_count += 1;
    }
    if let Some((start_char, start_byte)) = word_start {
        words.push((start_char, char_count, &msg[start_byte..]));
    }
    words
}

/// UTC timestamp as `YYYY-MM-DD HH:MM:SS`, via Hinnant's civil-from-days.
fn format_ms(ms: u128) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn list_count(action: &ParsedAction) -> usize {
    action
        .input_u64("count")
        .unwrap_or(MAILBOX_LIST_DEFAULT_COUNT)
        .clamp(1, MAILBOX_LIST_MAX_COUNT) as usize
}

fn str_value(mail: &serde_json::Value, key: &str) -> String {
    mail.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn u128_value(mail: &serde_json::Value, key: &str) -> u128 {
    mail.get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u128
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

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_mailbox_tests.rs"]
mod tests;
