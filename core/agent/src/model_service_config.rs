use crate::{
    default_api_protocol, default_base_url, default_model, parse_api_protocol,
    parse_openai_compatible_cache_mode, parse_token_count, validate_model_http_headers,
    validate_model_request_fields, ApiProtocol, ModelHttpTransportOptions, ModelServiceConfig,
    OpenAiCompatibleCacheMode, OpenAiCompatibleOptions, ParallelToolCalls, ToolCallMode,
};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelServiceConfigSource {
    pub api_protocol: Option<String>,
    pub api_key: Option<String>,
    pub http_headers: Option<std::collections::BTreeMap<String, String>>,
    pub request_fields: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub timeout_secs: Option<u64>,
    pub max_llm_output_tokens: Option<u32>,
    pub max_llm_input_tokens: Option<u32>,
    pub local_api_key: Option<String>,
    pub enable_thinking: Option<bool>,
    pub reasoning_effort: Option<String>,
    pub stream: Option<bool>,
    pub openai_cache_mode: Option<String>,
    pub tool_call_mode: Option<ToolCallMode>,
    pub parallel_tool_calls: Option<ParallelToolCalls>,
    pub allow_cross_origin_redirects: Option<bool>,
    pub private_ca_pem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalLLMKeyFile {
    pub api_key: String,
    pub available_models: Vec<String>,
}

impl LocalLLMKeyFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut section = "";
        let mut api_key = String::new();
        let mut available_models = Vec::new();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.eq_ignore_ascii_case("key:") {
                section = "key";
                continue;
            }
            if line.eq_ignore_ascii_case("available_model:")
                || line.eq_ignore_ascii_case("available_models:")
            {
                section = "available_model";
                continue;
            }
            match section {
                "key" if api_key.is_empty() => api_key = line.to_string(),
                "available_model" => available_models.push(line.to_string()),
                _ => {}
            }
        }

        validate_api_key(&api_key)?;
        if available_models.is_empty() {
            return Err("missing_available_model".to_string());
        }
        Ok(Self {
            api_key,
            available_models,
        })
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&text)
    }

    pub fn random_model(&self) -> &str {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as usize)
            .unwrap_or(0);
        let pid = std::process::id() as usize;
        let index = (now ^ pid) % self.available_models.len();
        &self.available_models[index]
    }

    pub fn to_model_service_config(&self, model: &str) -> ModelServiceConfig {
        ModelServiceConfig {
            model: model.to_string(),
            base_url: default_base_url(&ApiProtocol::OpenAiCompatible).to_string(),
            api_key: self.api_key.clone(),
            http_headers: Default::default(),
            request_fields: Default::default(),
            timeout_secs: 120,
            max_llm_output_tokens: 512,
            max_llm_input_tokens: 100_000,
            api_protocol: ApiProtocol::OpenAiCompatible,
            response_protocol: crate::ResponseProtocolKind::default(),
            interaction: crate::InteractionConfig {
                tool_call_mode: ToolCallMode::Auto,
                ..crate::InteractionConfig::default()
            },
            openai_compatible: OpenAiCompatibleOptions::default(),
            http_transport: ModelHttpTransportOptions::default(),
        }
    }
}

pub fn model_service_config_from_sources(
    source: &ModelServiceConfigSource,
    env: &HashMap<String, String>,
) -> Result<ModelServiceConfig, String> {
    model_service_config_from_sources_with_key_policy(source, env, true)
}

pub fn model_service_config_from_sources_allow_missing_api_key(
    source: &ModelServiceConfigSource,
    env: &HashMap<String, String>,
) -> Result<ModelServiceConfig, String> {
    model_service_config_from_sources_with_key_policy(source, env, false)
}

