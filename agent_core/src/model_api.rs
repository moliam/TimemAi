use serde_json::{json, Value};

use crate::{
    plan_prompt_cache, redact_value, stable_text_fingerprint, CacheControl, CoreProfile,
    LlmResponse, ModelInteractionRequest, NativeToolCall, PromptBlock, PromptBlockRole,
    ResponseProtocolKind, ToolDefinition, UsageStats,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub fn default_api_protocol() -> ApiProtocol {
    ApiProtocol::OpenAiCompatible
}

pub fn default_model() -> &'static str {
    "qwen-plus"
}

pub fn is_default_model(model: &str) -> bool {
    model.trim() == default_model()
}

pub fn default_base_url(api_protocol: &ApiProtocol) -> &'static str {
    match api_protocol {
        ApiProtocol::OpenAiCompatible => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        ApiProtocol::OpenAiResponses => "https://api.openai.com/v1",
        ApiProtocol::Anthropic => "https://api.anthropic.com",
    }
}

pub fn is_default_base_url(api_protocol: &ApiProtocol, base_url: &str) -> bool {
    base_url.trim_end_matches('/') == default_base_url(api_protocol).trim_end_matches('/')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelServiceConfig {
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub timeout_secs: u64,
    pub max_llm_output_tokens: u32,
    pub max_llm_input_tokens: u32,
    pub api_protocol: ApiProtocol,
    pub response_protocol: ResponseProtocolKind,
    pub interaction: crate::InteractionConfig,
    pub openai_compatible: OpenAiCompatibleOptions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpenAiCompatibleCacheMode {
    #[default]
    Auto,
    Off,
    Ephemeral,
}

impl OpenAiCompatibleCacheMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
            Self::Ephemeral => "ephemeral",
        }
    }
}

pub fn parse_openai_compatible_cache_mode(
    value: &str,
) -> Result<OpenAiCompatibleCacheMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(OpenAiCompatibleCacheMode::Auto),
        "off" => Ok(OpenAiCompatibleCacheMode::Off),
        "ephemeral" => Ok(OpenAiCompatibleCacheMode::Ephemeral),
        _ => Err("invalid_TIMEM_OPENAI_CACHE_MODE: expected auto, off, or ephemeral".to_string()),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenAiCompatibleOptions {
    pub enable_thinking: Option<bool>,
    pub reasoning_effort: Option<String>,
    pub stream: bool,
    pub cache_mode: OpenAiCompatibleCacheMode,
}

