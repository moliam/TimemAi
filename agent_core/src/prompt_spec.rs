use serde_json::Value;

const RESPONSE_V1_SUMMARY: &str = include_str!("../../resources/response_v1_summary.json");

pub fn response_v1_summary_value() -> Value {
    serde_json::from_str(RESPONSE_V1_SUMMARY)
        .expect("response_v1 summary resource must be valid JSON")
}

pub fn enrich_static_prompt_with_response_schema(static_prompt: &str) -> String {
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
            .get("optional")
            .and_then(|value| value.get("report_job_progress?"))
            .is_some());
        assert!(summary
            .get("action_object_spec")
            .and_then(|value| value.get("intent"))
            .is_some());
    }

    #[test]
    fn prompt_spec_injects_response_schema_summary() {
        let enriched = enrich_static_prompt_with_response_schema(
            r#"{"Response_rule":{"json_schema_summary":"stale"}}"#,
        );

        assert!(
            enriched.contains("\"$id\": \"https://timem.local/schemas/response_v1.schema.json\"")
        );
        assert!(enriched.contains("\"report_job_progress?\""));
        assert!(!enriched.contains("stale"));
    }
}
