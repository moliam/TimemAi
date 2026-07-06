use agent_core::{provider_config_from_sources, ProviderConfigSource, UsageStats};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

mod final_answer_renderer;
mod observation;
mod profiler;

pub use agent_core::{
    append_audit_event as append_audit, apply_runtime_config_value,
    apply_workspace_command_to_path, bash_approval_mode_from_sources, bash_approval_mode_label,
    capabilities_dir_from_sources, collect_storage_profile, combine_additional_contexts,
    compact_runtime_status_text, default_api_protocol_for_provider, default_base_url_for_provider,
    default_data_root, default_model_for_provider, estimate_prompt_context_tokens,
    host_start_audit_event, is_default_base_url_for_provider, is_default_model_for_provider,
    known_default_base_url_for_provider, layout_for_space, load_workspace_dirs_from_path,
    local_time_label, meaningful_latest_usage, model_retry_audit_event, normalize_workspace_dir,
    parse_api_protocol, parse_token_count, resolve_topic_reply, runtime_active_elapsed_secs,
    runtime_config_apply_report, runtime_config_field_value, runtime_config_menu_report,
    runtime_config_report, runtime_profile_report, runtime_retry_status_view, runtime_time_context,
    runtime_token_status_view, stale_context_decision_request, stale_context_prompt_needed,
    supporting_context, topic_event_status_hint, work_instruction_load_report,
    work_instruction_load_request, work_instruction_load_topic_event,
    work_instruction_mode_from_sources, workspace_config_file, workspace_menu_report,
    workspace_reference_context, ApiProtocol, CapabilityHostProfile, CoreActionTopic,
    CoreLifecycleEvent, CoreLifecycleTopic, CoreMemoryActivity, CoreModelResponseTopic,
    CoreTopicEvent, HostDecision, HostDecisionDefault, HostDecisionRequest, HostStatusLevel,
    HostStatusMessage, LocalLLMKeyFile, LongRunningCommandContinueRequest, ModelDirection,
    ModelProfile, NoopTurnUi, OutputExpansionRequest, ProviderConfig, RoundLimitDecisionRequest,
    RuntimeConfigApplyError, RuntimeConfigApplyMessage, RuntimeConfigApplyMessageKind,
    RuntimeConfigApplyReport, RuntimeConfigEffect, RuntimeConfigField, RuntimeConfigMenuItem,
    RuntimeConfigMenuReport, RuntimeConfigReport, RuntimeConfigReportInput,
    RuntimeConfigReportItem, RuntimeConfigReportRow, RuntimeConfigRowKind, RuntimeConfigSection,
    RuntimeProfiler, RuntimeRetryStatus, RuntimeRetryStatusView, RuntimeTokenStatusView,
    StaleContextDecisionRequest, StorageProfile, SupportingContextInput, TokenUsageBreakdown,
    TopicReply, TopicReplyError, TurnInput, TurnOutcome, TurnStopDetail, TurnStopReason,
    TurnStopSummary, TurnUi, WorkInstructionLoadMessage, WorkInstructionLoadMessageKind,
    WorkInstructionLoadMode, WorkInstructionLoadReport, WorkInstructionLoadRequest,
    WorkInstructionLoadStatus, WorkspaceChange, WorkspaceCommand, WorkspaceCommandMessage,
    WorkspaceCommandMessageKind, WorkspaceCommandOutcome, WorkspaceCommandReport,
    WorkspaceMenuReport, WorkspaceState, WorkspaceUnchangedReason, CORE_TOPIC_ACTION,
    CORE_TOPIC_MODEL_RESPONSE, DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT, DEFAULT_STALE_CONTEXT_IDLE,
    DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD, RUNTIME_CONFIG_FIELDS,
};
pub use agent_core::{cancelled_turn_result, run_session_turn};
pub use final_answer_renderer::{
    render_final_answer_markdown, FinalAnswerRenderer, TermimadFinalAnswerRenderer,
};
pub use observation::{
    observation_events_from_core_topic_events, observation_panel_width_for_terminal,
    render_observation_panel, render_observation_panel_at,
    render_observation_panel_at_with_elapsed, ObservationEvent, ObservationLine,
    ObservationLineStyle, ObservationPanel,
};
pub use profiler::render_prof_report_data;

pub const TIMEM_LOGO: &str = "𝓣𝓲𝓶𝓮𝓶";
pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_BRIGHT_TIMEM: &str = "\x1b[92;1m";
pub const ANSI_DIM: &str = "\x1b[2m";
pub const ANSI_BOLD: &str = "\x1b[1m";
pub const SPINNER_ICONS: [&str; 27] = [
    "🦩", "🐧", "🦅", "🦆", "🦢", "🦉", "🦄", "🦖", "🐉", "🐌", "🦏", "🦛", "🐫", "🦙", "🦑", "🦞",
    "🦐", "🦁", "🐮", "🐷", "🐸", "🐒", "🐭", "🐹", "🐰", "🦊", "🦝",
];

pub type ShellStatusSnapshot = agent_core::RuntimeStatusSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingViewSnapshot {
    pub status: ShellStatusSnapshot,
    pub observations: ObservationPanel,
}

fn timem_prefix(time_label: &str) -> String {
    timem_prefix_with_worker(time_label, None)
}

fn timem_prefix_with_worker(time_label: &str, worker_label: Option<&str>) -> String {
    let worker = worker_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(|label| format!(" {label}"))
        .unwrap_or_default();
    format!("{ANSI_BRIGHT_TIMEM}[{time_label}] {TIMEM_LOGO}{worker}  ⬇{ANSI_RESET}")
}

fn dim_line(text: &str) -> String {
    format!("{ANSI_RESET}{ANSI_DIM}{text}{ANSI_RESET}")
}

pub fn format_token_count(value: u32) -> String {
    if value % 1_000 == 0 && value >= 1_000 {
        format!("{}K", value / 1_000)
    } else {
        value.to_string()
    }
}

pub fn render_shell_status_bar(message: &HostStatusMessage) -> String {
    let icon = match message.level {
        HostStatusLevel::Info => "ⓘ",
        HostStatusLevel::Warning => "!",
        HostStatusLevel::Error => "×",
    };
    dim_line(&format!(" {icon} {}", message.text.trim()))
}

