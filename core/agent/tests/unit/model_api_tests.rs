use super::*;
use crate::reasoning::{ReasoningDemand, ReasoningPreference};
use crate::{NativeExchange, NativeToolChoice, NativeToolResult, ToolCallMode};

fn config(api_protocol: ApiProtocol) -> ModelServiceConfig {
    ModelServiceConfig {
        interaction: Default::default(),
        model: "test-model".to_string(),
        base_url: "https://example.invalid/v1".to_string(),
        api_key: "dummy".to_string(),
        http_headers: Default::default(),
        request_fields: Default::default(),
        timeout_secs: 1,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        api_protocol,
        response_protocol: ResponseProtocolKind::Json,
        openai_compatible: crate::OpenAiCompatibleOptions::default(),
        http_transport: Default::default(),
    }
}

#[test]
fn model_service_defaults_are_protocol_based() {
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

    assert_eq!(default_api_protocol(), ApiProtocol::OpenAiCompatible);
    assert_eq!(default_model(), "qwen-plus");
}

#[test]
fn custom_model_headers_override_protocol_defaults_case_insensitively() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config
        .http_headers
        .insert("authorization".to_string(), "Basic custom".to_string());
    config
        .http_headers
        .insert("X-Tenant".to_string(), "tenant-one".to_string());
    let request = prepare_model_http_request(&config, "hello");
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name == "Authorization" && value == "Basic custom"));
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name == "X-Tenant" && value == "tenant-one"));
    assert_eq!(
        request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .count(),
        1
    );
}

#[test]
fn model_and_base_url_defaults_do_not_require_service_identity() {
    assert!(is_default_model("qwen-plus"));
    assert!(!is_default_model("claude-sonnet-4"));
    assert!(is_default_base_url(
        &ApiProtocol::OpenAiCompatible,
        "https://dashscope.aliyuncs.com/compatible-mode/v1/"
    ));
    assert!(!is_default_base_url(
        &ApiProtocol::OpenAiResponses,
        "https://example.invalid/v1"
    ));
    assert_eq!(
        default_base_url(&ApiProtocol::OpenAiResponses),
        "https://api.openai.com/v1"
    );
    assert_eq!(
        default_base_url(&ApiProtocol::Anthropic),
        "https://api.anthropic.com"
    );
}

#[test]
fn openai_compatible_request_uses_messages_and_structured_output() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.openai_compatible.cache_mode = OpenAiCompatibleCacheMode::Ephemeral;
    config.model = "qwen-plus".to_string();
    config.max_llm_output_tokens = 2048;
    let body = build_model_request(
        &config,
        &[ModelPromptBlock {
            role: ModelPromptRole::System,
            text: "Return JSON".to_string(),
            cache: ModelCacheControl::Ephemeral,
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
fn custom_request_fields_are_merged_as_typed_top_level_json() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.request_fields = BTreeMap::from([
        ("service_tier".to_string(), json!("fast")),
        (
            "vendor_options".to_string(),
            json!({"priority": 2, "enabled": true}),
        ),
    ]);
    let body = build_model_request(&config, &[], StructuredOutputHint::None);
    assert_eq!(body["service_tier"], "fast");
    assert_eq!(body["vendor_options"]["priority"], 2);
    assert_eq!(body["vendor_options"]["enabled"], true);
}

#[test]
fn custom_request_fields_reject_reserved_request_keys() {
    assert!(validate_model_request_fields(&BTreeMap::from([(
        "model".to_string(),
        json!("override"),
    )]))
    .is_err());
    assert!(
        validate_model_request_fields(&BTreeMap::from([("messages".to_string(), json!([]),)]))
            .is_err()
    );
    assert_eq!(
        validate_model_request_fields(&BTreeMap::from([(
            "service_tier".to_string(),
            json!("fast"),
        )])),
        Ok(())
    );
}

#[test]
fn openai_compatible_cache_mode_auto_uses_server_side_prefix_caching_without_wire_marks() {
    let config = config(ApiProtocol::OpenAiCompatible);
    let body = build_model_request(
        &config,
        &[ModelPromptBlock {
            role: ModelPromptRole::System,
            text: "stable prefix".to_string(),
            cache: ModelCacheControl::Ephemeral,
        }],
        StructuredOutputHint::None,
    );

    assert!(body["messages"][0].get("cache_control").is_none());
    let prepared = prepare_model_request(
        &config,
        "[BEGIN SYSTEM PROMPT]\nstable prefix\n[END SYSTEM PROMPT]",
    );
    assert_eq!(prepared.cache_wire_mode, "auto");
    assert_eq!(prepared.cache_mark_count, 0);
    assert!(!prepared.cache_fallback);
}

#[test]
fn openai_compatible_cache_mode_off_does_not_send_wire_marks() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.openai_compatible.cache_mode = OpenAiCompatibleCacheMode::Off;
    let body = build_model_request(
        &config,
        &[ModelPromptBlock {
            role: ModelPromptRole::System,
            text: "stable prefix".to_string(),
            cache: ModelCacheControl::Ephemeral,
        }],
        StructuredOutputHint::None,
    );

    assert!(body["messages"][0].get("cache_control").is_none());
    let prepared = prepare_model_request(
        &config,
        "[BEGIN SYSTEM PROMPT]\nstable prefix\n[END SYSTEM PROMPT]",
    );
    assert_eq!(prepared.cache_wire_mode, "off");
    assert_eq!(prepared.cache_mark_count, 0);
}

#[test]
fn openai_compatible_cache_mode_ephemeral_sends_planned_wire_marks() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.openai_compatible.cache_mode = OpenAiCompatibleCacheMode::Ephemeral;
    let prepared = prepare_model_request(
        &config,
        "[BEGIN SYSTEM PROMPT]\nstable prefix\n[END SYSTEM PROMPT]",
    );

    assert_eq!(
        prepared.body["messages"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(prepared.cache_wire_mode, "ephemeral");
    let actual_mark_count = prepared.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message.get("cache_control").is_some())
        .count();
    assert!(actual_mark_count > 0);
    assert_eq!(prepared.cache_mark_count, actual_mark_count);
}

#[test]
fn openai_compatible_request_supports_official_thinking_stream_options() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.model = "ZHIPU/GLM-5.2".to_string();
    config.openai_compatible = OpenAiCompatibleOptions {
        requirements: Default::default(),
        catalog_id: None,
        enable_thinking: Some(true),
        reasoning_effort: Some("max".to_string()),
        stream: true,
        cache_mode: OpenAiCompatibleCacheMode::Auto,
    };

    let blocks = &[ModelPromptBlock {
        role: ModelPromptRole::User,
        text: "hello".to_string(),
        cache: ModelCacheControl::None,
    }];
    let body =
        build_model_request_with_reasoning(&config, blocks, StructuredOutputHint::None, true);

    assert_eq!(body["enable_thinking"], true);
    assert_eq!(body["reasoning_effort"], "max");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[test]
fn structured_output_strategy_is_response_and_api_protocol_specific() {
    let mut aliyun = config(ApiProtocol::OpenAiCompatible);
    aliyun.response_protocol = ResponseProtocolKind::Json;
    assert_eq!(
        plan_structured_output(&aliyun),
        StructuredOutputHint::JsonObject
    );

    aliyun.response_protocol = ResponseProtocolKind::Xml;
    assert_eq!(plan_structured_output(&aliyun), StructuredOutputHint::None);
    let xml_body = build_model_request(
        &aliyun,
        &[ModelPromptBlock {
            role: ModelPromptRole::System,
            text: "The top-level response is XML, not JSON or Markdown.".to_string(),
            cache: ModelCacheControl::None,
        }],
        plan_structured_output(&aliyun),
    );
    assert!(xml_body.get("response_format").is_none());

    let mut custom = config(ApiProtocol::OpenAiCompatible);
    custom.response_protocol = ResponseProtocolKind::Json;
    assert_eq!(
        plan_structured_output(&custom),
        StructuredOutputHint::JsonObject
    );
    let body = build_model_request(
        &custom,
        &[ModelPromptBlock {
            role: ModelPromptRole::System,
            text: "hello".to_string(),
            cache: ModelCacheControl::None,
        }],
        plan_structured_output(&custom),
    );
    assert_eq!(body["response_format"]["type"], "json_object");

    let anthropic = config(ApiProtocol::Anthropic);
    assert_eq!(
        plan_structured_output(&anthropic),
        StructuredOutputHint::None
    );
}

#[test]
fn anthropic_request_maps_cache_strategy_blocks_to_content_blocks() {
    let mut config = config(ApiProtocol::Anthropic);
    config.model = "claude-sonnet-4-20250514".to_string();
    config.max_llm_output_tokens = 2048;
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## TIMEM_ASSISTANT\ndelta1\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: pd_2\n\n## USER\ndelta2\n[END DELTA]";

    let prepared = prepare_model_request(&config, prompt);
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
fn anthropic_request_sends_the_current_response_trailer_as_an_uncached_tail() {
    let config = config(ApiProtocol::Anthropic);
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nhello\n[END DELTA]\n\nPlease continue the work and respond as protocol requires in user's language:";

    let prepared = prepare_model_request(&config, prompt);
    let content = prepared.body["messages"][0]["content"].as_array().unwrap();

    assert!(content.iter().any(|block| {
        block["text"]
            .as_str()
            .is_some_and(|text| text.contains("hello"))
    }));
    assert_eq!(
        content.last().unwrap()["text"],
        "Please continue the work and respond as protocol requires in user's language:"
    );
    assert!(content.last().unwrap().get("cache_control").is_none());
}

#[test]
fn openai_responses_request_uses_official_shape() {
    let mut config = config(ApiProtocol::OpenAiResponses);
    config.model = "gpt-4o".to_string();
    config.max_llm_output_tokens = 2048;
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC_GLOBAL\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nhello\n[END DELTA]";

    let prepared = prepare_model_request(&config, prompt);
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
    assert!(body.get("reasoning").is_none());
}

#[test]
fn openai_compatible_reasoning_effort_disabled_turns_thinking_off() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.openai_compatible.reasoning_effort = Some("disabled".to_string());

    let body = build_model_request(
        &config,
        &[ModelPromptBlock {
            role: ModelPromptRole::User,
            text: "hello".to_string(),
            cache: ModelCacheControl::None,
        }],
        StructuredOutputHint::None,
    );

    assert_eq!(body["reasoning_effort"], "none");
    assert!(body.get("thinking").is_none());
}

#[test]
fn openai_responses_reasoning_effort_disabled_maps_to_none() {
    let mut config = config(ApiProtocol::OpenAiResponses);
    config.openai_compatible.reasoning_effort = Some("disabled".to_string());

    let prepared = prepare_model_request(&config, "hello");

    assert_eq!(prepared.body["reasoning"]["effort"], "none");
}

#[test]
fn openai_responses_request_carries_reasoning_effort_only_for_critical_requests() {
    let mut config = config(ApiProtocol::OpenAiResponses);
    config.openai_compatible.reasoning_effort = Some("high".to_string());

    let ordinary = prepare_model_request(&config, "hello");
    assert_eq!(ordinary.body["reasoning"]["effort"], "none");

    let critical = prepare_model_request_with_reasoning(&config, "hello", true);
    assert_eq!(critical.body["reasoning"]["effort"], "high");
}

#[test]
fn ordinary_requests_disable_reasoning_by_default() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.openai_compatible = OpenAiCompatibleOptions {
        requirements: Default::default(),
        catalog_id: None,
        enable_thinking: Some(true),
        reasoning_effort: Some("high".to_string()),
        stream: false,
        cache_mode: OpenAiCompatibleCacheMode::Auto,
    };
    let blocks = &[ModelPromptBlock {
        role: ModelPromptRole::User,
        text: "hello".to_string(),
        cache: ModelCacheControl::None,
    }];

    let ordinary =
        build_model_request_with_reasoning(&config, blocks, StructuredOutputHint::None, false);
    assert_eq!(ordinary["enable_thinking"], false);
    assert_eq!(ordinary["reasoning_effort"], "none");
    assert!(ordinary.get("thinking").is_none());

    let critical =
        build_model_request_with_reasoning(&config, blocks, StructuredOutputHint::None, true);
    assert_eq!(critical["enable_thinking"], true);
    assert_eq!(critical["reasoning_effort"], "high");
    assert!(critical.get("thinking").is_none());
}

