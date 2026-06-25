use agent_core::{CoreProfile, LlmResponse, UsageStats};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

mod observation;
mod profiler;
mod prompt_cache;
mod protocol_adapter;
mod structured_output;

pub use observation::{
    observation_events_from_model_response, render_observation_panel, render_observation_panel_at,
    ObservationEvent, ObservationLine, ObservationLineStyle, ObservationPanel,
};
pub use profiler::{collect_storage_profile, render_prof_report, RuntimeProfiler, StorageProfile};
pub use prompt_cache::{
    plan_incremental_cache, plan_prompt_cache, prompt_parts_from_rendered_prompt,
    split_old_and_new_delta, split_prompt, stable_text_fingerprint, CacheControl, PromptBlock,
    PromptBlockRole, PromptParts,
};
pub use structured_output::{plan_structured_output, StructuredOutputHint};

pub const TIMEM_LOGO: &str = "𝓣𝓲𝓶𝓮𝓶";
pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_BRIGHT_TIMEM: &str = "\x1b[92;1m";
pub const ANSI_DIM: &str = "\x1b[2m";
pub const ANSI_BOLD: &str = "\x1b[1m";
pub const SPINNER_ICONS: [&str; 27] = [
    "🦩", "🐧", "🦅", "🦆", "🦢", "🦉", "🦄", "🦖", "🐉", "🐌", "🦏", "🦛", "🐫", "🦙", "🦑", "🦞",
    "🦐", "🦁", "🐮", "🐷", "🐸", "🐒", "🐭", "🐹", "🐰", "🦊", "🦝",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelDirection {
    Upstream,
    Downstream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellStatusSnapshot {
    pub provider: String,
    pub model: String,
    pub intent: String,
    pub memory_marker: String,
    pub model_round: u32,
    pub direction: ModelDirection,
    pub usage: UsageStats,
    pub tick: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingViewSnapshot {
    pub status: ShellStatusSnapshot,
    pub observations: ObservationPanel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellStatusTone {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellStatusMessage {
    pub tone: ShellStatusTone,
    pub text: String,
}

fn timem_prefix(time_label: &str) -> String {
    format!("{ANSI_BRIGHT_TIMEM}[{time_label}] {TIMEM_LOGO}  ⬇{ANSI_RESET}")
}

fn dim_line(text: &str) -> String {
    format!("{ANSI_RESET}{ANSI_DIM}{text}{ANSI_RESET}")
}

pub fn render_shell_status_bar(message: &ShellStatusMessage) -> String {
    let icon = match message.tone {
        ShellStatusTone::Info => "ⓘ",
        ShellStatusTone::Warning => "!",
    };
    dim_line(&format!(" {icon} {}", message.text.trim()))
}

pub fn render_thinking_block_at(snapshot: &ShellStatusSnapshot, time_label: &str) -> String {
    let icon = SPINNER_ICONS[(snapshot.tick / 4) % SPINNER_ICONS.len()];
    let direction = match snapshot.direction {
        ModelDirection::Upstream => "▲",
        ModelDirection::Downstream => "▼",
    };
    let mut status_parts = Vec::new();
    if !snapshot.memory_marker.trim().is_empty() {
        status_parts.push(snapshot.memory_marker.clone());
    }
    status_parts.push(format!(
        "{}:{}: {}{}",
        snapshot.provider, snapshot.model, direction, snapshot.model_round
    ));
    status_parts.push(token_status(&snapshot.usage));
    let intent = compact_status_text(&snapshot.intent, 36);
    let intent_line = dim_line(&format!("{icon} {intent}..."));
    let status_line = dim_line(&format!("  {}", status_parts.join(" ║ ")));
    format!(
        "{}\n{intent_line}\n{status_line}\n",
        timem_prefix(time_label)
    )
}

pub fn render_thinking_view_at(snapshot: &ThinkingViewSnapshot, time_label: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("{}\n", timem_prefix(time_label)));
    out.push_str(&render_observation_panel_at(
        &snapshot.observations,
        snapshot.status.tick,
    ));
    out.push_str(&render_thinking_status_line(&snapshot.status));
    out.push('\n');
    out
}

fn render_thinking_status_line(snapshot: &ShellStatusSnapshot) -> String {
    let direction = match snapshot.direction {
        ModelDirection::Upstream => "▲",
        ModelDirection::Downstream => "▼",
    };
    let mut status_parts = Vec::new();
    if !snapshot.memory_marker.trim().is_empty() {
        status_parts.push(snapshot.memory_marker.clone());
    }
    status_parts.push(format!(
        "{}:{}: {}{}",
        snapshot.provider, snapshot.model, direction, snapshot.model_round
    ));
    status_parts.push(token_status(&snapshot.usage));
    dim_line(&format!("  {}", status_parts.join(" ║ ")))
}

pub fn compact_status_text(text: &str, max_chars: usize) -> String {
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

pub fn render_final_response_at(
    text: &str,
    stats: &UsageStats,
    provider: &str,
    model: &str,
    elapsed_secs: u64,
    time_label: &str,
) -> String {
    let mut status_parts = Vec::new();
    let memory = memory_marker(stats);
    if !memory.is_empty() {
        status_parts.push(memory.to_string());
    }
    status_parts.push(format!("{}:{}: {}", provider, model, stats.llm_calls));
    status_parts.push(token_status(stats));
    let status_line = dim_line(&format!(
        " ↳ {elapsed_secs}s    ( {} )",
        status_parts.join(" ║ ")
    ));
    let body = render_terminal_markdown(text);
    format!("{}\n{body}\n{status_line}\n\n", timem_prefix(time_label))
}

pub fn render_terminal_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    let mut bold = false;
    while let Some(idx) = rest.find("**") {
        out.push_str(&rest[..idx]);
        if bold {
            out.push_str(ANSI_RESET);
            bold = false;
        } else {
            out.push_str(ANSI_BOLD);
            bold = true;
        }
        rest = &rest[idx + 2..];
    }
    out.push_str(rest);
    if bold {
        out.push_str(ANSI_RESET);
    }
    out
}

pub fn token_status(stats: &UsageStats) -> String {
    let mut input = format!("▲{}", compact_count(stats.prompt_tokens));
    let mut input_annotations = Vec::new();
    if stats.cached_tokens > 0 {
        input_annotations.push(format!("⌁{}", compact_count(stats.cached_tokens)));
    }
    if stats.shrunk_tokens > 0 {
        input_annotations.push(format!("⇃{}", compact_count(stats.shrunk_tokens)));
    }
    if !input_annotations.is_empty() {
        input.push_str(&format!("({})", input_annotations.join(" , ")));
    }
    format!(
        "Token: {} ▼{}",
        input,
        compact_count(stats.completion_tokens)
    )
}

pub fn compact_count(value: u32) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    if value < 1_000_000 {
        return trim_decimal(format!("{:.1}", value as f64 / 1_000.0)) + "K";
    }
    trim_decimal(format!("{:.2}", value as f64 / 1_000_000.0)) + "M"
}

fn trim_decimal(mut text: String) -> String {
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

pub fn memory_marker(stats: &UsageStats) -> &'static str {
    match (stats.mem_reads > 0, stats.mem_writes > 0) {
        (true, true) => "◂▸⛃",
        (true, false) => "◂⛃",
        (false, true) => "▸⛃",
        (false, false) => "",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionStatusHint {
    pub intent: String,
    pub memory_marker: String,
}

pub fn action_status_hint(content: &str) -> Option<ActionStatusHint> {
    let value: Value = serde_json::from_str(content.trim()).ok()?;
    let first = value.get("next_actions")?.as_array()?.first()?;
    let action = first.get("action").and_then(Value::as_str).unwrap_or("");
    let intent = first
        .get("intent")
        .and_then(Value::as_str)
        .or_else(|| {
            first
                .get("input")
                .and_then(|input| input.get("intent"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    match action {
        "chat_history_query" => Some(ActionStatusHint {
            intent: intent.unwrap_or_else(|| "查询聊天记录".to_string()),
            memory_marker: String::new(),
        }),
        "query_memory" | "memory_query" | "sql_read" | "memory_sql_query" | "memory_schema" => {
            Some(ActionStatusHint {
                intent: intent.unwrap_or_else(|| "查询记忆".to_string()),
                memory_marker: "◂⛃".to_string(),
            })
        }
        "memory_write" | "write_memory" | "memory_update" => Some(ActionStatusHint {
            intent: intent.unwrap_or_else(|| "写入记忆".to_string()),
            memory_marker: "▸⛃".to_string(),
        }),
        "run_bash" => Some(ActionStatusHint {
            intent: intent.unwrap_or_else(|| "检查本地文件".to_string()),
            memory_marker: String::new(),
        }),
        "shell_job_status" => Some(ActionStatusHint {
            intent: intent.unwrap_or_else(|| "检查后台任务".to_string()),
            memory_marker: String::new(),
        }),
        _ => None,
    }
}

pub fn supporting_context(provider: &str, model: &str, _user_input: &str) -> String {
    format!(
        "provider: {provider}, model: {model}\nruntime: timem_native_shell\nrun_bash_target: user_local_machine\nruntime_time: {}",
        runtime_time_context()
    )
}

pub fn local_time_label() -> String {
    local_time_parts()
        .map(|parts| format!("{:02}:{:02}:{:02}", parts.hour, parts.minute, parts.second))
        .unwrap_or_else(|| "00:00:00".to_string())
}

pub fn runtime_time_context() -> String {
    local_time_parts()
        .map(|parts| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} local_time, weekday={}/{}",
                parts.year,
                parts.month,
                parts.day,
                parts.hour,
                parts.minute,
                parts.second,
                weekday_zh(parts.weekday),
                weekday_en(parts.weekday)
            )
        })
        .unwrap_or_else(|| "local_time_unavailable".to_string())
}

#[derive(Debug, Clone, Copy)]
struct LocalTimeParts {
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    weekday: i32,
}

fn local_time_parts() -> Option<LocalTimeParts> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as libc::time_t;
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let ptr = unsafe { libc::localtime_r(&secs, tm.as_mut_ptr()) };
    if ptr.is_null() {
        return None;
    }
    let tm = unsafe { tm.assume_init() };
    Some(LocalTimeParts {
        year: tm.tm_year + 1900,
        month: tm.tm_mon + 1,
        day: tm.tm_mday,
        hour: tm.tm_hour,
        minute: tm.tm_min,
        second: tm.tm_sec,
        weekday: tm.tm_wday,
    })
}

fn weekday_zh(weekday: i32) -> &'static str {
    match weekday {
        0 => "周日",
        1 => "周一",
        2 => "周二",
        3 => "周三",
        4 => "周四",
        5 => "周五",
        6 => "周六",
        _ => "未知",
    }
}

fn weekday_en(weekday: i32) -> &'static str {
    match weekday {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "Unknown",
    }
}

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
                // Anthropic 原生 endpoint 为 /v1/messages。
                // 对 api.anthropic.com，base_url 通常到 https://api.anthropic.com，
                // 拼接后得到 https://api.anthropic.com/v1/messages。
                // Custom gateways may already expose a /v1 base path; do not
                // append /v1 again in that case.
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

#[derive(Debug, Clone, Default)]
pub struct CliOptions {
    pub space: Option<String>,
    pub provider: Option<String>,
    pub api_protocol: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub data_dir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub max_llm_output_tokens: Option<u32>,
    pub max_llm_input_tokens: Option<u32>,
    pub once_json_input: Option<String>,
    pub supporting_context: Option<String>,
    pub bash_approval: Option<String>,
}

pub fn parse_cli_args(args: &[String]) -> CliOptions {
    let mut options = CliOptions::default();
    let mut idx = 0;
    while idx < args.len() {
        let key = args[idx].as_str();
        let value = args.get(idx + 1).cloned();
        match (key, value) {
            ("--space", Some(v)) => {
                options.space = Some(v);
                idx += 2;
            }
            ("--gateway-provider", Some(v)) => {
                options.provider = Some(v);
                idx += 2;
            }
            ("--api-protocol", Some(v)) => {
                options.api_protocol = Some(v);
                idx += 2;
            }
            ("--api-key", Some(v)) => {
                options.api_key = Some(v);
                idx += 2;
            }
            ("--model", Some(v)) => {
                options.model = Some(v);
                idx += 2;
            }
            ("--base-url", Some(v)) => {
                options.base_url = Some(v);
                idx += 2;
            }
            ("--data-dir", Some(v)) => {
                options.data_dir = Some(v);
                idx += 2;
            }
            ("--timeout", Some(v)) => {
                options.timeout_secs = v.parse().ok();
                idx += 2;
            }
            ("--max-llm-output", Some(v)) => {
                options.max_llm_output_tokens = parse_token_count(&v);
                idx += 2;
            }
            ("--max-llm-input", Some(v)) => {
                options.max_llm_input_tokens = parse_token_count(&v);
                idx += 2;
            }
            ("--once-json", Some(v)) => {
                options.once_json_input = Some(v);
                idx += 2;
            }
            ("--supporting-context", Some(v)) => {
                options.supporting_context = Some(v);
                idx += 2;
            }
            ("--bash-approval", Some(v)) => {
                options.bash_approval = Some(v);
                idx += 2;
            }
            _ => {
                idx += 1;
            }
        }
    }
    options
}

pub fn provider_config_from_env(
    options: &CliOptions,
    env: &HashMap<String, String>,
) -> Result<ProviderConfig, String> {
    let provider = options
        .provider
        .clone()
        .or_else(|| env.get("TIMEM_GATEWAY_PROVIDER").cloned())
        .unwrap_or_else(|| "aliyun".to_string())
        .to_lowercase();
    let api_protocol = options
        .api_protocol
        .clone()
        .or_else(|| env.get("TIMEM_API_PROTOCOL").cloned())
        .map(|value| parse_api_protocol(&value))
        .transpose()?
        .unwrap_or_else(|| default_api_protocol(&provider));
    let model = options
        .model
        .clone()
        .or_else(|| env.get("TIMEM_MODEL").cloned())
        .unwrap_or_else(|| default_model(&provider).to_string());
    let base_url = options
        .base_url
        .clone()
        .or_else(|| env.get("TIMEM_BASE_URL").cloned())
        .or_else(|| vendor_base_url(&provider, env))
        .unwrap_or_else(|| default_base_url(&provider).to_string());
    let local_key_file = if provider == "aliyun" || provider == "dashscope" {
        LocalLLMKeyFile::load(&local_llm_key_file_path()).ok()
    } else {
        None
    };
    let api_key = options
        .api_key
        .clone()
        .or_else(|| env.get("TIMEM_API_KEY").cloned())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| vendor_api_key(&provider, env))
        .or_else(|| local_key_file.as_ref().map(|file| file.api_key.clone()))
        .ok_or_else(|| {
            format!(
                "missing_api_key: set TIMEM_API_KEY, {}, or rust/key",
                vendor_key_hint(&provider)
            )
        })?;
    validate_api_key(&api_key)?;
    let timeout_secs = options
        .timeout_secs
        .or_else(|| env.get("TIMEM_TIMEOUT").and_then(|v| v.parse().ok()))
        .unwrap_or(120);
    let max_llm_output_tokens = options
        .max_llm_output_tokens
        .or_else(|| {
            env.get("TIMEM_MAX_LLM_OUTPUT")
                .and_then(|value| parse_token_count(value))
        })
        .unwrap_or(10_000);
    let max_llm_input_tokens = options
        .max_llm_input_tokens
        .or_else(|| {
            env.get("TIMEM_MAX_LLM_INPUT")
                .and_then(|value| parse_token_count(value))
        })
        .unwrap_or(100_000);
    Ok(ProviderConfig {
        provider,
        model,
        base_url,
        api_key,
        timeout_secs,
        max_llm_output_tokens,
        max_llm_input_tokens,
        api_protocol,
    })
}

pub fn parse_token_count(value: &str) -> Option<u32> {
    let raw = value.trim().to_lowercase();
    let (number, multiplier) = if let Some(prefix) = raw.strip_suffix('k') {
        (prefix.trim(), 1_000f64)
    } else if let Some(prefix) = raw.strip_suffix('m') {
        (prefix.trim(), 1_000_000f64)
    } else {
        (raw.as_str(), 1f64)
    };
    let parsed = number.parse::<f64>().ok()?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return None;
    }
    Some((parsed * multiplier).round().clamp(1.0, u32::MAX as f64) as u32)
}

fn parse_api_protocol(value: &str) -> Result<ApiProtocol, String> {
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

fn default_api_protocol(provider: &str) -> ApiProtocol {
    match provider {
        "openai" => ApiProtocol::OpenAiResponses,
        "anthropic" => ApiProtocol::Anthropic,
        _ => ApiProtocol::OpenAiCompatible,
    }
}

pub fn default_api_protocol_for_provider(provider: &str) -> ApiProtocol {
    default_api_protocol(&provider.to_lowercase())
}

fn default_model(provider: &str) -> &str {
    match provider {
        "openai" => "gpt-4o",
        "anthropic" => "claude-sonnet-4-20250514",
        _ => "qwen-plus",
    }
}

pub fn is_default_model_for_provider(provider: &str, model: &str) -> bool {
    let provider = provider.to_lowercase();
    let model = model.to_lowercase();
    match provider.as_str() {
        "openai" => model.contains("gpt"),
        "anthropic" => model.contains("claude"),
        "aliyun" | "dashscope" => model.contains("qwen"),
        _ => true,
    }
}

fn default_base_url(provider: &str) -> &str {
    match provider {
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com",
        "aliyun" | "dashscope" => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        _ => "https://dashscope.aliyuncs.com/compatible-mode/v1",
    }
}

pub fn default_base_url_for_provider(provider: &str) -> String {
    default_base_url(&provider.to_lowercase()).to_string()
}

pub fn is_default_base_url_for_provider(provider: &str, base_url: &str) -> bool {
    let provider = provider.to_lowercase();
    match provider.as_str() {
        "openai" | "anthropic" | "aliyun" | "dashscope" => {
            base_url.trim_end_matches('/') == default_base_url(&provider).trim_end_matches('/')
        }
        _ => true,
    }
}

fn vendor_api_key(provider: &str, env: &HashMap<String, String>) -> Option<String> {
    let key = match provider {
        "openai" => env.get("OPENAI_API_KEY").cloned(),
        "anthropic" => env
            .get("ANTHROPIC_API_KEY")
            .cloned()
            .or_else(|| env.get("ANTHROPIC_AUTH_TOKEN").cloned()),
        "aliyun" | "dashscope" => env.get("DASHSCOPE_API_KEY").cloned(),
        _ => None,
    };
    key.filter(|value| !value.trim().is_empty())
}

fn vendor_base_url(provider: &str, env: &HashMap<String, String>) -> Option<String> {
    match provider {
        "openai" => env.get("OPENAI_BASE_URL").cloned(),
        "anthropic" => env.get("ANTHROPIC_BASE_URL").cloned(),
        "aliyun" | "dashscope" => env.get("DASHSCOPE_BASE_URL").cloned(),
        _ => None,
    }
}

fn vendor_key_hint(provider: &str) -> &str {
    match provider {
        "openai" => "OPENAI_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        _ => "DASHSCOPE_API_KEY",
    }
}

fn validate_api_key(api_key: &str) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err("missing_api_key".to_string());
    }
    if !api_key.is_ascii() {
        return Err("invalid_api_key_non_ascii".to_string());
    }
    Ok(())
}