pub fn shell_status_message_from_lifecycle_topic(
    lifecycle: &CoreLifecycleTopic,
) -> HostStatusMessage {
    match lifecycle.event {
        CoreLifecycleEvent::Initialized => {
            let worker_label = lifecycle
                .worker
                .as_ref()
                .map(|worker| format!(" {}", worker.display_name))
                .unwrap_or_default();
            HostStatusMessage::info(format!(
                "Timem Core{} 启动成功：{}:{}，response protocol={}，tools={}，skills={}",
                worker_label,
                lifecycle.profile.provider,
                lifecycle.profile.model,
                lifecycle.response_protocol,
                lifecycle.tool_count,
                lifecycle.skill_count
            ))
        }
    }
}

pub fn shell_status_message_from_core_topic(event: &CoreTopicEvent) -> Option<HostStatusMessage> {
    if let Some(lifecycle) = event.as_lifecycle() {
        return Some(shell_status_message_from_lifecycle_topic(&lifecycle));
    }
    let work = event.as_work_instruction_load()?;
    match work.status.as_str() {
        "loaded" => {
            let names = work.file_names.join(", ");
            Some(HostStatusMessage::info(format!(
                "已加载当前工作目录指令：{names}"
            )))
        }
        "failed" => Some(HostStatusMessage::warning(format!(
            "工作目录指令加载失败：{}",
            work.error.unwrap_or_else(|| "unknown_error".to_string())
        ))),
        _ => None,
    }
}

pub fn render_thinking_block_at(snapshot: &ShellStatusSnapshot, time_label: &str) -> String {
    let icon = SPINNER_ICONS[(snapshot.tick / 4) % SPINNER_ICONS.len()];
    let memory_marker = memory_activity_marker(snapshot.memory_activity);
    let memory_prefix = if memory_marker.is_empty() {
        String::new()
    } else {
        format!("{memory_marker} ")
    };
    let intent = compact_status_text(&snapshot.intent, 36);
    let intent_line = dim_line(&format!("{icon} {memory_prefix}{intent}..."));
    let status_line = render_thinking_status_line(snapshot);
    format!(
        "{}\n{intent_line}\n{status_line}\n",
        timem_prefix(time_label)
    )
}

pub fn render_thinking_view_at(snapshot: &ThinkingViewSnapshot, time_label: &str) -> String {
    render_named_thinking_view_at(snapshot, time_label, None)
}

pub fn render_worker_thinking_view_at(
    snapshot: &ThinkingViewSnapshot,
    time_label: &str,
    worker_label: &str,
) -> String {
    render_named_thinking_view_at(snapshot, time_label, Some(worker_label))
}

pub fn render_worker_thinking_views_at(
    views: &[(&str, &ThinkingViewSnapshot)],
    time_label: &str,
) -> String {
    views
        .iter()
        .map(|(worker_label, snapshot)| {
            render_worker_thinking_view_at(snapshot, time_label, worker_label)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_named_thinking_view_at(
    snapshot: &ThinkingViewSnapshot,
    time_label: &str,
    worker_label: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}\n",
        timem_prefix_with_worker(time_label, worker_label)
    ));
    out.push_str(&render_observation_panel_at_with_elapsed(
        &snapshot.observations,
        snapshot.status.tick,
        Some(&format_elapsed_clock(snapshot.status.elapsed_secs)),
    ));
    out.push_str(&render_thinking_status_line(&snapshot.status));
    out.push('\n');
    out
}

fn render_thinking_status_line(snapshot: &ShellStatusSnapshot) -> String {
    let lines = thinking_status_lines(snapshot);
    dim_line(&format!("  {}", lines.join("\n  ")))
}

fn format_elapsed_clock(secs: u64) -> String {
    if secs < 3600 {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    } else {
        format!(
            "{:02}:{:02}:{:02}",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        )
    }
}

pub fn compact_status_text(text: &str, max_chars: usize) -> String {
    compact_runtime_status_text(text, max_chars)
}

pub fn render_final_response_at(
    text: &str,
    stats: &UsageStats,
    latest_usage: Option<&UsageStats>,
    provider: &str,
    model: &str,
    elapsed_secs: u64,
    max_llm_input_tokens: u32,
    time_label: &str,
) -> String {
    let status = final_status_line(
        stats,
        latest_usage,
        provider,
        model,
        elapsed_secs,
        max_llm_input_tokens,
    );
    let status_line = dim_line(&status);
    let body = render_terminal_markdown(text);
    let body = body.trim_end_matches('\n');
    format!("{}\n{body}\n{status_line}\n\n", timem_prefix(time_label))
}

pub fn render_turn_outcome_text(outcome: &TurnOutcome) -> String {
    outcome
        .stop_summary
        .as_ref()
        .map(render_turn_stop_summary)
        .unwrap_or_else(|| outcome.text.clone())
}

pub fn render_turn_stop_summary(stop: &TurnStopSummary) -> String {
    match &stop.detail {
        TurnStopDetail::None if stop.stop_reason == TurnStopReason::CancelledByUser => {
            "已取消本轮。".to_string()
        }
        TurnStopDetail::ModelError { error } => format!("模型调用失败：{error}"),
        TurnStopDetail::OutputLimit { current_tokens } => format!(
            "模型输出达到当前上限 {}，已按你的选择停止本轮。可用 /config 调大 TIMEM_MAX_LLM_OUTPUT 后重试。",
            format_token_count(*current_tokens)
        ),
        TurnStopDetail::RoundLimit { max_rounds } => {
            format!("已达到本轮最大交互次数 {max_rounds}，已停止。你可以继续输入来开启新一轮。")
        }
        TurnStopDetail::ProtocolRepairFailure {
            first_issue,
            final_issue,
            truncated,
        } => {
            if *truncated {
                "模型回复被 API 提供商按最大输出 token 限制截断，导致返回协议不完整。可用 /config 调大 TIMEM_MAX_LLM_OUTPUT 后重试。".to_string()
            } else {
                let issue = if final_issue.is_empty() {
                    first_issue
                } else {
                    final_issue
                };
                format!("模型的回复不符合本地协议，已拦截原始报文展示。原因：{issue}。请重试或换一个更具体的问题。")
            }
        }
        TurnStopDetail::None => format!("本轮已停止：{:?}", stop.stop_reason),
    }
}

