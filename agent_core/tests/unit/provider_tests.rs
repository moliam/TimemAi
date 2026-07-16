use super::*;

fn config(api_protocol: ApiProtocol) -> ProviderConfig {
    ProviderConfig {
        provider: "test".to_string(),
        model: "test-model".to_string(),
        base_url: "https://example.invalid/v1".to_string(),
        api_key: "dummy".to_string(),
        timeout_secs: 1,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        api_protocol,
        response_protocol: ResponseProtocolKind::Markdown,
    }
}

#[test]
fn provider_defaults_are_core_owned_pure_functions() {
    assert_eq!(
        parse_api_protocol("openai-compatible").unwrap(),
        ApiProtocol::OpenAiCompatible
    );
    assert_eq!(
        parse_api_protocol("responses").unwrap(),
        ApiProtocol::OpenAiResponses
    );
    assert_eq!(
        parse_api_protocol("claude").unwrap(),
        ApiProtocol::Anthropic
    );
    assert!(parse_api_protocol("unknown").is_err());

    assert_eq!(
        default_api_protocol_for_provider("openai"),
        ApiProtocol::OpenAiResponses
    );
    assert_eq!(
        default_api_protocol_for_provider("anthropic"),
        ApiProtocol::Anthropic
    );
    assert_eq!(
        default_api_protocol_for_provider("aliyun"),
        ApiProtocol::OpenAiCompatible
    );
    assert_eq!(default_model_for_provider("openai"), "gpt-4o");
    assert_eq!(
        default_model_for_provider("anthropic"),
        "claude-sonnet-4-20250514"
    );
    assert_eq!(default_model_for_provider("custom"), "qwen-plus");
}

#[test]
fn provider_default_detection_handles_known_and_custom_gateways() {
    assert!(is_default_model_for_provider("aliyun", "qwen-plus"));
    assert!(is_default_model_for_provider(
        "anthropic",
        "claude-sonnet-4"
    ));
    assert!(!is_default_model_for_provider("openai", "claude-sonnet-4"));
    assert!(is_default_model_for_provider("private", "any-model-name"));

    assert_eq!(
        known_default_base_url_for_provider("dashscope").as_deref(),
        Some("https://dashscope.aliyuncs.com/compatible-mode/v1")
    );
    assert_eq!(known_default_base_url_for_provider("private"), None);
    assert!(is_default_base_url_for_provider(
        "aliyun",
        "https://dashscope.aliyuncs.com/compatible-mode/v1/"
    ));
    assert!(!is_default_base_url_for_provider(
        "openai",
        "https://example.invalid/v1"
    ));
    assert!(is_default_base_url_for_provider(
        "private",
        "https://example.invalid/v1"
    ));
}

#[test]
fn openai_compatible_request_uses_messages_and_structured_output() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.provider = "aliyun".to_string();
    config.model = "qwen-plus".to_string();
    config.max_llm_output_tokens = 2048;
    let body = build_provider_request(
        &config,
        &[ProviderPromptBlock {
            role: ProviderPromptRole::System,
            text: "Return JSON".to_string(),
            cache: ProviderCacheControl::Ephemeral,
        }],
        StructuredOutputHint::JsonObject,
    );

    assert_eq!(body["max_tokens"], 2048);
    assert_eq!(body["model"], "qwen-plus");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(body["response_format"]["type"], "json_object");
}