pub fn build_request(config: &ProviderConfig, prompt: &str) -> Value {
    let blocks = prompt_cache::plan_prompt_cache(prompt);
    let structured_output = structured_output::plan_structured_output(config);
    protocol_adapter::build_request_from_blocks(config, &blocks, structured_output)
}

fn prompt_cache_plan_audit(blocks: &[PromptBlock]) -> Value {
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

pub fn parse_llm_response(config: &ProviderConfig, raw: &Value) -> Result<LlmResponse, String> {
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
                    ..UsageStats::zero()
                },
                stop_reason == "max_tokens",
            )
        }
    };
    if content.trim().is_empty() {
        return Err("empty_model_content".to_string());
    }
    Ok(LlmResponse {
        content,
        model_name: config.model.clone(),
        usage,
        truncated,
    })
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

pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut next = serde_json::Map::new();
            for (key, val) in map {
                if key.to_lowercase().contains("key")
                    || key.eq_ignore_ascii_case("authorization")
                    || key.eq_ignore_ascii_case("x-api-key")
                {
                    next.insert(key.clone(), Value::String("***REDACTED***".to_string()));
                } else {
                    next.insert(key.clone(), redact_value(val));
                }
            }
            Value::Object(next)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        _ => value.clone(),
    }
}

pub fn append_audit(path: &Path, event: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(event).unwrap_or_default())
}