#[test]
fn openai_compatible_request_splits_static_and_dynamic_prompt() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.openai_compatible.cache_mode = OpenAiCompatibleCacheMode::Ephemeral;
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC_GLOBAL\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nsecret\n[END DELTA]";

    let prepared = prepare_model_request(&config, prompt);
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
    config.model = "qwen-plus".to_string();
    config.openai_compatible.cache_mode = OpenAiCompatibleCacheMode::Ephemeral;
    let mut prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n".to_string();
    for idx in 1..=5 {
        prompt.push_str(&format!(
            "[BEGIN DELTA]\ndelta_id: pd_{idx}\n\n## TIMEM_ASSISTANT\ndelta {idx}\n[END DELTA]\n"
        ));
    }

    let prepared = prepare_model_request(&config, &prompt);
    let messages = prepared.body["messages"].as_array().unwrap();

    assert_eq!(messages.len(), 6);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "STATIC");
    assert_eq!(messages[0]["cache_control"]["type"], "ephemeral");
    assert!(messages[1]["content"].as_str().unwrap().contains("delta 1"));
    assert!(messages[2]["content"].as_str().unwrap().contains("delta 2"));
    assert_eq!(messages[1].get("cache_control"), None);
    assert_eq!(messages[2].get("cache_control"), None);

    for (idx, message) in messages.iter().enumerate().take(6).skip(3) {
        assert!(message["content"]
            .as_str()
            .unwrap()
            .contains(&format!("delta {idx}")));
        assert_eq!(message["cache_control"]["type"], "ephemeral");
    }
}

#[test]
fn openai_compatible_request_sends_the_current_response_trailer_as_an_uncached_tail() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.openai_compatible.cache_mode = OpenAiCompatibleCacheMode::Ephemeral;
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nhello\n[END DELTA]\n\nPlease continue the work and respond as protocol requires in user's language:";

    let prepared = prepare_model_request(&config, prompt);
    let messages = prepared.body["messages"].as_array().unwrap();

    assert!(messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|text| text.contains("hello"))
    }));
    assert_eq!(
        messages.last().unwrap()["content"],
        "Please continue the work and respond as protocol requires in user's language:"
    );
    assert!(messages.last().unwrap().get("cache_control").is_none());
}

