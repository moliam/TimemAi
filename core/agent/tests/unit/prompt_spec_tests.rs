use super::*;
const TEST_JSON_RESPONSE_SCHEMA: &str =
    include_str!("../../../../resources/protocol/json/response_schema_summary.json");

#[test]
fn json_response_v1_summary_resource_is_valid() {
    let summary = response_v1_summary_value(TEST_JSON_RESPONSE_SCHEMA);

    assert!(summary.get("$id").is_none());
    assert!(summary
        .get("fields")
        .and_then(|value| value.get("status?"))
        .is_some());
    assert!(summary
        .get("fields")
        .and_then(|value| value.get("final_answer?"))
        .is_some());
    assert!(summary
        .get("fields")
        .and_then(|value| value.get("free_talk?"))
        .and_then(Value::as_str)
        .is_some());
    assert!(summary
        .get("fields")
        .and_then(|value| value.get("free_talk?"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .contains("kept visible"));
    assert!(summary
        .get("action_object_spec")
        .and_then(|value| value.get("intent?"))
        .is_none());
    let text = serde_json::to_string(&summary).unwrap();
    assert!(text.contains("context_compact may be followed by other actions"));
    for legacy_field in [
        "acceptance_check?",
        "continuation?",
        "memory_candidates?",
        "diagnostics.intent_inference?",
        "diagnostics.note?",
        "diagnostics.self_audit?",
        "forward compatibility",
        "Default true when absent",
        "runtime treats it as true",
    ] {
        assert!(
            !text.contains(legacy_field),
            "legacy response prompt field leaked into schema summary: {legacy_field}"
        );
    }
}

#[test]
fn prompt_spec_injects_json_response_summary_into_plain_placeholder() {
    let enriched = enrich_static_prompt_with_response_schema(
        "## Response Protocol\n{{RESPONSE_V1_SCHEMA}}",
        TEST_JSON_RESPONSE_SCHEMA,
    );

    assert!(enriched.contains("status?"));
    assert!(enriched.contains("final_answer?"));
    assert!(enriched.contains("working_still_action?"));
    assert!(!enriched.contains("{{RESPONSE_V1_SCHEMA}}"));
}

#[test]
fn prompt_spec_does_not_magic_rewrite_legacy_json_prompt_fields() {
    let legacy = r#"{"Response_rule":{"json_schema_summary":"stale"}}"#;
    let enriched = enrich_static_prompt_with_response_schema(legacy, TEST_JSON_RESPONSE_SCHEMA);

    assert_eq!(enriched, legacy);
}