pub fn audit_path(space: &str) -> PathBuf {
    data_root().join(space).join("api_audit.jsonl")
}

pub fn data_root() -> PathBuf {
    std::env::var("TIMEM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"))
}

pub fn call_model(
    config: &ProviderConfig,
    prompt: &str,
    audit_file: &Path,
) -> Result<LlmResponse, String> {
    let prompt_blocks = prompt_cache::plan_prompt_cache(prompt);
    let structured_output = structured_output::plan_structured_output(config);
    let request_body =
        protocol_adapter::build_request_from_blocks(config, &prompt_blocks, structured_output);
    let endpoint = config.endpoint();
    let request_audit = json!({
        "type":"llm_request",
        "provider":config.provider,
        "model":config.model,
        "endpoint":endpoint,
        "prompt_cache_plan": prompt_cache_plan_audit(&prompt_blocks),
        "structured_output": match structured_output {
            StructuredOutputHint::None => "none",
            StructuredOutputHint::JsonObject => "json_object",
        },
        "body": redact_value(&request_body)
    });
    let _ = append_audit(audit_file, &request_audit);

    let mut command = Command::new("curl");
    command
        .arg("-sS")
        .arg("--max-time")
        .arg(config.timeout_secs.to_string())
        .arg("-w")
        .arg("\n%{http_code}")
        .arg("-X")
        .arg("POST")
        .arg(endpoint)
        .arg("-H")
        .arg("Content-Type: application/json");
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible | ApiProtocol::OpenAiResponses => {
            command
                .arg("-H")
                .arg(format!("Authorization: Bearer {}", config.api_key));
        }
        ApiProtocol::Anthropic => {
            command
                .arg("-H")
                .arg(format!("x-api-key: {}", config.api_key));
            command.arg("-H").arg("anthropic-version: 2023-06-01");
        }
    }
    let body = serde_json::to_string(&request_body).map_err(|e| e.to_string())?;
    let output = command
        .arg("--data")
        .arg(body)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() && stdout.trim().is_empty() {
        return Err(if stderr.is_empty() {
            "curl_failed".to_string()
        } else {
            stderr
        });
    }
    let (raw_text, status) = split_curl_body_status(&stdout)?;
    let raw_json: Value = serde_json::from_str(&raw_text)
        .unwrap_or_else(|_| json!({"raw_text": raw_text, "stderr": stderr}));
    let response_audit =
        json!({"type":"llm_response","status":status,"body":redact_value(&raw_json)});
    let _ = append_audit(audit_file, &response_audit);
    if !(200..300).contains(&status) {
        return Err(format!("provider_http_{}", status));
    }
    parse_llm_response(config, &raw_json)
}