pub fn render_terminal_markdown(text: &str) -> String {
    render_final_answer_markdown(text)
}

pub fn token_status(stats: &UsageStats) -> String {
    token_status_with_latest(stats, None, TokenStatusMode::Plain)
}

fn thinking_status_lines(snapshot: &ShellStatusSnapshot) -> Vec<String> {
    let view = runtime_token_status_view(
        &snapshot.usage,
        snapshot.latest_usage.as_ref(),
        snapshot.max_llm_input_tokens,
        snapshot.model_round,
    );
    let latest = view.latest.as_ref();
    let retry_lines = retry_status_lines(snapshot);
    let has_retry = !retry_lines.is_empty();
    let mut lines = Vec::new();
    lines.push(format!(
        "{}:{} ⇌{} ║ {}",
        snapshot.provider,
        snapshot.model,
        model_round_with_repairs(view.model_rounds, view.repair_calls),
        compact_token_totals(&view.total)
    ));
    lines.push(format!(
        "  {} context : {}",
        if has_retry || latest.is_some() {
            "├─"
        } else {
            "└─"
        },
        context_bar(&view)
    ));
    let latest_prefix = if has_retry { "├─" } else { "└─" };
    if let Some(usage) = latest {
        lines.push(format!("  {latest_prefix} {}", compact_token_latest(usage)));
    } else if has_retry {
        lines.push(format!("  {latest_prefix} △0  ▽0"));
    }
    lines.extend(retry_lines);
    lines
}

fn retry_status_lines(snapshot: &ShellStatusSnapshot) -> Vec<String> {
    let Some(retry) = snapshot.retry.as_ref() else {
        return Vec::new();
    };
    let view = runtime_retry_status_view(retry, current_epoch_ms());
    retry_status_lines_from_view(&view)
}

fn retry_status_lines_from_view(view: &RuntimeRetryStatusView) -> Vec<String> {
    let main = format!(
        "网络错误，{}s 后重试（第{}/{}次）",
        view.remaining_secs, view.attempt, view.max_attempts
    );
    let mut lines = Vec::new();
    if let Some(error) = view.error.as_deref() {
        lines.push(format!("  ├─ {main}"));
        lines.push(format!("  └─ 详情：{}", compact_retry_notice(error)));
    } else {
        lines.push(format!("  └─ {main}"));
    }
    lines
}

fn current_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn compact_retry_notice(text: &str) -> String {
    const MAX_CHARS: usize = 78;
    compact_runtime_status_text(text, MAX_CHARS)
}

fn final_status_line(
    stats: &UsageStats,
    latest_usage: Option<&UsageStats>,
    provider: &str,
    model: &str,
    elapsed_secs: u64,
    max_llm_input_tokens: u32,
) -> String {
    let view =
        runtime_token_status_view(stats, latest_usage, max_llm_input_tokens, stats.llm_calls);
    let mut parts = Vec::new();
    if view.latest.is_some() {
        if view.context_percent > 0 {
            parts.push(format!("ctx[{}%]", view.context_percent));
        }
    }
    parts.push(format!("▲{}", compact_count(view.total.input_tokens)));
    parts.push(format!("▼{}", compact_count(view.total.output_tokens)));
    if let Some(kvc) = kvc_status(view.total.cached_tokens, view.total.cache_created_tokens) {
        parts.push(kvc);
    }
    format!(
        " ↳  {}s    {}:{} ⇌{} ║ {}",
        elapsed_secs,
        provider,
        model,
        model_round_with_repairs(view.model_rounds, view.repair_calls),
        parts.join("  ")
    )
}

fn model_round_with_repairs(rounds: u32, repairs: u32) -> String {
    if repairs == 0 {
        rounds.to_string()
    } else {
        format!("{rounds} (⚠{repairs})")
    }
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

fn compact_token_totals(stats: &TokenUsageBreakdown) -> String {
    let mut parts = vec![
        format!("▲{}", compact_count(stats.input_tokens)),
        format!("▼{}", compact_count(stats.output_tokens)),
    ];
    if let Some(kvc) = kvc_status(stats.cached_tokens, stats.cache_created_tokens) {
        parts.push(kvc);
    }
    parts.join(" | ")
}

fn compact_token_latest(usage: &TokenUsageBreakdown) -> String {
    let mut parts = vec![
        format!("△{}", compact_count(usage.input_tokens)),
        format!("▽{}", compact_count(usage.output_tokens)),
    ];
    if let Some(kvc) = kvc_status(usage.cached_tokens, usage.cache_created_tokens) {
        parts.push(kvc);
    }
    parts.join("  ")
}

fn kvc_status(cached_tokens: u32, cache_created_tokens: u32) -> Option<String> {
    let mut parts = Vec::new();
    if cached_tokens > 0 {
        parts.push(format!("⌁{}", compact_count(cached_tokens)));
    }
    if cache_created_tokens > 0 {
        parts.push(format!("✚{}", compact_count(cache_created_tokens)));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("KVC({})", parts.join(" ")))
    }
}

