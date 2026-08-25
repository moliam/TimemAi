use super::*;
use crate::{NativeExchange, NativeToolChoice, NativeToolResult, ToolCallMode};

fn config(api_protocol: ApiProtocol) -> ModelServiceConfig {
    ModelServiceConfig {
        interaction: Default::default(),
        model: "test-model".to_string(),
        base_url: "https://example.invalid/v1".to_string(),
        api_key: "dummy".to_string(),
        http_headers: Default::default(),
        timeout_secs: 1,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        api_protocol,
        response_protocol: ResponseProtocolKind::Json,
        openai_compatible: crate::OpenAiCompatibleOptions::default(),
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
        enable_thinking: Some(true),
        reasoning_effort: Some("max".to_string()),
        stream: true,
        cache_mode: OpenAiCompatibleCacheMode::Auto,
    };

    let body = build_model_request(
        &config,
        &[ModelPromptBlock {
            role: ModelPromptRole::User,
            text: "hello".to_string(),
            cache: ModelCacheControl::None,
        }],
        StructuredOutputHint::None,
    );

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
    }
}

#[test]
fn native_tool_wires_are_provider_specific_and_parallel_is_explicit() {
    let mut request = native_request();
    request.tools.push(ToolDefinition {
        name: "mcp.demo.search".to_string(),
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
fn anthropic_native_cache_breakpoint_ends_at_static_builtin_tool_prefix() {
    let mut request = native_request();
    request.tools.push(ToolDefinition {
        name: "mcp.demo.search".to_string(),
        description: "Dynamic MCP search.".to_string(),
        input_schema: json!({"type": "object"}),
    });
    let prepared =
        prepare_model_interaction_http_request(&config(ApiProtocol::Anthropic), &request);
    let tools = prepared.model_request.body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "count_lines");
    assert_eq!(tools[0]["cache_control"], json!({"type": "ephemeral"}));
    assert_eq!(tools[1]["name"], "mcp.demo.search");
    assert_eq!(tools[1]["description"], "Dynamic MCP search.");
    assert!(tools[1].get("input_schema").is_some());
    assert!(tools[1].get("cache_control").is_none());
    assert!(prepared.model_request.cache_mark_count >= 1);
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
fn native_exchanges_follow_owning_delta_order_for_all_providers() {
    let rendered_prompt = concat!(
        "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n",
        "[BEGIN DELTA delta_id: pd_1, time_ms: 1]\n\n## USER\nQ1\n",
        "[BEGIN DELTA delta_id: pd_2, time_ms: 2]\n\n## USER\nQ2\n\n",
        "Continue the work in the user's language. Call API tools when more evidence or actions are needed; otherwise give the final user-facing answer:"
    ).to_string();
    let exchange = |delta_id: &str, call_id: &str, result: &str| NativeExchange {
        delta_id: delta_id.to_string(),
        assistant_text: format!("work {call_id}"),
        calls: vec![NativeToolCall {
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