fn model_service_config_from_sources_with_key_policy(
    source: &ModelServiceConfigSource,
    env: &HashMap<String, String>,
    require_api_key: bool,
) -> Result<ModelServiceConfig, String> {
    let api_protocol = source
        .api_protocol
        .clone()
        .or_else(|| env.get("TIMEM_API_PROTOCOL").cloned())
        .map(|value| parse_api_protocol(&value))
        .transpose()?
        .unwrap_or_else(default_api_protocol);
    let model = source
        .model
        .clone()
        .or_else(|| env.get("TIMEM_MODEL").cloned())
        .unwrap_or_else(|| default_model().to_string());
    let base_url = source
        .base_url
        .clone()
        .or_else(|| env.get("TIMEM_BASE_URL").cloned())
        .or_else(|| protocol_base_url(&api_protocol, env))
        .unwrap_or_else(|| default_base_url(&api_protocol).to_string());
    let api_key = source
        .api_key
        .clone()
        .or_else(|| env.get("TIMEM_API_KEY").cloned())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| protocol_api_key(&api_protocol, env))
        .or_else(|| source.local_api_key.clone())
        .unwrap_or_default();
    if api_key.is_empty() {
        if require_api_key {
            return Err(
                "missing_api_key: set TIMEM_API_KEY or configure the Session API key".to_string(),
            );
        }
    } else {
        validate_api_key(&api_key)?;
    }
    let http_headers = match source.http_headers.clone() {
        Some(headers) => headers,
        None => env
            .get("TIMEM_HTTP_HEADERS")
            .map(|value| {
                serde_json::from_str(value)
                    .map_err(|error| format!("invalid_TIMEM_HTTP_HEADERS:{error}"))
            })
            .transpose()?
            .unwrap_or_default(),
    };
    validate_model_http_headers(&http_headers)?;
    let request_fields = match source.request_fields.clone() {
        Some(fields) => fields,
        None => env
            .get("TIMEM_REQUEST_FIELDS")
            .map(|value| {
                serde_json::from_str(value)
                    .map_err(|error| format!("invalid_TIMEM_REQUEST_FIELDS:{error}"))
            })
            .transpose()?
            .unwrap_or_default(),
    };
    validate_model_request_fields(&request_fields)?;
    let timeout_secs = source
        .timeout_secs
        .or_else(|| env.get("TIMEM_TIMEOUT").and_then(|v| v.parse().ok()))
        .unwrap_or(120);
    let max_llm_output_tokens = source
        .max_llm_output_tokens
        .or_else(|| {
            env.get("TIMEM_MAX_LLM_OUTPUT")
                .and_then(|value| parse_token_count(value))
        })
        .unwrap_or(20_000);
    let max_llm_input_tokens = source
        .max_llm_input_tokens
        .or_else(|| {
            env.get("TIMEM_MAX_LLM_INPUT")
                .and_then(|value| parse_token_count(value))
        })
        .unwrap_or(100_000);
    let enable_thinking = match source.enable_thinking {
        Some(value) => Some(value),
        None => env
            .get("TIMEM_ENABLE_THINKING")
            .map(|value| parse_bool_env("TIMEM_ENABLE_THINKING", value))
            .transpose()?,
    };
    let reasoning_effort = source
        .reasoning_effort
        .clone()
        .or_else(|| env.get("TIMEM_REASONING_EFFORT").cloned())
        .map(|value| validate_reasoning_effort(&value))
        .transpose()?;
    let stream = match source.stream {
        Some(value) => value,
        None => match env.get("TIMEM_STREAM") {
            Some(value) => parse_bool_env("TIMEM_STREAM", value)?,
            None => false,
        },
    };
    let cache_mode = source
        .openai_cache_mode
        .clone()
        .or_else(|| env.get("TIMEM_OPENAI_CACHE_MODE").cloned())
        .map(|value| parse_openai_compatible_cache_mode(&value))
        .transpose()?
        .unwrap_or(OpenAiCompatibleCacheMode::Auto);
    let tool_call_mode = match source.tool_call_mode {
        Some(mode) => mode,
        None => env
            .get("TIMEM_TOOL_CALL_MODE")
            .map(|value| crate::parse_tool_call_mode(value))
            .transpose()?
            .unwrap_or(ToolCallMode::Auto),
    };
    let parallel_tool_calls = match source.parallel_tool_calls {
        Some(mode) => mode,
        None => env
            .get("TIMEM_PARALLEL_TOOL_CALLS")
            .map(|value| crate::parse_parallel_tool_calls(value))
            .transpose()?
            .unwrap_or_default(),
    };
    Ok(ModelServiceConfig {
        model,
        base_url,
        api_key,
        http_headers,
        request_fields,
        timeout_secs,
        max_llm_output_tokens,
        max_llm_input_tokens,
        api_protocol,
        response_protocol: crate::ResponseProtocolKind::default(),
        interaction: crate::InteractionConfig {
            tool_call_mode,
            parallel_tool_calls,
            ..crate::InteractionConfig::default()
        },
        openai_compatible: OpenAiCompatibleOptions {
            enable_thinking,
            reasoning_effort,
            stream,
            cache_mode,
        },
        http_transport: ModelHttpTransportOptions {
            allow_cross_origin_redirects: match source.allow_cross_origin_redirects {
                Some(value) => value,
                None => env
                    .get("TIMEM_ALLOW_CROSS_ORIGIN_REDIRECTS")
                    .map(|value| parse_bool_env("TIMEM_ALLOW_CROSS_ORIGIN_REDIRECTS", value))
                    .transpose()?
                    .unwrap_or(false),
            },
            private_ca_pem: source
                .private_ca_pem
                .clone()
                .or_else(|| env.get("TIMEM_PRIVATE_CA_PEM").cloned())
                .filter(|value| !value.trim().is_empty()),
        },
    })
}

