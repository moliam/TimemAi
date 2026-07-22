use serde_json::{json, Value};

use crate::{
    plan_prompt_cache, redact_value, stable_text_fingerprint, CacheControl, CoreProfile,
    LlmResponse, PromptBlock, PromptBlockRole, ResponseProtocolKind, UsageStats,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiProtocol {
    OpenAiCompatible,
    OpenAiResponses,
    Anthropic,
}

impl ApiProtocol {
    pub fn label(&self) -> &'static str {
        match self {
            ApiProtocol::OpenAiCompatible => "openai-compatible",
            ApiProtocol::OpenAiResponses => "openai-responses",
            ApiProtocol::Anthropic => "anthropic",
        }
    }
}

pub fn parse_api_protocol(value: &str) -> Result<ApiProtocol, String> {
    match value.trim().to_lowercase().as_str() {
        "openai" | "openai-compatible" | "openai_compatible" | "chat-completions"
        | "chat_completions" => Ok(ApiProtocol::OpenAiCompatible),
        "openai-responses" | "openai_responses" | "responses" => Ok(ApiProtocol::OpenAiResponses),
        "anthropic" | "claude" | "messages" => Ok(ApiProtocol::Anthropic),
        other => Err(format!(
            "invalid_api_protocol: {other}; expected openai-compatible, openai-responses, or anthropic"
        )),
    }
}

pub fn default_api_protocol_for_provider(provider: &str) -> ApiProtocol {
    match provider.trim().to_lowercase().as_str() {
        "openai" => ApiProtocol::OpenAiResponses,
        "anthropic" => ApiProtocol::Anthropic,
        _ => ApiProtocol::OpenAiCompatible,
    }
}

pub fn default_model_for_provider(provider: &str) -> &'static str {
    match provider.trim().to_lowercase().as_str() {
        "openai" => "gpt-4o",
        "anthropic" => "claude-sonnet-4-20250514",
        _ => "qwen-plus",
    }
}

pub fn is_default_model_for_provider(provider: &str, model: &str) -> bool {
    let provider = provider.trim().to_lowercase();
    let model = model.trim().to_lowercase();
    match provider.as_str() {
        "openai" => model.contains("gpt"),
        "anthropic" => model.contains("claude"),
        "aliyun" | "dashscope" => model.contains("qwen"),
        _ => true,
    }
}

fn default_base_url(provider: &str) -> &'static str {
    match provider {
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com",
        "aliyun" | "dashscope" => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        _ => "https://dashscope.aliyuncs.com/compatible-mode/v1",
    }
}

pub fn default_base_url_for_provider(provider: &str) -> String {
    default_base_url(&provider.trim().to_lowercase()).to_string()
}

pub fn known_default_base_url_for_provider(provider: &str) -> Option<String> {
    let provider = provider.trim().to_lowercase();
    matches!(
        provider.as_str(),
        "openai" | "anthropic" | "aliyun" | "dashscope"
    )
    .then(|| default_base_url(&provider).to_string())
}