impl ModelServiceConfig {
    pub fn core_profile(&self) -> CoreProfile {
        CoreProfile {
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
pub enum ModelPromptRole {
    System,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCacheControl {
    None,
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPromptBlock {
    pub role: ModelPromptRole,
    pub text: String,
    pub cache: ModelCacheControl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedModelRequest {
    pub body: Value,
    pub prompt_cache_plan: Value,
    pub structured_output: StructuredOutputHint,
    pub cache_wire_mode: String,
    pub cache_mark_count: usize,
    pub cache_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedModelHttpRequest {
    pub endpoint: String,
    pub headers: Vec<(String, String)>,
    pub model_request: PreparedModelRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHttpResponseInterpretation {
    pub status: u16,
    pub raw_json: Value,
    pub result: Result<LlmResponse, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredOutputHint {
    None,
    JsonObject,
}

pub fn plan_structured_output(config: &ModelServiceConfig) -> StructuredOutputHint {
    if config.response_protocol != ResponseProtocolKind::Json {
        return StructuredOutputHint::None;
    }
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible => StructuredOutputHint::JsonObject,
        _ => StructuredOutputHint::None,
    }
}

pub fn build_model_request(
    config: &ModelServiceConfig,
    blocks: &[ModelPromptBlock],
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

pub fn prepare_model_request(
    config: &ModelServiceConfig,
    rendered_prompt: &str,
) -> PreparedModelRequest {
    let prompt_blocks = plan_prompt_cache(rendered_prompt);
    let structured_output = plan_structured_output(config);
    let model_blocks = model_prompt_blocks(&prompt_blocks);
    let body = build_model_request(config, &model_blocks, structured_output);
    let cache_mark_count = count_cache_control_marks(&body);
    PreparedModelRequest {
        body,
        prompt_cache_plan: prompt_cache_plan_audit(&prompt_blocks),
        structured_output,
        cache_wire_mode: cache_wire_mode(config).to_string(),
        cache_mark_count,
        cache_fallback: false,
    }
}

pub fn prepare_model_http_request(
    config: &ModelServiceConfig,
    rendered_prompt: &str,
) -> PreparedModelHttpRequest {
    PreparedModelHttpRequest {
        endpoint: config.endpoint(),
        headers: model_http_headers(config),
        model_request: prepare_model_request(config, rendered_prompt),
    }
}

pub fn prepare_model_interaction_http_request(
    config: &ModelServiceConfig,
    interaction: &ModelInteractionRequest,
) -> PreparedModelHttpRequest {
    let mut request = prepare_model_http_request(config, &interaction.rendered_prompt);
    if interaction.is_native() {
        apply_native_interaction(config, &mut request.model_request.body, interaction);
        request.model_request.cache_mark_count =
            count_cache_control_marks(&request.model_request.body);
        request.model_request.structured_output = StructuredOutputHint::None;
        request
            .model_request
            .body
            .as_object_mut()
            .map(|body| body.remove("response_format"));
    }
    request
}

fn apply_native_interaction(
    config: &ModelServiceConfig,
    body: &mut Value,
    interaction: &ModelInteractionRequest,
) {
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible => {
            body["tools"] = Value::Array(
                interaction
                    .tools
                    .iter()
                    .enumerate()
                    .map(|(index, tool)| {
                        openai_chat_tool_definition(tool, index >= interaction.static_tool_count)
                    })
                    .collect(),
            );
            body["tool_choice"] = json!(native_tool_choice_label(interaction.tool_choice));
            body["parallel_tool_calls"] = json!(interaction.parallel_tool_calls);
            if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
                append_openai_chat_exchanges(messages, interaction);
            }
        }
        ApiProtocol::OpenAiResponses => {
            body["tools"] = Value::Array(
                interaction
                    .tools
                    .iter()
                    .enumerate()
                    .map(|(index, tool)| {
                        openai_responses_tool_definition(
                            tool,
                            index >= interaction.static_tool_count,
                        )
                    })
                    .collect(),
            );
            body["tool_choice"] = json!(native_tool_choice_label(interaction.tool_choice));
            body["parallel_tool_calls"] = json!(interaction.parallel_tool_calls);
            let mut input = match body.get("input") {
                Some(Value::Array(items)) => items.clone(),
                Some(Value::String(text)) => vec![json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}],
                })],
                _ => Vec::new(),
            };
            append_openai_responses_exchanges(&mut input, interaction);
            body["input"] = Value::Array(input);
        }
        ApiProtocol::Anthropic => {
            let mut tools = interaction
                .tools
                .iter()
                .enumerate()
                .map(|(index, tool)| {
                    anthropic_tool_definition(tool, index >= interaction.static_tool_count)
                })
                .collect::<Vec<_>>();
            mark_anthropic_static_tool_prefix(&mut tools, interaction.static_tool_count);
            body["tools"] = Value::Array(tools);
            body["tool_choice"] = json!({
                "type": anthropic_tool_choice_label(interaction.tool_choice),
                "disable_parallel_tool_use": !interaction.parallel_tool_calls,
            });
            if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
                append_anthropic_exchanges(messages, interaction);
            }
        }
    }
}

fn native_tool_choice_label(choice: crate::NativeToolChoice) -> &'static str {
    match choice {
        crate::NativeToolChoice::Auto => "auto",
        crate::NativeToolChoice::Required => "required",
    }
}

fn anthropic_tool_choice_label(choice: crate::NativeToolChoice) -> &'static str {
    match choice {
        crate::NativeToolChoice::Auto => "auto",
        crate::NativeToolChoice::Required => "any",
    }
}

fn openai_chat_tool_definition(tool: &ToolDefinition, include_description: bool) -> Value {
    let mut function = serde_json::Map::from_iter([
        ("name".to_string(), json!(tool.name)),
        ("parameters".to_string(), tool.input_schema.clone()),
    ]);
    if include_description {
        function.insert("description".to_string(), json!(tool.description));
    }
    json!({"type": "function", "function": function})
}