#[test]
fn structured_output_strategy_is_provider_and_protocol_specific() {
    let mut aliyun = config(ApiProtocol::OpenAiCompatible);
    aliyun.provider = "aliyun".to_string();
    aliyun.response_protocol = ResponseProtocolKind::Json;
    assert_eq!(
        plan_structured_output(&aliyun),
        StructuredOutputHint::JsonObject
    );

    aliyun.response_protocol = ResponseProtocolKind::Markdown;
    assert_eq!(plan_structured_output(&aliyun), StructuredOutputHint::None);
    let markdown_body = build_provider_request(
        &aliyun,
        &[ProviderPromptBlock {
            role: ProviderPromptRole::System,
            text: "The top-level response is Markdown, not JSON.".to_string(),
            cache: ProviderCacheControl::None,
        }],
        plan_structured_output(&aliyun),
    );
    assert!(markdown_body.get("response_format").is_none());

    aliyun.response_protocol = ResponseProtocolKind::Xml;
    assert_eq!(plan_structured_output(&aliyun), StructuredOutputHint::None);
    let xml_body = build_provider_request(
        &aliyun,
        &[ProviderPromptBlock {
            role: ProviderPromptRole::System,
            text: "The top-level response is XML, not JSON or Markdown.".to_string(),
            cache: ProviderCacheControl::None,
        }],
        plan_structured_output(&aliyun),
    );
    assert!(xml_body.get("response_format").is_none());

    let mut custom = config(ApiProtocol::OpenAiCompatible);
    custom.provider = "custom".to_string();
    custom.response_protocol = ResponseProtocolKind::Json;
    assert_eq!(plan_structured_output(&custom), StructuredOutputHint::None);
    let body = build_provider_request(
        &custom,
        &[ProviderPromptBlock {
            role: ProviderPromptRole::System,
            text: "hello".to_string(),
            cache: ProviderCacheControl::None,
        }],
        plan_structured_output(&custom),
    );
    assert!(body.get("response_format").is_none());

    let mut anthropic = config(ApiProtocol::Anthropic);
    anthropic.provider = "anthropic".to_string();
    assert_eq!(
        plan_structured_output(&anthropic),
        StructuredOutputHint::None
    );
}

