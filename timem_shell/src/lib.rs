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
    runtime_config_report, runtime_info_context, runtime_profile_report, runtime_retry_status_view,
    runtime_time_context, runtime_token_status_view, stale_context_decision_request,
    stale_context_prompt_needed, supporting_context, topic_event_status_hint,
    work_instruction_load_report, work_instruction_load_request, work_instruction_load_topic_event,
    work_instruction_mode_from_sources, workspace_config_file, workspace_menu_report,
    workspace_reference_context, ApiProtocol, CapabilityHostProfile, CoreActionTopic,
    CoreLifecycleEvent, CoreLifecycleTopic, CoreMemoryActivity, CoreModelResponseTopic,
    CoreTopicEvent, HostDecision, HostDecisionDefault, HostDecisionRequest, HostStatusLevel,
    HostStatusMessage, LocalLLMKeyFile, LongRunningCommandContinueRequest, ModelDirection,
    ModelProfile, NoopTurnUi, OutputExpansionRequest, ProviderConfig, RoundLimitDecisionRequest,
    RunningShellJob, RuntimeConfigApplyError, RuntimeConfigApplyMessage,
    RuntimeConfigApplyMessageKind, RuntimeConfigApplyReport, RuntimeConfigEffect,
    RuntimeConfigField, RuntimeConfigMenuItem, RuntimeConfigMenuReport, RuntimeConfigReport,
    RuntimeConfigReportInput, RuntimeConfigReportItem, RuntimeConfigReportRow,
    RuntimeConfigRowKind, RuntimeConfigSection, RuntimeProfiler, RuntimeRetryStatus,
    RuntimeRetryStatusView, RuntimeTokenStatusView, StaleContextDecisionRequest, StorageProfile,
    SupportingContextInput, TokenUsageBreakdown, TopicReply, TopicReplyError, TurnInput,
    TurnOutcome, TurnStopDetail, TurnStopReason, TurnStopSummary, TurnUi,
    WorkInstructionLoadMessage, WorkInstructionLoadMessageKind, WorkInstructionLoadMode,
    WorkInstructionLoadReport, WorkInstructionLoadRequest, WorkInstructionLoadStatus,
    WorkspaceChange, WorkspaceCommand, WorkspaceCommandMessage, WorkspaceCommandMessageKind,
    WorkspaceCommandOutcome, WorkspaceCommandReport, WorkspaceMenuReport, WorkspaceState,
    WorkspaceUnchangedReason, CORE_TOPIC_ACTION, CORE_TOPIC_MODEL_RESPONSE,
    DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT, DEFAULT_STALE_CONTEXT_IDLE,
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
    if value.checked_rem(1_000) == Some(0) && value >= 1_000 {
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
    let mut text = outcome
        .stop_summary
        .as_ref()
        .map(render_turn_stop_summary)
        .unwrap_or_else(|| outcome.text.clone());
    if !outcome.running_jobs.is_empty() {
        if !text.trim().is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&render_running_jobs_for_user(&outcome.running_jobs));
    }
    text
}

pub fn render_running_jobs_for_user(jobs: &[RunningShellJob]) -> String {
    let mut out = String::from("RUNNING JOB LIST:\n");
    for job in jobs {
        out.push_str(&format!(
            "- pid={}, {}, cmd={}, still running\n",
            job.pid,
            job.description(),
            job.command
        ));
    }
    out.trim_end().to_string()
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
    if view.latest.is_some() && view.context_percent > 0 {
        parts.push(format!("ctx[{}%]", view.context_percent));
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
            enable_thinking: None,
            reasoning_effort: None,
            stream: None,
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
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