fn openai_responses_tool_definition(tool: &ToolDefinition, include_description: bool) -> Value {
    let mut definition = serde_json::Map::from_iter([
        ("type".to_string(), json!("function")),
        ("name".to_string(), json!(tool.name)),
        ("parameters".to_string(), tool.input_schema.clone()),
    ]);
    if include_description {
        definition.insert("description".to_string(), json!(tool.description));
    }
    Value::Object(definition)
}

fn anthropic_tool_definition(tool: &ToolDefinition, include_description: bool) -> Value {
    let mut definition = serde_json::Map::from_iter([
        ("name".to_string(), json!(tool.name)),
        ("input_schema".to_string(), tool.input_schema.clone()),
    ]);
    if include_description {
        definition.insert("description".to_string(), json!(tool.description));
    }
    Value::Object(definition)
}

fn mark_anthropic_static_tool_prefix(tools: &mut [Value], static_tool_count: usize) {
    let Some(last_static) = static_tool_count
        .checked_sub(1)
        .and_then(|index| tools.get_mut(index))
    else {
        return;
    };
    if let Some(tool) = last_static.as_object_mut() {
        tool.insert("cache_control".to_string(), json!({"type": "ephemeral"}));
    }
}

fn append_openai_chat_exchanges(messages: &mut Vec<Value>, interaction: &ModelInteractionRequest) {
    for exchange in &interaction.native_exchanges {
        messages.push(json!({
            "role": "assistant",
            "content": optional_text(&exchange.assistant_text),
            "tool_calls": exchange.calls.iter().map(|call| json!({
                "id": call.id,
                "type": "function",
                "function": {"name": call.name, "arguments": call.raw_arguments},
            })).collect::<Vec<_>>(),
        }));
        for result in &exchange.results {
            messages.push(json!({
                "role": "tool",
                "tool_call_id": result.call_id,
                "content": result.content,
            }));
        }
    }
}

fn append_openai_responses_exchanges(
    input: &mut Vec<Value>,
    interaction: &ModelInteractionRequest,
) {
    for exchange in &interaction.native_exchanges {
        if !exchange.assistant_text.trim().is_empty() {
            input.push(json!({
                "role": "assistant",
                "content": [{"type": "output_text", "text": exchange.assistant_text}],
            }));
        }
        input.extend(exchange.calls.iter().map(|call| {
            json!({
                "type": "function_call",
                "call_id": call.id,
                "name": call.name,
                "arguments": call.raw_arguments,
            })
        }));
        input.extend(exchange.results.iter().map(|result| {
            json!({
                "type": "function_call_output",
                "call_id": result.call_id,
                "output": result.content,
            })
        }));
    }
}

fn append_anthropic_exchanges(messages: &mut Vec<Value>, interaction: &ModelInteractionRequest) {
    for exchange in &interaction.native_exchanges {
        let mut assistant_content = Vec::new();
        if !exchange.assistant_text.trim().is_empty() {
            assistant_content.push(json!({"type": "text", "text": exchange.assistant_text}));
        }
        assistant_content.extend(exchange.calls.iter().map(|call| {
            json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": call.arguments,
            })
        }));
        messages.push(json!({"role": "assistant", "content": assistant_content}));
        messages.push(json!({
            "role": "user",
            "content": exchange.results.iter().map(|result| json!({
                "type": "tool_result",
                "tool_use_id": result.call_id,
                "content": result.content,
                "is_error": result.is_error,
            })).collect::<Vec<_>>(),
        }));
    }
}

fn optional_text(text: &str) -> Value {
    if text.trim().is_empty() {
        Value::Null
    } else {
        Value::String(text.to_string())
    }
}