fn context_bar(view: &RuntimeTokenStatusView) -> String {
    let filled = view.context_bar_filled.min(view.context_bar_total);
    let empty = view.context_bar_total.saturating_sub(filled);
    format!(
        "{}{}",
        "▰".repeat(filled as usize),
        "▱".repeat(empty as usize)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStatusMode {
    Plain,
    Thinking,
    Final,
}

pub fn token_status_with_latest(
    stats: &UsageStats,
    latest_usage: Option<&UsageStats>,
    mode: TokenStatusMode,
) -> String {
    let latest = meaningful_latest_usage(latest_usage);
    let ctx = if mode == TokenStatusMode::Final {
        latest
            .map(|usage| format!(" [ctx {}]", compact_count(usage.prompt_tokens)))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let mut input = format!("▲{}", compact_count(stats.prompt_tokens));
    let mut input_annotations = Vec::new();
    if stats.cached_tokens > 0 {
        input_annotations.push(format!("KVC:⌁{}", compact_count(stats.cached_tokens)));
    }
    if stats.cache_created_tokens > 0 {
        input_annotations.push(format!(
            "KVC:✚{}",
            compact_count(stats.cache_created_tokens)
        ));
    }
    if stats.shrunk_tokens > 0 {
        input_annotations.push(format!("⇃{}", compact_count(stats.shrunk_tokens)));
    }
    if !input_annotations.is_empty() {
        input.push_str(&format!("({})", input_annotations.join(" , ")));
    }
    if mode == TokenStatusMode::Thinking {
        if let Some(usage) = latest {
            if usage.prompt_tokens > 0 && stats.prompt_tokens > 0 {
                input.push_str(&format!("(+{})", compact_count(usage.prompt_tokens)));
            } else if usage.prompt_tokens > 0 && stats.prompt_tokens == 0 {
                input = format!("▲{}", compact_count(usage.prompt_tokens));
            }
        }
    }
    let mut output = format!("▼{}", compact_count(stats.completion_tokens));
    if mode == TokenStatusMode::Thinking {
        if let Some(usage) = latest {
            if usage.completion_tokens > 0 && stats.completion_tokens > 0 {
                output.push_str(&format!("(+{})", compact_count(usage.completion_tokens)));
            } else if usage.completion_tokens > 0 && stats.completion_tokens == 0 {
                output = format!("▼{}", compact_count(usage.completion_tokens));
            }
        }
    }
    if ctx.is_empty() {
        format!("Token: {} {}", input, output)
    } else {
        format!("Token{} {} {}", ctx, input, output)
    }
}

pub fn memory_marker(stats: &UsageStats) -> &'static str {
    match (stats.mem_reads > 0, stats.mem_writes > 0) {
        (true, true) => "◂▸⛃",
        (true, false) => "◂⛃",
        (false, true) => "▸⛃",
        (false, false) => "",
    }
}

pub fn memory_activity_marker(activity: CoreMemoryActivity) -> &'static str {
    match activity {
        CoreMemoryActivity::None => "",
        CoreMemoryActivity::Read => "◂⛃",
        CoreMemoryActivity::Write => "▸⛃",
    }
}

#[derive(Debug, Clone, Default)]
pub struct CliOptions {
    pub space: Option<String>,
    pub provider: Option<String>,
    pub api_protocol: Option<String>,
    pub response_protocol: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub data_dir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub max_llm_output_tokens: Option<u32>,
    pub max_llm_input_tokens: Option<u32>,
    pub capabilities_dir: Option<String>,
    pub once_json_input: Option<String>,
    pub supporting_context: Option<String>,
    pub bash_approval: Option<String>,
    pub work_instructions: Option<String>,
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
            ("--response-protocol", Some(v)) => {
                options.response_protocol = Some(v);
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
            ("--capabilities-dir", Some(v)) => {
                options.capabilities_dir = Some(v);
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
            ("--work-instructions", Some(v)) => {
                options.work_instructions = Some(v);
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
    provider_config_from_sources(
        &ProviderConfigSource {
            provider: options.provider.clone(),
            api_protocol: options.api_protocol.clone(),
            api_key: options.api_key.clone(),
            model: options.model.clone(),
            base_url: options.base_url.clone(),
            timeout_secs: options.timeout_secs,
            max_llm_output_tokens: options.max_llm_output_tokens,
            max_llm_input_tokens: options.max_llm_input_tokens,
            local_api_key: LocalLLMKeyFile::load(&local_llm_key_file_path())
                .ok()
                .map(|file| file.api_key),
        },
        env,
    )
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

pub fn local_llm_key_file_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../key")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use unicode_width::UnicodeWidthStr;

    fn env(items: &[(&str, &str)]) -> HashMap<String, String> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn strip_ansi_for_test(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for code_ch in chars.by_ref() {
                    if code_ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn visible_width_for_test(text: &str) -> usize {
        UnicodeWidthStr::width(strip_ansi_for_test(text).as_str())
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
        assert_eq!(
            config.response_protocol,
            agent_core::ResponseProtocolKind::Xml
        );
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
            "--response-protocol",
            "xml",
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
            "--capabilities-dir",
            "/tmp/timem-capabilities",
            "--once-json",
            "你好",
            "--supporting-context",
            "previous transcript",
            "--bash-approval",
            "approve",
            "--work-instructions",
            "ask",
        ]
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
        let options = parse_cli_args(&args);
        assert_eq!(options.space.as_deref(), Some(".x"));
        assert_eq!(options.provider.as_deref(), Some("custom-claude-gateway"));
        assert_eq!(options.api_protocol.as_deref(), Some("openai-compatible"));
        assert_eq!(options.response_protocol.as_deref(), Some("xml"));
        assert_eq!(options.api_key.as_deref(), Some("cli-key"));
        assert_eq!(options.model.as_deref(), Some("gpt-x"));
        assert_eq!(options.base_url.as_deref(), Some("http://local/v1"));
        assert_eq!(options.data_dir.as_deref(), Some("/tmp/timem-data"));
        assert_eq!(options.timeout_secs, Some(33));
        assert_eq!(options.max_llm_output_tokens, Some(10_000));
        assert_eq!(options.max_llm_input_tokens, Some(100_000));
        assert_eq!(
            options.capabilities_dir.as_deref(),
            Some("/tmp/timem-capabilities")
        );
        assert_eq!(options.once_json_input.as_deref(), Some("你好"));
        assert_eq!(
            options.supporting_context.as_deref(),
            Some("previous transcript")
        );
        assert_eq!(options.bash_approval.as_deref(), Some("approve"));
        assert_eq!(options.work_instructions.as_deref(), Some("ask"));
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
    fn chinese_backspace_removes_one_character() {
        let mut line = LineBuffer::default();
        line.push_str("中文测试");
        assert!(line.backspace());
        assert_eq!(line.as_string(), "中文测");
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
            None,
            "aliyun",
            "qwen-plus",
            1,
            100_000,
            "10:52:57",
        );
        assert!(rendered.contains("aliyun:qwen-plus ⇌3 ║ ▲1.2K  ▼88  KVC(⌁1.21M)"));
    }

    #[test]
    fn shell_renders_stopped_turn_text_from_core_summary() {
        let stopped = TurnStopSummary::model_error("provider_http_400").into_stopped_turn();
        let outcome = TurnOutcome::stopped("", stopped, Duration::from_secs(1));

        assert!(outcome.text.is_empty());
        assert_eq!(
            render_turn_outcome_text(&outcome),
            "模型调用失败：provider_http_400"
        );
    }

    #[test]
    fn final_status_shows_repair_call_count_when_present() {
        let rendered = render_final_response_at(
            "ok",
            &UsageStats {
                llm_calls: 13,
                repair_calls: 3,
                prompt_tokens: 85_000,
                completion_tokens: 3_500,
                cached_tokens: 53_900,
                ..UsageStats::zero()
            },
            Some(&UsageStats {
                prompt_tokens: 80_000,
                completion_tokens: 321,
                ..UsageStats::zero()
            }),
            "custom",
            "aws-claude-opus-4-7",
            6,
            100_000,
            "22:29:07",
        );
        assert!(rendered
            .contains("custom:aws-claude-opus-4-7 ⇌13 (⚠3) ║ ctx[80%]  ▲85K  ▼3.5K  KVC(⌁53.9K)"));
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
            "Token: ▲22.2K(KVC:⌁1.2K , ⇃200) ▼1.4K"
        );
    }

    #[test]
    fn token_status_shows_latest_delta_and_context_window() {
        let total = UsageStats {
            prompt_tokens: 4_400,
            completion_tokens: 56,
            ..UsageStats::zero()
        };
        let latest = UsageStats {
            prompt_tokens: 2_000,
            completion_tokens: 32,
            ..UsageStats::zero()
        };
        assert_eq!(
            token_status_with_latest(&total, Some(&latest), TokenStatusMode::Thinking),
            "Token: ▲4.4K(+2K) ▼56(+32)"
        );
        assert_eq!(
            token_status_with_latest(&total, Some(&latest), TokenStatusMode::Final),
            "Token [ctx 2K] ▲4.4K ▼56"
        );
    }

    #[test]
    fn token_status_groups_cache_creation_as_kvc() {
        let total = UsageStats {
            prompt_tokens: 4_900,
            completion_tokens: 39,
            cache_created_tokens: 4_900,
            ..UsageStats::zero()
        };
        let latest = UsageStats {
            prompt_tokens: 4_900,
            completion_tokens: 39,
            cache_created_tokens: 4_900,
            ..UsageStats::zero()
        };
        let view = runtime_token_status_view(&total, Some(&latest), 100_000, 0);
        assert_eq!(
            compact_token_totals(&view.total),
            "▲4.9K | ▼39 | KVC(✚4.9K)"
        );
        assert_eq!(
            compact_token_latest(view.latest.as_ref().unwrap()),
            "△4.9K  ▽39  KVC(✚4.9K)"
        );
        assert_eq!(
            final_status_line(&total, Some(&latest), "aliyun", "qwen-plus", 1, 100_000),
            " ↳  1s    aliyun:qwen-plus ⇌0 ║ ctx[5%]  ▲4.9K  ▼39  KVC(✚4.9K)"
        );
    }

    #[test]
    fn token_status_uses_pending_request_as_current_when_total_is_zero() {
        let pending = UsageStats {
            prompt_tokens: 5_000,
            ..UsageStats::zero()
        };
        assert_eq!(
            token_status_with_latest(
                &UsageStats::zero(),
                Some(&pending),
                TokenStatusMode::Thinking
            ),
            "Token: ▲5K ▼0"
        );
        assert!(!token_status_with_latest(
            &UsageStats::zero(),
            Some(&pending),
            TokenStatusMode::Thinking
        )
        .contains("▲0(+5K)"));
    }

    #[test]
    fn final_token_status_does_not_show_latest_output_delta() {
        let rendered = render_final_response_at(
            "Hi!",
            &UsageStats {
                llm_calls: 1,
                prompt_tokens: 5_100,
                completion_tokens: 45,
                ..UsageStats::zero()
            },
            Some(&UsageStats {
                prompt_tokens: 5_100,
                completion_tokens: 45,
                ..UsageStats::zero()
            }),
            "custom",
            "aws-claude-sonnet-4-6",
            2,
            100_000,
            "09:24:00",
        );
        assert!(rendered.contains("custom:aws-claude-sonnet-4-6 ⇌1 ║ ctx[6%]  ▲5.1K  ▼45"));
        assert!(!rendered.contains("▼45(+45)"));
    }

    #[test]
    fn thinking_block_visual_contract() {
        let block = render_thinking_block_at(
            &ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "查询记忆".into(),
                memory_activity: CoreMemoryActivity::Read,
                model_round: 2,
                direction: ModelDirection::Downstream,
                usage: UsageStats {
                    prompt_tokens: 210,
                    completion_tokens: 21,
                    cached_tokens: 0,
                    ..UsageStats::zero()
                },
                latest_usage: Some(UsageStats {
                    prompt_tokens: 110,
                    completion_tokens: 9,
                    ..UsageStats::zero()
                }),
                tick: 0,
                elapsed_secs: 7,
                max_llm_input_tokens: 100_000,
                retry: None,
            },
            "08:56:33",
        );
        assert!(block.contains("[08:56:33] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
        assert!(block.contains("🦩 ◂⛃ 查询记忆..."));
        assert!(block.contains("aliyun:qwen-plus ⇌2 ║ ▲210 | ▼21"));
        assert!(block.contains("├─ context : ▰▱▱▱▱▱▱▱▱▱"));
        assert!(block.contains("└─ △110  ▽9"));
        assert!(!block.contains("已用 7s"));
        assert!(!block.contains("⚡cache"));
        assert_eq!(block.lines().count(), 5);
        assert!(!block.contains("thinking..."));
    }

    #[test]
    fn thinking_block_compacts_long_model_intent_to_two_lines() {
        let block = render_thinking_block_at(
            &ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "Check local system date and calendar to verify current date context and compute June 12 significance (e.g., holiday, observance, personal memory).".into(),
                memory_activity: CoreMemoryActivity::None,
                model_round: 1,
                direction: ModelDirection::Upstream,
                usage: UsageStats::zero(),
                latest_usage: Some(UsageStats {
                    prompt_tokens: 812,
                    ..UsageStats::zero()
                }),
                tick: 8,
                elapsed_secs: 65,
                max_llm_input_tokens: 100_000,
                retry: None,
            },
            "23:33:05",
        );

        assert_eq!(block.lines().count(), 5);
        assert!(block.contains("Check local system"));
        assert!(block.contains("├─ context : ▰▱▱▱▱▱▱▱▱▱"));
        assert!(block.contains('…'));
        assert!(!block.contains("observance"));
    }

    #[test]
    fn thinking_view_renders_observation_panel_and_status_line() {
        let mut observations = ObservationPanel::new(8, 60);
        observations.apply(ObservationEvent::Persistent("正在分析用户请求".into()));
        observations.apply(ObservationEvent::Active("rg --files | wc -l".into()));
        let view = render_thinking_view_at(
            &ThinkingViewSnapshot {
                status: ShellStatusSnapshot {
                    provider: "aliyun".into(),
                    model: "qwen-plus".into(),
                    intent: "ignored in panel mode".into(),
                    memory_activity: CoreMemoryActivity::None,
                    model_round: 2,
                    direction: ModelDirection::Downstream,
                    usage: UsageStats {
                        prompt_tokens: 1200,
                        completion_tokens: 20,
                        cached_tokens: 300,
                        ..UsageStats::zero()
                    },
                    latest_usage: Some(UsageStats {
                        prompt_tokens: 800,
                        completion_tokens: 12,
                        ..UsageStats::zero()
                    }),
                    tick: 0,
                    elapsed_secs: 12,
                    max_llm_input_tokens: 100_000,
                    retry: None,
                },
                observations,
            },
            "12:00:00",
        );

        assert!(view.contains("[12:00:00] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
        assert!(view.contains("Thought / Action"));
        assert!(view.contains("Thought / Action  ⏳ 00:12"));
        assert!(view.contains("· 正在分析用户请求"));
        assert!(view.contains("\x1b[38;5;245m· rg --files | wc -l"));
        assert!(view.contains("aliyun:qwen-plus ⇌2 ║ ▲1.2K | ▼20 | KVC(⌁300)"));
        assert!(view.contains("├─ context : ▰▱▱▱▱▱▱▱▱▱"));
        assert!(view.contains("└─ △800  ▽12"));
        assert!(!view.contains("已用 12s"));
        assert!(!view.contains("ignored in panel mode"));
    }

    #[test]
    fn multi_worker_thinking_view_keeps_identity_and_bounded_layout() {
        fn worker_snapshot(
            model_round: u32,
            repair_calls: u32,
            tick: usize,
            progress: &str,
            command: &str,
        ) -> ThinkingViewSnapshot {
            let mut observations = ObservationPanel::new(20, 84);
            observations.apply(ObservationEvent::Persistent(format!("⚙️ {progress}")));
            observations.apply(ObservationEvent::Persistent(
                "整理任务现场：保留用户目标、当前进度、下一步，不展示模型私有 thought。".into(),
            ));
            observations.apply(ObservationEvent::ActiveChild {
                text: format!("{command}"),
                is_last: true,
            });
            observations.apply(ObservationEvent::Transient("思考中...".into()));
            ThinkingViewSnapshot {
                status: ShellStatusSnapshot {
                    provider: "aliyun".into(),
                    model: "qwen-plus".into(),
                    intent: "private model thought should not render".into(),
                    memory_activity: CoreMemoryActivity::None,
                    model_round,
                    direction: ModelDirection::Upstream,
                    usage: UsageStats {
                        repair_calls,
                        prompt_tokens: 85_000,
                        completion_tokens: 3_500,
                        cached_tokens: 53_900,
                        cache_created_tokens: 4_900,
                        ..UsageStats::zero()
                    },
                    latest_usage: Some(UsageStats {
                        prompt_tokens: 5_800,
                        completion_tokens: 123,
                        cached_tokens: 3_900,
                        ..UsageStats::zero()
                    }),
                    tick,
                    elapsed_secs: 80 + u64::from(model_round),
                    max_llm_input_tokens: 100_000,
                    retry: None,
                },
                observations,
            }
        }

        let ai1 = worker_snapshot(
            12,
            3,
            0,
            "正在做 5 worker / 30 turn 的压力回放，并检查 UI 是否在长进度下保持稳定。",
            "cargo test -p agent_core session_workers_stress_ui_threads_supplements_and_renames -- --nocapture",
        );
        let ai2 = worker_snapshot(
            22,
            0,
            1,
            "正在处理超长 action：命令会被折行展示，但不能撑破窗口或重复刷屏。",
            "printf '%s' 'very-long-command-with-cjk-参数-参数-参数-参数-参数-参数-参数-参数' && wc -c target/output.log",
        );
        let ai3 = worker_snapshot(
            7,
            1,
            2,
            "正在等待补充输入合入当前 turn，worker 身份必须一直可见，避免多 session 串台。",
            "rg -n 'user_supplement|CoreTopicEvent|TopicReply' agent_core timem_shell",
        );

        let rendered = render_worker_thinking_views_at(
            &[("ID0", &ai1), ("ID1", &ai2), ("Review", &ai3)],
            "09:30:00",
        );

        assert!(rendered.contains("[09:30:00] 𝓣𝓲𝓶𝓮𝓶 ID0  ⬇"));
        assert!(rendered.contains("[09:30:00] 𝓣𝓲𝓶𝓮𝓶 ID1  ⬇"));
        assert!(rendered.contains("[09:30:00] 𝓣𝓲𝓶𝓮𝓶 Review  ⬇"));
        assert_eq!(rendered.matches("Thought / Action  ⏳").count(), 3);
        assert_eq!(rendered.matches("思考中...").count(), 3);
        assert!(rendered.contains("aliyun:qwen-plus ⇌12 (⚠3)"));
        assert!(rendered.contains("KVC(⌁53.9K ✚4.9K)"));
        assert!(rendered.contains("└─ cargo test -p agent_core"));
        assert!(rendered.contains("└─ printf"));
        assert!(rendered.contains("└─ rg -n"));
        assert!(!rendered.contains("private model thought"));
        assert!(!rendered.contains("run_bash"));

        for line in rendered.lines() {
            assert!(
                visible_width_for_test(line) <= 110,
                "line too wide ({}): {line}\n{rendered}",
                visible_width_for_test(line)
            );
        }
    }

    #[test]
    fn thinking_status_line_shows_retry_from_structured_fields() {
        let view = render_thinking_view_at(
            &ThinkingViewSnapshot {
                status: ShellStatusSnapshot {
                    provider: "aliyun".into(),
                    model: "qwen-plus".into(),
                    intent: "ignored in panel mode".into(),
                    memory_activity: CoreMemoryActivity::None,
                    model_round: 1,
                    direction: ModelDirection::Upstream,
                    usage: UsageStats::zero(),
                    latest_usage: None,
                    tick: 0,
                    elapsed_secs: 3,
                    max_llm_input_tokens: 100_000,
                    retry: Some(RuntimeRetryStatus {
                        until_epoch_ms: Some(current_epoch_ms() + 10_000),
                        error: None,
                        attempt: Some(1),
                        max_attempts: Some(5),
                    }),
                },
                observations: ObservationPanel::default(),
            },
            "12:00:00",
        );

        assert!(view.contains("├─ context : ▱▱▱▱▱▱▱▱▱▱"));
        assert!(view.contains("├─ △0  ▽0"));
        assert!(view.contains("└─ 网络错误，10s 后重试（第1/5次）"));
    }

    #[test]
    fn thinking_status_line_compacts_long_retry_detail_to_one_line() {
        let long_error = "provider_network_error: curl: (16) Error in the HTTP2 framing layer while reading response headers from upstream gateway after a long timeout";
        let view = render_thinking_view_at(
            &ThinkingViewSnapshot {
                status: ShellStatusSnapshot {
                    provider: "aliyun".into(),
                    model: "qwen-plus".into(),
                    intent: String::new(),
                    memory_activity: CoreMemoryActivity::None,
                    model_round: 1,
                    direction: ModelDirection::Upstream,
                    usage: UsageStats::zero(),
                    latest_usage: None,
                    tick: 0,
                    elapsed_secs: 3,
                    max_llm_input_tokens: 100_000,
                    retry: Some(RuntimeRetryStatus {
                        until_epoch_ms: Some(current_epoch_ms() + 10_000),
                        error: Some(long_error.into()),
                        attempt: Some(1),
                        max_attempts: Some(5),
                    }),
                },
                observations: ObservationPanel::default(),
            },
            "12:00:00",
        );

        let retry_lines: Vec<_> = view
            .lines()
            .filter(|line| line.contains("详情：provider_network_error"))
            .collect();
        assert_eq!(retry_lines.len(), 1, "{view}");
        assert!(retry_lines[0].contains('…'), "{view}");
        assert!(retry_lines[0].chars().count() < 120, "{view}");
        assert!(!view.contains("reading response headers from upstream gateway"));
    }

    #[test]
    fn thinking_status_line_shows_network_retry_countdown_and_detail_line() {
        let view = render_thinking_view_at(
            &ThinkingViewSnapshot {
                status: ShellStatusSnapshot {
                    provider: "aliyun".into(),
                    model: "qwen-plus".into(),
                    intent: String::new(),
                    memory_activity: CoreMemoryActivity::None,
                    model_round: 1,
                    direction: ModelDirection::Upstream,
                    usage: UsageStats::zero(),
                    latest_usage: None,
                    tick: 0,
                    elapsed_secs: 3,
                    max_llm_input_tokens: 100_000,
                    retry: Some(RuntimeRetryStatus {
                        until_epoch_ms: Some(current_epoch_ms() + 10_000),
                        error: Some(
                            "provider_network_error: curl: (16) Error in the HTTP2 framing layer while reading response headers from upstream gateway"
                                .into(),
                        ),
                        attempt: Some(1),
                        max_attempts: Some(5),
                    }),
                },
                observations: ObservationPanel::default(),
            },
            "12:00:00",
        );

        assert!(
            view.contains("├─ 网络错误，10s 后重试（第1/5次）"),
            "{view}"
        );
        assert!(view.contains("└─ 详情：provider_network_error"), "{view}");
        assert!(!view.contains("网络错误，10s 后重试（第1次）"), "{view}");
        assert!(!view.contains("reading response headers from upstream gateway"));
    }

    #[test]
    fn retry_status_renderer_consumes_core_retry_view() {
        let lines = retry_status_lines_from_view(&RuntimeRetryStatusView {
            remaining_secs: 7,
            attempt: 2,
            max_attempts: 5,
            error: Some("provider_timeout: upstream gateway timed out".to_string()),
        });

        assert_eq!(lines[0], "  ├─ 网络错误，7s 后重试（第2/5次）");
        assert_eq!(
            lines[1],
            "  └─ 详情：provider_timeout: upstream gateway timed out"
        );
    }

    #[test]
    fn thinking_status_line_shows_repair_call_count_when_present() {
        let view = render_thinking_view_at(
            &ThinkingViewSnapshot {
                status: ShellStatusSnapshot {
                    provider: "custom".into(),
                    model: "aws-claude-sonnet-4-6".into(),
                    intent: "ignored in panel mode".into(),
                    memory_activity: CoreMemoryActivity::None,
                    model_round: 13,
                    direction: ModelDirection::Upstream,
                    usage: UsageStats {
                        repair_calls: 3,
                        prompt_tokens: 85_000,
                        completion_tokens: 3_500,
                        cached_tokens: 53_900,
                        ..UsageStats::zero()
                    },
                    latest_usage: Some(UsageStats {
                        prompt_tokens: 5_800,
                        ..UsageStats::zero()
                    }),
                    tick: 0,
                    elapsed_secs: 80,
                    max_llm_input_tokens: 100_000,
                    retry: None,
                },
                observations: ObservationPanel::default(),
            },
            "12:00:00",
        );

        assert!(view.contains("custom:aws-claude-sonnet-4-6 ⇌13 (⚠3) ║ ▲85K | ▼3.5K | KVC(⌁53.9K)"));
    }

    #[test]
    fn final_response_visual_contract() {
        let rendered = render_final_response_at(
            "测试代号是 ALPHA-42。",
            &UsageStats {
                llm_calls: 2,
                mem_reads: 1,
                mem_writes: 1,
                prompt_tokens: 812,
                completion_tokens: 52,
                cached_tokens: 384,
                ..UsageStats::zero()
            },
            Some(&UsageStats {
                prompt_tokens: 410,
                completion_tokens: 31,
                ..UsageStats::zero()
            }),
            "aliyun",
            "qwen-plus",
            2,
            100_000,
            "08:56:46",
        );
        assert!(rendered.contains("[08:56:46] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
        assert!(rendered.contains("\x1b[92;1m"));
        assert!(rendered.contains("𝓣𝓲𝓶𝓮𝓶  ⬇"));
        assert!(rendered
            .lines()
            .nth(1)
            .is_some_and(|line| line == "测试代号是 ALPHA-42。"));
        assert!(rendered.contains("测试代号是 ALPHA-42。"));
        assert!(rendered.contains("aliyun:qwen-plus ⇌2 ║ ctx[1%]  ▲812  ▼52  KVC(⌁384)"));
        assert!(!rendered.contains("▼52(+31)"));
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
            None,
            "custom",
            "aws-claude-sonnet-4-6",
            1,
            100_000,
            "17:20:00",
        );
        assert!(rendered.contains(&format!("{ANSI_BOLD}系统{ANSI_RESET}：macOS")));
        assert!(!rendered.contains("**系统**"));
    }

    #[test]
    fn final_response_renders_common_markdown_shapes() {
        let rendered = render_final_response_at(
            "# 结论\n> 关键观察\n\n运行 `cargo test`：\n```text\nok 12 passed\n```",
            &UsageStats {
                llm_calls: 1,
                prompt_tokens: 10,
                completion_tokens: 2,
                ..UsageStats::zero()
            },
            None,
            "custom",
            "qwen-plus",
            1,
            100_000,
            "17:20:00",
        );

        assert!(rendered.contains("结论"));
        assert!(rendered.contains("关键观察"));
        assert!(rendered.contains("cargo test"));
        assert!(rendered.contains("ok 12 passed"));
        assert!(!rendered.contains("# 结论"));
        assert!(!rendered.contains("```text"));
        assert!(!rendered.contains("`cargo test`"));
    }

    #[test]
    fn final_response_markdown_renderer_resets_unclosed_inline_styles() {
        let rendered = render_terminal_markdown("先 `code\n再 **bold");
        assert!(rendered.contains("code"));
        assert!(rendered.contains("bold"));
        assert!(!rendered.contains("**bold"));
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
            None,
            "aliyun",
            "qwen-plus",
            1,
            100_000,
            "10:00:00",
        );
        let status_line = rendered.lines().nth(2).unwrap();
        assert!(status_line.starts_with(&format!("{ANSI_RESET}{ANSI_DIM}")));
        assert!(status_line.ends_with(ANSI_RESET));
        assert!(status_line.contains("↳  1s"));
    }

    #[test]
    fn shell_status_bar_is_dim_wrapped_and_extensible() {
        let rendered = render_shell_status_bar(&HostStatusMessage {
            level: HostStatusLevel::Info,
            text: "已取消当前输入。Ctrl+D 退出。".to_string(),
        });
        assert!(rendered.starts_with(&format!("{ANSI_RESET}{ANSI_DIM}")));
        assert!(rendered.ends_with(ANSI_RESET));
        assert!(rendered.contains("ⓘ 已取消当前输入。Ctrl+D 退出。"));

        let warning = render_shell_status_bar(&HostStatusMessage {
            level: HostStatusLevel::Warning,
            text: "状态异常".to_string(),
        });
        assert!(warning.contains("! 状态异常"));

        let error = render_shell_status_bar(&HostStatusMessage {
            level: HostStatusLevel::Error,
            text: "状态失败".to_string(),
        });
        assert!(error.contains("× 状态失败"));
    }

    #[test]
    fn shell_renders_core_lifecycle_topic_as_startup_status() {
        let profile = agent_core::CoreProfile {
            name: "test".to_string(),
            provider: "aliyun".to_string(),
            model: "qwen-plus".to_string(),
        };
        let event = agent_core::core_initialized_topic_event(
            "session_a",
            &profile,
            "xml",
            100_000,
            50,
            6,
            0,
        );

        let message = shell_status_message_from_core_topic(&event)
            .expect("shell should understand core lifecycle topic");
        assert_eq!(message.level, HostStatusLevel::Info);
        assert!(message.text.contains("Timem Core 启动成功"));
        assert!(message.text.contains("aliyun:qwen-plus"));
        assert!(message.text.contains("response protocol=xml"));
        assert!(message.text.contains("tools=6"));

        let rendered = render_shell_status_bar(&message);
        assert!(rendered.contains("ⓘ"));
        assert!(rendered.contains("Timem Core 启动成功"));
    }

    #[test]
    fn shell_renders_work_instruction_load_topic_as_status() {
        let report = agent_core::WorkInstructionLoadReport {
            status: agent_core::WorkInstructionLoadStatus::Loaded,
            directory: "/tmp/project".into(),
            file_names: vec!["AGENTS.md".to_string()],
            context: Some("guide".to_string()),
            error: None,
        };
        let event = agent_core::work_instruction_load_topic_event("session_a", &report);

        let message = shell_status_message_from_core_topic(&event)
            .expect("shell should understand work instruction status topic");
        assert_eq!(message.level, HostStatusLevel::Info);
        assert_eq!(message.text, "已加载当前工作目录指令：AGENTS.md");

        let rendered = render_shell_status_bar(&message);
        assert!(rendered.contains("ⓘ"));
        assert!(rendered.contains("已加载当前工作目录指令：AGENTS.md"));
    }

    #[test]
    fn shell_renders_worker_identity_from_lifecycle_topic() {
        let profile = agent_core::CoreProfile {
            name: "test".to_string(),
            provider: "local".to_string(),
            model: "fake-model".to_string(),
        };
        let identity = agent_core::CoreSessionWorkerIdentity::new(
            "session_worker",
            4,
            Some("日志分析".to_string()),
            Some("parent".to_string()),
        );
        let event = agent_core::core_initialized_topic_event_with_worker(
            "session_worker",
            &profile,
            "markdown",
            100_000,
            50,
            6,
            0,
            Some(&identity),
            None,
            None,
        );

        let message = shell_status_message_from_core_topic(&event)
            .expect("shell should render worker lifecycle topic");
        assert!(message.text.contains("Timem Core 日志分析 启动成功"));
        assert!(message.text.contains("local:fake-model"));
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
            None,
            "aliyun",
            "qwen-plus",
            1,
            100_000,
            "10:08:43",
        );
        assert!(rendered.contains("aliyun:qwen-plus ⇌1 ║ ▲237  ▼26"));
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