#[test]
fn anthropic_request_maps_cache_strategy_blocks_to_content_blocks() {
    let mut config = config(ApiProtocol::Anthropic);
    config.provider = "anthropic".to_string();
    config.model = "claude-sonnet-4-20250514".to_string();
    config.max_llm_output_tokens = 2048;
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## TIMEM_ASSISTANT\ndelta1\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_2\n\n## USER\ndelta2\n[END DELTA]";

    let prepared = prepare_provider_request(&config, prompt);
    let body = prepared.body;

    assert_eq!(body["max_tokens"], 2048);
    assert_eq!(body["system"][0]["text"], "STATIC");
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    assert!(body["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("delta1"));
    assert_eq!(
        body["messages"][0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert!(body["messages"][0]["content"][1]["text"]
        .as_str()
        .unwrap()
        .contains("delta2"));
    assert_eq!(
        body["messages"][0]["content"][1]["cache_control"]["type"],
        "ephemeral"
    );
}

#[test]
fn anthropic_request_sends_formatted_response_trailer_without_cache_control() {
    let mut config = config(ApiProtocol::Anthropic);
    config.provider = "anthropic".to_string();
    let prompt = format!(
            "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nhello\n[END DELTA]\n\n{}",
            crate::prompt_render::formatted_response_trailer("XML", "Ai4")
        );

    let prepared = prepare_provider_request(&config, &prompt);
    let content = prepared.body["messages"][0]["content"].as_array().unwrap();

    assert_eq!(
        content.last().unwrap()["text"],
        "please fulfill your response in XML only:\n## Ai4"
    );
    assert_eq!(content.last().unwrap().get("cache_control"), None);
    assert!(!content[0]["text"]
        .as_str()
        .unwrap()
        .contains("please fulfill your response only"));
}

#[test]
fn openai_responses_request_uses_official_shape() {
    let mut config = config(ApiProtocol::OpenAiResponses);
    config.provider = "openai".to_string();
    config.model = "gpt-4o".to_string();
    config.max_llm_output_tokens = 2048;
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC_GLOBAL\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nhello\n[END DELTA]";

    let prepared = prepare_provider_request(&config, prompt);
    let body = prepared.body;

    assert_eq!(config.endpoint(), "https://example.invalid/v1/responses");
    assert_eq!(body["model"], "gpt-4o");
    assert_eq!(body["max_output_tokens"], 2048);
    assert!(body["instructions"]
        .as_str()
        .unwrap()
        .contains("STATIC_GLOBAL"));
    assert!(body["input"].as_str().unwrap().contains("[BEGIN DELTA]"));
    assert!(body.get("messages").is_none());
    assert!(body.get("max_llm_output_tokens").is_none());
}

#[test]
fn openai_compatible_request_splits_static_and_dynamic_prompt() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.provider = "aliyun".to_string();
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC_GLOBAL\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nsecret\n[END DELTA]";

    let prepared = prepare_provider_request(&config, prompt);
    let body = prepared.body;
    let system_content = body["messages"][0]["content"].as_str().unwrap();
    let user_content = body["messages"][1]["content"].as_str().unwrap();

    assert!(system_content.contains("STATIC_GLOBAL"));
    assert!(!system_content.contains("[BEGIN DELTA]"));
    assert_eq!(body["messages"][0]["cache_control"]["type"], "ephemeral");
    assert!(!system_content.contains("prompt_0"));
    assert!(user_content.contains("[BEGIN DELTA]"));
    assert!(user_content.contains("secret"));
    assert!(!user_content.contains("STATIC_GLOBAL"));
}

#[test]
fn openai_compatible_request_maps_cache_strategy_to_messages() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.provider = "aliyun".to_string();
    config.model = "qwen-plus".to_string();
    let mut prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n".to_string();
    for idx in 1..=5 {
        prompt.push_str(&format!(
            "[BEGIN DELTA]\ndelta_id: pd_{idx}\n\n## TIMEM_ASSISTANT\ndelta {idx}\n[END DELTA]\n"
        ));
    }

    let prepared = prepare_provider_request(&config, &prompt);
    let messages = prepared.body["messages"].as_array().unwrap();

    assert_eq!(messages.len(), 6);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "STATIC");
    assert_eq!(messages[0]["cache_control"]["type"], "ephemeral");
    assert!(messages[1]["content"].as_str().unwrap().contains("delta 1"));
    assert!(messages[2]["content"].as_str().unwrap().contains("delta 2"));
    assert_eq!(messages[1].get("cache_control"), None);
    assert_eq!(messages[2].get("cache_control"), None);

    for idx in 3..=5 {
        assert!(messages[idx]["content"]
            .as_str()
            .unwrap()
            .contains(&format!("delta {idx}")));
        assert_eq!(messages[idx]["cache_control"]["type"], "ephemeral");
    }
}

#[test]
fn openai_compatible_request_sends_formatted_response_trailer_without_cache_control() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.provider = "aliyun".to_string();
    let prompt = format!(
            "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nhello\n[END DELTA]\n\n{}",
            crate::prompt_render::formatted_response_trailer("Markdown", "Ai9")
        );

    let prepared = prepare_provider_request(&config, &prompt);
    let messages = prepared.body["messages"].as_array().unwrap();

    assert_eq!(
        messages.last().unwrap()["content"],
        "please fulfill your response only:\n## Ai9"
    );
    assert_eq!(messages.last().unwrap().get("cache_control"), None);
    assert!(!messages[messages.len() - 2]["content"]
        .as_str()
        .unwrap()
        .contains("please fulfill your response only"));
}