fn model_http_headers(config: &ModelServiceConfig) -> Vec<(String, String)> {
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

pub fn model_request_audit_event(
    config: &ModelServiceConfig,
    prepared_request: &PreparedModelRequest,
) -> Value {
    json!({
        "type": "llm_request",
        "model": config.model,
        "api_protocol": config.api_protocol.label(),
        "endpoint": config.endpoint(),
        "prompt_cache_plan": prepared_request.prompt_cache_plan,
        "structured_output": structured_output_label(prepared_request.structured_output),
        "prompt_cache_wire": {
            "mode": prepared_request.cache_wire_mode,
            "mark_count": prepared_request.cache_mark_count,
            "fallback": prepared_request.cache_fallback,
        },
        "body": redact_value(&prepared_request.body),
    })
}

pub fn model_response_audit_event(status: u16, raw_body: &Value) -> Value {
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

pub fn model_prompt_blocks(blocks: &[PromptBlock]) -> Vec<ModelPromptBlock> {
    blocks
        .iter()
        .map(|block| ModelPromptBlock {
            role: match block.role {
                PromptBlockRole::System => ModelPromptRole::System,
                PromptBlockRole::User => ModelPromptRole::User,
            },
            text: block.text.clone(),
            cache: match block.cache {
                CacheControl::None => ModelCacheControl::None,
                CacheControl::Ephemeral => ModelCacheControl::Ephemeral,
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
    config: &ModelServiceConfig,
    blocks: &[ModelPromptBlock],
    structured_output: StructuredOutputHint,
) -> Value {
    let messages = blocks
        .iter()
        .map(|block| {
            let mut message = json!({
                "role": role_label(block.role),
                "content": block.text,
            });
            if config.openai_compatible.cache_mode == OpenAiCompatibleCacheMode::Ephemeral {
                apply_cache_control(&mut message, block.cache);
            }
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
    config: &ModelServiceConfig,
    blocks: &[ModelPromptBlock],
) -> Value {
    let instructions = blocks
        .iter()
        .filter(|block| block.role == ModelPromptRole::System)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let input = blocks
        .iter()
        .filter(|block| block.role == ModelPromptRole::User)
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

fn build_anthropic_request(config: &ModelServiceConfig, blocks: &[ModelPromptBlock]) -> Value {
    let system = blocks
        .iter()
        .filter(|block| block.role == ModelPromptRole::System)
        .map(|block| {
            let mut item = json!({"type":"text", "text": block.text});
            apply_cache_control(&mut item, block.cache);
            item
        })
        .collect::<Vec<_>>();
    let content = blocks
        .iter()
        .filter(|block| block.role == ModelPromptRole::User)
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

fn role_label(role: ModelPromptRole) -> &'static str {
    match role {
        ModelPromptRole::System => "system",
        ModelPromptRole::User => "user",
    }
}

fn apply_cache_control(value: &mut Value, cache: ModelCacheControl) {
    if cache == ModelCacheControl::Ephemeral {
        if let Some(map) = value.as_object_mut() {
            map.insert("cache_control".to_string(), json!({"type":"ephemeral"}));
        }
    }
}

fn cache_wire_mode(config: &ModelServiceConfig) -> &'static str {
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible => config.openai_compatible.cache_mode.label(),
        ApiProtocol::Anthropic => "ephemeral",
        ApiProtocol::OpenAiResponses => "auto",
    }
}

fn count_cache_control_marks(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.iter().map(count_cache_control_marks).sum(),
        Value::Object(values) => {
            usize::from(values.contains_key("cache_control"))
                + values
                    .values()
                    .map(count_cache_control_marks)
                    .sum::<usize>()
        }
        _ => 0,
    }
}

pub fn without_openai_compatible_cache_control(
    request: &PreparedModelHttpRequest,
) -> PreparedModelHttpRequest {
    let mut request = request.clone();
    if let Some(messages) = request
        .model_request
        .body
        .get_mut("messages")
        .and_then(Value::as_array_mut)
    {
        for message in messages {
            if let Some(object) = message.as_object_mut() {
                object.remove("cache_control");
            }
        }
    }
    request.model_request.cache_wire_mode = "auto-fallback".to_string();
    request.model_request.cache_mark_count = count_cache_control_marks(&request.model_request.body);
    request.model_request.cache_fallback = true;
    request
}

fn apply_structured_output(value: &mut Value, structured_output: StructuredOutputHint) {
    if structured_output == StructuredOutputHint::JsonObject {
        if let Some(map) = value.as_object_mut() {
            map.insert("response_format".to_string(), json!({"type":"json_object"}));
        }
    }
}

pub fn parse_model_response(
    config: &ModelServiceConfig,
    raw: &Value,
) -> Result<LlmResponse, String> {
    let tool_calls = parse_native_tool_calls(config.api_protocol, raw)?;
    if tool_calls.len() > config.interaction.max_tool_calls_per_response {
        return Err(format!(
            "too_many_tool_calls: received {}, limit {}",
            tool_calls.len(),
            config.interaction.max_tool_calls_per_response
        ));
    }
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
            let cache_created_tokens = usage
                .pointer("/prompt_tokens_details/cached_creation_tokens")
                .and_then(Value::as_u64)
                .or_else(|| {
                    usage
                        .pointer("/prompt_tokens_details/cache_creation_tokens")
                        .and_then(Value::as_u64)
                })
                .or_else(|| {
                    usage
                        .get("cache_creation_input_tokens")
                        .and_then(Value::as_u64)
                })
                .unwrap_or(0) as u32;
            (
                content,
                UsageStats {
                    llm_calls: 1,
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    cached_tokens,
                    cache_created_tokens,
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
        tool_calls,
        model_name: config.model.clone(),
        usage,
        truncated,
    })
}

pub fn interpret_model_http_response(
    config: &ModelServiceConfig,
    status: u16,
    body_text: &str,
    stderr_text: &str,
) -> ModelHttpResponseInterpretation {
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
        Err(model_http_error_message(status, &raw_json))
    } else if !parsed_json {
        Ok(LlmResponse {
            tool_calls: Vec::new(),
            content: body_text.to_string(),
            model_name: config.model.clone(),
            usage: UsageStats::zero(),
            truncated: false,
        })
    } else {
        parse_model_response(config, &raw_json)
    };
    ModelHttpResponseInterpretation {
        status,
        raw_json,
        result,
    }
}

fn parse_native_tool_calls(
    api_protocol: ApiProtocol,
    raw: &Value,
) -> Result<Vec<NativeToolCall>, String> {
    match api_protocol {
        ApiProtocol::OpenAiCompatible => raw
            .pointer("/choices/0/message/tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .enumerate()
                    .map(|(index, call)| {
                        parse_string_arguments_tool_call(
                            call.get("id").and_then(Value::as_str),
                            call.pointer("/function/name").and_then(Value::as_str),
                            call.pointer("/function/arguments"),
                            index,
                        )
                    })
                    .collect()
            })
            .transpose()
            .map(Option::unwrap_or_default),
        ApiProtocol::OpenAiResponses => raw
            .get("output")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get("type").and_then(Value::as_str) == Some("function_call")
                    })
                    .enumerate()
                    .map(|(index, call)| {
                        parse_string_arguments_tool_call(
                            call.get("call_id")
                                .and_then(Value::as_str)
                                .or_else(|| call.get("id").and_then(Value::as_str)),
                            call.get("name").and_then(Value::as_str),
                            call.get("arguments"),
                            index,
                        )
                    })
                    .collect()
            })
            .transpose()
            .map(Option::unwrap_or_default),
        ApiProtocol::Anthropic => raw
            .get("content")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
                    .enumerate()
                    .map(|(index, call)| {
                        let id = required_tool_call_field(
                            call.get("id").and_then(Value::as_str),
                            "id",
                            index,
                        )?;
                        let name = required_tool_call_field(
                            call.get("name").and_then(Value::as_str),
                            "name",
                            index,
                        )?;
                        let arguments = call.get("input").cloned().unwrap_or_else(|| json!({}));
                        if !arguments.is_object() {
                            return Err(format!("invalid_tool_call[{index}].input_must_be_object"));
                        }
                        Ok(NativeToolCall {
                            id,
                            name,
                            raw_arguments: serde_json::to_string(&arguments)
                                .unwrap_or_else(|_| "{}".to_string()),
                            arguments,
                        })
                    })
                    .collect()
            })
            .transpose()
            .map(Option::unwrap_or_default),
    }
}