#[test]
fn prepared_request_builds_body_and_prompt_cache_audit_without_prompt_text() {
    let config = config(ApiProtocol::Anthropic);
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC SECRET\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\ndelta secret\n[END DELTA]";

    let prepared = prepare_model_request(&config, prompt);

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
fn prepared_http_request_keeps_model_api_headers_in_core() {
    let mut openai_like = config(ApiProtocol::OpenAiCompatible);
    openai_like.api_key = "test-openai-key".to_string();

    let http = prepare_model_http_request(&openai_like, "Return JSON\nhello");
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
    assert_eq!(http.model_request.body["model"], openai_like.model);

    let mut anthropic = config(ApiProtocol::Anthropic);
    anthropic.api_key = "test-anthropic-key".to_string();

    let http = prepare_model_http_request(&anthropic, "hello");
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
    config.base_url = "https://example.com/api/v1".to_string();
    assert_eq!(config.endpoint(), "https://example.com/api/v1/messages");

    config.base_url = "https://api.anthropic.com".to_string();
    assert_eq!(config.endpoint(), "https://api.anthropic.com/v1/messages");
}

#[test]
fn model_http_response_interpretation_is_core_owned() {
    let config = config(ApiProtocol::OpenAiCompatible);
    let interpreted = interpret_model_http_response(
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

    let interpreted = interpret_model_http_response(
        &config,
        429,
        r#"{"error":{"message":"rate limit sk-sensitive-token"}}"#,
        "",
    );
    assert_eq!(interpreted.status, 429);
    let err = interpreted.result.unwrap_err();
    assert!(err.contains("model_http_429"));
    assert!(!err.contains("sk-sensitive-token"));

    let interpreted = interpret_model_http_response(&config, 200, "not json", "curl stderr detail");
    assert_eq!(interpreted.raw_json["raw_text"], "not json");
    assert_eq!(interpreted.raw_json["stderr"], "curl stderr detail");
    assert_eq!(interpreted.result.unwrap().content, "not json");
}

#[test]
fn openai_compatible_sse_collects_content_and_usage_without_exposing_reasoning() {
    let config = config(ApiProtocol::OpenAiCompatible);
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"private plan\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"<ASSISTANT>\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok</ASSISTANT>\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":16,\"completion_tokens\":7,\"total_tokens\":23,\"completion_tokens_details\":{\"reasoning_tokens\":5}}}\n\n",
        "data: [DONE]\n",
    );

    let interpreted = interpret_model_http_response(&config, 200, body, "");
    let response = interpreted.result.unwrap();
    assert_eq!(response.content, "<ASSISTANT>ok</ASSISTANT>");
    assert_eq!(response.usage.prompt_tokens, 16);
    assert_eq!(response.usage.completion_tokens, 7);
    assert_eq!(interpreted.raw_json["stream_metadata"]["event_count"], 4);
    assert_eq!(
        interpreted.raw_json["stream_metadata"]["reasoning_chunk_count"],
        1
    );
    assert!(!interpreted.raw_json.to_string().contains("private plan"));
}

#[test]
fn malformed_openai_compatible_sse_is_a_transport_error_not_model_content() {
    let interpreted = interpret_model_http_response(
        &config(ApiProtocol::OpenAiCompatible),
        200,
        "data: {not-json}\n\ndata: [DONE]\n",
        "",
    );
    assert!(interpreted
        .result
        .unwrap_err()
        .starts_with("invalid_model_sse_event:"));
}

#[test]
fn model_request_audit_event_is_redacted_and_ui_neutral() {
    let mut config = config(ApiProtocol::OpenAiCompatible);
    config.api_key = "sk-sensitive-token".to_string();
    config.response_protocol = ResponseProtocolKind::Json;
    let mut prepared = prepare_model_request(&config, "Return JSON\nhello");
    prepared.body["metadata"] = json!({"api_key":"sk-sensitive-token"});

    let audit = model_request_audit_event(&config, &prepared);

    assert_eq!(audit["type"], "llm_request");
    assert_eq!(audit["model"], config.model);
    assert_eq!(audit["api_protocol"], "openai-compatible");
    assert_eq!(audit["endpoint"], config.endpoint());
    assert_eq!(audit["structured_output"], "json_object");
    assert!(audit["prompt_cache_plan"].is_array());
    assert_eq!(audit["prompt_cache_wire"]["mode"], "auto");
    assert_eq!(audit["prompt_cache_wire"]["mark_count"], 0);
    assert_eq!(audit["prompt_cache_wire"]["fallback"], false);
    let audit_text = audit.to_string();
    assert!(audit_text.contains("***REDACTED***"));
    assert!(!audit_text.contains("sk-sensitive-token"));
}

#[test]
fn model_response_audit_event_is_redacted() {
    let audit = model_response_audit_event(
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
fn openai_compatible_response_counts_cache_creation_token_variants() {
    for (details, top_level, expected) in [
        (json!({"cached_creation_tokens": 321}), json!({}), 321),
        (json!({"cache_creation_tokens": 654}), json!({}), 654),
        (json!({}), json!({"cache_creation_input_tokens": 987}), 987),
    ] {
        let mut usage = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 10,
            "total_tokens": 1010,
            "prompt_tokens_details": details,
        });
        for (key, value) in top_level.as_object().unwrap() {
            usage[key] = value.clone();
        }
        let response = parse_model_response(
            &config(ApiProtocol::OpenAiCompatible),
            &json!({
                "choices": [{
                    "message": {"content": "ok"},
                    "finish_reason": "stop"
                }],
                "usage": usage,
            }),
        )
        .unwrap();
        assert_eq!(response.usage.cache_created_tokens, expected);
    }
}

#[test]
fn anthropic_response_counts_cache_tokens() {
    let response = parse_model_response(
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
    let empty = parse_model_response(
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

    let response = parse_model_response(
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

    let response = parse_model_response(
            &config(ApiProtocol::OpenAiCompatible),
            &json!({
                "choices":[{"finish_reason":"length","message":{"content":"{\"free_talk\":\"partial\"}"}}],
                "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}
            }),
        )
        .unwrap();
    assert!(response.truncated);

    let response = parse_model_response(
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
    let response = parse_model_response(
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

    let response = parse_model_response(
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

    let response = parse_model_response(
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
    let response = parse_model_response(
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

    let response = parse_model_response(
        &config(ApiProtocol::Anthropic),
        &json!({
            "stop_reason":"max_tokens",
            "content":[{"type":"text","text":"{\"free_talk\":\"partial\"}"}],
            "usage":{"input_tokens":10,"output_tokens":10}
        }),
    )
    .unwrap();
    assert!(response.truncated);

    let response = parse_model_response(
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
fn output_limit_truncation_requires_protocol_specific_terminal_metadata() {
    let cases = [
        (
            ApiProtocol::OpenAiCompatible,
            json!({
                "choices":[{"finish_reason":"length","message":{"content":"partial"}}],
                "usage":{}
            }),
            true,
        ),
        (
            ApiProtocol::OpenAiCompatible,
            json!({
                "choices":[{"finish_reason":"max_tokens","message":{"content":"partial"}}],
                "usage":{}
            }),
            true,
        ),
        (
            ApiProtocol::OpenAiCompatible,
            json!({
                "choices":[{"finish_reason":"content_filter","message":{"content":"partial"}}],
                "usage":{}
            }),
            false,
        ),
        (
            ApiProtocol::OpenAiResponses,
            json!({
                "status":"incomplete",
                "incomplete_details":{"reason":"max_output_tokens"},
                "output_text":"partial",
                "usage":{}
            }),
            true,
        ),
        (
            ApiProtocol::OpenAiResponses,
            json!({
                "status":"completed",
                "incomplete_details":{"reason":"max_output_tokens"},
                "output_text":"partial",
                "usage":{}
            }),
            false,
        ),
        (
            ApiProtocol::OpenAiResponses,
            json!({
                "status":"incomplete",
                "incomplete_details":{"reason":"content_filter"},
                "output_text":"partial",
                "usage":{}
            }),
            false,
        ),
        (
            ApiProtocol::Anthropic,
            json!({
                "stop_reason":"max_tokens",
                "content":[{"type":"text","text":"partial"}],
                "usage":{}
            }),
            true,
        ),
        (
            ApiProtocol::Anthropic,
            json!({
                "stop_reason":"end_turn",
                "content":[{"type":"text","text":"partial"}],
                "usage":{}
            }),
            false,
        ),
    ];

    for (protocol, raw, expected) in cases {
        let response = parse_model_response(&config(protocol), &raw).unwrap();
        assert_eq!(
            response.truncated, expected,
            "unexpected truncation classification for {protocol:?}: {raw}"
        );
    }
}

#[test]
fn model_http_error_includes_sanitized_service_reason() {
    let openai_like = json!({
        "error": {
            "message": "The model `missing-model` does not exist or you do not have access to it.",
            "type": "invalid_request_error"
        }
    });
    assert_eq!(
        model_http_error_message(400, &openai_like),
        "model_http_400: The model `missing-model` does not exist or you do not have access to it."
    );

    let anthropic_like = json!({
        "type": "error",
        "error": {
            "type": "not_found_error",
            "message": "model: claude-missing not found"
        }
    });
    assert_eq!(
        model_http_error_message(404, &anthropic_like),
        "model_http_404: model: claude-missing not found"
    );

    let raw_text = json!({"raw_text":"invalid Authorization Bearer sk-secret-token"});
    let rendered = model_http_error_message(401, &raw_text);
    assert!(rendered.starts_with("model_http_401:"));
    assert!(rendered.contains("***REDACTED***"));
    assert!(!rendered.contains("sk-secret-token"));

    let long = model_http_error_message(400, &json!({"error":{"message":"x ".repeat(400)}}));
    assert!(long.contains('…'));
    assert!(long.len() < 280);

    let timeout = model_http_error_message(
        0,
        &json!({"raw_text":"","stderr":"curl: (28) Operation timed out after 120006 milliseconds with 0 bytes received"}),
    );
    assert!(timeout.starts_with("model_timeout:"));
    assert!(timeout.contains("Operation timed out"));
}

#[test]
fn model_http_error_is_resilient_to_unusual_bodies() {
    for body in [
        Value::Null,
        json!("plain string error"),
        json!(["array", "error"]),
        json!({"error":{"message":null,"details":[{"x":1}]}}),
        json!({"detail":{"nested":"not a string"}}),
        json!({"raw_text":""}),
    ] {
        let rendered = model_http_error_message(500, &body);
        assert!(rendered.starts_with("model_http_500"));
        assert!(rendered.len() < 280);
    }
}

fn native_request() -> ModelInteractionRequest {
    ModelInteractionRequest {
        rendered_prompt: "SYSTEM PROMPT\n\n---USER---\ncount files".to_string(),
        images: Vec::new(),
        static_tool_count: 1,
        tools: vec![ToolDefinition {
            name: "count_lines".to_string(),
            description: "Count source lines.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"language": {"type": "string"}},
                "required": ["language"]
            }),
        }],
        native_exchanges: Vec::new(),
        resolved_mode: ToolCallMode::Native,
        parallel_tool_calls: true,
        tool_choice: NativeToolChoice::Auto,
        critical_reasoning: false,
    }
}

fn image_interaction_request(resolved_mode: ToolCallMode) -> ModelInteractionRequest {
    ModelInteractionRequest {
        rendered_prompt: "SYSTEM PROMPT

---USER---
What is in this screenshot?"
            .to_string(),
        images: vec![ModelImagePart::new("image/png", "QUJD")],
        static_tool_count: 0,
        tools: Vec::new(),
        native_exchanges: Vec::new(),
        resolved_mode,
        parallel_tool_calls: false,
        tool_choice: NativeToolChoice::Auto,
        critical_reasoning: false,
    }
}

#[test]
fn attached_images_reach_every_provider_wire_format() {
    // OpenAI chat: trailing user message with image_url parts.
    let body = prepare_model_interaction_http_request(
        &config(ApiProtocol::OpenAiCompatible),
        &image_interaction_request(ToolCallMode::Inline),
    )
    .model_request
    .body;
    let messages = body["messages"].as_array().unwrap();
    let last = messages.last().unwrap();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"].as_array().unwrap().len(), 1);
    assert_eq!(last["content"][0]["type"], "image_url");
    assert_eq!(
        last["content"][0]["image_url"]["url"],
        "data:image/png;base64,QUJD"
    );

    // OpenAI responses: input item with input_image; string input is promoted.
    let body = prepare_model_interaction_http_request(
        &config(ApiProtocol::OpenAiResponses),
        &image_interaction_request(ToolCallMode::Inline),
    )
    .model_request
    .body;
    let input = body["input"].as_array().unwrap();
    let last = input.last().unwrap();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"][0]["type"], "input_image");
    assert_eq!(
        last["content"][0]["image_url"],
        "data:image/png;base64,QUJD"
    );

    // Anthropic inline: parts merge into the single user message (no
    // consecutive user messages).
    let body = prepare_model_interaction_http_request(
        &config(ApiProtocol::Anthropic),
        &image_interaction_request(ToolCallMode::Inline),
    )
    .model_request
    .body;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    let content = messages[0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["media_type"], "image/png");
    assert_eq!(content[1]["source"]["data"], "QUJD");
}

#[test]
fn attached_images_append_after_native_history_without_touching_cache_marks() {
    let mut request = image_interaction_request(ToolCallMode::Native);
    request.native_exchanges = vec![NativeExchange {
        delta_id: "pd_1".to_string(),
        assistant_text: "looking".to_string(),
        calls: vec![NativeToolCall {
            assistant_continuation: None,
            id: "call_1".to_string(),
            name: "count_lines".to_string(),
            arguments: json!({}),
            raw_arguments: "{}".to_string(),
        }],
        results: vec![NativeToolResult {
            call_id: "call_1".to_string(),
            name: "count_lines".to_string(),
            content: "42".to_string(),
            is_error: false,
        }],
    }];
    request.rendered_prompt = concat!(
        "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n",
        "[BEGIN DELTA delta_id: pd_1, time_ms: 1]\n\n## USER\nWhat is in this screenshot?\n\n",
        "Continue the work and express thought in the user's language.  Use tools smartly. When all work is genuinely done, call the task_finished tool with the complete final answer as its summary:"
    )
    .to_string();
    let body = prepare_model_interaction_http_request(&config(ApiProtocol::Anthropic), &request)
        .model_request
        .body;
    let messages = body["messages"].as_array().unwrap();
    let message_type = |message: &Value| -> String {
        message["content"]
            .as_array()
            .and_then(|parts| parts.first())
            .and_then(|part| part.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("text")
            .to_string()
    };
    let types = messages.iter().map(&message_type).collect::<Vec<_>>();
    let result_index = types
        .iter()
        .position(|kind| kind == "tool_result")
        .expect("projected tool result message");
    assert_eq!(types.last().unwrap(), "image", "images ride last");
    assert!(
        result_index < messages.len() - 1,
        "tool result must stay before the image message"
    );
    // The cache-marked user delta block keeps its cache_control untouched.
    let first_user = messages
        .iter()
        .find(|message| message_type(message) == "text")
        .unwrap();
    assert!(first_user["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part.get("cache_control").is_some()));
}

#[test]
fn multimodal_audit_events_redact_image_payloads() {
    let request = image_interaction_request(ToolCallMode::Inline);
    let prepared =
        prepare_model_interaction_http_request(&config(ApiProtocol::OpenAiCompatible), &request)
            .model_request;
    let audit = model_request_audit_event(&config(ApiProtocol::OpenAiCompatible), &prepared);
    let body = audit["body"].to_string();
    assert!(!body.contains("QUJD"), "base64 payload leaked into audit");
    assert!(
        !body.contains("data:image/png"),
        "data URL leaked into audit"
    );
    assert!(body.contains("[image payload redacted;"));
}

#[test]
fn native_tool_wires_are_provider_specific_and_parallel_is_explicit() {
    let mut request = native_request();
    request.tools.push(ToolDefinition {
        name: "mcp_demo__search".to_string(),
        description: "Dynamic MCP search.".to_string(),
        input_schema: json!({"type": "object"}),
    });
    let chat_body =
        prepare_model_interaction_http_request(&config(ApiProtocol::OpenAiCompatible), &request)
            .model_request
            .body;
    assert_eq!(chat_body["parallel_tool_calls"], json!(true));
    assert_eq!(chat_body["tools"][0]["function"]["name"], "count_lines");
    assert!(chat_body["tools"][0]["function"]
        .get("description")
        .is_none());
    assert!(chat_body["tools"][0]["function"]
        .get("parameters")
        .is_some());
    assert_eq!(
        chat_body["tools"][1]["function"]["description"],
        "Dynamic MCP search."
    );

    let responses_body =
        prepare_model_interaction_http_request(&config(ApiProtocol::OpenAiResponses), &request)
            .model_request
            .body;
    assert_eq!(responses_body["tools"][0]["name"], "count_lines");
    assert!(responses_body["tools"][0].get("description").is_none());
    assert!(responses_body["tools"][0].get("parameters").is_some());
    assert_eq!(
        responses_body["tools"][1]["description"],
        "Dynamic MCP search."
    );
    assert!(responses_body["input"].is_array());

    let anthropic_body =
        prepare_model_interaction_http_request(&config(ApiProtocol::Anthropic), &request)
            .model_request
            .body;
    assert_eq!(anthropic_body["tools"][0]["name"], "count_lines");
    assert!(anthropic_body["tools"][0].get("description").is_none());
    assert!(anthropic_body["tools"][0].get("input_schema").is_some());
    assert_eq!(
        anthropic_body["tools"][1]["description"],
        "Dynamic MCP search."
    );
    assert_eq!(
        anthropic_body["tools"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );
    assert_eq!(
        anthropic_body["tool_choice"]["disable_parallel_tool_use"],
        json!(false)
    );
}

#[test]
fn builtin_tool_schemas_render_per_protocol_without_weakening_registry_validation() {
    let registry = crate::capability::CapabilityRegistry::builtin_for_host(
        crate::capability::CapabilityHostProfile::with_local_command_execution(),
    );
    let original_tools = registry.native_builtin_tool_definitions();
    let original_run_bash = original_tools
        .iter()
        .find(|tool| tool.name == "run_bash")
        .expect("run_bash builtin");
    assert!(original_run_bash.input_schema.get("oneOf").is_some());
    assert!(original_run_bash.input_schema.get("allOf").is_some());

    let request = ModelInteractionRequest {
        rendered_prompt: "SYSTEM PROMPT\n\n---USER---\nuse tools".to_string(),
        images: Vec::new(),
        static_tool_count: original_tools.len(),
        tools: original_tools.clone(),
        native_exchanges: Vec::new(),
        resolved_mode: ToolCallMode::Native,
        parallel_tool_calls: true,
        tool_choice: NativeToolChoice::Auto,
        critical_reasoning: false,
    };

    let anthropic =
        prepare_model_interaction_http_request(&config(ApiProtocol::Anthropic), &request);
    let anthropic_tools = anthropic.model_request.body["tools"].as_array().unwrap();
    assert_eq!(anthropic_tools.len(), original_tools.len());
    for tool in anthropic_tools {
        let schema = tool["input_schema"].as_object().unwrap();
        for unsupported in ["oneOf", "allOf", "anyOf"] {
            assert!(
                !schema.contains_key(unsupported),
                "{} retained unsupported root {unsupported}",
                tool["name"]
            );
        }
        assert_eq!(schema.get("type"), Some(&json!("object")));
        assert!(schema.get("properties").is_some());
    }
    let cache_marks = anthropic_tools
        .iter()
        .filter(|tool| tool.get("cache_control").is_some())
        .count();
    assert_eq!(cache_marks, 1);
    assert_eq!(
        anthropic_tools.last().unwrap()["cache_control"],
        json!({"type": "ephemeral"})
    );

    // Both OpenAI wire protocols must preserve every tool schema byte-for-value,
    // independently of the enabled assistant response protocol. Before the
    // provider renderer refactor these fields were direct input_schema clones.
    for api_protocol in [ApiProtocol::OpenAiCompatible, ApiProtocol::OpenAiResponses] {
        for response_protocol in [ResponseProtocolKind::Json, ResponseProtocolKind::Xml] {
            let mut openai_config = config(api_protocol);
            openai_config.response_protocol = response_protocol;
            let openai = prepare_model_interaction_http_request(&openai_config, &request);
            let rendered_tools = openai.model_request.body["tools"].as_array().unwrap();
            assert_eq!(rendered_tools.len(), original_tools.len());
            for (rendered, original) in rendered_tools.iter().zip(&original_tools) {
                let (name, parameters) = match api_protocol {
                    ApiProtocol::OpenAiCompatible => (
                        &rendered["function"]["name"],
                        &rendered["function"]["parameters"],
                    ),
                    ApiProtocol::OpenAiResponses => (&rendered["name"], &rendered["parameters"]),
                    ApiProtocol::Anthropic => unreachable!(),
                };
                assert_eq!(name, &json!(original.name));
                assert_eq!(parameters, &original.input_schema);
            }
        }
    }

    // Request rendering operates on copies. Executor-facing definitions retain
    // every original constraint after both provider adapters run.
    assert_eq!(registry.native_builtin_tool_definitions(), original_tools);
}

#[test]
fn anthropic_native_cache_marks_never_exceed_bedrock_limit() {
    let mut request = native_request();
    request.rendered_prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n".to_string();
    for idx in 1..=5 {
        request.rendered_prompt.push_str(&format!(
            "[BEGIN DELTA]\ndelta_id: pd_{idx}\n\n## TIMEM_ASSISTANT\ndelta {idx}\n[END DELTA]\n"
        ));
    }

    let prepared =
        prepare_model_interaction_http_request(&config(ApiProtocol::Anthropic), &request);
    let body = &prepared.model_request.body;

    assert_eq!(
        prepared.model_request.cache_mark_count,
        ANTHROPIC_MAX_CACHE_CONTROL_BLOCKS
    );
    assert_eq!(
        count_cache_control_marks(body),
        ANTHROPIC_MAX_CACHE_CONTROL_BLOCKS
    );
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");

    let marked_message_text = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().unwrap())
        .filter(|block| block.get("cache_control").is_some())
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(marked_message_text.len(), 2);
    assert!(marked_message_text
        .iter()
        .any(|text| text.contains("delta 4")));
    assert!(marked_message_text
        .iter()
        .any(|text| text.contains("delta 5")));
    assert!(!marked_message_text
        .iter()
        .any(|text| text.contains("delta 3")));
}

#[test]
fn anthropic_native_cache_breakpoint_ends_at_static_builtin_tool_prefix() {
    let mut request = native_request();
    request.tools.push(ToolDefinition {
        name: "mcp_demo__search".to_string(),
        description: "Dynamic MCP search.".to_string(),
        input_schema: json!({"type": "object"}),
    });
    let prepared =
        prepare_model_interaction_http_request(&config(ApiProtocol::Anthropic), &request);
    let tools = prepared.model_request.body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "count_lines");
    assert_eq!(tools[0]["cache_control"], json!({"type": "ephemeral"}));
    assert_eq!(tools[1]["name"], "mcp_demo__search");
    assert_eq!(tools[1]["description"], "Dynamic MCP search.");
    assert!(tools[1].get("input_schema").is_some());
    assert!(tools[1].get("cache_control").is_none());
    assert!(prepared.model_request.cache_mark_count >= 1);
}

#[test]
fn truncated_openai_compatible_tool_arguments_become_repairable_model_output() {
    let response = parse_model_response(
        &config(ApiProtocol::OpenAiCompatible),
        &json!({
            "choices": [{
                "message": {"content": null, "tool_calls": [{
                    "id": "call_partial",
                    "type": "function",
                    "function": {
                        "name": "run_bash",
                        "arguments": "{\"cmd\":\"python3 - <<'PY'\\nprint(\"partial"
                    }
                }]},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5206, "total_tokens": 5216}
        }),
    )
    .unwrap();

    assert!(response.truncated);
    assert!(response.tool_calls.is_empty());
    assert!(response
        .content
        .contains("[TRUNCATED NATIVE TOOL CALL OUTPUT]"));
    assert!(response
        .content
        .contains("invalid_tool_call[0].arguments_json"));
    assert!(response.content.contains("tool_call[0] name=run_bash"));
    assert!(response.content.contains("arguments_fragment={\"cmd\""));
}

#[test]
fn nontruncated_openai_compatible_invalid_tool_arguments_remain_an_error() {
    let error = parse_model_response(
        &config(ApiProtocol::OpenAiCompatible),
        &json!({
            "choices": [{
                "message": {"content": null, "tool_calls": [{
                    "id": "call_invalid",
                    "type": "function",
                    "function": {"name": "run_bash", "arguments": "{\"cmd\":\"partial"}
                }]},
                "finish_reason": "tool_calls"
            }],
            "usage": {}
        }),
    )
    .unwrap_err();

    assert!(error.starts_with("invalid_tool_call[0].arguments_json:"));
}

#[test]
fn openai_compatible_tool_calls_do_not_depend_on_finish_reason() {
    let response = parse_model_response(
        &config(ApiProtocol::OpenAiCompatible),
        &json!({
            "choices": [{
                "message": {"content": null, "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "count_lines", "arguments": "{\"language\":\"Rust\"}"}
                }]},
                "finish_reason": "stop"
            }],
            "usage": {}
        }),
    )
    .unwrap();
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].arguments["language"], "Rust");
}

#[test]
fn openai_compatible_sse_length_with_partial_tool_arguments_is_repairable() {
    let chunk = json!({"choices":[{
        "delta":{"tool_calls":[{
            "index":0,
            "id":"call_partial",
            "function":{"name":"run_bash","arguments":"{\"cmd\":\"very long partial"}
        }]},
        "finish_reason":"length"
    }],"usage":{"prompt_tokens":10,"completion_tokens":5206,"total_tokens":5216}});
    let body = format!("data: {chunk}\n\ndata: [DONE]\n");
    let response =
        interpret_model_http_response(&config(ApiProtocol::OpenAiCompatible), 200, &body, "")
            .result
            .unwrap();

    assert!(response.truncated);
    assert!(response.tool_calls.is_empty());
    assert!(response
        .content
        .contains("[TRUNCATED NATIVE TOOL CALL OUTPUT]"));
    assert!(response.content.contains("tool_call[0] name=run_bash"));
    assert!(response.content.contains("very long partial"));
}

#[test]
fn openai_compatible_sse_assembles_parallel_tool_arguments_by_index() {
    let first = json!({"choices":[{"delta":{"tool_calls":[
        {"index":0,"id":"a","function":{"name":"count_lines","arguments":"{\"lang\""}},
        {"index":1,"id":"b","function":{"name":"count_lines","arguments":"{\"lang\""}}
    ]}}]});
    let second = json!({"choices":[{"delta":{"tool_calls":[
        {"index":0,"function":{"arguments":":\"Rust\"}"}},
        {"index":1,"function":{"arguments":":\"Go\"}"}}
    ]},"finish_reason":"tool_calls"}],"usage":{}});
    let body = format!("data: {first}\n\ndata: {second}\n\ndata: [DONE]\n");
    let response =
        interpret_model_http_response(&config(ApiProtocol::OpenAiCompatible), 200, &body, "")
            .result
            .unwrap();
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(response.tool_calls[0].arguments["lang"], "Rust");
    assert_eq!(response.tool_calls[1].arguments["lang"], "Go");
}

#[test]
fn native_exchange_can_be_owned_by_a_visible_delta_without_text_slices() {
    let rendered_prompt = concat!(
        "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n",
        "[BEGIN DELTA delta_id: pd_1, time_ms: 1]\n\n## USER\nQ1\n",
        "[BEGIN DELTA delta_id: pd_2, time_ms: 2]\n",
        "[BEGIN DELTA delta_id: pd_3, time_ms: 3]\n\n",
        "Continue the work and express thought in the user's language.  Use tools smartly. When all work is genuinely done, call the task_finished tool with the complete final answer as its summary:"
    )
    .to_string();
    let exchange = |delta_id: &str, call_id: &str, result: &str| NativeExchange {
        delta_id: delta_id.to_string(),
        assistant_text: format!("work {call_id}"),
        calls: vec![NativeToolCall {
            assistant_continuation: None,
            id: call_id.to_string(),
            name: "demo".to_string(),
            arguments: json!({"id": call_id}),
            raw_arguments: format!(r#"{{"id":"{call_id}"}}"#),
        }],
        results: vec![NativeToolResult {
            call_id: call_id.to_string(),
            name: "demo".to_string(),
            content: result.to_string(),
            is_error: false,
        }],
    };
    let request = ModelInteractionRequest {
        images: Vec::new(),
        rendered_prompt,
        static_tool_count: 0,
        tools: Vec::new(),
        native_exchanges: vec![
            exchange("pd_2", "call_empty_delta_1", "R1"),
            exchange("pd_3", "call_empty_delta_2", "R2"),
        ],
        resolved_mode: ToolCallMode::Native,
        parallel_tool_calls: false,
        tool_choice: NativeToolChoice::Auto,
        critical_reasoning: false,
    };

    for protocol in [
        ApiProtocol::OpenAiCompatible,
        ApiProtocol::OpenAiResponses,
        ApiProtocol::Anthropic,
    ] {
        let body = prepare_model_interaction_http_request(&config(protocol), &request)
            .model_request
            .body;
        let text = body.to_string();
        assert!(text.contains("delta_id: pd_2"), "{protocol:?}: {text}");
        assert!(text.contains("delta_id: pd_3"), "{protocol:?}: {text}");
        assert!(
            text.find("delta_id: pd_2").unwrap() < text.find("call_empty_delta_1").unwrap(),
            "{protocol:?}: {text}"
        );
        assert!(
            text.find("R1").unwrap() < text.find("delta_id: pd_3").unwrap(),
            "{protocol:?}: {text}"
        );
        assert!(
            text.find("delta_id: pd_3").unwrap() < text.find("call_empty_delta_2").unwrap(),
            "{protocol:?}: {text}"
        );
        assert!(
            text.find("R2").unwrap() < text.find("Continue the work").unwrap(),
            "{protocol:?}: {text}"
        );
    }
}

#[test]
fn native_exchanges_follow_owning_delta_order_for_all_providers() {
    let rendered_prompt = concat!(
        "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n",
        "[BEGIN DELTA delta_id: pd_1, time_ms: 1]\n\n## USER\nQ1\n",
        "[BEGIN DELTA delta_id: pd_2, time_ms: 2]\n\n## USER\nQ2\n\n",
        "Continue the work and express thought in the user's language.  Use tools smartly. When all work is genuinely done, call the task_finished tool with the complete final answer as its summary:"
    ).to_string();
    let exchange = |delta_id: &str, call_id: &str, result: &str| NativeExchange {
        delta_id: delta_id.to_string(),
        assistant_text: format!("work {call_id}"),
        calls: vec![NativeToolCall {
            assistant_continuation: None,
            id: call_id.to_string(),
            name: "demo".to_string(),
            arguments: json!({"id": call_id}),
            raw_arguments: format!(r#"{{"id":"{call_id}"}}"#),
        }],
        results: vec![NativeToolResult {
            call_id: call_id.to_string(),
            name: "demo".to_string(),
            content: result.to_string(),
            is_error: false,
        }],
    };
    let request = ModelInteractionRequest {
        images: Vec::new(),
        rendered_prompt,
        static_tool_count: 0,
        tools: Vec::new(),
        native_exchanges: vec![
            exchange("pd_1", "call_1", "R1"),
            exchange("pd_2", "call_2", "R2"),
        ],
        resolved_mode: ToolCallMode::Native,
        parallel_tool_calls: false,
        tool_choice: NativeToolChoice::Auto,
        critical_reasoning: false,
    };
    for protocol in [
        ApiProtocol::OpenAiCompatible,
        ApiProtocol::OpenAiResponses,
        ApiProtocol::Anthropic,
    ] {
        let body = prepare_model_interaction_http_request(&config(protocol), &request)
            .model_request
            .body;
        let text = body.to_string();
        assert!(
            text.find("Q1").unwrap() < text.find("call_1").unwrap(),
            "{protocol:?}: {text}"
        );
        assert!(
            text.find("R1").unwrap() < text.find("Q2").unwrap(),
            "{protocol:?}: {text}"
        );
        assert!(
            text.find("Q2").unwrap() < text.find("call_2").unwrap(),
            "{protocol:?}: {text}"
        );
        assert!(
            text.find("R2").unwrap() < text.find("Continue the work").unwrap(),
            "{protocol:?}: {text}"
        );
    }
}

#[test]
fn reasoning_indicator_follows_explicit_outgoing_request_fields() {
    for body in [
        json!({}),
        json!({"enable_thinking": false}),
        json!({"reasoning": {"effort": "none"}}),
        json!({"reasoning_effort": "disabled"}),
        json!({"thinking": {"type": "disabled"}, "reasoning_effort": "high"}),
    ] {
        assert!(!request_uses_reasoning(&body), "{body}");
    }
    for body in [
        json!({"enable_thinking": true}),
        json!({"reasoning_effort": "high"}),
        json!({"reasoning_effort": "max"}),
        json!({"reasoning": {"effort": "low"}}),
        json!({"thinking": {"type": "adaptive"}}),
        json!({"thinking": {"type": "enabled"}}),
    ] {
        assert!(request_uses_reasoning(&body), "{body}");
    }
}

#[test]
fn periodic_and_compact_reasoning_reach_http_payload() {
    let root = std::env::temp_dir().join(crate::unique_id("reasoning_payload"));
    let mut core = crate::AgentCore::new(
        "static",
        crate::CoreProfile {
            model: "test".into(),
        },
        &root,
    );
    let mut cfg = config(ApiProtocol::OpenAiCompatible);
    cfg.openai_compatible.reasoning_effort = Some("medium".into());
    for i in 0..31 {
        core.submit_prompt_component(
            crate::PromptComponentRole::User,
            "user_question",
            format!("message {i}"),
            "user_input",
        );
    }
    core.build_next_prompt();
    for n in 1..=70 {
        let base = core.render_prompt();
        let prompt = core.build_model_request_prompt(&base);
        let request = core.model_interaction_request(prompt);
        let body = crate::prepare_model_interaction_http_request(&cfg, &request)
            .model_request
            .body;
        if n % 35 == 0 {
            assert_eq!(body["reasoning_effort"], "medium", "request {n}");
            assert!(body.get("thinking").is_none());
        } else {
            assert_eq!(body["reasoning_effort"], "none");
            assert!(body.get("thinking").is_none());
        }
    }
    core.request_manual_context_compact();
    let base = core.build_next_prompt();
    let prompt = core.build_model_request_prompt(&base);
    let body = crate::prepare_model_interaction_http_request(
        &cfg,
        &core.model_interaction_request(prompt),
    )
    .model_request
    .body;
    assert_eq!(body["reasoning_effort"], "medium");
    drop(core);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn chat_reasoning_wire_effort_respects_user_selection_and_disable() {
    for selected in [
        "minimal", "low", "medium", "high", "xhigh", "max", "disabled",
    ] {
        for critical in [false, true] {
            let mut config = config(ApiProtocol::OpenAiCompatible);
            config.openai_compatible.reasoning_effort = Some(selected.into());
            config.openai_compatible.enable_thinking = Some(true);
            let prepared = prepare_model_request_with_reasoning(&config, "hello", critical);
            let expected = if critical && selected != "disabled" {
                selected
            } else {
                "none"
            };
            assert_eq!(
                prepared.body["reasoning_effort"], expected,
                "selected={selected}, critical={critical}"
            );
            assert!(prepared.body.get("thinking").is_none());
            assert_eq!(prepared.body["enable_thinking"], expected != "none");
            assert_eq!(request_uses_reasoning(&prepared.body), expected != "none");
        }
    }
    let config = config(ApiProtocol::OpenAiCompatible);
    for critical in [false, true] {
        let prepared = prepare_model_request_with_reasoning(&config, "hello", critical);
        assert!(prepared.body.get("reasoning_effort").is_none());
        assert!(prepared.body.get("thinking").is_none());
    }
}

#[test]
fn semantic_reasoning_policy_maps_to_every_protocol() {
    use crate::reasoning::EffectiveReasoning;
    for protocol in [
        ApiProtocol::OpenAiCompatible,
        ApiProtocol::OpenAiResponses,
        ApiProtocol::Anthropic,
    ] {
        for policy in [
            EffectiveReasoning::Unspecified,
            EffectiveReasoning::Disabled,
            EffectiveReasoning::Enabled {
                intensity: Some("high".into()),
            },
            EffectiveReasoning::Enabled { intensity: None },
        ] {
            let cfg = config(protocol);
            let body =
                build_model_request_with_policy(&cfg, &[], StructuredOutputHint::None, &policy);
            match (&policy, protocol) {
                (EffectiveReasoning::Unspecified, _) => {
                    for key in [
                        "thinking",
                        "reasoning",
                        "reasoning_effort",
                        "output_config",
                        "enable_thinking",
                    ] {
                        assert!(body.get(key).is_none());
                    }
                }
                (EffectiveReasoning::Disabled, ApiProtocol::OpenAiCompatible) => {
                    assert_eq!(body["reasoning_effort"], "none")
                }
                (EffectiveReasoning::Disabled, ApiProtocol::OpenAiResponses) => {
                    assert_eq!(body["reasoning"]["effort"], "none")
                }
                (EffectiveReasoning::Disabled, ApiProtocol::Anthropic) => {
                    assert_eq!(body["thinking"]["type"], "disabled");
                    assert!(body.get("output_config").is_none());
                }
                (EffectiveReasoning::Enabled { intensity }, ApiProtocol::OpenAiCompatible) => {
                    if let Some(level) = intensity {
                        assert_eq!(body["reasoning_effort"], level.as_str());
                    } else {
                        assert_eq!(body["enable_thinking"], true);
                    }
                }
                (EffectiveReasoning::Enabled { intensity }, ApiProtocol::OpenAiResponses) => {
                    if let Some(level) = intensity {
                        assert_eq!(body["reasoning"]["effort"], level.as_str());
                    } else {
                        assert_eq!(body["reasoning"], json!({}));
                    }
                }
                (EffectiveReasoning::Enabled { intensity }, ApiProtocol::Anthropic) => {
                    assert_eq!(body["thinking"]["type"], "adaptive");
                    if let Some(level) = intensity {
                        assert_eq!(body["output_config"]["effort"], level.as_str());
                    } else {
                        assert!(body.get("output_config").is_none());
                    }
                }
            }
            if protocol != ApiProtocol::OpenAiCompatible {
                assert!(body.get("reasoning_effort").is_none());
                assert!(body.get("enable_thinking").is_none());
            }
            if protocol != ApiProtocol::Anthropic {
                assert!(body.get("thinking").is_none());
                assert!(body.get("output_config").is_none());
            }
        }
    }
}

#[test]
fn legacy_preferences_resolve_before_all_protocol_adapters() {
    for protocol in [
        ApiProtocol::OpenAiCompatible,
        ApiProtocol::OpenAiResponses,
        ApiProtocol::Anthropic,
    ] {
        for enabled in [None, Some(false), Some(true)] {
            for level in [None, Some("disabled"), Some("none"), Some("high")] {
                for critical in [false, true] {
                    let mut cfg = config(protocol);
                    cfg.openai_compatible.enable_thinking = enabled;
                    cfg.openai_compatible.reasoning_effort = level.map(str::to_owned);
                    let prepared = prepare_model_request_with_reasoning(&cfg, "hello", critical);
                    let policy =
                        ReasoningPreference::from_legacy(enabled, level).resolve(if critical {
                            ReasoningDemand::Required
                        } else {
                            ReasoningDemand::Ordinary
                        });
                    let expected = build_model_request_with_policy(
                        &cfg,
                        &model_prompt_blocks(&plan_prompt_cache("hello")),
                        plan_structured_output(&cfg),
                        &policy,
                    );
                    assert_eq!(prepared.body, expected);
                    if enabled == Some(false)
                        || matches!(level, Some("disabled" | "none"))
                        || !critical
                    {
                        assert!(!request_uses_reasoning(&prepared.body));
                    }
                }
            }
        }
    }
}

#[test]
fn custom_fields_cannot_override_reasoning_policy() {
    for field in ["reasoning", "thinking", "output_config"] {
        let fields = BTreeMap::from([(field.into(), json!({}))]);
        assert!(validate_model_request_fields(&fields).is_err());
    }
}

#[test]
fn anthropic_unsupported_intensity_is_rejected_without_downgrade() {
    for level in [
        "minimal",
        "low",
        "medium",
        "high",
        "xhigh",
        "max",
        "provider-level",
    ] {
        let body = build_model_request_with_policy(
            &config(ApiProtocol::Anthropic),
            &[],
            StructuredOutputHint::None,
            &EffectiveReasoning::Enabled {
                intensity: Some(level.into()),
            },
        );
        assert_eq!(body["output_config"]["effort"], level);
        assert_eq!(
            validate_reasoning_wire(ApiProtocol::Anthropic, &body).is_ok(),
            matches!(level, "low" | "medium" | "high" | "max")
        );
    }
}

#[test]
fn catalog_request_payload_obeys_each_enabled_protocol_and_effort() {
    for model in crate::model_catalog::models() {
        for profile in &model.protocols {
            if profile.disabled_reason.is_some() {
                continue;
            }
            for effort in &model.efforts {
                if profile
                    .fixed_effort
                    .as_ref()
                    .is_some_and(|fixed| fixed != effort)
                {
                    continue;
                }
                let mut cfg = config(parse_api_protocol(&profile.protocol).unwrap());
                cfg.model = model.model.clone();
                cfg.max_llm_output_tokens = 8000;
                cfg.openai_compatible.catalog_id = Some(model.id.clone());
                cfg.openai_compatible.reasoning_effort = Some(effort.clone());
                cfg.request_fields
                    .insert("vendor_options".into(), json!({"flag":true}));
                let body = build_model_request(&cfg, &[], StructuredOutputHint::None);
                let (path, output) = if cfg.api_protocol == ApiProtocol::OpenAiResponses {
                    ("/reasoning/effort", "max_output_tokens")
                } else {
                    ("/reasoning_effort", "max_tokens")
                };
                assert_eq!(
                    body.pointer(path).and_then(Value::as_str),
                    Some(effort.as_str()),
                    "{}",
                    model.id
                );
                assert_eq!(body[output], 8000);
                assert_eq!(body["model"], model.model);
                assert_eq!(body["vendor_options"], json!({"flag":true}));
                assert!(body.get("catalog_id").is_none());
                crate::model_catalog::validate_request(&cfg, &body).unwrap();
            }
        }
    }
}

#[test]
fn catalog_request_rejects_forbidden_wire_effort_and_budget() {
    let mut cfg = config(ApiProtocol::OpenAiCompatible);
    cfg.model = "gpt-6-sol".into();
    cfg.openai_compatible.catalog_id = Some("openai/gpt-6-sol".into());
    cfg.openai_compatible.reasoning_effort = Some("high".into());
    let body = build_model_request(&cfg, &[], StructuredOutputHint::None);
    assert!(crate::model_catalog::validate_request(&cfg, &body).is_err());
    cfg.openai_compatible.reasoning_effort = Some("none".into());
    cfg.max_llm_output_tokens = 128001;
    let body = build_model_request(&cfg, &[], StructuredOutputHint::None);
    assert!(crate::model_catalog::validate_request(&cfg, &body).is_err());
    cfg.openai_compatible.catalog_id = None;
    assert!(crate::model_catalog::validate_request(&cfg, &body).is_ok());
}

#[test]
fn catalog_final_wire_rejects_model_or_output_tampering() {
    let mut cfg = config(ApiProtocol::OpenAiResponses);
    cfg.model = "gpt-6-astra".into();
    cfg.openai_compatible.catalog_id = Some("openai/gpt-6-astra".into());
    cfg.openai_compatible.reasoning_effort = Some("high".into());
    let body = build_model_request(&cfg, &[], StructuredOutputHint::None);
    crate::model_catalog::validate_request(&cfg, &body).unwrap();
    let mut wrong = body.clone();
    wrong["max_output_tokens"] = json!(999999);
    assert!(crate::model_catalog::validate_request(&cfg, &wrong).is_err());
    let mut wrong = body;
    wrong["model"] = json!("other-model");
    assert!(crate::model_catalog::validate_request(&cfg, &wrong).is_err());
}

#[test]
fn responses_stream_terminal_parses_text_tools_usage_and_incomplete() {
    let mut cfg = config(ApiProtocol::OpenAiResponses);
    cfg.openai_compatible.stream = true;
    let request = build_model_request(&cfg, &[], StructuredOutputHint::None);
    assert_eq!(request["stream"], true);
    assert!(request.get("stream_options").is_none());
    let response = json!({"model":"test-model","status":"completed","output":[
        {"type":"message","content":[{"type":"output_text","text":"你好"}]},
        {"type":"function_call","call_id":"call_1","name":"readfile","arguments":"{\"path\":\"a\"}"}],
        "usage":{"input_tokens":12,"output_tokens":7,"total_tokens":19}});
    let wire = format!(
        "event: response.output_text.delta\ndata: {}\n\nevent: response.completed\ndata: {}\n\n",
        json!({"type":"response.output_text.delta","delta":"你好"}),
        json!({"type":"response.completed","response":response})
    );
    let parsed = interpret_model_http_response(&cfg, 200, &wire, "")
        .result
        .unwrap();
    assert_eq!(parsed.content, "你好");
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.usage.total_tokens, 19);
    assert!(!parsed.truncated);
    let wire = format!(
        "data: {}\n\n",
        json!({"type":"response.incomplete","response":{"model":"test-model","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[]}})
    );
    assert!(
        interpret_model_http_response(&cfg, 200, &wire, "")
            .result
            .unwrap()
            .truncated
    );
    for wire in [
        "data: {bad}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
        "data: {\"type\":\"response.failed\"}\n\n",
    ] {
        assert!(interpret_model_http_response(&cfg, 200, wire, "")
            .result
            .is_err());
    }
}

fn zhipu_config(suffix: &str) -> ModelServiceConfig {
    let mut cfg = config(ApiProtocol::OpenAiCompatible);
    cfg.model = format!("glm-{suffix}");
    cfg.openai_compatible.catalog_id = Some(format!("z-glm{suffix}"));
    cfg.openai_compatible.stream = true;
    cfg
}

#[test]
fn zhipu_reasoning_defaults_disable_and_final_wire_guards() {
    for suffix in ["5.2", "5.3", "5.3-flash"] {
        let mut cfg = zhipu_config(suffix);
        let body = build_model_request(&cfg, &[], StructuredOutputHint::None);
        assert_eq!(body["reasoning_effort"], "max");
        assert_eq!(
            body["thinking"],
            json!({"type":"enabled","clear_thinking":true})
        );
        assert!(body.get("enable_thinking").is_none());
        assert!(body.get("stream_options").is_none());
        crate::model_catalog::validate_request(&cfg, &body).unwrap();
        for patch in [
            json!({"type":"disabled","clear_thinking":true}),
            json!({"type":"enabled","clear_thinking":false}),
            json!({"type":"enabled"}),
        ] {
            let mut wrong = body.clone();
            wrong["thinking"] = patch;
            assert!(crate::model_catalog::validate_request(&cfg, &wrong).is_err());
        }
        for field in ["enable_thinking", "stream_options"] {
            let mut wrong = body.clone();
            wrong[field] = json!(true);
            assert!(crate::model_catalog::validate_request(&cfg, &wrong).is_err());
        }
        cfg.openai_compatible.enable_thinking = Some(false);
        let body = build_model_request(&cfg, &[], StructuredOutputHint::None);
        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(
            crate::model_catalog::validate_request(&cfg, &body).is_ok(),
            suffix == "5.2"
        );
        cfg.openai_compatible.enable_thinking = None;
        for effort in ["none", "disabled", "minimal", "medium", "xhigh", "unknown"] {
            cfg.openai_compatible.reasoning_effort = Some(effort.into());
            let body = build_model_request(&cfg, &[], StructuredOutputHint::None);
            assert_eq!(
                crate::model_catalog::validate_request(&cfg, &body).is_ok(),
                suffix == "5.2" && matches!(effort, "none" | "disabled")
            );
        }
    }
}

#[test]
fn zhipu_native_reasoning_roundtrip_json_and_sse_stays_out_of_public_text() {
    for suffix in ["5.2", "5.3", "5.3-flash"] {
        let cfg = zhipu_config(suffix);
        let tool = json!({"id":"call_1","type":"function","function":{"name":"readfile","arguments":"{\"path\":\"fixture\"}"}});
        let raw = json!({"choices":[{"message":{"content":"可见正文","reasoning_content":"opaque 片段\n unchanged", "tool_calls":[tool]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}});
        let mut chunk = tool.clone();
        chunk["index"] = json!(0);
        let sse = [
            json!({"choices":[{"delta":{"reasoning_content":"opaque 片段\n"}}]}),
            json!({"choices":[{"delta":{"reasoning_content":" unchanged", "content":"可见正文","tool_calls":[chunk]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}}),
        ].iter().map(|event| format!("data: {event}\n\n")).collect::<String>() + "data: [DONE]\n\n";
        for wire in [raw.to_string(), sse] {
            let response = interpret_model_http_response(&cfg, 200, &wire, "")
                .result
                .unwrap();
            assert_eq!(response.content, "可见正文");
            assert_eq!(response.usage.total_tokens, 30);
            assert_eq!(response.tool_calls[0].arguments, json!({"path":"fixture"}));
            let metadata = response.tool_calls[0]
                .assistant_continuation
                .as_ref()
                .unwrap();
            assert_eq!(metadata.reasoning_content, "opaque 片段\n unchanged");
            let mut request = native_request();
            request.rendered_prompt =
                "[BEGIN DELTA delta_id: pd_1, time_ms: 1]\n\n## USER\nfixture".into();
            let exchange = NativeExchange {
                delta_id: "pd_1".into(),
                assistant_text: response.content,
                calls: response.tool_calls,
                results: vec![NativeToolResult {
                    call_id: "call_1".into(),
                    name: "readfile".into(),
                    content: "fixture-result".into(),
                    is_error: false,
                }],
            };
            // Persistence roundtrip retains metadata, old records remain readable.
            request.native_exchanges =
                vec![serde_json::from_value(serde_json::to_value(exchange).unwrap()).unwrap()];
            let body = prepare_model_interaction_http_request(&cfg, &request)
                .model_request
                .body;
            crate::model_catalog::validate_request(&cfg, &body).unwrap();
            let messages = body["messages"].as_array().unwrap();
            let assistant = messages
                .iter()
                .position(|m| m["role"] == "assistant")
                .unwrap();
            assert_eq!(
                messages[assistant]["reasoning_content"],
                "opaque 片段\n unchanged"
            );
            assert_eq!(messages[assistant]["content"], "可见正文");
            assert_eq!(messages[assistant + 1]["role"], "tool");
            assert_eq!(messages[assistant + 1]["content"], "fixture-result");
            for other in [
                config(ApiProtocol::OpenAiCompatible),
                zhipu_config("different-model"),
                zhipu_config(if suffix == "5.2" { "5.3" } else { "5.2" }),
            ] {
                let body = prepare_model_interaction_http_request(&other, &request)
                    .model_request
                    .body;
                assert!(!body.to_string().contains("opaque 片段"));
            }
        }
    }
    let legacy: NativeToolCall = serde_json::from_value(
        json!({"id":"old","name":"readfile","arguments":{},"raw_arguments":"{}"}),
    )
    .unwrap();
    assert!(legacy.assistant_continuation.is_none());
    assert!(serde_json::to_value(legacy)
        .unwrap()
        .get("assistant_continuation")
        .is_none());
}

#[test]
fn zhipu_stream_rejects_oversized_continuation_and_tool_index() {
    let cfg = zhipu_config("5.3");
    for delta in [
        json!({"reasoning_content":"x".repeat(MAX_ASSISTANT_CONTINUATION_BYTES + 1)}),
        json!({"tool_calls":[{"index":u64::MAX}]}),
    ] {
        let wire = format!("data: {}\n\n", json!({"choices":[{"delta":delta}]}));
        assert!(interpret_model_http_response(&cfg, 200, &wire, "")
            .result
            .is_err());
    }
}

#[test]
fn demand_v1_daily_adaptive_mapping_and_final_wire_validation() {
    use crate::model_requirements::{reasoning_upgrade, validate_config, EndpointRequirements};
    for provider in ["openai", "zhipu"] {
        for protocol in [ApiProtocol::OpenAiCompatible, ApiProtocol::OpenAiResponses] {
            if provider == "zhipu" && protocol == ApiProtocol::OpenAiResponses {
                continue;
            }
            let mut cfg = if provider == "zhipu" {
                zhipu_config("5.3")
            } else {
                config(protocol)
            };
            cfg.openai_compatible.catalog_id = None;
            cfg.openai_compatible.requirements = EndpointRequirements {
                version: 1,
                provider: Some(provider.into()),
                allowed_reasoning: Some(vec!["low".into(), "high".into(), "max".into()]),
                adaptive_reasoning: Some(true),
                ..Default::default()
            };
            cfg.openai_compatible.reasoning_effort = Some("low".into());
            validate_config(&cfg).unwrap();
            for (critical, expected) in [(false, "low"), (true, "high")] {
                let body = build_model_request_with_reasoning(
                    &cfg,
                    &[],
                    StructuredOutputHint::None,
                    critical,
                );
                crate::model_payload::validate_request(&cfg, &body, critical).unwrap();
                let path = if protocol == ApiProtocol::OpenAiResponses {
                    "/reasoning/effort"
                } else {
                    "/reasoning_effort"
                };
                assert_eq!(body.pointer(path).and_then(Value::as_str), Some(expected));
                assert!(body.get("enable_thinking").is_none());
                if provider == "zhipu" {
                    assert_eq!(
                        body["thinking"],
                        json!({"type":"enabled", "clear_thinking":true})
                    );
                }
                let mut wrong = body.clone();
                *wrong.pointer_mut(path).unwrap() = json!("max"); // valid enum, wrong demand
                assert!(crate::model_payload::validate_request(&cfg, &wrong, critical).is_err());
            }
            let upgrade = reasoning_upgrade(&cfg, true).unwrap();
            assert_eq!(
                (upgrade.from.as_str(), upgrade.to.as_str()),
                ("low", "high")
            );
            assert!(reasoning_upgrade(&cfg, false).is_none());
            cfg.openai_compatible.requirements.adaptive_reasoning = Some(false);
            assert!(reasoning_upgrade(&cfg, true).is_none());
            cfg.openai_compatible.requirements.allowed_reasoning = Some(vec!["high".into()]);
            assert!(validate_config(&cfg).is_err());
        }
    }
}

#[test]
fn demand_v1_identity_is_independent_of_template_and_url() {
    let mut cfg = zhipu_config("5.3");
    cfg.openai_compatible.requirements.version = 1;
    assert_eq!(crate::model_requirements::provider(&cfg), None);
    cfg.openai_compatible.requirements.provider = Some("zhipu".into());
    cfg.openai_compatible.catalog_id = Some("openai/gpt-6-astra".into());
    cfg.base_url = "https://proxy.invalid/custom".into();
    assert!(crate::model_catalog::uses_zhipu_chat(&cfg));
    crate::model_requirements::validate_config(&cfg).unwrap();
    cfg.model = "not-declared".into();
    assert!(crate::model_requirements::validate_config(&cfg).is_err());
    cfg.openai_compatible.requirements.provider = Some("openai".into());
    crate::model_requirements::validate_config(&cfg).unwrap();
}

#[test]
fn demand_v1_rejects_excluded_default_and_unsupported_provider_protocol() {
    let mut cfg = zhipu_config("5.3");
    cfg.openai_compatible.reasoning_effort = None;
    cfg.openai_compatible.requirements =
        serde_json::from_value(json!({"version":1,"provider":"zhipu","allowed_reasoning":["low"]}))
            .unwrap();
    assert_eq!(
        crate::model_requirements::validate_config(&cfg).unwrap_err(),
        "daily_reasoning_not_in_allowed_set"
    );
    cfg.openai_compatible.requirements.allowed_reasoning = None;
    cfg.api_protocol = ApiProtocol::OpenAiResponses;
    assert_eq!(
        crate::model_requirements::validate_config(&cfg).unwrap_err(),
        "provider_protocol_adapter_not_implemented"
    );
}

#[test]
fn clearing_endpoint_preferences_removes_stale_disable_and_effort() {
    let mut options = crate::OpenAiCompatibleOptions {
        enable_thinking: Some(false),
        reasoning_effort: Some("max".into()),
        ..Default::default()
    };
    for key in ["TIMEM_ENABLE_THINKING", "TIMEM_REASONING_EFFORT"] {
        crate::model_service_config::apply_openai_compatible_env_value(&mut options, key, "")
            .unwrap();
    }
    assert_eq!(options.enable_thinking, None);
    assert_eq!(options.reasoning_effort, None);
}