fn split_curl_body_status(stdout: &str) -> Result<(String, u16), String> {
    let trimmed = stdout.trim_end();
    let split_at = trimmed
        .rfind('\n')
        .ok_or_else(|| "missing_http_status".to_string())?;
    let (body, status_text) = trimmed.split_at(split_at);
    let status = status_text
        .trim()
        .parse::<u16>()
        .map_err(|_| "invalid_http_status".to_string())?;
    Ok((body.to_string(), status))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LineBuffer {
    chars: Vec<char>,
}
impl LineBuffer {
    pub fn push_str(&mut self, input: &str) {
        self.chars.extend(input.chars());
    }
    pub fn backspace(&mut self) -> bool {
        self.chars.pop().is_some()
    }
    pub fn as_string(&self) -> String {
        self.chars.iter().collect()
    }
    pub fn clear(&mut self) {
        self.chars.clear();
    }
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

    pub fn to_provider_config(&self, model: &str) -> ProviderConfig {
        ProviderConfig {
            provider: "aliyun".to_string(),
            model: model.to_string(),
            base_url: default_base_url("aliyun").to_string(),
            api_key: self.api_key.clone(),
            timeout_secs: 120,
            max_llm_output_tokens: 512,
            max_llm_input_tokens: 100_000,
            api_protocol: ApiProtocol::OpenAiCompatible,
        }
    }
}

pub fn local_llm_key_file_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../key")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(items: &[(&str, &str)]) -> HashMap<String, String> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn local_llm_key_file_parses_key_and_models() {
        let parsed =
            LocalLLMKeyFile::parse("\nkey:\nsk-test\n\navailable_model:\nqwen3.7-plus\nglm-5.2\n")
                .unwrap();
        assert_eq!(parsed.api_key, "sk-test");
        assert_eq!(parsed.available_models, vec!["qwen3.7-plus", "glm-5.2"]);
    }

    #[test]
    fn local_llm_key_file_rejects_missing_models() {
        let err = LocalLLMKeyFile::parse("key:\nsk-test\n").unwrap_err();
        assert_eq!(err, "missing_available_model");
    }

    #[test]
    fn local_llm_key_file_builds_aliyun_provider_config() {
        let parsed =
            LocalLLMKeyFile::parse("key:\nsk-test\navailable_model:\nqwen3.7-plus\n").unwrap();
        let config = parsed.to_provider_config("qwen3.7-plus");
        assert_eq!(config.provider, "aliyun");
        assert_eq!(config.model, "qwen3.7-plus");
        assert_eq!(config.api_key, "sk-test");
        assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
    }

    #[test]
    #[ignore = "requires rust/key with a real Aliyun-compatible API key and network access"]
    fn real_aliyun_model_from_key_file_returns_usage_and_text() {
        let key_file = LocalLLMKeyFile::load(&local_llm_key_file_path()).unwrap();
        let model = key_file.random_model().to_string();
        let config = key_file.to_provider_config(&model);
        let mut audit_file = std::env::temp_dir();
        audit_file.push(format!(
            "timem_real_llm_{}_{}.jsonl",
            model.replace('/', "_"),
            std::process::id()
        ));
        let _ = std::fs::remove_file(&audit_file);

        let response = call_model(
            &config,
            r#"Return exactly this JSON object and no markdown: {"response_to_user":"pong","acceptance_check":{"is_satisfied":true}}"#,
            &audit_file,
        )
        .unwrap();

        assert_eq!(response.model_name, model);
        assert!(response.content.contains("response_to_user") || response.content.contains("pong"));
        assert!(response.usage.llm_calls >= 1);
        assert!(response.usage.prompt_tokens > 0 || response.usage.total_tokens > 0);

        let audit_text = std::fs::read_to_string(&audit_file).unwrap();
        assert!(audit_text.contains("llm_request"));
        assert!(audit_text.contains("llm_response"));
        assert!(!audit_text.contains(&key_file.api_key));
        let _ = std::fs::remove_file(audit_file);
    }

    #[test]
    fn generic_api_key_wins_over_vendor_key() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                ..CliOptions::default()
            },
            &env(&[
                ("TIMEM_API_KEY", "generic"),
                ("DASHSCOPE_API_KEY", "vendor"),
            ]),
        )
        .unwrap();
        assert_eq!(config.api_key, "generic");
    }

    #[test]
    fn default_gateway_provider_is_aliyun() {
        let config =
            provider_config_from_env(&CliOptions::default(), &env(&[("TIMEM_API_KEY", "k")]))
                .unwrap();
        assert_eq!(config.provider, "aliyun");
        assert_eq!(config.model, "qwen-plus");
        assert_eq!(
            config.base_url,
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
    }

    #[test]
    fn known_providers_have_explicit_default_base_urls() {
        let cases = [
            (
                "aliyun",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                ApiProtocol::OpenAiCompatible,
            ),
            (
                "dashscope",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                ApiProtocol::OpenAiCompatible,
            ),
            (
                "openai",
                "https://api.openai.com/v1",
                ApiProtocol::OpenAiResponses,
            ),
            (
                "anthropic",
                "https://api.anthropic.com",
                ApiProtocol::Anthropic,
            ),
        ];

        for (provider, expected_base_url, expected_protocol) in cases {
            let config = provider_config_from_env(
                &CliOptions {
                    provider: Some(provider.to_string()),
                    ..CliOptions::default()
                },
                &env(&[("TIMEM_API_KEY", "k")]),
            )
            .unwrap();
            assert_eq!(config.base_url, expected_base_url);
            assert_eq!(config.api_protocol, expected_protocol);
        }
    }

    #[test]
    fn custom_gateway_provider_does_not_inherit_aliyun_default_model_or_url_rules() {
        assert!(is_default_model_for_provider(
            "custom",
            "aws-claude-sonnet-4-6"
        ));
        assert!(is_default_model_for_provider("private", "any-model-name"));
        assert!(is_default_base_url_for_provider(
            "custom",
            "https://your-private-gateway.example/v1"
        ));
        assert!(is_default_base_url_for_provider(
            "private",
            "https://your-private-gateway.example/v1"
        ));
        assert!(!is_default_base_url_for_provider(
            "aliyun",
            "https://example.com/v1"
        ));
        assert!(!is_default_model_for_provider(
            "aliyun",
            "aws-claude-sonnet-4-6"
        ));
    }

    #[test]
    fn empty_generic_api_key_falls_back_to_vendor_key() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", ""), ("DASHSCOPE_API_KEY", "vendor")]),
        )
        .unwrap();
        assert_eq!(config.api_key, "vendor");
    }

    #[test]
    fn empty_api_key_reports_missing_key() {
        let err = provider_config_from_env(
            &CliOptions {
                provider: Some("openai".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", ""), ("OPENAI_API_KEY", "")]),
        )
        .unwrap_err();
        assert!(err.contains("missing_api_key"));
    }

    #[test]
    fn non_ascii_api_key_reports_clear_error() {
        let err = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "你的token")]),
        )
        .unwrap_err();
        assert!(err.contains("invalid_api_key_non_ascii"));
    }

    #[test]
    fn parse_cli_args_reads_provider_model_and_limits() {
        let args = [
            "--space",
            ".x",
            "--gateway-provider",
            "custom-claude-gateway",
            "--api-protocol",
            "openai-compatible",
            "--api-key",
            "cli-key",
            "--model",
            "gpt-x",
            "--base-url",
            "http://local/v1",
            "--data-dir",
            "/tmp/timem-data",
            "--timeout",
            "33",
            "--max-llm-output",
            "10K",
            "--max-llm-input",
            "100K",
            "--once-json",
            "你好",
            "--supporting-context",
            "previous transcript",
            "--bash-approval",
            "approve",
        ]
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
        let options = parse_cli_args(&args);
        assert_eq!(options.space.as_deref(), Some(".x"));
        assert_eq!(options.provider.as_deref(), Some("custom-claude-gateway"));
        assert_eq!(options.api_protocol.as_deref(), Some("openai-compatible"));
        assert_eq!(options.api_key.as_deref(), Some("cli-key"));
        assert_eq!(options.model.as_deref(), Some("gpt-x"));
        assert_eq!(options.base_url.as_deref(), Some("http://local/v1"));
        assert_eq!(options.data_dir.as_deref(), Some("/tmp/timem-data"));
        assert_eq!(options.timeout_secs, Some(33));
        assert_eq!(options.max_llm_output_tokens, Some(10_000));
        assert_eq!(options.max_llm_input_tokens, Some(100_000));
        assert_eq!(options.once_json_input.as_deref(), Some("你好"));
        assert_eq!(
            options.supporting_context.as_deref(),
            Some("previous transcript")
        );
        assert_eq!(options.bash_approval.as_deref(), Some("approve"));
    }

    #[test]
    fn cli_api_key_overrides_env_api_key() {
        let config = provider_config_from_env(
            &CliOptions {
                api_key: Some("cli-key".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "env-key")]),
        )
        .unwrap();
        assert_eq!(config.api_key, "cli-key");
    }

    #[test]
    fn default_token_limits_are_input_100k_and_output_10k() {
        let config =
            provider_config_from_env(&CliOptions::default(), &env(&[("TIMEM_API_KEY", "k")]))
                .unwrap();
        assert_eq!(config.max_llm_input_tokens, 100_000);
        assert_eq!(config.max_llm_output_tokens, 10_000);
    }

    #[test]
    fn cli_options_override_env_config_values() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("custom".into()),
                api_protocol: Some("anthropic".into()),
                model: Some("cli-model".into()),
                base_url: Some("https://cli.example/v1".into()),
                timeout_secs: Some(33),
                max_llm_output_tokens: Some(1234),
                max_llm_input_tokens: Some(64_000),
                api_key: Some("cli-key".into()),
                ..CliOptions::default()
            },
            &env(&[
                ("TIMEM_GATEWAY_PROVIDER", "aliyun"),
                ("TIMEM_API_PROTOCOL", "openai-compatible"),
                ("TIMEM_MODEL", "env-model"),
                ("TIMEM_BASE_URL", "https://env.example/v1"),
                ("TIMEM_TIMEOUT", "99"),
                ("TIMEM_MAX_LLM_OUTPUT", "9999"),
                ("TIMEM_MAX_LLM_INPUT", "128K"),
                ("TIMEM_API_KEY", "env-key"),
            ]),
        )
        .unwrap();

        assert_eq!(config.provider, "custom");
        assert_eq!(config.api_protocol, ApiProtocol::Anthropic);
        assert_eq!(config.model, "cli-model");
        assert_eq!(config.base_url, "https://cli.example/v1");
        assert_eq!(config.timeout_secs, 33);
        assert_eq!(config.max_llm_output_tokens, 1234);
        assert_eq!(config.max_llm_input_tokens, 64_000);
        assert_eq!(config.api_key, "cli-key");
    }

    #[test]
    fn gateway_provider_env_selects_gateway_and_context_window() {
        let config = provider_config_from_env(
            &CliOptions::default(),
            &env(&[
                ("TIMEM_API_KEY", "k"),
                ("TIMEM_GATEWAY_PROVIDER", "custom"),
                ("TIMEM_MAX_LLM_INPUT", "128K"),
            ]),
        )
        .unwrap();
        assert_eq!(config.provider, "custom");
        assert_eq!(config.max_llm_input_tokens, 128_000);
    }

    #[test]
    fn build_request_uses_max_llm_output_tokens_for_openai_compatible() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                max_llm_output_tokens: Some(2048),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let body = build_request(&config, "hello");
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["model"], "qwen-plus");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "hello");
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn structured_output_strategy_is_provider_and_protocol_specific() {
        let aliyun = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(
            plan_structured_output(&aliyun),
            StructuredOutputHint::JsonObject
        );

        let custom = provider_config_from_env(
            &CliOptions {
                provider: Some("custom".into()),
                api_protocol: Some("openai-compatible".into()),
                base_url: Some("https://your-gateway.example/v1".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(plan_structured_output(&custom), StructuredOutputHint::None);
        assert!(build_request(&custom, "hello")
            .get("response_format")
            .is_none());

        let anthropic = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(
            plan_structured_output(&anthropic),
            StructuredOutputHint::None
        );
        assert!(build_request(&anthropic, "hello")
            .get("response_format")
            .is_none());
    }

    #[test]
    fn prompt_cache_strategy_marks_incremental_prefixes() {
        let prompt1 = "[BEGIN SEGMENT 0: prompt_0]\nSTATIC\n[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]\ndelta1\n[END SEGMENT 1: prompt_delta]";
        let blocks1 = plan_prompt_cache(prompt1);
        assert_eq!(blocks1.len(), 2);
        assert_eq!(blocks1[0].role, PromptBlockRole::System);
        assert_eq!(blocks1[0].text, "STATIC");
        assert_eq!(blocks1[0].cache, CacheControl::Ephemeral);
        assert!(blocks1[1].text.contains("delta1"));
        assert_eq!(blocks1[1].cache, CacheControl::None);

        let prompt2 = "[BEGIN SEGMENT 0: prompt_0]\nSTATIC\n[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]\ndelta1\n[END SEGMENT 1: prompt_delta]\n[BEGIN SEGMENT 2: prompt_delta]\ndelta2\n[END SEGMENT 2: prompt_delta]";
        let blocks2 = plan_prompt_cache(prompt2);
        assert_eq!(blocks2.len(), 3);
        assert!(blocks2[1].text.contains("delta1"));
        assert!(!blocks2[1].text.contains("delta2"));
        assert_eq!(blocks2[1].cache, CacheControl::Ephemeral);
        assert!(blocks2[2].text.contains("delta2"));
        assert_eq!(blocks2[2].cache, CacheControl::None);

        let prompt3 = "[BEGIN SEGMENT 0: prompt_0]\nSTATIC\n[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]\ndelta1\n[END SEGMENT 1: prompt_delta]\n[BEGIN SEGMENT 2: prompt_delta]\ndelta2\n[END SEGMENT 2: prompt_delta]\n[BEGIN SEGMENT 3: prompt_delta]\ndelta3\n[END SEGMENT 3: prompt_delta]";
        let blocks3 = plan_prompt_cache(prompt3);
        assert_eq!(blocks3.len(), 3);
        assert!(blocks3[1].text.contains("delta1"));
        assert!(blocks3[1].text.contains("delta2"));
        assert!(!blocks3[1].text.contains("delta3"));
        assert_eq!(blocks3[1].cache, CacheControl::Ephemeral);
        assert!(blocks3[2].text.contains("delta3"));
        assert_eq!(blocks3[2].cache, CacheControl::None);
    }

    #[test]
    fn prompt_cache_strategy_keeps_multi_slice_delta_together() {
        let prompt = "[BEGIN SEGMENT 0: prompt_0]\nSTATIC\n[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]\ndelta_id: pd_1\nslice_id: ps_1_s001\nslice: 1/1\ndelta1\n[END SEGMENT 1: prompt_delta]\n[BEGIN SEGMENT 2: prompt_delta]\ndelta_id: pd_2\nslice_id: ps_2_s001\nslice: 1/2\ndelta2 slice1\n[END SEGMENT 2: prompt_delta]\n[BEGIN SEGMENT 3: prompt_delta]\ndelta_id: pd_2\nslice_id: ps_2_s002\nslice: 2/2\ndelta2 slice2\n[END SEGMENT 3: prompt_delta]";
        let blocks = plan_prompt_cache(prompt);
        assert_eq!(blocks.len(), 3);
        assert!(blocks[1].text.contains("delta1"));
        assert!(!blocks[1].text.contains("delta2"));
        assert!(blocks[2].text.contains("delta2 slice1"));
        assert!(blocks[2].text.contains("delta2 slice2"));
        assert_eq!(blocks[2].cache, CacheControl::None);
    }

    #[test]
    fn anthropic_request_maps_cache_strategy_blocks_to_content_blocks() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let prompt = "[BEGIN SEGMENT 0: prompt_0]\nSTATIC\n[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]\ndelta1\n[END SEGMENT 1: prompt_delta]\n[BEGIN SEGMENT 2: prompt_delta]\ndelta2\n[END SEGMENT 2: prompt_delta]";
        let body = build_request(&config, prompt);
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
        assert!(body["messages"][0]["content"][1]
            .get("cache_control")
            .is_none());
    }

    #[test]
    fn prompt_cache_audit_summary_has_hashes_without_text() {
        let blocks = plan_prompt_cache("[BEGIN SEGMENT 0: prompt_0]\nSTATIC\n[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]\ndelta1\n[END SEGMENT 1: prompt_delta]");
        let summary = prompt_cache_plan_audit(&blocks);
        let rendered = summary.to_string();
        assert!(rendered.contains("\"hash\""));
        assert!(rendered.contains("\"chars\""));
        assert!(!rendered.contains("STATIC"));
        assert!(!rendered.contains("delta1"));
    }

    #[test]
    fn build_request_uses_official_openai_responses_shape() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("openai".into()),
                max_llm_output_tokens: Some(2048),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(config.api_protocol, ApiProtocol::OpenAiResponses);
        assert_eq!(config.endpoint(), "https://api.openai.com/v1/responses");

        let prompt = "[BEGIN SEGMENT 0: prompt_0]\nSTATIC_GLOBAL\n[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]\nprompt_type: user_question\nUser question:\nhello\ntime: 1\n[END SEGMENT 1: prompt_delta]";
        let body = build_request(&config, prompt);
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["max_output_tokens"], 2048);
        assert!(body["instructions"]
            .as_str()
            .unwrap()
            .contains("STATIC_GLOBAL"));
        assert!(body["input"].as_str().unwrap().contains("[BEGIN SEGMENT 1"));
        assert!(body.get("messages").is_none());
        assert!(body.get("max_llm_output_tokens").is_none());
    }

    #[test]
    fn api_protocol_controls_wire_protocol_independent_of_provider_label() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("custom".into()),
                api_protocol: Some("openai-compatible".into()),
                base_url: Some("https://your-gateway.example/v1".into()),
                model: Some("aws-claude-opus-4-7".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();

        assert_eq!(config.provider, "custom");
        assert_eq!(config.base_url, "https://your-gateway.example/v1");
        assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
        assert_eq!(
            config.endpoint(),
            "https://your-gateway.example/v1/chat/completions"
        );
        let body = build_request(&config, "hello");
        assert_eq!(body["model"], "aws-claude-opus-4-7");
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[test]
    fn explicit_base_url_overrides_provider_default_url() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("custom".into()),
                api_protocol: Some("openai-compatible".into()),
                base_url: Some("http://local-gateway/v1".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();

        assert_eq!(config.base_url, "http://local-gateway/v1");
        assert_eq!(
            config.endpoint(),
            "http://local-gateway/v1/chat/completions"
        );
    }

    #[test]
    fn build_request_uses_max_llm_output_tokens_for_anthropic() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                max_llm_output_tokens: Some(2048),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let body = build_request(&config, "hello");
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert!(body["system"].is_array());
        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], "hello");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
        assert!(body["messages"][0]["content"][0]
            .get("cache_control")
            .is_none());
    }

    #[test]
    fn build_request_splits_static_and_dynamic_prompt() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                max_llm_output_tokens: Some(2048),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let prompt = "[BEGIN SEGMENT 0: prompt_0]\nSTATIC_GLOBAL\n[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]\nprompt_type: user_question\nUser question:\nsecret\ntime: 1\n[END SEGMENT 1: prompt_delta]";
        let body = build_request(&config, prompt);
        let system_content = body["messages"][0]["content"].as_str().unwrap();
        let user_content = body["messages"][1]["content"].as_str().unwrap();
        assert!(system_content.contains("STATIC_GLOBAL"));
        assert!(!system_content.contains("[BEGIN SEGMENT 1"));
        assert_eq!(body["messages"][0]["cache_control"]["type"], "ephemeral");
        assert!(!system_content.contains("prompt_0"));
        assert!(user_content.contains("[BEGIN SEGMENT 1"));
        assert!(user_content.contains("secret"));
        assert!(!user_content.contains("STATIC_GLOBAL"));
    }

    #[test]
    fn same_model_keeps_provider_endpoint_distinct() {
        let aliyun = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                model: Some("qwen-plus".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let openai = provider_config_from_env(
            &CliOptions {
                provider: Some("openai".into()),
                model: Some("qwen-plus".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_ne!(aliyun.endpoint(), openai.endpoint());
        assert_ne!(aliyun.core_profile().label(), openai.core_profile().label());
    }

    #[test]
    fn chinese_backspace_removes_one_character() {
        let mut line = LineBuffer::default();
        line.push_str("我叫默默");
        assert!(line.backspace());
        assert_eq!(line.as_string(), "我叫默");
    }

    #[test]
    fn custom_gateway_can_use_anthropic_protocol_for_cache_control() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("custom".into()),
                api_protocol: Some("anthropic".into()),
                base_url: Some("https://your-gateway.example/v1".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(config.api_protocol, ApiProtocol::Anthropic);
        assert_eq!(
            config.endpoint(),
            "https://your-gateway.example/v1/messages"
        );
        let body = build_request(&config, "hello");
        assert!(body["system"].is_array());
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn anthropic_endpoint_avoids_double_v1_when_base_already_ends_with_v1() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                base_url: Some("https://example.com/api/v1".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(config.endpoint(), "https://example.com/api/v1/messages");
        let config2 = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(config2.endpoint(), "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn compact_count_formats_token_numbers() {
        assert_eq!(compact_count(100), "100");
        assert_eq!(compact_count(1_220), "1.2K");
        assert_eq!(compact_count(1_000), "1K");
        assert_eq!(compact_count(1_210_000), "1.21M");
        assert_eq!(compact_count(1_200_000), "1.2M");
    }

    #[test]
    fn token_status_uses_compact_numbers() {
        let rendered = render_final_response_at(
            "ok",
            &UsageStats {
                llm_calls: 3,
                prompt_tokens: 1_220,
                completion_tokens: 88,
                cached_tokens: 1_210_000,
                ..UsageStats::zero()
            },
            "aliyun",
            "qwen-plus",
            1,
            "10:52:57",
        );
        assert!(rendered.contains("Token: ▲1.2K(⌁1.21M) ▼88"));
    }

    #[test]
    fn token_status_omits_zero_cache_and_shrink_annotations() {
        assert_eq!(
            token_status(&UsageStats {
                prompt_tokens: 22_200,
                completion_tokens: 1_400,
                ..UsageStats::zero()
            }),
            "Token: ▲22.2K ▼1.4K"
        );
    }

    #[test]
    fn token_status_shows_cache_and_shrink_only_when_present() {
        assert_eq!(
            token_status(&UsageStats {
                prompt_tokens: 22_200,
                completion_tokens: 1_400,
                cached_tokens: 1_200,
                shrunk_tokens: 200,
                ..UsageStats::zero()
            }),
            "Token: ▲22.2K(⌁1.2K , ⇃200) ▼1.4K"
        );
    }

    #[test]
    fn openai_usage_reads_cached_tokens() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({"choices":[{"message":{"content":"{\"response_to_user\":\"hi\"}"}}],"usage":{"prompt_tokens":3019,"completion_tokens":104,"total_tokens":3123,"prompt_tokens_details":{"cached_tokens":2048}}});
        let response = parse_llm_response(&config, &raw).unwrap();
        assert_eq!(response.usage.prompt_tokens, 3019);
        assert_eq!(response.usage.completion_tokens, 104);
        assert_eq!(response.usage.cached_tokens, 2048);
        assert!(!response.truncated);
    }

    #[test]
    fn openai_compatible_finish_reason_length_marks_response_truncated() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("aliyun".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "choices":[{"finish_reason":"length","message":{"content":"{\"response_to_user\":\"partial\"}"}}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert!(response.truncated);
    }

    #[test]
    fn openai_compatible_usage_reads_anthropic_cache_read_tokens() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("custom".into()),
                api_protocol: Some("openai-compatible".into()),
                base_url: Some("https://your-gateway.example/v1".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "choices":[{"message":{"content":"{\"response_to_user\":\"hi\"}"}}],
            "usage":{
                "prompt_tokens":8868,
                "cache_creation_input_tokens":0,
                "cache_read_input_tokens":4096,
                "completion_tokens":1095,
                "total_tokens":9963
            }
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert_eq!(response.usage.prompt_tokens, 8868);
        assert_eq!(response.usage.completion_tokens, 1095);
        assert_eq!(response.usage.cached_tokens, 4096);
    }

    #[test]
    fn openai_responses_usage_reads_official_cached_tokens() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("openai".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "output_text":"{\"response_to_user\":\"hi\"}",
            "usage":{
                "input_tokens":8438,
                "input_tokens_details":{"cached_tokens":4096},
                "output_tokens":398,
                "output_tokens_details":{"reasoning_tokens":0},
                "total_tokens":8836
            }
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert_eq!(response.content, "{\"response_to_user\":\"hi\"}");
        assert_eq!(response.usage.prompt_tokens, 8438);
        assert_eq!(response.usage.completion_tokens, 398);
        assert_eq!(response.usage.total_tokens, 8836);
        assert_eq!(response.usage.cached_tokens, 4096);
        assert!(!response.truncated);
    }

    #[test]
    fn openai_responses_incomplete_max_output_marks_response_truncated() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("openai".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "status":"incomplete",
            "incomplete_details":{"reason":"max_output_tokens"},
            "output_text":"{\"response_to_user\":\"partial\"}",
            "usage":{"input_tokens":10,"output_tokens":10,"total_tokens":20}
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert!(response.truncated);
    }

    #[test]
    fn openai_responses_extracts_text_from_output_items() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("openai".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "output":[{
                "type":"message",
                "role":"assistant",
                "content":[{"type":"output_text","text":"{\"response_to_user\":\"from output\"}","annotations":[]}]
            }],
            "usage":{
                "input_tokens":32,
                "input_tokens_details":{"cached_tokens":0},
                "output_tokens":18,
                "output_tokens_details":{"reasoning_tokens":0},
                "total_tokens":50
            }
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert_eq!(response.content, "{\"response_to_user\":\"from output\"}");
        assert_eq!(response.usage.prompt_tokens, 32);
        assert_eq!(response.usage.completion_tokens, 18);
        assert_eq!(response.usage.cached_tokens, 0);
    }

    #[test]
    fn anthropic_usage_counts_cache_creation_as_prompt_tokens() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "content":[{"type":"text","text":"ok"}],
            "usage":{
                "input_tokens":3,
                "cache_creation_input_tokens":6155,
                "cache_read_input_tokens":0,
                "output_tokens":318
            }
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert_eq!(response.usage.prompt_tokens, 6158);
        assert_eq!(response.usage.completion_tokens, 318);
        assert_eq!(response.usage.total_tokens, 6476);
        assert_eq!(response.usage.cached_tokens, 0);
        assert!(!response.truncated);
    }

    #[test]
    fn anthropic_stop_reason_max_tokens_marks_response_truncated() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "stop_reason":"max_tokens",
            "content":[{"type":"text","text":"{\"response_to_user\":\"partial\"}"}],
            "usage":{"input_tokens":10,"output_tokens":10}
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert!(response.truncated);
    }

    #[test]
    fn anthropic_usage_reads_cache_read_tokens() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "content":[{"type":"text","text":"ok"}],
            "usage":{
                "input_tokens":500,
                "cache_creation_input_tokens":200,
                "cache_read_input_tokens":300,
                "output_tokens":50
            }
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert_eq!(response.usage.prompt_tokens, 1000);
        assert_eq!(response.usage.completion_tokens, 50);
        assert_eq!(response.usage.total_tokens, 1050);
        assert_eq!(response.usage.cached_tokens, 300);
    }

    #[test]
    fn anthropic_usage_missing_cache_fields_defaults_to_zero() {
        let config = provider_config_from_env(
            &CliOptions {
                provider: Some("anthropic".into()),
                ..CliOptions::default()
            },
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        let raw = json!({
            "content":[{"type":"text","text":"ok"}],
            "usage":{"input_tokens":10,"output_tokens":5}
        });
        let response = parse_llm_response(&config, &raw).unwrap();
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.completion_tokens, 5);
        assert_eq!(response.usage.total_tokens, 15);
        assert_eq!(response.usage.cached_tokens, 0);
    }

    #[test]
    fn audit_redacts_secret_fields() {
        let redacted = redact_value(
            &json!({"api_key":"abc","nested":{"Authorization":"Bearer abc"},"ok":"v"}),
        );
        assert_eq!(redacted["api_key"], "***REDACTED***");
        assert_eq!(redacted["nested"]["Authorization"], "***REDACTED***");
        assert_eq!(redacted["ok"], "v");
    }

    #[test]
    fn append_audit_writes_jsonl() {
        let mut path = std::env::temp_dir();
        path.push(format!("timem_shell_audit_{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        append_audit(&path, &json!({"type":"turn_final","ok":true})).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("turn_final"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn audit_path_defaults_to_project_data_dir_and_allows_override() {
        std::env::remove_var("TIMEM_DATA_DIR");
        assert_eq!(
            audit_path(".test_mem"),
            std::path::PathBuf::from("data/.test_mem/api_audit.jsonl")
        );

        std::env::set_var("TIMEM_DATA_DIR", "/tmp/timem-shell-data-test");
        assert_eq!(
            audit_path(".test_mem"),
            std::path::PathBuf::from("/tmp/timem-shell-data-test/.test_mem/api_audit.jsonl")
        );
        std::env::remove_var("TIMEM_DATA_DIR");
    }

    #[test]
    fn action_status_hint_uses_model_intent() {
        let hint = action_status_hint(r#"{"next_actions":[{"action":"query_memory","intent":"确认用户姓名","input":{"query":"名字"}}]}"#).unwrap();
        assert_eq!(hint.intent, "确认用户姓名");
        assert_eq!(hint.memory_marker, "◂⛃");
    }

    #[test]
    fn action_status_hint_marks_chat_history_without_memory_icon() {
        let hint = action_status_hint(r#"{"next_actions":[{"action":"chat_history_query","intent":"查询刚才说法","input":{"query":"刚才"}}]}"#).unwrap();
        assert_eq!(hint.intent, "查询刚才说法");
        assert_eq!(hint.memory_marker, "");
    }

    #[test]
    fn action_status_hint_marks_run_bash_without_memory_icon() {
        let hint = action_status_hint(r#"{"next_actions":[{"action":"run_bash","intent":"统计日志行数","input":{"command":"rg --files | wc -l"}}]}"#).unwrap();
        assert_eq!(hint.intent, "统计日志行数");
        assert_eq!(hint.memory_marker, "");
    }

    #[test]
    fn action_status_hint_marks_shell_job_status_without_memory_icon() {
        let hint = action_status_hint(
            r#"{"next_actions":[{"action":"shell_job_status","intent":"检查后台测试","input":{"job_id":"job_1"}}]}"#,
        )
        .unwrap();
        assert_eq!(hint.intent, "检查后台测试");
        assert_eq!(hint.memory_marker, "");
    }

    #[test]
    fn action_status_hint_marks_sql_read_as_memory_lookup() {
        let hint = action_status_hint(r#"{"next_actions":[{"action":"memory_sql_query","intent":"按入库时间查询","input":{"sql":"SELECT content FROM memories","limit":5}}]}"#).unwrap();
        assert_eq!(hint.intent, "按入库时间查询");
        assert_eq!(hint.memory_marker, "◂⛃");
    }

    #[test]
    fn supporting_context_does_not_infer_memory_lookup_from_language() {
        let identity_context = supporting_context("aliyun", "qwen-plus", "我叫什么名字");
        let explicit_memory_text_context = supporting_context("aliyun", "qwen-plus", "查记忆");
        assert!(!identity_context.contains("memory_lookup_hint"));
        assert!(!explicit_memory_text_context.contains("memory_lookup_hint"));
    }

    #[test]
    fn supporting_context_always_includes_runtime_time() {
        let context = supporting_context("aliyun", "qwen-plus", "当前时间");
        assert!(context.contains("provider: aliyun, model: qwen-plus"));
        assert!(context.contains("runtime: timem_native_shell"));
        assert!(context.contains("run_bash_target: user_local_machine"));
        assert!(context.contains("runtime_time:"));
        assert!(context.contains("local_time"));
        assert!(!context.contains("memory_lookup_hint"));
    }

    #[test]
    fn thinking_block_visual_contract() {
        let block = render_thinking_block_at(
            &ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "查询记忆".into(),
                memory_marker: "◂⛃".into(),
                model_round: 2,
                direction: ModelDirection::Downstream,
                usage: UsageStats {
                    prompt_tokens: 210,
                    completion_tokens: 21,
                    cached_tokens: 0,
                    ..UsageStats::zero()
                },
                tick: 0,
            },
            "08:56:33",
        );
        assert!(block.contains("[08:56:33] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
        assert!(block.contains("🦩 查询记忆..."));
        assert!(block.contains("◂⛃ ║ aliyun:qwen-plus: ▼2"));
        assert!(block.contains("Token: ▲210 ▼21"));
        assert!(!block.contains("⚡cache"));
        assert_eq!(block.lines().count(), 3);
        assert!(!block.contains("thinking..."));
    }

    #[test]
    fn thinking_block_compacts_long_model_intent_to_two_lines() {
        let block = render_thinking_block_at(
            &ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "Check local system date and calendar to verify current date context and compute June 12 significance (e.g., holiday, observance, personal memory).".into(),
                memory_marker: String::new(),
                model_round: 1,
                direction: ModelDirection::Upstream,
                usage: UsageStats::zero(),
                tick: 8,
            },
            "23:33:05",
        );

        assert_eq!(block.lines().count(), 3);
        assert!(block.contains("Check local system"));
        assert!(block.contains('…'));
        assert!(!block.contains("observance"));
    }

    #[test]
    fn thinking_view_renders_observation_panel_and_status_line() {
        let mut observations = ObservationPanel::new(8, 60);
        observations.apply(ObservationEvent::Persistent("正在分析用户请求".into()));
        observations.apply(ObservationEvent::Active(
            "执行 Bash: rg --files | wc -l".into(),
        ));
        let view = render_thinking_view_at(
            &ThinkingViewSnapshot {
                status: ShellStatusSnapshot {
                    provider: "aliyun".into(),
                    model: "qwen-plus".into(),
                    intent: "ignored in panel mode".into(),
                    memory_marker: String::new(),
                    model_round: 2,
                    direction: ModelDirection::Downstream,
                    usage: UsageStats {
                        prompt_tokens: 1200,
                        completion_tokens: 20,
                        cached_tokens: 300,
                        ..UsageStats::zero()
                    },
                    tick: 0,
                },
                observations,
            },
            "12:00:00",
        );

        assert!(view.contains("[12:00:00] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
        assert!(view.contains("Thought / Action"));
        assert!(view.contains("· 正在分析用户请求"));
        assert!(view.contains("\x1b[38;5;245m· 执行 Bash: rg --files | wc -l"));
        assert!(view.contains("aliyun:qwen-plus: ▼2"));
        assert!(view.contains("Token: ▲1.2K(⌁300) ▼20"));
        assert!(!view.contains("ignored in panel mode"));
    }

    #[test]
    fn final_response_visual_contract() {
        let rendered = render_final_response_at(
            "你叫默默。",
            &UsageStats {
                llm_calls: 2,
                mem_reads: 1,
                mem_writes: 1,
                prompt_tokens: 812,
                completion_tokens: 52,
                cached_tokens: 384,
                ..UsageStats::zero()
            },
            "aliyun",
            "qwen-plus",
            2,
            "08:56:46",
        );
        assert!(rendered.contains("[08:56:46] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
        assert!(rendered.contains("\x1b[92;1m"));
        assert!(rendered.contains("𝓣𝓲𝓶𝓮𝓶  ⬇"));
        assert!(rendered
            .lines()
            .nth(1)
            .is_some_and(|line| line == "你叫默默。"));
        assert!(rendered.contains("你叫默默。"));
        assert!(rendered.contains("◂▸⛃ ║ aliyun:qwen-plus: 2"));
        assert!(rendered.contains("Token: ▲812(⌁384) ▼52"));
        assert!(!rendered.contains("你 >"));
        assert!(!rendered.contains("thinking..."));
    }

    #[test]
    fn final_response_renders_simple_markdown_bold() {
        let rendered = render_final_response_at(
            "- **系统**：macOS",
            &UsageStats {
                llm_calls: 1,
                prompt_tokens: 10,
                completion_tokens: 2,
                ..UsageStats::zero()
            },
            "custom",
            "aws-claude-sonnet-4-6",
            1,
            "17:20:00",
        );
        assert!(rendered.contains(&format!("{ANSI_BOLD}系统{ANSI_RESET}：macOS")));
        assert!(!rendered.contains("**系统**"));
    }

    #[test]
    fn final_status_line_is_always_dim_wrapped() {
        let rendered = render_final_response_at(
            "ok",
            &UsageStats {
                llm_calls: 1,
                prompt_tokens: 10,
                completion_tokens: 2,
                ..UsageStats::zero()
            },
            "aliyun",
            "qwen-plus",
            1,
            "10:00:00",
        );
        let status_line = rendered.lines().nth(2).unwrap();
        assert!(status_line.starts_with(&format!("{ANSI_RESET}{ANSI_DIM}")));
        assert!(status_line.ends_with(ANSI_RESET));
        assert!(status_line.contains("↳ 1s"));
    }

    #[test]
    fn shell_status_bar_is_dim_wrapped_and_extensible() {
        let rendered = render_shell_status_bar(&ShellStatusMessage {
            tone: ShellStatusTone::Info,
            text: "已取消当前输入。Ctrl+D 退出。".to_string(),
        });
        assert!(rendered.starts_with(&format!("{ANSI_RESET}{ANSI_DIM}")));
        assert!(rendered.ends_with(ANSI_RESET));
        assert!(rendered.contains("ⓘ 已取消当前输入。Ctrl+D 退出。"));

        let warning = render_shell_status_bar(&ShellStatusMessage {
            tone: ShellStatusTone::Warning,
            text: "状态异常".to_string(),
        });
        assert!(warning.contains("! 状态异常"));
    }

    #[test]
    fn no_memory_status_omits_memory_icon() {
        let rendered = render_final_response_at(
            "Hello",
            &UsageStats {
                llm_calls: 1,
                prompt_tokens: 237,
                completion_tokens: 26,
                ..UsageStats::zero()
            },
            "aliyun",
            "qwen-plus",
            1,
            "10:08:43",
        );
        assert!(rendered.contains("( aliyun:qwen-plus: 1 ║ Token: ▲237 ▼26 )"));
        assert!(!rendered.contains("⛃"));
    }

    #[test]
    fn memory_marker_visual_variants() {
        assert_eq!(
            memory_marker(&UsageStats {
                mem_reads: 1,
                ..UsageStats::zero()
            }),
            "◂⛃"
        );
        assert_eq!(
            memory_marker(&UsageStats {
                mem_writes: 1,
                ..UsageStats::zero()
            }),
            "▸⛃"
        );
        assert_eq!(
            memory_marker(&UsageStats {
                mem_reads: 1,
                mem_writes: 1,
                ..UsageStats::zero()
            }),
            "◂▸⛃"
        );
        assert_eq!(memory_marker(&UsageStats::zero()), "");
    }
}
