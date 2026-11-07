use super::*;
use crate::CoreProfile;
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

struct NoopRuntime;
impl ActionRuntime for NoopRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }

    fn request_mail_send(
        &mut self,
        _event: &crate::host::CoreTopicEvent,
    ) -> Option<crate::host::MailSendReply> {
        Some(crate::host::MailSendReply {
            mail_id: "mail_noop01".to_string(),
            initially_read: false,
        })
    }
}

struct RecordingRuntime {
    topics: Vec<crate::host::CoreTopicEvent>,
    initially_read: bool,
}

impl ActionRuntime for RecordingRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }

    fn on_core_topic_events(&mut self, events: &[crate::host::CoreTopicEvent]) {
        self.topics.extend_from_slice(events);
    }

    fn request_mail_send(
        &mut self,
        event: &crate::host::CoreTopicEvent,
    ) -> Option<crate::host::MailSendReply> {
        self.topics.push(event.clone());
        Some(crate::host::MailSendReply {
            mail_id: "mail_abc123".to_string(),
            initially_read: self.initially_read,
        })
    }
}

fn mailbox_test_core(tag: &str) -> AgentCore {
    let root = std::env::temp_dir().join(format!(
        "timem_mailbox_tool_{tag}_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    )
}

fn write_mailbox_json(core: &AgentCore, state: serde_json::Value) {
    let dir = core
        .memory_file()
        .parent()
        .unwrap()
        .join("sessions")
        .join(core.current_session_id());
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("mailbox.json"),
        serde_json::to_string(&state).unwrap(),
    )
    .unwrap();
}

fn write_mem_mail(core: &AgentCore, id: &str, msg: &str) {
    let dir = core.memory_file().parent().unwrap().join("mail");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("mail_{id}.json")),
        serde_json::to_string(&json!({
            "id": id, "from": "mb-self", "to": "mb-peer", "msg": msg, "created_at_ms": 0
        }))
        .unwrap(),
    )
    .unwrap();
}

fn seeded_state(core: &AgentCore) -> serde_json::Value {
    for (id, msg) in [
        ("m_old", "deploy the staging cluster tonight"),
        ("m_new", "please run the regression suite for staging"),
        ("s_mid", "handoff: verify the spool recovery path"),
    ] {
        write_mem_mail(core, id, msg);
    }
    json!({
        "mailbox_name": "mb-self",
        "entries": [
            {"id":"m_old","dir":"in","peer":"mb-aaa","created_at_ms":1000,"state":"queued","receipt_sent":false},
            {"id":"m_new","dir":"in","peer":"mb-bbb","created_at_ms":2000,"state":"read","receipt_sent":true},
            {"id":"s_mid","dir":"out","peer":"mb-ccc","created_at_ms":3000,"state":"sent","resp_timeout_secs":600,"notified":false}
        ]
    })
}

fn run(core: &mut AgentCore, input: serde_json::Value) -> String {
    let mut runtime = NoopRuntime;
    let action = ParsedAction {
        action: "mailbox".to_string(),
        name: None,
        call_id: "test_call".to_string(),
        raw_input: input,
    };
    match execute_action(core, &action, &mut runtime) {
        ActionExecution::Completed(outcome) => outcome.text,
        ActionExecution::NeedsApproval(_) => panic!("mailbox never needs approval"),
    }
}

#[test]
fn mailbox_list_merges_in_and_out_newest_first() {
    let mut core = mailbox_test_core("list");
    write_mailbox_json(&core, seeded_state(&core));
    let text = run(&mut core, json!({"action": "list"}));

    assert!(
        text.contains("Most recent 3 of 3 mails (in + out), newest first:"),
        "{text}"
    );
    let s_mid = text
        .find("[s_mid] out mb-ccc 1970-01-01 00:00:03 sent")
        .unwrap();
    let m_new = text
        .find("[m_new] in mb-bbb 1970-01-01 00:00:02 read")
        .unwrap();
    let m_old = text
        .find("[m_old] in mb-aaa 1970-01-01 00:00:01 unread")
        .unwrap();
    assert!(s_mid < m_new && m_new < m_old, "{text}");
    assert!(
        text.contains("\n  handoff: verify the spool recovery path"),
        "{text}"
    );

    let capped = run(&mut core, json!({"action": "list", "count": 2}));
    assert!(capped.contains("Most recent 2 of 3 mails"), "{capped}");
    assert!(!capped.contains("[m_old]"), "{capped}");
}

#[test]
fn mailbox_search_matches_case_insensitively_with_word_window() {
    let mut core = mailbox_test_core("search");
    write_mailbox_json(&core, seeded_state(&core));

    let hit = run(
        &mut core,
        json!({"action": "search", "query": "REGRESSION"}),
    );
    assert!(
        hit.contains("Found 1 mails matching \"REGRESSION\""),
        "{hit}"
    );
    assert!(
        hit.contains("[m_new] in mb-bbb 1970-01-01 00:00:02 read"),
        "{hit}"
    );
    assert!(hit.contains("regression suite"), "{hit}");

    let miss = run(&mut core, json!({"action": "search", "query": "quantum"}));
    assert!(miss.contains("no mail matching \"quantum\""), "{miss}");

    let missing_query = run(&mut core, json!({"action": "search"}));
    assert!(
        missing_query.contains("error: query_required"),
        "{missing_query}"
    );
}