#[test]
fn prepared_request_builds_body_and_prompt_cache_audit_without_prompt_text() {
    let mut config = config(ApiProtocol::Anthropic);
    config.provider = "anthropic".to_string();
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC SECRET\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\ndelta secret\n[END DELTA]";

    let prepared = prepare_provider_request(&config, prompt);

    assert_eq!(prepared.structured_output, StructuredOutputHint::None);
    assert_eq!(prepared.body["system"][0]["text"], "STATIC SECRET");
    assert_eq!(
        prepared.body["system"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert!(prepared.body["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("delta secret"));

    let audit = prepared.prompt_cache_plan.to_string();
    assert!(audit.contains("\"hash\""));
    assert!(audit.contains("\"chars\""));
    assert!(!audit.contains("STATIC SECRET"));
    assert!(!audit.contains("delta secret"));
}

#[test]
fn prepared_http_request_keeps_provider_headers_in_core() {
    let mut openai_like = config(ApiProtocol::OpenAiCompatible);
    openai_like.provider = "aliyun".to_string();
    openai_like.api_key = "test-openai-key".to_string();

    let http = prepare_provider_http_request(&openai_like, "Return JSON\nhello");
    assert_eq!(
        http.endpoint,
        "https://example.invalid/v1/chat/completions".to_string()
    );
    assert!(http
        .headers
        .contains(&("Content-Type".to_string(), "application/json".to_string())));
    assert!(http.headers.contains(&(
        "Authorization".to_string(),
        "Bearer test-openai-key".to_string()
    )));
    assert_eq!(http.provider_request.body["model"], openai_like.model);

    let mut anthropic = config(ApiProtocol::Anthropic);
    anthropic.provider = "anthropic".to_string();
    anthropic.api_key = "test-anthropic-key".to_string();

    let http = prepare_provider_http_request(&anthropic, "hello");
    assert_eq!(http.endpoint, "https://example.invalid/v1/messages");
    assert!(http
        .headers
        .contains(&("x-api-key".to_string(), "test-anthropic-key".to_string())));
    assert!(http
        .headers
        .contains(&("anthropic-version".to_string(), "2023-06-01".to_string())));
}

#[test]
fn anthropic_endpoint_avoids_double_v1_when_base_already_ends_with_v1() {
    let mut config = config(ApiProtocol::Anthropic);
    config.provider = "anthropic".to_string();
    config.base_url = "https://example.com/api/v1".to_string();
    assert_eq!(config.endpoint(), "https://example.com/api/v1/messages");

    config.base_url = "https://api.anthropic.com".to_string();
    assert_eq!(config.endpoint(), "https://api.anthropic.com/v1/messages");
}

#[test]
fn provider_http_response_interpretation_is_core_owned() {
    let config = config(ApiProtocol::OpenAiCompatible);
    let interpreted = interpret_provider_http_response(
        &config,
        200,
        r#"{
                "choices": [{"message": {"content": "{\"status\":\"finished\",\"final_answer\":\"ok\"}"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 2}
            }"#,
        "",
    );
    assert_eq!(interpreted.status, 200);
    let response = interpreted.result.unwrap();
    assert!(response.content.contains("final_answer"));
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 2);

    let interpreted = interpret_provider_http_response(
        &config,
        429,
        r#"{"error":{"message":"rate limit sk-sensitive-token"}}"#,
        "",
    );
    assert_eq!(interpreted.status, 429);
    let err = interpreted.result.unwrap_err();
    assert!(err.contains("provider_http_429"));
    assert!(!err.contains("sk-sensitive-token"));

    let interpreted =
        interpret_provider_http_response(&config, 200, "not json", "curl stderr detail");
    assert_eq!(interpreted.raw_json["raw_text"], "not json");
    assert_eq!(interpreted.raw_json["stderr"], "curl stderr detail");
    assert_eq!(interpreted.result.unwrap().content, "not json");
}

#[test]
fn provider_request_audit_event_is_redacted_and_ui_neutral() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.provider = "aliyun".to_string();
    config.api_key = "sk-sensitive-token".to_string();
    config.response_protocol = ResponseProtocolKind::Json;
    let mut prepared = prepare_provider_request(&config, "Return JSON\nhello");
    prepared.body["metadata"] = json!({"api_key":"sk-sensitive-token"});

    let audit = provider_request_audit_event(&config, &prepared);

    assert_eq!(audit["type"], "llm_request");
    assert_eq!(audit["provider"], config.provider);
    assert_eq!(audit["model"], config.model);
    assert_eq!(audit["endpoint"], config.endpoint());
    assert_eq!(audit["structured_output"], "json_object");
    assert!(audit["prompt_cache_plan"].is_array());
    let audit_text = audit.to_string();
    assert!(audit_text.contains("***REDACTED***"));
    assert!(!audit_text.contains("sk-sensitive-token"));
}

#[test]
fn provider_response_audit_event_is_redacted() {
    let audit = provider_response_audit_event(
        401,
        &json!({
            "error": {"message": "bad token sk-sensitive-token"},
            "api_key": "sk-sensitive-token"
        }),
    );

    assert_eq!(audit["type"], "llm_response");
    assert_eq!(audit["status"], 401);
    assert_eq!(audit["error_kind"], "http_error");
    assert_eq!(
        audit["response"]["error"]["message"],
        json!("bad token ***REDACTED***")
    );
    let audit_text = audit.to_string();
    assert!(audit_text.contains("***REDACTED***"));
    assert!(!audit_text.contains("sk-sensitive-token"));
}

#[test]
fn anthropic_response_counts_cache_tokens() {
    let response = parse_provider_response(
        &config(ApiProtocol::Anthropic),
        &json!({
            "content":[{"type":"text","text":"ok"}],
            "usage":{
                "input_tokens":10,
                "cache_read_input_tokens":20,
                "cache_creation_input_tokens":30,
                "output_tokens":4
            }
        }),
    )
    .unwrap();

    assert_eq!(response.content, "ok");
    assert_eq!(response.usage.prompt_tokens, 60);
    assert_eq!(response.usage.cached_tokens, 20);
    assert_eq!(response.usage.cache_created_tokens, 30);
    assert_eq!(response.usage.completion_tokens, 4);
}

#[test]
fn openai_compatible_response_reads_cache_and_truncation() {
    let empty = parse_provider_response(
        &config(ApiProtocol::OpenAiCompatible),
        &json!({
            "choices":[{"finish_reason":"stop","message":{"content":"","role":"assistant"}}],
            "usage":{"prompt_tokens":15707,"completion_tokens":2,"total_tokens":15709}
        }),
    )
    .unwrap();
    assert_eq!(empty.content, "");
    assert_eq!(empty.usage.prompt_tokens, 15707);
    assert_eq!(empty.usage.completion_tokens, 2);
    assert!(!empty.truncated);

    let response = parse_provider_response(
        &config(ApiProtocol::OpenAiCompatible),
        &json!({
            "choices":[{"message":{"content":"{\"free_talk\":\"hi\"}"}}],
            "usage":{
                "prompt_tokens":3019,
                "completion_tokens":104,
                "total_tokens":3123,
                "prompt_tokens_details":{"cached_tokens":2048}
            }
        }),
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 3019);
    assert_eq!(response.usage.completion_tokens, 104);
    assert_eq!(response.usage.cached_tokens, 2048);
    assert!(!response.truncated);

    let response = parse_provider_response(
            &config(ApiProtocol::OpenAiCompatible),
            &json!({
                "choices":[{"finish_reason":"length","message":{"content":"{\"free_talk\":\"partial\"}"}}],
                "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}
            }),
        )
        .unwrap();
    assert!(response.truncated);

    let response = parse_provider_response(
        &config(ApiProtocol::OpenAiCompatible),
        &json!({
            "choices":[{"message":{"content":"{\"free_talk\":\"hi\"}"}}],
            "usage":{
                "prompt_tokens":8868,
                "cache_creation_input_tokens":0,
                "cache_read_input_tokens":4096,
                "completion_tokens":1095,
                "total_tokens":9963
            }
        }),
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 8868);
    assert_eq!(response.usage.completion_tokens, 1095);
    assert_eq!(response.usage.cached_tokens, 4096);
}

#[test]
fn openai_responses_response_reads_usage_text_and_truncation() {
    let response = parse_provider_response(
        &config(ApiProtocol::OpenAiResponses),
        &json!({
            "output_text":"{\"free_talk\":\"hi\"}",
            "usage":{
                "input_tokens":8438,
                "input_tokens_details":{"cached_tokens":4096},
                "output_tokens":398,
                "output_tokens_details":{"reasoning_tokens":0},
                "total_tokens":8836
            }
        }),
    )
    .unwrap();
    assert_eq!(response.content, "{\"free_talk\":\"hi\"}");
    assert_eq!(response.usage.prompt_tokens, 8438);
    assert_eq!(response.usage.completion_tokens, 398);
    assert_eq!(response.usage.total_tokens, 8836);
    assert_eq!(response.usage.cached_tokens, 4096);
    assert!(!response.truncated);

    let response = parse_provider_response(
        &config(ApiProtocol::OpenAiResponses),
        &json!({
            "status":"incomplete",
            "incomplete_details":{"reason":"max_output_tokens"},
            "output_text":"{\"free_talk\":\"partial\"}",
            "usage":{"input_tokens":10,"output_tokens":10,"total_tokens":20}
        }),
    )
    .unwrap();
    assert!(response.truncated);

    let response = parse_provider_response(
            &config(ApiProtocol::OpenAiResponses),
            &json!({
                "output":[{
                    "type":"message",
                    "role":"assistant",
                    "content":[{"type":"output_text","text":"{\"free_talk\":\"from output\"}","annotations":[]}]
                }],
                "usage":{
                    "input_tokens":32,
                    "input_tokens_details":{"cached_tokens":0},
                    "output_tokens":18,
                    "output_tokens_details":{"reasoning_tokens":0},
                    "total_tokens":50
                }
            }),
        )
        .unwrap();
    assert_eq!(response.content, "{\"free_talk\":\"from output\"}");
    assert_eq!(response.usage.prompt_tokens, 32);
    assert_eq!(response.usage.completion_tokens, 18);
    assert_eq!(response.usage.cached_tokens, 0);
}

#[test]
fn anthropic_response_reads_cache_creation_truncation_and_missing_cache_defaults() {
    let response = parse_provider_response(
        &config(ApiProtocol::Anthropic),
        &json!({
            "content":[{"type":"text","text":"ok"}],
            "usage":{
                "input_tokens":3,
                "cache_creation_input_tokens":6155,
                "cache_read_input_tokens":0,
                "output_tokens":318
            }
        }),
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 6158);
    assert_eq!(response.usage.completion_tokens, 318);
    assert_eq!(response.usage.total_tokens, 6476);
    assert_eq!(response.usage.cached_tokens, 0);
    assert_eq!(response.usage.cache_created_tokens, 6155);
    assert!(!response.truncated);

    let response = parse_provider_response(
        &config(ApiProtocol::Anthropic),
        &json!({
            "stop_reason":"max_tokens",
            "content":[{"type":"text","text":"{\"free_talk\":\"partial\"}"}],
            "usage":{"input_tokens":10,"output_tokens":10}
        }),
    )
    .unwrap();
    assert!(response.truncated);

    let response = parse_provider_response(
        &config(ApiProtocol::Anthropic),
        &json!({
            "content":[{"type":"text","text":"ok"}],
            "usage":{"input_tokens":10,"output_tokens":5}
        }),
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.total_tokens, 15);
    assert_eq!(response.usage.cached_tokens, 0);
}

#[test]
fn provider_http_error_includes_sanitized_provider_reason() {
    let openai_like = json!({
        "error": {
            "message": "The model `missing-model` does not exist or you do not have access to it.",
            "type": "invalid_request_error"
        }
    });
    assert_eq!(
            provider_http_error_message(400, &openai_like),
            "provider_http_400: The model `missing-model` does not exist or you do not have access to it."
        );

    let anthropic_like = json!({
        "type": "error",
        "error": {
            "type": "not_found_error",
            "message": "model: claude-missing not found"
        }
    });
    assert_eq!(
        provider_http_error_message(404, &anthropic_like),
        "provider_http_404: model: claude-missing not found"
    );

    let raw_text = json!({"raw_text":"invalid Authorization Bearer sk-secret-token"});
    let rendered = provider_http_error_message(401, &raw_text);
    assert!(rendered.starts_with("provider_http_401:"));
    assert!(rendered.contains("***REDACTED***"));
    assert!(!rendered.contains("sk-secret-token"));

    let long = provider_http_error_message(400, &json!({"error":{"message":"x ".repeat(400)}}));
    assert!(long.contains('…'));
    assert!(long.len() < 280);

    let timeout = provider_http_error_message(
        0,
        &json!({"raw_text":"","stderr":"curl: (28) Operation timed out after 120006 milliseconds with 0 bytes received"}),
    );
    assert!(timeout.starts_with("provider_timeout:"));
    assert!(timeout.contains("Operation timed out"));
}

#[test]
fn provider_http_error_is_resilient_to_unusual_bodies() {
    for body in [
        Value::Null,
        json!("plain string error"),
        json!(["array", "error"]),
        json!({"error":{"message":null,"details":[{"x":1}]}}),
        json!({"detail":{"nested":"not a string"}}),
        json!({"raw_text":""}),
    ] {
        let rendered = provider_http_error_message(500, &body);
        assert!(rendered.starts_with("provider_http_500"));
        assert!(rendered.len() < 280);
    }
}
