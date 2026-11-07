use crate::response_protocol::ParsedAction;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use super::{from_action, KEYWORDS_HARD_CAP_WORDS};

fn action(raw_input: Value) -> ParsedAction {
    ParsedAction {
        action: "context_compress".to_string(),
        name: None,
        call_id: "call_test".to_string(),
        raw_input,
    }
}

fn keywords_of(count: usize) -> String {
    (0..count)
        .map(|index| format!("kw{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn object(input: Value) -> Value {
    let map: BTreeMap<String, Value> =
        serde_json::from_value(input).expect("test input must be an object");
    serde_json::to_value(map).expect("serialize test input")
}

#[test]
fn parses_full_input_dedupes_ids_and_keeps_fields() {
    let parsed = from_action(&action(object(json!({
        "keep": ["pd_4", "pd_4"],
        "offload": ["pd_2"],
        "summary": "  checkpoint  ",
        "keywords": "alpha, beta"
    }))))
    .unwrap();
    assert_eq!(parsed.keep_delta_ids, vec!["pd_4"]);
    assert_eq!(parsed.offload_delta_ids, vec!["pd_2"]);
    assert_eq!(parsed.delta_ids.len(), 2);
    assert_eq!(parsed.summary, "checkpoint");
    assert_eq!(parsed.keywords, "alpha, beta");
}

#[test]
fn rejects_overlap_between_keep_and_offload() {
    let error = from_action(&action(object(json!({
        "keep": ["pd_4"],
        "offload": ["pd_4"],
        "summary": "s",
        "keywords": "k"
    }))))
    .unwrap_err();
    assert!(error.contains("keep_offload_overlap"), "{error}");
}

#[test]
fn enforces_keyword_hard_cap_promised_by_the_prompt() {
    let over = from_action(&action(object(json!({
        "summary": "s",
        "keywords": keywords_of(KEYWORDS_HARD_CAP_WORDS + 1)
    }))))
    .unwrap_err();
    assert!(over.contains("keywords_too_long"), "{over}");

    let exact = from_action(&action(object(json!({
        "summary": "s",
        "keywords": keywords_of(KEYWORDS_HARD_CAP_WORDS)
    }))))
    .unwrap();
    assert!(!exact.keywords.is_empty());
}

#[test]
fn requires_summary_and_keywords() {
    let error = from_action(&action(object(json!({"keywords": "k"})))).unwrap_err();
    assert!(error.contains("summary_required"), "{error}");
    let error = from_action(&action(object(json!({"summary": "s"})))).unwrap_err();
    assert!(error.contains("keywords_required"), "{error}");
}
