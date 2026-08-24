use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const DEFAULT_MAX_TOOL_CALLS_PER_RESPONSE: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallMode {
    #[default]
    Auto,
    Native,
    Inline,
}

impl ToolCallMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Native => "native",
            Self::Inline => "inline",
        }
    }
}

impl fmt::Display for ToolCallMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

pub fn parse_tool_call_mode(value: &str) -> Result<ToolCallMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(ToolCallMode::Auto),
        "native" => Ok(ToolCallMode::Native),
        "inline" => Ok(ToolCallMode::Inline),
        _ => Err("invalid_TIMEM_TOOL_CALL_MODE: expected auto, native, or inline".to_string()),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelToolCalls {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

impl ParallelToolCalls {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

pub fn parse_parallel_tool_calls(value: &str) -> Result<ParallelToolCalls, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(ParallelToolCalls::Auto),
        "true" | "1" | "yes" | "on" | "enabled" => Ok(ParallelToolCalls::Enabled),
        "false" | "0" | "no" | "off" | "disabled" => Ok(ParallelToolCalls::Disabled),
        _ => Err("invalid_TIMEM_PARALLEL_TOOL_CALLS: expected auto, true, or false".to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionConfig {
    pub tool_call_mode: ToolCallMode,
    pub parallel_tool_calls: ParallelToolCalls,
    pub max_tool_calls_per_response: usize,
}

impl Default for InteractionConfig {
    fn default() -> Self {
        Self {
            // Direct programmatic construction keeps the historical inline
            // behavior. Host configuration explicitly selects Auto so test
            // doubles and embedders never trigger unexpected network probes.
            tool_call_mode: ToolCallMode::Inline,
            parallel_tool_calls: ParallelToolCalls::Auto,
            max_tool_calls_per_response: DEFAULT_MAX_TOOL_CALLS_PER_RESPONSE,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    /// The provider's exact argument representation. Retaining it makes replay
    /// lossless even when a compatible gateway accepts non-canonical JSON.
    pub raw_arguments: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeToolResult {
    pub call_id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeExchange {
    /// Prompt delta that was open when this provider-native exchange was created.
    /// The exchange is projected immediately after that delta in model input order.
    pub delta_id: String,
    pub assistant_text: String,
    pub calls: Vec<NativeToolCall>,
    pub results: Vec<NativeToolResult>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelInteractionRequest {
    pub rendered_prompt: String,
    /// Number of leading tool definitions that are stable built-in
    /// capabilities. Any request-scoped tools follow this cacheable prefix.
    pub static_tool_count: usize,
    pub tools: Vec<ToolDefinition>,
    pub native_exchanges: Vec<NativeExchange>,
    pub resolved_mode: ToolCallMode,
    pub parallel_tool_calls: bool,
    pub tool_choice: NativeToolChoice,
}

impl ModelInteractionRequest {
    pub fn inline(rendered_prompt: impl Into<String>) -> Self {
        Self {
            rendered_prompt: rendered_prompt.into(),
            static_tool_count: 0,
            tools: Vec::new(),
            native_exchanges: Vec::new(),
            resolved_mode: ToolCallMode::Inline,
            parallel_tool_calls: false,
            tool_choice: NativeToolChoice::Auto,
        }
    }

    pub fn is_native(&self) -> bool {
        self.resolved_mode == ToolCallMode::Native
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NativeToolChoice {
    #[default]
    Auto,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProbeSource {
    Explicit,
    Probe,
    Cache,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionProfile {
    pub api_protocol: String,
    pub model: String,
    pub gateway: String,
    pub requested_mode: ToolCallMode,
    pub resolved_mode: ToolCallMode,
    #[serde(default)]
    pub active_prompt_protocol: String,
    pub parallel_supported: bool,
    pub parallel_enabled: bool,
    pub source: CapabilityProbeSource,
    pub reason: String,
    pub probe_latency_ms: Option<u64>,
    pub observed_tool_calls: usize,
}
