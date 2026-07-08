use serde_json::Value;

pub fn response_v1_summary_value(response_schema_summary: &str) -> Value {
    serde_json::from_str(response_schema_summary)
        .expect("response_v1 summary resource must be valid JSON")
}

pub fn enrich_static_prompt_with_response_schema(
    static_prompt: &str,
    response_schema_summary: &str,
) -> String {
    if static_prompt.contains("{{RESPONSE_V1_SCHEMA}}") {
        return static_prompt.replace("{{RESPONSE_V1_SCHEMA}}", response_schema_summary);
    }
    static_prompt.to_string()
}

pub(crate) fn replace_markdown_placeholder_with_text(
    source: &str,
    placeholder: &str,
    replacement: &str,
) -> Option<String> {
    source
        .contains(placeholder)
        .then(|| source.replace(placeholder, replacement))
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEST_JSON_RESPONSE_SCHEMA: &str =
        include_str!("../../resources/protocol/json/response_schema_summary.json");
    const TEST_MARKDOWN_RESPONSE_SCHEMA: &str =
        include_str!("../../resources/protocol/markdown/response_schema_summary.md");

    #[test]
    fn json_response_v1_summary_resource_is_valid() {
        let summary = response_v1_summary_value(TEST_JSON_RESPONSE_SCHEMA);

        assert!(summary.get("$id").is_none());
        assert!(summary
            .get("fields")
            .and_then(|value| value.get("progress?"))
            .is_some());
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
            .is_some());
        let text = serde_json::to_string(&summary).unwrap();
        assert!(text.contains("context_compact?"));
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
    fn prompt_spec_injects_markdown_response_summary_into_plain_placeholder() {
        let enriched = enrich_static_prompt_with_response_schema(
            "## Response Protocol\n{{RESPONSE_V1_SCHEMA}}",
            TEST_MARKDOWN_RESPONSE_SCHEMA,
        );

        assert!(enriched.contains("Markdown response sections."));
        assert!(enriched.contains("The top-level response is Markdown, not JSON."));
        assert!(enriched.contains("`## Status`"));
        assert!(enriched.contains("`## Progress`"));
        assert!(enriched.contains("`## Final_Answer`"));
        assert!(enriched.contains("`## Free_talk`"));
        assert!(enriched.contains("`## Working_Still_Action`"));
        assert!(enriched.contains("`## Context Compact`"));
        assert!(enriched.contains("inside `## Working_Still_Action` use JSON objects."));
        assert!(!enriched.contains("{{RESPONSE_V1_SCHEMA}}"));
        assert!(!enriched.contains("\"sections\""));
        assert!(!enriched.contains("\"fields\""));
    }

    #[test]
    fn prompt_spec_does_not_magic_rewrite_legacy_json_prompt_fields() {
        let legacy = r#"{"Response_rule":{"json_schema_summary":"stale"}}"#;
        let enriched =
            enrich_static_prompt_with_response_schema(legacy, TEST_MARKDOWN_RESPONSE_SCHEMA);

        assert_eq!(enriched, legacy);
    }
}