#[test]
fn mailbox_search_window_caps_at_128_words_each_side() {
    let mut core = mailbox_test_core("window");
    let mut words: Vec<String> = (0..400).map(|i| format!("w{i}")).collect();
    words[200] = "needle".to_string();
    write_mem_mail(&core, "m_long", &words.join(" "));
    write_mailbox_json(
        &core,
        json!({
            "mailbox_name": "mb-self",
            "entries": [
                {"id":"m_long","dir":"in","peer":"mb-aaa","created_at_ms":1000,"state":"queued","receipt_sent":false}
            ]
        }),
    );

    let hit = run(&mut core, json!({"action": "search", "query": "needle"}));
    let preview_line = hit
        .lines()
        .find(|line| line.starts_with("  "))
        .unwrap()
        .trim();
    assert!(preview_line.starts_with("... w72 "), "{preview_line}");
    assert!(preview_line.ends_with(" w328 ..."), "{preview_line}");
    // 128 words before + needle + 128 after, plus the two ellipsis tokens.
    assert_eq!(preview_line.split_whitespace().count(), 128 + 1 + 128 + 2);
}

#[test]
fn mailbox_read_prints_full_body_and_reports_unknown_ids() {
    let mut core = mailbox_test_core("read");
    write_mailbox_json(&core, seeded_state(&core));

    let hit = run(&mut core, json!({"action": "read", "mail_id": "s_mid"}));
    assert!(
        hit.starts_with("Action result: mailbox\n[s_mid] out mb-ccc 1970-01-01 00:00:03 sent\n"),
        "{hit}"
    );
    assert!(
        hit.ends_with("handoff: verify the spool recovery path"),
        "{hit}"
    );

    let miss = run(&mut core, json!({"action": "read", "mail_id": "nope"}));
    assert!(miss.contains("no mail with id nope"), "{miss}");

    let missing = run(&mut core, json!({"action": "read"}));
    assert!(missing.contains("error: mail_id_required"), "{missing}");
}

#[test]
fn mailbox_list_on_empty_or_missing_mailbox_is_empty() {
    let mut core = mailbox_test_core("empty");
    let text = run(&mut core, json!({"action": "list"}));
    assert!(text.contains("mailbox is empty: no mail yet"), "{text}");
}

#[test]
fn mailbox_unknown_action_fails_closed() {
    let mut core = mailbox_test_core("unknown");
    let text = run(&mut core, json!({"action": "frobnicate"}));
    assert!(text.contains("error: unknown_action:frobnicate"), "{text}");
}

#[test]
fn mailbox_send_reports_the_runtime_assigned_id() {
    let mut core = mailbox_test_core("send_id");
    let mut runtime = RecordingRuntime {
        topics: Vec::new(),
        initially_read: false,
    };
    let action = ParsedAction {
        action: "mailbox".to_string(),
        name: None,
        call_id: "test_call".to_string(),
        raw_input: json!({"to": "mb-999", "msg": "hello there"}),
    };
    let text = match execute_action(&mut core, &action, &mut runtime) {
        ActionExecution::Completed(outcome) => outcome.text,
        ActionExecution::NeedsApproval(_) => panic!("mailbox never needs approval"),
    };
    assert!(text.contains("Mail sent to mailbox mb-999"), "{text}");
    assert!(
        text.contains("mail_abc123"),
        "action result must name the runtime-assigned id: {text}"
    );
    let sent = runtime.topics.first().expect("send topic");
    assert_eq!(sent.payload["op"], "send");
    assert!(
        sent.payload.get("mail_id").is_none(),
        "the tool must not invent a mail id: {sent:?}"
    );
    let request_id = sent.payload["request_id"]
        .as_str()
        .expect("topic payload carries the request id");
    assert!(
        request_id.starts_with("mailbox_send_"),
        "request id correlates the runtime reply: {request_id}"
    );
}

#[test]
fn mailbox_send_folds_a_fast_read_receipt_into_the_result() {
    let mut core = mailbox_test_core("send_consumed");
    let mut runtime = RecordingRuntime {
        topics: Vec::new(),
        initially_read: true,
    };
    let action = ParsedAction {
        action: "mailbox".to_string(),
        name: None,
        call_id: "test_call".to_string(),
        raw_input: json!({"to": "mb-999", "msg": "hello there"}),
    };
    let text = match execute_action(&mut core, &action, &mut runtime) {
        ActionExecution::Completed(outcome) => outcome.text,
        ActionExecution::NeedsApproval(_) => panic!("mailbox never needs approval"),
    };
    assert!(
        text.contains(
            "Mail sent to mailbox mb-999 (mail id mail_abc123) and already consumed by receiver."
        ),
        "a receipt observed during the send wait must fold into the result: {text}"
    );
    assert!(
        !text.contains("will be notified"),
        "no follow-up notice promise is needed once consumed: {text}"
    );
}

#[test]
fn mailbox_send_keeps_legacy_behavior_with_and_without_action() {
    let mut core = mailbox_test_core("send");
    let implicit = run(&mut core, json!({"to": "mb-999", "msg": "hello there"}));
    assert!(
        implicit.contains("Mail sent to mailbox mb-999"),
        "{implicit}"
    );
    let explicit = run(
        &mut core,
        json!({"action": "send", "to": "mb-999", "msg": "hello again"}),
    );
    assert!(
        explicit.contains("Mail sent to mailbox mb-999"),
        "{explicit}"
    );
    let missing_to = run(&mut core, json!({"action": "send", "msg": "hi"}));
    assert!(missing_to.contains("error: to_required"), "{missing_to}");
}