fn parse_bool_env(key: &str, value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!("invalid_{key}: expected true or false")),
    }
}

fn validate_reasoning_effort(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 32
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("invalid_TIMEM_REASONING_EFFORT".to_string());
    }
    Ok(value.to_string())
}

pub fn apply_openai_compatible_env_value(
    options: &mut OpenAiCompatibleOptions,
    key: &str,
    value: &str,
) -> Result<bool, String> {
    match key {
        "TIMEM_ENABLE_THINKING" => {
            options.enable_thinking = Some(parse_bool_env(key, value)?);
        }
        "TIMEM_REASONING_EFFORT" => {
            options.reasoning_effort = Some(validate_reasoning_effort(value)?);
        }
        "TIMEM_STREAM" => {
            options.stream = parse_bool_env(key, value)?;
        }
        "TIMEM_OPENAI_CACHE_MODE" => {
            options.cache_mode = parse_openai_compatible_cache_mode(value)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn protocol_api_key(api_protocol: &ApiProtocol, env: &HashMap<String, String>) -> Option<String> {
    let key = match api_protocol {
        ApiProtocol::Anthropic => env
            .get("ANTHROPIC_API_KEY")
            .cloned()
            .or_else(|| env.get("ANTHROPIC_AUTH_TOKEN").cloned()),
        ApiProtocol::OpenAiResponses => env.get("OPENAI_API_KEY").cloned(),
        ApiProtocol::OpenAiCompatible => env
            .get("DASHSCOPE_API_KEY")
            .cloned()
            .or_else(|| env.get("OPENAI_API_KEY").cloned()),
    };
    key.filter(|value| !value.trim().is_empty())
}

fn protocol_base_url(api_protocol: &ApiProtocol, env: &HashMap<String, String>) -> Option<String> {
    match api_protocol {
        ApiProtocol::Anthropic => env.get("ANTHROPIC_BASE_URL").cloned(),
        ApiProtocol::OpenAiResponses => env.get("OPENAI_BASE_URL").cloned(),
        ApiProtocol::OpenAiCompatible => env
            .get("DASHSCOPE_BASE_URL")
            .cloned()
            .or_else(|| env.get("OPENAI_BASE_URL").cloned()),
    }
}

pub fn validate_api_key(api_key: &str) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err("missing_api_key".to_string());
    }
    if !api_key.is_ascii() {
        return Err("invalid_api_key_non_ascii".to_string());
    }
    if api_key
        .chars()
        .any(|character| character.is_ascii_control() || character.is_ascii_whitespace())
    {
        return Err("invalid_api_key_control_or_whitespace".to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/model_service_config_tests.rs"]
mod tests;