pub fn is_default_base_url_for_provider(provider: &str, base_url: &str) -> bool {
    let provider = provider.trim().to_lowercase();
    match provider.as_str() {
        "openai" | "anthropic" | "aliyun" | "dashscope" => {
            base_url.trim_end_matches('/') == default_base_url(&provider).trim_end_matches('/')
        }
        _ => true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub timeout_secs: u64,
    pub max_llm_output_tokens: u32,
    pub max_llm_input_tokens: u32,
    pub api_protocol: ApiProtocol,
    pub response_protocol: ResponseProtocolKind,
    pub openai_compatible: OpenAiCompatibleOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenAiCompatibleOptions {
    pub enable_thinking: Option<bool>,
    pub reasoning_effort: Option<String>,
    pub stream: bool,
}

impl ProviderConfig {
    pub fn core_profile(&self) -> CoreProfile {
        CoreProfile {
            name: self.provider.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
        }
    }

    pub fn endpoint(&self) -> String {
        match self.api_protocol {
            ApiProtocol::OpenAiCompatible => {
                format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
            }
            ApiProtocol::OpenAiResponses => {
                format!("{}/responses", self.base_url.trim_end_matches('/'))
            }
            ApiProtocol::Anthropic => {
                let base = self.base_url.trim_end_matches('/');
                if base.ends_with("/v1") {
                    format!("{}/messages", base)
                } else {
                    format!("{}/v1/messages", base)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPromptRole {
    System,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCacheControl {
    None,
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPromptBlock {
    pub role: ProviderPromptRole,
    pub text: String,
    pub cache: ProviderCacheControl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedProviderRequest {
    pub body: Value,
    pub prompt_cache_plan: Value,
    pub structured_output: StructuredOutputHint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedProviderHttpRequest {
    pub endpoint: String,
    pub headers: Vec<(String, String)>,
    pub provider_request: PreparedProviderRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHttpResponseInterpretation {
    pub status: u16,
    pub raw_json: Value,
    pub result: Result<LlmResponse, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredOutputHint {
    None,
    JsonObject,
}

pub fn plan_structured_output(config: &ProviderConfig) -> StructuredOutputHint {
    if config.response_protocol != ResponseProtocolKind::Json {
        return StructuredOutputHint::None;
    }
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible
            if supports_openai_compatible_json_object(&config.provider) =>
        {
            StructuredOutputHint::JsonObject
        }
        _ => StructuredOutputHint::None,
    }
}

fn supports_openai_compatible_json_object(provider: &str) -> bool {
    matches!(provider, "aliyun" | "dashscope" | "openai")
}

pub fn build_provider_request(
    config: &ProviderConfig,
    blocks: &[ProviderPromptBlock],
    structured_output: StructuredOutputHint,
) -> Value {
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible => {
            build_openai_compatible_request(config, blocks, structured_output)
        }
        ApiProtocol::OpenAiResponses => build_openai_responses_request(config, blocks),
        ApiProtocol::Anthropic => build_anthropic_request(config, blocks),
    }
}

pub fn prepare_provider_request(
    config: &ProviderConfig,
    rendered_prompt: &str,
) -> PreparedProviderRequest {
    let prompt_blocks = plan_prompt_cache(rendered_prompt);
    let structured_output = plan_structured_output(config);
    let provider_blocks = provider_prompt_blocks(&prompt_blocks);
    PreparedProviderRequest {
        body: build_provider_request(config, &provider_blocks, structured_output),
        prompt_cache_plan: prompt_cache_plan_audit(&prompt_blocks),
        structured_output,
    }
}

pub fn prepare_provider_http_request(
    config: &ProviderConfig,
    rendered_prompt: &str,
) -> PreparedProviderHttpRequest {
    PreparedProviderHttpRequest {
        endpoint: config.endpoint(),
        headers: provider_http_headers(config),
        provider_request: prepare_provider_request(config, rendered_prompt),
    }
}

fn provider_http_headers(config: &ProviderConfig) -> Vec<(String, String)> {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible | ApiProtocol::OpenAiResponses => {
            headers.push((
                "Authorization".to_string(),
                format!("Bearer {}", config.api_key),
            ));
        }
        ApiProtocol::Anthropic => {
            headers.push(("x-api-key".to_string(), config.api_key.clone()));
            headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
        }
    }
    headers
}

pub fn provider_request_audit_event(
    config: &ProviderConfig,
    prepared_request: &PreparedProviderRequest,
) -> Value {
    json!({
        "type": "llm_request",
        "provider": config.provider,
        "model": config.model,
        "endpoint": config.endpoint(),
        "prompt_cache_plan": prepared_request.prompt_cache_plan,
        "structured_output": structured_output_label(prepared_request.structured_output),
        "body": redact_value(&prepared_request.body),
    })
}

pub fn provider_response_audit_event(status: u16, raw_body: &Value) -> Value {
    let error_kind = if !(200..400).contains(&status) {
        "http_error"
    } else {
        "http_success"
    };
    let response = if status >= 400 {
        match raw_body.get("error") {
            Some(e) => json!({ "error": redact_value(e) }),
            None => json!({}),
        }
    } else {
        json!({})
    };
    json!({
        "type": "llm_response",
        "status": status,
        "error_kind": error_kind,
        "response": response,
        "body": redact_value(raw_body),
    })
}

fn structured_output_label(value: StructuredOutputHint) -> &'static str {
    match value {
        StructuredOutputHint::None => "none",
        StructuredOutputHint::JsonObject => "json_object",
    }
}

pub fn provider_prompt_blocks(blocks: &[PromptBlock]) -> Vec<ProviderPromptBlock> {
    blocks
        .iter()
        .map(|block| ProviderPromptBlock {
            role: match block.role {
                PromptBlockRole::System => ProviderPromptRole::System,
                PromptBlockRole::User => ProviderPromptRole::User,
            },
            text: block.text.clone(),
            cache: match block.cache {
                CacheControl::None => ProviderCacheControl::None,
                CacheControl::Ephemeral => ProviderCacheControl::Ephemeral,
            },
        })
        .collect()
}

pub fn prompt_cache_plan_audit(blocks: &[PromptBlock]) -> Value {
    Value::Array(
        blocks
            .iter()
            .map(|block| {
                json!({
                    "role": match block.role {
                        PromptBlockRole::System => "system",
                        PromptBlockRole::User => "user",
                    },
                    "cache": match block.cache {
                        CacheControl::None => "none",
                        CacheControl::Ephemeral => "ephemeral",
                    },
                    "chars": block.text.chars().count(),
                    "hash": stable_text_fingerprint(&block.text),
                })
            })
            .collect(),
    )
}

fn build_openai_compatible_request(
    config: &ProviderConfig,
    blocks: &[ProviderPromptBlock],
    structured_output: StructuredOutputHint,
) -> Value {
    let messages = blocks
        .iter()
        .map(|block| {
            let mut message = json!({
                "role": role_label(block.role),
                "content": block.text,
            });
            apply_cache_control(&mut message, block.cache);
            message
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": config.max_llm_output_tokens
    });
    if let Some(enable_thinking) = config.openai_compatible.enable_thinking {
        body["enable_thinking"] = json!(enable_thinking);
    }
    if let Some(reasoning_effort) = &config.openai_compatible.reasoning_effort {
        body["reasoning_effort"] = json!(reasoning_effort);
    }
    if config.openai_compatible.stream {
        body["stream"] = json!(true);
        body["stream_options"] = json!({"include_usage": true});
    }
    apply_structured_output(&mut body, structured_output);
    body
}

fn build_openai_responses_request(
    config: &ProviderConfig,
    blocks: &[ProviderPromptBlock],
) -> Value {
    let instructions = blocks
        .iter()
        .filter(|block| block.role == ProviderPromptRole::System)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let input = blocks
        .iter()
        .filter(|block| block.role == ProviderPromptRole::User)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "model": config.model,
        "instructions": instructions,
        "input": input,
        "max_output_tokens": config.max_llm_output_tokens
    })
}

fn build_anthropic_request(config: &ProviderConfig, blocks: &[ProviderPromptBlock]) -> Value {
    let system = blocks
        .iter()
        .filter(|block| block.role == ProviderPromptRole::System)
        .map(|block| {
            let mut item = json!({"type":"text", "text": block.text});
            apply_cache_control(&mut item, block.cache);
            item
        })
        .collect::<Vec<_>>();
    let content = blocks
        .iter()
        .filter(|block| block.role == ProviderPromptRole::User)
        .map(|block| {
            let mut item = json!({"type":"text", "text": block.text});
            apply_cache_control(&mut item, block.cache);
            item
        })
        .collect::<Vec<_>>();
    json!({
        "model": config.model,
        "max_tokens": config.max_llm_output_tokens,
        "system": system,
        "messages": [{"role":"user", "content": content}]
    })
}

fn role_label(role: ProviderPromptRole) -> &'static str {
    match role {
        ProviderPromptRole::System => "system",
        ProviderPromptRole::User => "user",
    }
}

fn apply_cache_control(value: &mut Value, cache: ProviderCacheControl) {
    if cache == ProviderCacheControl::Ephemeral {
        if let Some(map) = value.as_object_mut() {
            map.insert("cache_control".to_string(), json!({"type":"ephemeral"}));
        }
    }
}

fn apply_structured_output(value: &mut Value, structured_output: StructuredOutputHint) {
    if structured_output == StructuredOutputHint::JsonObject {
        if let Some(map) = value.as_object_mut() {
            map.insert("response_format".to_string(), json!({"type":"json_object"}));
        }
    }
}

pub fn parse_provider_response(
    config: &ProviderConfig,
    raw: &Value,
) -> Result<LlmResponse, String> {
    let (content, usage, truncated) = match config.api_protocol {
        ApiProtocol::OpenAiCompatible => {
            let content = raw
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let finish_reason = raw
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                .unwrap_or("");
            let usage = raw.get("usage").unwrap_or(&Value::Null);
            let prompt_tokens = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let completion_tokens = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let total_tokens = usage
                .get("total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(prompt_tokens as u64 + completion_tokens as u64)
                as u32;
            let cached_tokens = usage
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
                .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_u64))
                .unwrap_or(0) as u32;
            (
                content,
                UsageStats {
                    llm_calls: 1,
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    cached_tokens,
                    cache_created_tokens: 0,
                    ..UsageStats::zero()
                },
                finish_reason == "length" || finish_reason == "max_tokens",
            )
        }
        ApiProtocol::OpenAiResponses => {
            let content = extract_openai_response_text(raw);
            let status = raw.get("status").and_then(Value::as_str).unwrap_or("");
            let incomplete_reason = raw
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
                .unwrap_or("");
            let usage = raw.get("usage").unwrap_or(&Value::Null);
            let prompt_tokens = usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let completion_tokens = usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let total_tokens = usage
                .get("total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(prompt_tokens as u64 + completion_tokens as u64)
                as u32;
            let cached_tokens = usage
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            (
                content,
                UsageStats {
                    llm_calls: 1,
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    cached_tokens,
                    cache_created_tokens: 0,
                    ..UsageStats::zero()
                },
                status == "incomplete" && incomplete_reason == "max_output_tokens",
            )
        }
        ApiProtocol::Anthropic => {
            let content = raw
                .get("content")
                .and_then(Value::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .find_map(|item| item.get("text").and_then(Value::as_str))
                })
                .unwrap_or("")
                .to_string();
            let stop_reason = raw.get("stop_reason").and_then(Value::as_str).unwrap_or("");
            let usage = raw.get("usage").unwrap_or(&Value::Null);
            let prompt_tokens = usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let cache_read_tokens = usage
                .get("cache_read_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let cache_creation_tokens = usage
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let completion_tokens = usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let billed_prompt_tokens = prompt_tokens + cache_read_tokens + cache_creation_tokens;
            (
                content,
                UsageStats {
                    llm_calls: 1,
                    prompt_tokens: billed_prompt_tokens,
                    completion_tokens,
                    total_tokens: billed_prompt_tokens + completion_tokens,
                    cached_tokens: cache_read_tokens,
                    cache_created_tokens: cache_creation_tokens,
                    ..UsageStats::zero()
                },
                stop_reason == "max_tokens",
            )
        }
    };
    Ok(LlmResponse {
        content,
        model_name: config.model.clone(),
        usage,
        truncated,
    })
}

pub fn interpret_provider_http_response(
    config: &ProviderConfig,
    status: u16,
    body_text: &str,
    stderr_text: &str,
) -> ProviderHttpResponseInterpretation {
    if (200..300).contains(&status)
        && config.api_protocol == ApiProtocol::OpenAiCompatible
        && looks_like_sse(body_text)
    {
        return interpret_openai_compatible_sse(config, status, body_text);
    }
    let mut parsed_json = true;
    let raw_json: Value = serde_json::from_str(body_text).unwrap_or_else(|_| {
        parsed_json = false;
        json!({
            "raw_text": body_text,
            "stderr": stderr_text,
        })
    });
    let result = if !(200..300).contains(&status) {
        Err(provider_http_error_message(status, &raw_json))
    } else if !parsed_json {
        Ok(LlmResponse {
            content: body_text.to_string(),
            model_name: config.model.clone(),
            usage: UsageStats::zero(),
            truncated: false,
        })
    } else {
        parse_provider_response(config, &raw_json)
    };
    ProviderHttpResponseInterpretation {
        status,
        raw_json,
        result,
    }
}

fn looks_like_sse(body: &str) -> bool {
    body.lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty() && !line.starts_with(':'))
        .is_some_and(|line| line.starts_with("data:") || line.starts_with("event:"))
}

fn interpret_openai_compatible_sse(
    config: &ProviderConfig,
    status: u16,
    body_text: &str,
) -> ProviderHttpResponseInterpretation {
    let mut content = String::new();
    let mut finish_reason = String::new();
    let mut usage = Value::Null;
    let mut event_count = 0_u64;
    let mut reasoning_chunk_count = 0_u64;
    let mut parse_error = None;

    for line in body_text.lines() {
        let line = line.trim_start();
        let Some(payload) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let event: Value = match serde_json::from_str(payload) {
            Ok(event) => event,
            Err(error) => {
                parse_error = Some(format!("invalid_provider_sse_event: {error}"));
                break;
            }
        };
        event_count += 1;
        if event
            .pointer("/choices/0/delta/reasoning_content")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
        {
            reasoning_chunk_count += 1;
        }
        if let Some(text) = event
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
        {
            content.push_str(text);
        }
        if let Some(reason) = event
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            finish_reason = reason.to_string();
        }
        if !event.get("usage").unwrap_or(&Value::Null).is_null() {
            usage = event["usage"].clone();
        }
    }

    let raw_json = json!({
        "stream": true,
        "stream_metadata": {
            "event_count": event_count,
            "reasoning_chunk_count": reasoning_chunk_count,
        },
        "choices": [{
            "message": {"content": content},
            "finish_reason": finish_reason,
        }],
        "usage": usage,
    });
    let result = match parse_error {
        Some(error) => Err(error),
        None if event_count == 0 => Err("empty_provider_sse_response".to_string()),
        None => parse_provider_response(config, &raw_json),
    };
    ProviderHttpResponseInterpretation {
        status,
        raw_json,
        result,
    }
}

pub fn provider_http_error_message(status: u16, body: &Value) -> String {
    let reason = provider_error_reason(body)
        .map(sanitize_provider_error_reason)
        .filter(|text| !text.trim().is_empty());
    if status == 0 {
        return match reason {
            Some(reason) if reason.to_lowercase().contains("timed out") => {
                format!("provider_timeout: {reason}")
            }
            Some(reason) => format!("provider_network_error: {reason}"),
            None => "provider_network_error".to_string(),
        };
    }
    match reason {
        Some(reason) => format!("provider_http_{status}: {reason}"),
        None => format!("provider_http_{status}"),
    }
}

fn provider_error_reason(body: &Value) -> Option<String> {
    for pointer in [
        "/error/message",
        "/error/code",
        "/error/type",
        "/message",
        "/detail",
        "/code",
    ] {
        if let Some(text) = body.pointer(pointer).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    if let Some(error) = body.get("error").and_then(Value::as_str) {
        if !error.trim().is_empty() {
            return Some(error.to_string());
        }
    }
    if let Some(raw) = body.get("raw_text").and_then(Value::as_str) {
        if !raw.trim().is_empty() {
            return Some(raw.to_string());
        }
    }
    if let Some(stderr) = body.get("stderr").and_then(Value::as_str) {
        if !stderr.trim().is_empty() {
            return Some(stderr.to_string());
        }
    }
    None
}

fn sanitize_provider_error_reason(reason: String) -> String {
    let single_line = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_secret_like_text(&single_line);
    compact_provider_error_text(&redacted, 240)
}

fn redact_secret_like_text(text: &str) -> String {
    text.split_whitespace()
        .map(|part| {
            let lower = part.to_lowercase();
            if lower.starts_with("sk-")
                || lower.starts_with("bearer")
                || lower.contains("api_key")
                || lower.contains("apikey")
                || lower.contains("authorization")
            {
                "***REDACTED***".to_string()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn compact_provider_error_text(text: &str, max_chars: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (idx, ch) in one_line.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}

fn extract_openai_response_text(raw: &Value) -> String {
    if let Some(text) = raw.get("output_text").and_then(Value::as_str) {
        if !text.is_empty() {
            return text.to_string();
        }
    }

    raw.get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .flat_map(|item| {
                    item.get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("output_text") => part.get("text").and_then(Value::as_str),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../tests/unit/provider_tests.rs"]
mod tests;