fn parse_string_arguments_tool_call(
    id: Option<&str>,
    name: Option<&str>,
    raw_arguments: Option<&Value>,
    index: usize,
) -> Result<NativeToolCall, String> {
    let id = required_tool_call_field(id, "id", index)?;
    let name = required_tool_call_field(name, "name", index)?;
    let raw_arguments = match raw_arguments {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Object(value)) => Value::Object(value.clone()).to_string(),
        None | Some(Value::Null) => "{}".to_string(),
        _ => return Err(format!("invalid_tool_call[{index}].arguments_must_be_json")),
    };
    let arguments: Value = serde_json::from_str(&raw_arguments)
        .map_err(|error| format!("invalid_tool_call[{index}].arguments_json:{error}"))?;
    if !arguments.is_object() {
        return Err(format!(
            "invalid_tool_call[{index}].arguments_must_be_object"
        ));
    }
    Ok(NativeToolCall {
        id,
        name,
        arguments,
        raw_arguments,
    })
}

fn required_tool_call_field(
    value: Option<&str>,
    field: &str,
    index: usize,
) -> Result<String, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("invalid_tool_call[{index}].{field}_required"))
}

fn looks_like_sse(body: &str) -> bool {
    body.lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty() && !line.starts_with(':'))
        .is_some_and(|line| line.starts_with("data:") || line.starts_with("event:"))
}

