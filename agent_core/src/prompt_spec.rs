use serde_json::Value;

const RESPONSE_V1_SUMMARY: &str = include_str!("../../resources/response_v1_summary.json");

pub fn response_v1_summary_value() -> Value {
    serde_json::from_str(RESPONSE_V1_SUMMARY)
        .expect("response_v1 summary resource must be valid JSON")
}

pub fn enrich_static_prompt_with_response_schema(static_prompt: &str) -> String {
    if let Some(enriched) = replace_json_string_field_with_value(
        static_prompt,
        "json_schema_summary",
        &response_v1_summary_value(),
    ) {
        return enriched;
    }

    let Ok(mut value) = serde_json::from_str::<Value>(static_prompt) else {
        return static_prompt.to_string();
    };
    if let Some(response_rule) = value
        .get_mut("Response_rule")
        .and_then(Value::as_object_mut)
    {
        response_rule.insert(
            "json_schema_summary".to_string(),
            response_v1_summary_value(),
        );
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| static_prompt.to_string())
}

pub(crate) fn replace_json_string_field_with_value(
    source: &str,
    field: &str,
    replacement: &Value,
) -> Option<String> {
    let needle = format!("\"{field}\"");
    let field_start = source.find(&needle)?;
    let after_field = field_start + needle.len();
    let colon_offset = source[after_field..].find(':')?;
    let colon = after_field + colon_offset;
    let mut value_start = colon + 1;
    while let Some(byte) = source.as_bytes().get(value_start) {
        if !byte.is_ascii_whitespace() {
            break;
        }
        value_start += 1;
    }
    if source.as_bytes().get(value_start).copied() != Some(b'"') {
        return None;
    }

    let mut value_end = value_start + 1;
    let mut escaped = false;
    for (offset, ch) in source[value_end..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            value_end += offset + ch.len_utf8();
            break;
        }
    }
    if source.as_bytes().get(value_end.saturating_sub(1)).copied() != Some(b'"') {
        return None;
    }

    let line_start = source[..value_start].rfind('\n').map_or(0, |idx| idx + 1);
    let base_indent = source[line_start..]
        .chars()
        .take_while(|ch| ch.is_ascii_whitespace())
        .map(char::len_utf8)
        .sum();
    let replacement_text = indent_pretty_json(replacement, base_indent);

    let mut output = String::with_capacity(source.len() + replacement_text.len());
    output.push_str(&source[..value_start]);
    output.push_str(&replacement_text);
    output.push_str(&source[value_end..]);
    Some(output)
}

fn indent_pretty_json(value: &Value, base_indent: usize) -> String {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    let indent = " ".repeat(base_indent);
    rendered
        .lines()
        .enumerate()
        .map(|(idx, line)| {
            if idx == 0 {
                line.to_string()
            } else {
                format!("{indent}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_v1_summary_resource_is_valid_and_named() {
        let summary = response_v1_summary_value();

        assert_eq!(
            summary.get("$id").and_then(Value::as_str),
            Some("https://timem.local/schemas/response_v1.schema.json")
        );
        assert!(summary
            .get("fields")
            .and_then(|value| value.get("report_job_progress?"))
            .is_some());
        assert!(summary
            .get("fields")
            .and_then(|value| value.get("continue"))
            .is_some());
        assert!(summary
            .get("action_object_spec")
            .and_then(|value| value.get("intent"))
            .is_some());
        let text = serde_json::to_string(&summary).unwrap();
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
    fn prompt_spec_injects_response_schema_summary() {
        let enriched = enrich_static_prompt_with_response_schema(
            r#"{"Response_rule":{"json_schema_summary":"stale"}}"#,
        );

        assert!(
            enriched.contains("\"$id\": \"https://timem.local/schemas/response_v1.schema.json\"")
        );
        assert!(enriched.contains("\"fields\""));
        assert!(enriched.contains("\"report_job_progress?\""));
        assert!(!enriched.contains("stale"));
    }

    #[test]
    fn string_field_replacement_preserves_surrounding_order() {
        let source = "{\n  \"first\": \"a\",\n  \"target\": \"replace me\",\n  \"last\": \"z\"\n}";
        let replaced = replace_json_string_field_with_value(
            source,
            "target",
            &serde_json::json!({"nested": true}),
        )
        .expect("field should be replaced");

        assert!(replaced.find("\"first\"").unwrap() < replaced.find("\"target\"").unwrap());
        assert!(replaced.find("\"target\"").unwrap() < replaced.find("\"last\"").unwrap());
        assert!(replaced.contains("\"nested\": true"));
    }
}
