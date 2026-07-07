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
    let error_kind = if status < 200 || status >= 400 {
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
mod tests {
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
            crate::prompt_render::formatted_response_trailer("XML")
        );

        let prepared = prepare_provider_request(&config, &prompt);
        let content = prepared.body["messages"][0]["content"].as_array().unwrap();

        assert_eq!(
            content.last().unwrap()["text"],
            "Follow the system prompt, give your XML formatted response:"
        );
        assert_eq!(content.last().unwrap().get("cache_control"), None);
        assert!(!content[0]["text"]
            .as_str()
            .unwrap()
            .contains("Follow the system prompt, give your XML formatted response:"));
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
            crate::prompt_render::formatted_response_trailer("JSON")
        );

        let prepared = prepare_provider_request(&config, &prompt);
        let messages = prepared.body["messages"].as_array().unwrap();

        assert_eq!(
            messages.last().unwrap()["content"],
            "Follow the system prompt, give your JSON formatted response:"
        );
        assert_eq!(messages.last().unwrap().get("cache_control"), None);
        assert!(!messages[messages.len() - 2]["content"]
            .as_str()
            .unwrap()
            .contains("Follow the system prompt, give your JSON formatted response:"));
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
                "choices":[{"message":{"content":"{\"report_job_progress\":\"hi\"}"}}],
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
                "choices":[{"finish_reason":"length","message":{"content":"{\"report_job_progress\":\"partial\"}"}}],
                "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}
            }),
        )
        .unwrap();
        assert!(response.truncated);

        let response = parse_provider_response(
            &config(ApiProtocol::OpenAiCompatible),
            &json!({
                "choices":[{"message":{"content":"{\"report_job_progress\":\"hi\"}"}}],
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
                "output_text":"{\"report_job_progress\":\"hi\"}",
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
        assert_eq!(response.content, "{\"report_job_progress\":\"hi\"}");
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
                "output_text":"{\"report_job_progress\":\"partial\"}",
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
                    "content":[{"type":"output_text","text":"{\"report_job_progress\":\"from output\"}","annotations":[]}]
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
        assert_eq!(
            response.content,
            "{\"report_job_progress\":\"from output\"}"
        );
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
                "content":[{"type":"text","text":"{\"report_job_progress\":\"partial\"}"}],
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
}