fn interpret_openai_compatible_sse(
    config: &ModelServiceConfig,
    status: u16,
    body_text: &str,
) -> ModelHttpResponseInterpretation {
    let mut content = String::new();
    let mut finish_reason = String::new();
    let mut usage = Value::Null;
    let mut event_count = 0_u64;
    let mut reasoning_chunk_count = 0_u64;
    let mut parse_error = None;
    let mut streamed_calls: Vec<(String, String, String)> = Vec::new();

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
                parse_error = Some(format!("invalid_model_sse_event: {error}"));
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
        if let Some(chunks) = event
            .pointer("/choices/0/delta/tool_calls")
            .and_then(Value::as_array)
        {
            for chunk in chunks {
                let index = chunk
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or(streamed_calls.len());
                if streamed_calls.len() <= index {
                    streamed_calls
                        .resize_with(index + 1, || (String::new(), String::new(), String::new()));
                }
                let (id, name, arguments) = &mut streamed_calls[index];
                if let Some(value) = chunk.get("id").and_then(Value::as_str) {
                    id.push_str(value);
                }
                if let Some(value) = chunk.pointer("/function/name").and_then(Value::as_str) {
                    name.push_str(value);
                }
                if let Some(value) = chunk.pointer("/function/arguments").and_then(Value::as_str) {
                    arguments.push_str(value);
                }
            }
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

    let tool_calls = streamed_calls
        .into_iter()
        .map(|(id, name, arguments)| {
            json!({
                "id": id,
                "type": "function",
                "function": {"name": name, "arguments": arguments},
            })
        })
        .collect::<Vec<_>>();
    let raw_json = json!({
        "stream": true,
        "stream_metadata": {
            "event_count": event_count,
            "reasoning_chunk_count": reasoning_chunk_count,
        },
        "choices": [{
            "message": {"content": content, "tool_calls": tool_calls},
            "finish_reason": finish_reason,
        }],
        "usage": usage,
    });
    let result = match parse_error {
        Some(error) => Err(error),
        None if event_count == 0 => Err("empty_model_sse_response".to_string()),
        None => parse_model_response(config, &raw_json),
    };
    ModelHttpResponseInterpretation {
        status,
        raw_json,
        result,
    }
}

pub fn model_http_error_message(status: u16, body: &Value) -> String {
    let reason = model_error_reason(body)
        .map(sanitize_model_error_reason)
        .filter(|text| !text.trim().is_empty());
    if status == 0 {
        return match reason {
            Some(reason) if reason.to_lowercase().contains("timed out") => {
                format!("model_timeout: {reason}")
            }
            Some(reason) => format!("model_network_error: {reason}"),
            None => "model_network_error".to_string(),
        };
    }
    match reason {
        Some(reason) => format!("model_http_{status}: {reason}"),
        None => format!("model_http_{status}"),
    }
}

fn model_error_reason(body: &Value) -> Option<String> {
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

fn sanitize_model_error_reason(reason: String) -> String {
    let single_line = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_secret_like_text(&single_line);
    compact_model_error_text(&redacted, 240)
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

fn compact_model_error_text(text: &str, max_chars: usize) -> String {
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
#[path = "../tests/unit/model_api_tests.rs"]
mod tests;
