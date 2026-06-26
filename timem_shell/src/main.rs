use agent_core::{AgentCore, ApprovalRequest, BashApprovalMode, CoreStep, UsageStats};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use timem_shell::{
    action_audit_path, action_status_hint, append_audit, audit_path, call_model, data_root,
    local_time_label, memory_path, observation_events_from_model_response, parse_cli_args,
    provider_config_from_env, render_final_response_at, render_prof_report,
    render_shell_status_bar, render_thinking_view_at, supporting_context, ApiProtocol,
    ModelDirection, ObservationEvent, ObservationPanel, RuntimeProfiler, ShellStatusMessage,
    ShellStatusSnapshot, ShellStatusTone, ThinkingViewSnapshot, SPINNER_ICONS, TIMEM_LOGO,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const STATIC_PROMPT: &str = include_str!("../../resources/static_v1.json");
const ANSI_RESET: &str = timem_shell::ANSI_RESET;
const ANSI_BOLD: &str = timem_shell::ANSI_BOLD;
const ANSI_HIGHLIGHT: &str = "\x1b[1;33m";
static TURN_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
const STALE_CONTEXT_IDLE: Duration = Duration::from_secs(3 * 60 * 60);
const STALE_CONTEXT_TOKEN_THRESHOLD: u32 = 10_000;

struct ConfigRow {
    key: String,
    value: String,
    desc: &'static str,
    highlight: bool,
}

enum ConfigTableItem {
    Section(&'static str),
    Row(ConfigRow),
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return;
    }
    let options = parse_cli_args(&args);
    if let Some(data_dir) = options.data_dir.as_deref() {
        std::env::set_var("TIMEM_DATA_DIR", data_dir);
    }
    let env: HashMap<String, String> = std::env::vars().collect();
    let mut config = match provider_config_from_env(&options, &env) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("[config_error] {err}");
            std::process::exit(2);
        }
    };
    let space = options
        .space
        .clone()
        .or_else(|| env.get("TIMEM_SPACE").cloned())
        .unwrap_or_else(|| ".test_mem".to_string());
    let audit_file = audit_path(&space);
    let action_audit_file = action_audit_path(&space);
    let memory_dir = memory_path(&space);
    let mut bash_approval_mode = bash_approval_mode_from_options(&options, &env);
    let mut profiler = RuntimeProfiler::default();
    let mut core = AgentCore::new(STATIC_PROMPT, config.core_profile(), &memory_dir);
    core.set_bash_approval_mode(bash_approval_mode);
    core.set_max_llm_input_tokens(config.max_llm_input_tokens);
    let session = session_id();

    if let Some(input) = options.once_json_input.as_deref() {
        let context = options.supporting_context.as_deref();
        let (text, stats, elapsed) = run_turn(
            &mut core,
            input,
            &session,
            &audit_file,
            &mut config,
            context,
            None,
            false,
            Some(&mut profiler),
        );
        println!(
            "{}",
            json!({
                "output": text,
                "session_id": session,
                "stats": stats,
                "status": "done",
                "elapsed_ms": elapsed.as_millis()
            })
        );
        return;
    }

    let _ = append_audit(
        &audit_file,
        &json!({
            "type":"shell_start",
            "session":session,
            "space":space,
            "gateway_provider":config.provider,
            "provider":config.provider,
            "base_url":config.base_url,
            "api_protocol":config.api_protocol.label(),
            "model":config.model,
            "max_llm_input_tokens":config.max_llm_input_tokens,
            "bash_approval":bash_approval_mode_label(bash_approval_mode)
        }),
    );

    println!("Timem native shell");
    print!(
        "{}",
        render_startup_banner(
            &space,
            &config,
            &audit_file,
            &action_audit_file,
            bash_approval_mode,
        )
    );
    println!("输入 /prof 查看运行 profiling；输入 /exit 退出；Ctrl+C 取消当前输入。\n");

    let history_file = audit_file.with_file_name("shell_history.txt");
    let mut editor = ShellLineEditor::new(history_file);
    let mut prompt_status = PromptStatusBar::default();
    let mut last_dialog_activity = Instant::now();

    loop {
        let prompt = render_user_input_prompt(&time_label());
        let (input, submitted_display) = match editor.readline(&prompt) {
            ShellReadline::Line { text, display } => (text, display),
            ShellReadline::Interrupted => {
                prompt_status.show_info("已取消当前输入。Ctrl+D 退出。");
                continue;
            }
            ShellReadline::Eof => {
                prompt_status.clear_before_exit();
                println!("Bye.");
                break;
            }
            ShellReadline::Error(err) => {
                eprintln!("[input_error] {err}");
                break;
            }
        };
        let input = sanitize_user_input(&input).trim().to_string();
        if input.is_empty() {
            prompt_status.clear_after_empty_input();
            continue;
        }
        if input == "/exit" || input == "/quit" {
            println!("Bye.");
            break;
        }
        if input == "/prof" {
            prompt_status.clear_before_exit();
            println!(
                "{}",
                render_prof_report(&profiler, &memory_dir, &audit_file, &action_audit_file)
            );
            continue;
        }
        if input == "/config" {
            prompt_status.clear_before_exit();
            if run_config_menu(&mut config, &mut core, &mut bash_approval_mode) {
                println!(
                    "{}",
                    render_startup_banner(
                        &space,
                        &config,
                        &audit_file,
                        &action_audit_file,
                        bash_approval_mode,
                    )
                );
            }
            continue;
        }

        rewrite_submitted_user_line(&submitted_display, prompt_status.take_visible());

        let idle = last_dialog_activity.elapsed();
        let dynamic_context_tokens = core.dynamic_context_estimated_tokens();
        if stale_context_prompt_needed(idle, dynamic_context_tokens) {
            let continue_old_context = request_stale_context_continue(idle, dynamic_context_tokens);
            let _ = append_audit(
                &audit_file,
                &json!({
                    "type":"stale_context_choice",
                    "session":session,
                    "idle_secs":idle.as_secs(),
                    "dynamic_context_tokens":dynamic_context_tokens,
                    "continue_old_context":continue_old_context
                }),
            );
            if !continue_old_context {
                core.clear_dynamic_context();
            }
        }

        let mut status = ThinkingStatus::start(&config.provider, &config.model);
        let (text, stats, elapsed) = run_turn(
            &mut core,
            &input,
            &session,
            &audit_file,
            &mut config,
            None,
            Some(&mut status),
            true,
            Some(&mut profiler),
        );
        status.finish();
        print_final_response(&text, &stats, &config.provider, &config.model, elapsed);
        last_dialog_activity = Instant::now();
    }
}

fn run_turn(
    core: &mut AgentCore,
    input: &str,
    session: &str,
    audit_file: &std::path::Path,
    config: &mut timem_shell::ProviderConfig,
    additional_context: Option<&str>,
    mut status: Option<&mut ThinkingStatus>,
    interactive_approval: bool,
    mut profiler: Option<&mut RuntimeProfiler>,
) -> (String, UsageStats, Duration) {
    TURN_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    let _sigint_guard = if status.is_some() {
        SigintGuard::install()
    } else {
        None
    };
    let turn_id = format!("turn_{}", epoch_millis());
    let _ = append_audit(
        audit_file,
        &json!({"type":"turn_start","session":session,"turn_id":turn_id,"user_input":input}),
    );
    let start = Instant::now();
    let mut context = supporting_context(&config.provider, &config.model, input);
    if let Some(extra) = additional_context
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        context.push_str("\n\n");
        context.push_str(extra);
    }
    let mut step = core.begin_turn(input, Some(&context));
    let mut rounds = 0u32;
    let mut model_wait_this_turn = Duration::ZERO;

    let (text, stats, repair_issue) = loop {
        if consume_turn_cancel_request() {
            break cancelled_turn_result();
        }
        match step {
            CoreStep::NeedModel { ref prompt, .. } => {
                rounds += 1;
                if let Some(status) = status.as_deref_mut() {
                    status.set_model_direction(rounds, ModelDirection::Upstream);
                    status.set_transient_observation("思考中...");
                }
                let model_wait_start = Instant::now();
                match call_model(config, &prompt, audit_file) {
                    Ok(response) => {
                        let model_wait = model_wait_start.elapsed();
                        model_wait_this_turn = model_wait_this_turn.saturating_add(model_wait);
                        if let Some(profiler) = profiler.as_deref_mut() {
                            profiler.record_model_wait(
                                &config.provider,
                                &response.model_name,
                                &response.usage,
                                model_wait,
                            );
                        }
                        if consume_turn_cancel_request() {
                            break cancelled_turn_result();
                        }
                        if response.truncated && interactive_approval {
                            if let Some(status) = status.as_deref_mut() {
                                status.clear_transient_observation();
                            }
                            if request_expand_output_tokens(config.max_llm_output_tokens) {
                                config.max_llm_output_tokens =
                                    config.max_llm_output_tokens.saturating_add(10_000);
                                let _ = append_audit(
                                    audit_file,
                                    &json!({
                                        "type":"max_llm_output_increased",
                                        "session":session,
                                        "turn_id":turn_id,
                                        "max_llm_output_tokens":config.max_llm_output_tokens
                                    }),
                                );
                                continue;
                            }
                            break (
                                format!(
                                    "模型输出达到当前上限 {}，已按你的选择停止本轮。可用 /config 调大 TIMEM_MAX_LLM_OUTPUT 后重试。",
                                    format_token_count(config.max_llm_output_tokens)
                                ),
                                response.usage,
                                Some("truncated_output_stopped_by_user".to_string()),
                            );
                        }
                        if let Some(status) = status.as_deref_mut() {
                            status.clear_transient_observation();
                            status.set_usage(response.usage.clone());
                            status.set_model_direction(rounds, ModelDirection::Downstream);
                            if let Some(hint) = action_status_hint(&response.content) {
                                status.set_intent(&hint.intent, &hint.memory_marker);
                            }
                            status.apply_observation_events(
                                observation_events_from_model_response(&response.content),
                            );
                        }
                        step = core.apply_model_response(response);
                        if let Some(status) = status.as_deref_mut() {
                            status.settle_active_observations();
                        }
                    }
                    Err(err) => {
                        let model_wait = model_wait_start.elapsed();
                        model_wait_this_turn = model_wait_this_turn.saturating_add(model_wait);
                        if let Some(profiler) = profiler.as_deref_mut() {
                            profiler.record_model_wait(
                                &config.provider,
                                &config.model,
                                &UsageStats::zero(),
                                model_wait,
                            );
                        }
                        if consume_turn_cancel_request() {
                            break cancelled_turn_result();
                        }
                        if let Some(status) = status.as_deref_mut() {
                            status.clear_transient_observation();
                        }
                        let _ = append_audit(
                            audit_file,
                            &json!({"type":"turn_error","session":session,"turn_id":turn_id,"error":err}),
                        );
                        break (format!("模型调用失败：{err}"), UsageStats::zero(), None);
                    }
                }
            }
            CoreStep::NeedsUserApproval { request } => {
                if let Some(status) = status.as_deref_mut() {
                    status.pause_for_user_approval();
                }
                let approved = if interactive_approval {
                    request_user_approval(&request)
                } else {
                    false
                };
                if consume_turn_cancel_request() {
                    step = core.resolve_user_approval(&request.approval_id, false);
                    continue;
                }
                let _ = append_audit(
                    audit_file,
                    &json!({
                        "type":"user_approval",
                        "session":session,
                        "turn_id":turn_id,
                        "approval_id":request.approval_id,
                        "action":request.action,
                        "command":request.command,
                        "risk":request.risk,
                        "reason":request.reason,
                        "approved":approved
                    }),
                );
                step = core.resolve_user_approval(&request.approval_id, approved);
                if let Some(status) = status.as_deref_mut() {
                    status.resume_after_user_approval();
                }
            }
            CoreStep::RoundLimitReached { max_rounds } => {
                if let Some(status) = status.as_deref_mut() {
                    status.pause_for_user_approval();
                }
                let should_continue =
                    interactive_approval && request_round_limit_continue(max_rounds);
                let _ = append_audit(
                    audit_file,
                    &json!({
                        "type":"round_limit",
                        "session":session,
                        "turn_id":turn_id,
                        "max_rounds":max_rounds,
                        "continued":should_continue
                    }),
                );
                if should_continue {
                    step = core.continue_after_round_limit();
                    if let Some(status) = status.as_deref_mut() {
                        status.resume_after_user_approval();
                    }
                } else {
                    break (
                        format!(
                            "已达到本轮最大交互次数 {max_rounds}，已停止。你可以继续输入来开启新一轮。"
                        ),
                        core.current_stats().clone(),
                        Some("round_limit_reached".to_string()),
                    );
                }
            }
            CoreStep::Final(turn) => {
                break (turn.response_to_user, turn.stats, turn.repair_issue);
            }
        }
    };
    let elapsed = start.elapsed();
    if let Some(profiler) = profiler.as_deref_mut() {
        profiler.record_turn(elapsed, model_wait_this_turn);
    }
    let _ = append_audit(
        audit_file,
        &json!({
            "type":"turn_final",
            "session":session,
            "turn_id":turn_id,
            "assistant_output":text,
            "stats":stats,
            "repair_issue":repair_issue,
            "elapsed_ms":elapsed.as_millis()
        }),
    );
    (text, stats, elapsed)
}

fn consume_turn_cancel_request() -> bool {
    TURN_CANCEL_REQUESTED.swap(false, Ordering::SeqCst)
}

fn cancelled_turn_result() -> (String, UsageStats, Option<String>) {
    (
        "已取消本轮。".to_string(),
        UsageStats::zero(),
        Some("cancelled_by_user".to_string()),
    )
}

struct ThinkingStatus {
    state: Arc<Mutex<ThinkingViewSnapshot>>,
    running: Arc<AtomicBool>,
    rendered_lines: Arc<Mutex<usize>>,
    handle: Option<JoinHandle<()>>,
}

impl ThinkingStatus {
    fn start(provider: &str, model: &str) -> Self {
        let state = Arc::new(Mutex::new(ThinkingViewSnapshot {
            status: ShellStatusSnapshot {
                provider: provider.to_string(),
                model: model.to_string(),
                intent: "思考中".to_string(),
                memory_marker: String::new(),
                model_round: 1,
                direction: ModelDirection::Upstream,
                usage: UsageStats::zero(),
                tick: random_spinner_tick(),
            },
            observations: ObservationPanel::default(),
        }));
        let running = Arc::new(AtomicBool::new(true));
        let rendered_lines = Arc::new(Mutex::new(0));
        render_thinking(&state.lock().unwrap(), &rendered_lines);
        let thread_state = Arc::clone(&state);
        let thread_running = Arc::clone(&running);
        let thread_rendered_lines = Arc::clone(&rendered_lines);
        let handle = thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1000));
                if let Ok(mut snapshot) = thread_state.lock() {
                    snapshot.status.tick = snapshot.status.tick.wrapping_add(1);
                    rerender_thinking(&snapshot, &thread_rendered_lines);
                }
            }
        });
        Self {
            state,
            running,
            rendered_lines,
            handle: Some(handle),
        }
    }

    fn set_model_direction(&mut self, round: u32, direction: ModelDirection) {
        if let Ok(mut state) = self.state.lock() {
            state.status.model_round = round;
            state.status.direction = direction;
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn set_usage(&mut self, usage: UsageStats) {
        if let Ok(mut state) = self.state.lock() {
            state.status.usage = usage;
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn set_intent(&mut self, intent: &str, memory_marker: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.status.intent = intent
                .trim_end_matches('…')
                .trim_end_matches("...")
                .to_string();
            state.status.memory_marker = memory_marker.to_string();
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn set_transient_observation(&mut self, text: &str) {
        if let Ok(mut state) = self.state.lock() {
            state
                .observations
                .apply(ObservationEvent::Transient(text.to_string()));
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn clear_transient_observation(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state
                .observations
                .apply(ObservationEvent::FinishTransient("思考中...".to_string()));
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn apply_observation_events(&mut self, events: Vec<ObservationEvent>) {
        if events.is_empty() {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.observations.apply_all(events);
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn settle_active_observations(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.observations.apply(ObservationEvent::SettleActive);
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn finish(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        clear_thinking_block(&self.rendered_lines);
    }

    fn pause_for_user_approval(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        clear_thinking_block(&self.rendered_lines);
    }

    fn resume_after_user_approval(&mut self) {
        if self.handle.is_some() {
            return;
        }
        self.running.store(true, Ordering::Relaxed);
        render_thinking(&self.state.lock().unwrap(), &self.rendered_lines);
        let thread_state = Arc::clone(&self.state);
        let thread_running = Arc::clone(&self.running);
        let thread_rendered_lines = Arc::clone(&self.rendered_lines);
        self.handle = Some(thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1000));
                if let Ok(mut snapshot) = thread_state.lock() {
                    snapshot.status.tick = snapshot.status.tick.wrapping_add(1);
                    rerender_thinking(&snapshot, &thread_rendered_lines);
                }
            }
        }));
    }
}

fn request_user_approval(request: &ApprovalRequest) -> bool {
    match choose_user_approval(request) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

fn request_round_limit_continue(max_rounds: u32) -> bool {
    match choose_round_limit_continue(max_rounds) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

fn request_expand_output_tokens(current: u32) -> bool {
    match choose_expand_output_tokens(current) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

fn request_stale_context_continue(idle: Duration, dynamic_context_tokens: u32) -> bool {
    match choose_stale_context_continue(idle, dynamic_context_tokens) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

fn stale_context_prompt_needed(idle: Duration, dynamic_context_tokens: u32) -> bool {
    idle >= STALE_CONTEXT_IDLE && dynamic_context_tokens > STALE_CONTEXT_TOKEN_THRESHOLD
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApprovalChoice {
    Allow,
    Deny,
}

fn render_user_approval_prompt(request: &ApprovalRequest) -> String {
    let intent_line = if request.intent.trim().is_empty() {
        String::new()
    } else {
        format!("  intent: {}\n", request.intent)
    };
    format!(
        "\n需要确认执行这个命令（超出低风险自动执行范围）。\n  command: {}\n{}使用 ←/→ 或 ↑/↓ 选择，回车确认。\n",
        request.command, intent_line
    )
}

fn render_approval_choices(selected: ApprovalChoice) -> String {
    let allow_label = "执行一次";
    let deny_label = "取消";
    match selected {
        ApprovalChoice::Allow => format!("\x1b[7m[ {} ]\x1b[0m   {}", allow_label, deny_label),
        ApprovalChoice::Deny => format!("  {}   \x1b[7m[ {} ]\x1b[0m", allow_label, deny_label),
    }
}

fn render_round_limit_prompt(max_rounds: u32) -> String {
    format!(
        "\n本轮已达到最大交互次数 {max_rounds}。\n继续后会为模型重新充值 rounds_remaining 为 20，当前任务上下文保持不变。\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n"
    )
}

fn render_round_limit_choices(selected: ApprovalChoice) -> String {
    let allow_label = "继续";
    let deny_label = "停止";
    match selected {
        ApprovalChoice::Allow => format!("\x1b[7m[ {} ]\x1b[0m   {}", allow_label, deny_label),
        ApprovalChoice::Deny => format!("  {}   \x1b[7m[ {} ]\x1b[0m", allow_label, deny_label),
    }
}

fn render_expand_output_prompt(current: u32) -> String {
    format!(
        "\n模型输出达到当前上限 {}，导致 JSON 被截断。\n是否将 TIMEM_MAX_LLM_OUTPUT 临时增加 10K 并自动重试本轮请求？\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n",
        format_token_count(current)
    )
}

fn render_expand_output_choices(selected: ApprovalChoice) -> String {
    let allow_label = "增加 10K 并重试";
    let deny_label = "停止";
    match selected {
        ApprovalChoice::Allow => format!("\x1b[7m[ {} ]\x1b[0m   {}", allow_label, deny_label),
        ApprovalChoice::Deny => format!("  {}   \x1b[7m[ {} ]\x1b[0m", allow_label, deny_label),
    }
}

fn render_stale_context_prompt(idle: Duration, dynamic_context_tokens: u32) -> String {
    format!(
        "\n距离上次对话已经过去 {}，当前旧任务上下文约 {} tokens。\n是否继续使用上次对话任务上下文？选择 NO 会清空旧动态上下文，从当前问题重新开始。\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n",
        format_idle_duration(idle),
        timem_shell::compact_count(dynamic_context_tokens)
    )
}

fn render_stale_context_choices(selected: ApprovalChoice) -> String {
    match selected {
        ApprovalChoice::Allow => "\x1b[7m[ YES ]\x1b[0m   NO".to_string(),
        ApprovalChoice::Deny => "  YES   \x1b[7m[ NO ]\x1b[0m".to_string(),
    }
}

fn format_idle_duration(duration: Duration) -> String {
    let total_minutes = duration.as_secs() / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 && minutes > 0 {
        format!("{hours} 小时 {minutes} 分钟")
    } else if hours > 0 {
        format!("{hours} 小时")
    } else {
        format!("{minutes} 分钟")
    }
}

fn render_paste_recovery_prompt(summary: &PasteRecoverySummary) -> String {
    format!(
        "\n检测到 {count} 个粘贴关联标签可能被误编辑，原始粘贴内容共 {lines} 行。\n是否恢复关联粘贴文本？选择 Yes 会提交原始粘贴内容；选择 No 会把已编辑标签当普通文本提交。\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n",
        count = summary.dirty_count,
        lines = summary.total_lines
    )
}

fn render_paste_recovery_choices(selected: ApprovalChoice) -> String {
    match selected {
        ApprovalChoice::Allow => "\x1b[7m[ Yes ]\x1b[0m   No".to_string(),
        ApprovalChoice::Deny => "  Yes   \x1b[7m[ No ]\x1b[0m".to_string(),
    }
}

fn choose_user_approval(request: &ApprovalRequest) -> ApprovalChoice {
    print!("{}", render_user_approval_prompt(request));
    choose_with_keyboard(render_approval_choices, ApprovalChoice::Deny)
}

fn choose_round_limit_continue(max_rounds: u32) -> ApprovalChoice {
    print!("{}", render_round_limit_prompt(max_rounds));
    choose_with_keyboard(render_round_limit_choices, ApprovalChoice::Allow)
}

fn choose_expand_output_tokens(current: u32) -> ApprovalChoice {
    print!("{}", render_expand_output_prompt(current));
    choose_with_keyboard(render_expand_output_choices, ApprovalChoice::Allow)
}

fn choose_stale_context_continue(idle: Duration, dynamic_context_tokens: u32) -> ApprovalChoice {
    print!(
        "{}",
        render_stale_context_prompt(idle, dynamic_context_tokens)
    );
    choose_with_keyboard(render_stale_context_choices, ApprovalChoice::Allow)
}

fn choose_paste_recovery(summary: &PasteRecoverySummary) -> ApprovalChoice {
    print!("{}", render_paste_recovery_prompt(summary));
    choose_with_keyboard(render_paste_recovery_choices, ApprovalChoice::Allow)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigField {
    Model,
    GatewayProvider,
    ApiProtocol,
    BaseUrl,
    MaxInput,
    MaxOutput,
    BashApproval,
}

impl ConfigField {
    fn label(self) -> &'static str {
        match self {
            ConfigField::Model => "TIMEM_MODEL",
            ConfigField::GatewayProvider => "TIMEM_GATEWAY_PROVIDER",
            ConfigField::ApiProtocol => "TIMEM_API_PROTOCOL",
            ConfigField::BaseUrl => "TIMEM_BASE_URL",
            ConfigField::MaxInput => "TIMEM_MAX_LLM_INPUT",
            ConfigField::MaxOutput => "TIMEM_MAX_LLM_OUTPUT",
            ConfigField::BashApproval => "TIMEM_BASH_APPROVAL",
        }
    }
}

const CONFIG_FIELDS: [ConfigField; 7] = [
    ConfigField::Model,
    ConfigField::GatewayProvider,
    ConfigField::ApiProtocol,
    ConfigField::BaseUrl,
    ConfigField::MaxInput,
    ConfigField::MaxOutput,
    ConfigField::BashApproval,
];

fn run_config_menu(
    config: &mut timem_shell::ProviderConfig,
    core: &mut AgentCore,
    bash_approval_mode: &mut BashApprovalMode,
) -> bool {
    let Some(field) = choose_config_field(config, *bash_approval_mode) else {
        println!("已取消配置修改。");
        return false;
    };
    let current = config_field_value(config, *bash_approval_mode, field);
    println!("\n{} 当前值：{}", field.label(), current);
    print!("请输入新值（留空取消）：");
    let _ = io::stdout().flush();
    let Some(raw_value) = read_tty_line() else {
        println!("读取输入失败，已取消。");
        return false;
    };
    let value = raw_value.trim();
    if value.is_empty() {
        println!("已取消配置修改。");
        return false;
    }
    match apply_config_value(config, core, bash_approval_mode, field, value) {
        Ok(()) => {
            println!("已更新 {}。", field.label());
            true
        }
        Err(err) => {
            println!("配置无效：{err}");
            false
        }
    }
}

fn choose_config_field(
    config: &timem_shell::ProviderConfig,
    bash_approval_mode: BashApprovalMode,
) -> Option<ConfigField> {
    println!("\n选择要修改的配置，使用 ↑/↓ 选择，回车确认，Esc/Ctrl+C 取消。\n");
    let mut input = ShellInputSource::open().ok()?;
    let fd = input.as_raw_fd();
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        return None;
    }
    let mut raw = original;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 1;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return None;
    }
    let mut terminal_mode = TerminalModeGuard::new(fd, original);
    let mut selected = 0usize;
    print!(
        "{}",
        render_config_menu(config, bash_approval_mode, selected)
    );
    let _ = io::stdout().flush();
    let result = loop {
        match read_menu_key(&mut input) {
            MenuKey::Up => selected = selected.saturating_sub(1),
            MenuKey::Down => selected = (selected + 1).min(CONFIG_FIELDS.len() - 1),
            MenuKey::Enter => break Some(CONFIG_FIELDS[selected]),
            MenuKey::Cancel => break None,
            MenuKey::Other => {}
        }
        print!(
            "\x1b[{}F{}",
            CONFIG_FIELDS.len(),
            render_config_menu(config, bash_approval_mode, selected)
        );
        let _ = io::stdout().flush();
    };
    terminal_mode.restore();
    println!();
    result
}

fn render_config_menu(
    config: &timem_shell::ProviderConfig,
    bash_approval_mode: BashApprovalMode,
    selected: usize,
) -> String {
    CONFIG_FIELDS
        .iter()
        .enumerate()
        .map(|(idx, field)| {
            let marker = if idx == selected { "▶" } else { " " };
            let line = format!(
                "{marker} {:<22} {}",
                field.label(),
                config_field_value(config, bash_approval_mode, *field)
            );
            if idx == selected {
                format!("\x1b[7m{line}\x1b[0m\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuKey {
    Up,
    Down,
    Enter,
    Cancel,
    Other,
}

fn read_menu_key(input: &mut impl Read) -> MenuKey {
    let mut buf = [0u8; 1];
    if input.read_exact(&mut buf).is_err() {
        return MenuKey::Cancel;
    }
    match buf[0] {
        b'\r' | b'\n' => MenuKey::Enter,
        3 | 4 => MenuKey::Cancel,
        27 => {
            let Some(seq) = read_escape_sequence(input) else {
                return MenuKey::Cancel;
            };
            match seq.as_slice() {
                [b'[', b'A'] => MenuKey::Up,
                [b'[', b'B'] => MenuKey::Down,
                _ => MenuKey::Cancel,
            }
        }
        b'k' => MenuKey::Up,
        b'j' => MenuKey::Down,
        _ => MenuKey::Other,
    }
}

fn config_field_value(
    config: &timem_shell::ProviderConfig,
    bash_approval_mode: BashApprovalMode,
    field: ConfigField,
) -> String {
    match field {
        ConfigField::Model => config.model.clone(),
        ConfigField::GatewayProvider => config.provider.clone(),
        ConfigField::ApiProtocol => config.api_protocol.label().to_string(),
        ConfigField::BaseUrl => config.base_url.clone(),
        ConfigField::MaxInput => format_token_count(config.max_llm_input_tokens),
        ConfigField::MaxOutput => format_token_count(config.max_llm_output_tokens),
        ConfigField::BashApproval => bash_approval_mode_label(bash_approval_mode).to_string(),
    }
}

fn apply_config_value(
    config: &mut timem_shell::ProviderConfig,
    core: &mut AgentCore,
    bash_approval_mode: &mut BashApprovalMode,
    field: ConfigField,
    value: &str,
) -> Result<(), String> {
    match field {
        ConfigField::Model => config.model = value.to_string(),
        ConfigField::GatewayProvider => config.provider = value.to_lowercase(),
        ConfigField::ApiProtocol => {
            config.api_protocol = parse_api_protocol_for_config(value)?;
        }
        ConfigField::BaseUrl => config.base_url = value.to_string(),
        ConfigField::MaxInput => {
            let tokens = timem_shell::parse_token_count(value)
                .ok_or_else(|| "请输入数字，或 100K/1M 这类格式".to_string())?;
            config.max_llm_input_tokens = tokens.max(3_000);
            core.set_max_llm_input_tokens(config.max_llm_input_tokens);
        }
        ConfigField::MaxOutput => {
            let tokens = timem_shell::parse_token_count(value)
                .ok_or_else(|| "请输入数字，或 10K 这类格式".to_string())?;
            config.max_llm_output_tokens = tokens.max(512);
        }
        ConfigField::BashApproval => {
            let mode = match value.trim().to_lowercase().as_str() {
                "approve" => BashApprovalMode::Approve,
                "ask" => BashApprovalMode::Ask,
                _ => return Err("bash 允许策略只能是 approve 或 ask".to_string()),
            };
            *bash_approval_mode = mode;
            core.set_bash_approval_mode(mode);
        }
    }
    Ok(())
}

fn parse_api_protocol_for_config(value: &str) -> Result<ApiProtocol, String> {
    match value.trim().to_lowercase().as_str() {
        "openai-compatible" | "openai_compatible" | "chat-completions" | "chat_completions" => {
            Ok(ApiProtocol::OpenAiCompatible)
        }
        "openai-responses" | "openai_responses" | "responses" => Ok(ApiProtocol::OpenAiResponses),
        "anthropic" | "claude" | "messages" => Ok(ApiProtocol::Anthropic),
        _ => Err("API protocol 只能是 openai-compatible、openai-responses 或 anthropic".into()),
    }
}

fn read_tty_line() -> Option<String> {
    let mut input = String::new();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let mut reader = io::BufReader::new(&mut file);
    std::io::BufRead::read_line(&mut reader, &mut input).ok()?;
    Some(input)
}

fn choose_with_keyboard(
    render_choices: fn(ApprovalChoice) -> String,
    initial: ApprovalChoice,
) -> ApprovalChoice {
    let mut selected = initial;
    print!("{}", render_choices(selected));
    let _ = io::stdout().flush();

    let Ok(mut input) = ShellInputSource::open() else {
        println!();
        return ApprovalChoice::Deny;
    };
    let fd = input.as_raw_fd();
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        println!();
        return ApprovalChoice::Deny;
    }
    let mut raw = original;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 1;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        println!();
        return ApprovalChoice::Deny;
    }
    let mut terminal_mode = TerminalModeGuard::new(fd, original);

    let result = loop {
        match read_approval_key(&mut input) {
            ApprovalKey::Toggle => {
                selected = match selected {
                    ApprovalChoice::Allow => ApprovalChoice::Deny,
                    ApprovalChoice::Deny => ApprovalChoice::Allow,
                };
                print!("\r\x1b[2K{}", render_choices(selected));
                let _ = io::stdout().flush();
            }
            ApprovalKey::Select(choice) => {
                selected = choice;
                print!("\r\x1b[2K{}", render_choices(selected));
                let _ = io::stdout().flush();
            }
            ApprovalKey::Enter => break selected,
            ApprovalKey::Cancel => break ApprovalChoice::Deny,
            ApprovalKey::Other => {}
        }
    };
    terminal_mode.restore();
    println!();
    result
}

struct TerminalModeGuard {
    fd: i32,
    original: libc::termios,
    active: bool,
}

impl TerminalModeGuard {
    fn new(fd: i32, original: libc::termios) -> Self {
        Self {
            fd,
            original,
            active: true,
        }
    }

    fn restore(&mut self) {
        if self.active {
            let _ = unsafe { libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.original) };
            self.active = false;
        }
    }
}

impl Drop for TerminalModeGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApprovalKey {
    Toggle,
    Select(ApprovalChoice),
    Enter,
    Cancel,
    Other,
}

fn read_approval_key(input: &mut impl Read) -> ApprovalKey {
    let mut buf = [0u8; 1];
    if input.read_exact(&mut buf).is_err() {
        return ApprovalKey::Cancel;
    }
    match buf[0] {
        b'\r' | b'\n' => ApprovalKey::Enter,
        3 | 4 | 27 => {
            if buf[0] != 27 {
                return ApprovalKey::Cancel;
            }
            let Some(seq) = read_escape_sequence(input) else {
                return ApprovalKey::Cancel;
            };
            match seq.as_slice() {
                [b'[', b'A' | b'B'] => ApprovalKey::Toggle,
                [b'[', b'D'] => ApprovalKey::Select(ApprovalChoice::Allow),
                [b'[', b'C'] => ApprovalKey::Select(ApprovalChoice::Deny),
                _ => ApprovalKey::Other,
            }
        }
        b'\t' | b' ' | b'h' | b'j' | b'k' | b'l' => ApprovalKey::Toggle,
        b'y' | b'Y' => ApprovalKey::Select(ApprovalChoice::Allow),
        b'n' | b'N' => ApprovalKey::Select(ApprovalChoice::Deny),
        _ => ApprovalKey::Other,
    }
}

fn read_escape_sequence(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut seq = Vec::with_capacity(8);
    for _ in 0..16 {
        let mut buf = [0u8; 1];
        if input.read_exact(&mut buf).is_err() {
            return None;
        }
        seq.push(buf[0]);
        if matches!(buf[0], b'A'..=b'Z' | b'a'..=b'z' | b'~') {
            break;
        }
    }
    Some(seq)
}

struct SigintGuard {
    previous: libc::sigaction,
    active: bool,
}

impl SigintGuard {
    fn install() -> Option<Self> {
        unsafe extern "C" fn handle_sigint(_: libc::c_int) {
            TURN_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
        }

        unsafe {
            let mut previous: libc::sigaction = std::mem::zeroed();
            let mut next: libc::sigaction = std::mem::zeroed();
            next.sa_sigaction = handle_sigint as usize;
            libc::sigemptyset(&mut next.sa_mask);
            next.sa_flags = 0;
            if libc::sigaction(libc::SIGINT, &next, &mut previous) != 0 {
                return None;
            }
            Some(Self {
                previous,
                active: true,
            })
        }
    }

    fn restore(&mut self) {
        if self.active {
            unsafe {
                let _ = libc::sigaction(libc::SIGINT, &self.previous, std::ptr::null_mut());
            }
            self.active = false;
        }
    }
}

impl Drop for SigintGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn sanitize_user_input(input: &str) -> String {
    let without_paste_markers = input.replace("\x1b[200~", "").replace("\x1b[201~", "");
    strip_csi_sequences(&without_paste_markers)
        .chars()
        .filter(|ch| *ch == '\t' || *ch == '\n' || (!ch.is_control() && *ch != '\r'))
        .collect()
}

fn strip_csi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && matches!(chars.peek(), Some('[')) {
            let _ = chars.next();
            for next in chars.by_ref() {
                if matches!(next, '\u{40}'..='\u{7e}') {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

enum ShellReadline {
    Line { text: String, display: String },
    Interrupted,
    Eof,
    Error(String),
}

struct ShellLineEditor {
    history_file: PathBuf,
    history: Vec<String>,
}

enum ShellInputSource {
    Tty(File),
    Stdin(File),
}

impl ShellInputSource {
    fn open() -> io::Result<Self> {
        if let Ok(tty) = OpenOptions::new().read(true).write(true).open("/dev/tty") {
            return Ok(Self::Tty(tty));
        }
        let fd = unsafe { libc::dup(libc::STDIN_FILENO) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        Ok(Self::Stdin(file))
    }
}

impl Read for ShellInputSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            ShellInputSource::Tty(file) | ShellInputSource::Stdin(file) => file.read(buf),
        }
    }
}

impl AsRawFd for ShellInputSource {
    fn as_raw_fd(&self) -> i32 {
        match self {
            ShellInputSource::Tty(file) | ShellInputSource::Stdin(file) => file.as_raw_fd(),
        }
    }
}

impl ShellLineEditor {
    fn new(history_file: PathBuf) -> Self {
        let history = fs::read_to_string(&history_file)
            .map(|content| {
                content
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            history_file,
            history,
        }
    }

    fn readline(&mut self, prompt: &str) -> ShellReadline {
        let Ok(mut input) = ShellInputSource::open() else {
            return ShellReadline::Error("failed to open terminal input".to_string());
        };
        let fd = input.as_raw_fd();
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return ShellReadline::Error("failed to read terminal mode".to_string());
        }
        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 1;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return ShellReadline::Error("failed to enter raw terminal mode".to_string());
        }
        let mut terminal_mode = TerminalModeGuard::new(fd, original);
        print!("\x1b[?2004h");

        let mut buffer = ShellInputBuffer::default();
        let mut rendered_rows = 0usize;
        let mut history_cursor: Option<usize> = None;
        render_shell_input(prompt, &buffer, &mut rendered_rows);

        let result = loop {
            match read_shell_key(&mut input) {
                ShellInputKey::Char(ch) => {
                    buffer.insert_text(&ch.to_string());
                    history_cursor = None;
                }
                ShellInputKey::Paste(text) => {
                    buffer.insert_paste(&sanitize_user_input(&text));
                    history_cursor = None;
                }
                ShellInputKey::Backspace => {
                    buffer.delete_before_cursor();
                    history_cursor = None;
                }
                ShellInputKey::Delete => {
                    buffer.delete_at_cursor();
                    history_cursor = None;
                }
                ShellInputKey::Left => buffer.move_left(),
                ShellInputKey::Right => buffer.move_right(),
                ShellInputKey::HistoryPrev => {
                    if !self.history.is_empty() {
                        let next = history_cursor
                            .map(|idx| idx.saturating_sub(1))
                            .unwrap_or_else(|| self.history.len().saturating_sub(1));
                        history_cursor = Some(next);
                        buffer = ShellInputBuffer::from_text(self.history[next].clone());
                    }
                }
                ShellInputKey::HistoryNext => {
                    if let Some(idx) = history_cursor {
                        let next = idx + 1;
                        if next < self.history.len() {
                            history_cursor = Some(next);
                            buffer = ShellInputBuffer::from_text(self.history[next].clone());
                        } else {
                            history_cursor = None;
                            buffer = ShellInputBuffer::default();
                        }
                    }
                }
                ShellInputKey::Enter => {
                    buffer.move_to_end();
                    render_shell_input(prompt, &buffer, &mut rendered_rows);
                    let display = buffer.display_plain();
                    println!();
                    print!("\x1b[?2004l");
                    terminal_mode.restore();
                    let restore_dirty = buffer
                        .dirty_paste_summary()
                        .map(|summary| {
                            matches!(choose_paste_recovery(&summary), ApprovalChoice::Allow)
                        })
                        .unwrap_or(false);
                    let text = buffer.submit_text(restore_dirty);
                    if !text.trim().is_empty() {
                        self.push_history(&text);
                    }
                    break ShellReadline::Line { text, display };
                }
                ShellInputKey::Cancel => {
                    println!();
                    break ShellReadline::Interrupted;
                }
                ShellInputKey::Eof => {
                    println!();
                    break ShellReadline::Eof;
                }
                ShellInputKey::Other => {}
            }
            render_shell_input(prompt, &buffer, &mut rendered_rows);
        };

        print!("\x1b[?2004l");
        let _ = io::stdout().flush();
        terminal_mode.restore();
        result
    }

    fn push_history(&mut self, text: &str) {
        if text.contains('\n') {
            return;
        }
        if self.history.last().is_some_and(|last| last == text) {
            return;
        }
        self.history.push(text.to_string());
        if self.history.len() > 500 {
            let drain = self.history.len() - 500;
            self.history.drain(0..drain);
        }
        let mut content = self.history.join("\n");
        content.push('\n');
        let _ = fs::write(&self.history_file, content);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ShellInputToken {
    Text(String),
    MultiLinePaste {
        original_placeholder: String,
        visible_placeholder: String,
        content: String,
        line_count: usize,
    },
}

impl ShellInputToken {
    fn display_plain(&self) -> &str {
        match self {
            ShellInputToken::Text(text) => text,
            ShellInputToken::MultiLinePaste {
                visible_placeholder,
                ..
            } => visible_placeholder,
        }
    }

    fn submit_text(&self, restore_dirty: bool) -> &str {
        match self {
            ShellInputToken::Text(text) => text,
            ShellInputToken::MultiLinePaste {
                original_placeholder,
                visible_placeholder,
                content,
                ..
            } if restore_dirty || visible_placeholder == original_placeholder => content,
            ShellInputToken::MultiLinePaste {
                visible_placeholder,
                ..
            } => visible_placeholder,
        }
    }

    fn display_len(&self) -> usize {
        self.display_plain().chars().count()
    }

    fn dirty_paste_line_count(&self) -> Option<usize> {
        match self {
            ShellInputToken::MultiLinePaste {
                original_placeholder,
                visible_placeholder,
                line_count,
                ..
            } if visible_placeholder != original_placeholder => Some(*line_count),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PasteRecoverySummary {
    dirty_count: usize,
    total_lines: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ShellInputBuffer {
    tokens: Vec<ShellInputToken>,
    cursor: usize,
}

impl ShellInputBuffer {
    fn from_text(text: String) -> Self {
        let cursor = text.chars().count();
        Self {
            tokens: vec![ShellInputToken::Text(text)],
            cursor,
        }
    }

    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.replace_visible_range(
            self.cursor,
            self.cursor,
            vec![ShellInputToken::Text(text.to_string())],
        );
        self.cursor += text.chars().count();
    }

    fn insert_paste(&mut self, text: &str) {
        let line_count = pasted_line_count(text);
        if line_count <= 1 {
            self.insert_text(text);
            return;
        }
        let placeholder = format!("[ pasted {line_count} lines ]");
        let placeholder_len = placeholder.chars().count();
        self.replace_visible_range(
            self.cursor,
            self.cursor,
            vec![ShellInputToken::MultiLinePaste {
                original_placeholder: placeholder.clone(),
                visible_placeholder: placeholder,
                content: text.to_string(),
                line_count,
            }],
        );
        self.cursor += placeholder_len;
    }

    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.cursor - 1;
        self.replace_visible_range(start, self.cursor, Vec::new());
        self.cursor = start;
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor >= self.visible_len() {
            return;
        }
        self.replace_visible_range(self.cursor, self.cursor + 1, Vec::new());
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.visible_len());
    }

    fn move_to_end(&mut self) {
        self.cursor = self.visible_len();
    }

    fn visible_len(&self) -> usize {
        self.tokens.iter().map(ShellInputToken::display_len).sum()
    }

    fn display_plain(&self) -> String {
        self.tokens
            .iter()
            .map(ShellInputToken::display_plain)
            .collect()
    }

    fn display_styled(&self) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                ShellInputToken::Text(text) => out.push_str(text),
                ShellInputToken::MultiLinePaste {
                    visible_placeholder,
                    ..
                } => {
                    out.push_str("\x1b[1m");
                    out.push_str(visible_placeholder);
                    out.push_str(ANSI_RESET);
                }
            }
        }
        out
    }

    fn submit_text(&self, restore_dirty: bool) -> String {
        self.tokens
            .iter()
            .map(|token| token.submit_text(restore_dirty))
            .collect()
    }

    fn dirty_paste_summary(&self) -> Option<PasteRecoverySummary> {
        let mut dirty_count = 0usize;
        let mut total_lines = 0usize;
        for token in &self.tokens {
            if let Some(lines) = token.dirty_paste_line_count() {
                dirty_count += 1;
                total_lines += lines;
            }
        }
        (dirty_count > 0).then_some(PasteRecoverySummary {
            dirty_count,
            total_lines,
        })
    }

    fn display_prefix_before_cursor(&self) -> String {
        char_prefix(&self.display_plain(), self.cursor)
    }

    fn replace_visible_range(
        &mut self,
        start: usize,
        end: usize,
        replacement: Vec<ShellInputToken>,
    ) {
        let mut next = Vec::new();
        let mut inserted = false;
        let mut pos = 0usize;

        for token in &self.tokens {
            let token_start = pos;
            let token_end = token_start + token.display_len();
            pos = token_end;

            if end <= token_start {
                if !inserted {
                    next.extend(replacement.clone());
                    inserted = true;
                }
                next.push(token.clone());
                continue;
            }
            if start >= token_end {
                next.push(token.clone());
                continue;
            }

            let visible = token.display_plain();
            let replacement_text = replacement
                .iter()
                .map(ShellInputToken::display_plain)
                .collect::<String>();
            if let ShellInputToken::MultiLinePaste {
                original_placeholder,
                content,
                line_count,
                ..
            } = token
            {
                let before = if start > token_start {
                    char_range(visible, 0, start - token_start)
                } else {
                    String::new()
                };
                let after = if end < token_end {
                    char_range(visible, end - token_start, token_end - token_start)
                } else {
                    String::new()
                };
                next.push(ShellInputToken::MultiLinePaste {
                    original_placeholder: original_placeholder.clone(),
                    visible_placeholder: format!("{before}{replacement_text}{after}"),
                    content: content.clone(),
                    line_count: *line_count,
                });
                inserted = true;
                continue;
            }
            if start > token_start {
                next.push(ShellInputToken::Text(char_range(
                    visible,
                    0,
                    start - token_start,
                )));
            }
            if !inserted {
                next.extend(replacement.clone());
                inserted = true;
            }
            if end < token_end {
                next.push(ShellInputToken::Text(char_range(
                    visible,
                    end - token_start,
                    token_end - token_start,
                )));
            }
        }

        if !inserted {
            next.extend(replacement);
        }
        self.tokens = merge_text_tokens(next);
    }
}

fn merge_text_tokens(tokens: Vec<ShellInputToken>) -> Vec<ShellInputToken> {
    let mut merged: Vec<ShellInputToken> = Vec::new();
    for token in tokens {
        match token {
            ShellInputToken::Text(text) if text.is_empty() => {}
            ShellInputToken::Text(text) => match merged.last_mut() {
                Some(ShellInputToken::Text(previous)) => previous.push_str(&text),
                _ => merged.push(ShellInputToken::Text(text)),
            },
            ShellInputToken::MultiLinePaste {
                original_placeholder,
                visible_placeholder,
                content,
                line_count,
            } => merged.push(ShellInputToken::MultiLinePaste {
                original_placeholder,
                visible_placeholder,
                content,
                line_count,
            }),
        }
    }
    merged
}

fn pasted_line_count(text: &str) -> usize {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    normalized.split('\n').count()
}

fn char_prefix(text: &str, end: usize) -> String {
    text.chars().take(end).collect()
}

fn char_range(text: &str, start: usize, end: usize) -> String {
    text.chars().skip(start).take(end - start).collect()
}

#[derive(Debug, PartialEq, Eq)]
enum ShellInputKey {
    Char(char),
    Paste(String),
    Backspace,
    Delete,
    Left,
    Right,
    HistoryPrev,
    HistoryNext,
    Enter,
    Cancel,
    Eof,
    Other,
}

fn read_shell_key(input: &mut impl Read) -> ShellInputKey {
    let mut buf = [0u8; 1];
    if input.read_exact(&mut buf).is_err() {
        return ShellInputKey::Eof;
    }
    match buf[0] {
        b'\r' | b'\n' => ShellInputKey::Enter,
        3 => ShellInputKey::Cancel,
        4 => ShellInputKey::Eof,
        8 | 127 => ShellInputKey::Backspace,
        27 => read_shell_escape(input),
        byte if byte.is_ascii_control() => ShellInputKey::Other,
        first => read_utf8_char(first, input)
            .map(ShellInputKey::Char)
            .unwrap_or(ShellInputKey::Other),
    }
}

fn read_shell_escape(input: &mut impl Read) -> ShellInputKey {
    let Some(seq) = read_escape_sequence(input) else {
        return ShellInputKey::Cancel;
    };
    match seq.as_slice() {
        [b'[', b'D'] => ShellInputKey::Left,
        [b'[', b'C'] => ShellInputKey::Right,
        [b'[', b'A'] => ShellInputKey::HistoryPrev,
        [b'[', b'B'] => ShellInputKey::HistoryNext,
        [b'[', b'3', b'~'] => ShellInputKey::Delete,
        [b'[', b'2', b'0', b'0', b'~'] => ShellInputKey::Paste(read_bracketed_paste(input)),
        _ => ShellInputKey::Other,
    }
}

fn read_bracketed_paste(input: &mut impl Read) -> String {
    let mut bytes = Vec::new();
    let end = b"\x1b[201~";
    loop {
        let mut buf = [0u8; 1];
        if input.read_exact(&mut buf).is_err() {
            break;
        }
        bytes.push(buf[0]);
        if bytes.ends_with(end) {
            let new_len = bytes.len().saturating_sub(end.len());
            bytes.truncate(new_len);
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn read_utf8_char(first: u8, input: &mut impl Read) -> Option<char> {
    let width = if first < 0x80 {
        1
    } else if first & 0b1110_0000 == 0b1100_0000 {
        2
    } else if first & 0b1111_0000 == 0b1110_0000 {
        3
    } else if first & 0b1111_1000 == 0b1111_0000 {
        4
    } else {
        return None;
    };
    let mut bytes = vec![first];
    for _ in 1..width {
        let mut buf = [0u8; 1];
        input.read_exact(&mut buf).ok()?;
        bytes.push(buf[0]);
    }
    std::str::from_utf8(&bytes).ok()?.chars().next()
}

fn render_shell_input(prompt: &str, buffer: &ShellInputBuffer, rendered_rows: &mut usize) {
    print!("{}", render_shell_input_clear_sequence(*rendered_rows));
    print!("{}{}", prompt, buffer.display_styled());

    let width = terminal_width().max(1);
    let prompt_width = display_width(prompt);
    let plain = buffer.display_plain();
    let total_width = prompt_width + display_width(&plain);
    let cursor_width = prompt_width + display_width(&buffer.display_prefix_before_cursor());
    let total_row = total_width / width;
    let cursor_row = cursor_width / width;
    let cursor_col = cursor_width % width;
    if total_row > 0 {
        print!("\x1b[{total_row}F");
    }
    print!("\r");
    if cursor_row > 0 {
        print!("\x1b[{cursor_row}B");
    }
    if cursor_col > 0 {
        print!("\x1b[{cursor_col}C");
    }
    *rendered_rows = wrapped_terminal_rows(total_width, width);
    let _ = io::stdout().flush();
}

fn render_shell_input_clear_sequence(rendered_rows: usize) -> String {
    let mut out = String::new();
    if rendered_rows > 1 {
        out.push_str(&format!("\r\x1b[{}F", rendered_rows - 1));
    } else {
        out.push('\r');
    }
    for idx in 0..rendered_rows {
        out.push_str("\x1b[2K");
        if idx + 1 < rendered_rows {
            out.push_str("\x1b[1E");
        }
    }
    if rendered_rows > 1 {
        out.push_str(&format!("\r\x1b[{}F", rendered_rows - 1));
    } else {
        out.push('\r');
    }
    out
}

fn render_thinking(snapshot: &ThinkingViewSnapshot, rendered_lines: &Arc<Mutex<usize>>) {
    let rendered = render_thinking_view_at(snapshot, &time_label());
    let line_count = rendered.lines().count();
    print!("{rendered}");
    if let Ok(mut previous) = rendered_lines.lock() {
        *previous = line_count;
    }
    let _ = io::stdout().flush();
}

fn rerender_thinking(snapshot: &ThinkingViewSnapshot, rendered_lines: &Arc<Mutex<usize>>) {
    clear_thinking_block(rendered_lines);
    render_thinking(snapshot, rendered_lines);
}

fn clear_thinking_block(rendered_lines: &Arc<Mutex<usize>>) {
    let previous = rendered_lines.lock().map(|lines| *lines).unwrap_or(0);
    if previous > 0 {
        print!("\x1b[{}F\x1b[J", previous);
    }
    if let Ok(mut lines) = rendered_lines.lock() {
        *lines = 0;
    }
    let _ = io::stdout().flush();
}

fn random_spinner_tick() -> usize {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as usize)
        .unwrap_or_default();
    let mixed = nanos ^ nanos.rotate_left(13) ^ nanos.rotate_right(7);
    (mixed % SPINNER_ICONS.len()) * 4
}

#[derive(Default)]
struct PromptStatusBar {
    visible: bool,
}

impl PromptStatusBar {
    fn show_info(&mut self, text: &str) {
        self.replace_with(ShellStatusMessage {
            tone: ShellStatusTone::Info,
            text: text.to_string(),
        });
    }

    fn replace_with(&mut self, message: ShellStatusMessage) {
        if self.visible {
            print!("\x1b[1A\r\x1b[2K");
        }
        println!("{}", render_shell_status_bar(&message));
        self.visible = true;
        let _ = io::stdout().flush();
    }

    fn clear_after_empty_input(&mut self) {
        if self.visible {
            print!("\x1b[2A\r\x1b[J");
            self.visible = false;
            let _ = io::stdout().flush();
        }
    }

    fn clear_before_exit(&mut self) {
        if self.visible {
            print!("\x1b[1A\r\x1b[2K");
            self.visible = false;
            let _ = io::stdout().flush();
        }
    }

    fn take_visible(&mut self) -> bool {
        let visible = self.visible;
        self.visible = false;
        visible
    }
}

fn rewrite_submitted_user_line(input: &str, status_line_visible: bool) {
    print!(
        "{}",
        render_submitted_user_line_rewrite(
            input,
            status_line_visible,
            terminal_width(),
            &time_label()
        )
    );
    let _ = io::stdout().flush();
}

fn render_user_input_prompt(time_label: &str) -> String {
    format!("[{time_label}] \x1b[1mYou\x1b[0m ❯❯ ")
}

fn render_submitted_user_line_rewrite(
    input: &str,
    status_line_visible: bool,
    terminal_width: usize,
    time_label: &str,
) -> String {
    let prompt_prefix = render_user_input_prompt(time_label);
    let prompt_width = display_width(&prompt_prefix);
    let input_rows = wrapped_terminal_rows(prompt_width + display_width(input), terminal_width);
    let rows_to_clear = input_rows + usize::from(status_line_visible);
    format!(
        "\x1b[{}F\r\x1b[J{}{}\n",
        rows_to_clear, prompt_prefix, input
    )
}

fn wrapped_terminal_rows(display_width: usize, terminal_width: usize) -> usize {
    let width = terminal_width.max(1);
    display_width.max(1).div_ceil(width)
}

fn terminal_width() -> usize {
    terminal_width_from_fd(io::stdout().as_raw_fd()).unwrap_or(80)
}

#[cfg(unix)]
fn terminal_width_from_fd(fd: i32) -> Option<usize> {
    let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
    let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) };
    if rc == 0 && size.ws_col > 0 {
        Some(size.ws_col as usize)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn terminal_width_from_fd(_fd: i32) -> Option<usize> {
    None
}

fn print_final_response(
    text: &str,
    stats: &UsageStats,
    provider: &str,
    model: &str,
    elapsed: Duration,
) {
    let rendered = render_final_response_at(
        text,
        stats,
        provider,
        model,
        elapsed.as_secs(),
        &time_label(),
    );
    print!("{rendered}");
    let _ = io::stdout().flush();
}

fn render_startup_banner(
    space: &str,
    config: &timem_shell::ProviderConfig,
    audit_file: &std::path::Path,
    action_audit_file: &std::path::Path,
    bash_approval_mode: BashApprovalMode,
) -> String {
    let default_protocol = timem_shell::default_api_protocol_for_provider(&config.provider);
    let items = [
        ConfigTableItem::Section("MODEL"),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_MODEL".to_string(),
            value: config.model.clone(),
            desc: "模型名称",
            highlight: !timem_shell::is_default_model_for_provider(&config.provider, &config.model),
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_GATEWAY_PROVIDER".to_string(),
            value: config.provider.clone(),
            desc: "流量平台，决定默认 base url",
            highlight: false,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_API_PROTOCOL".to_string(),
            value: config.api_protocol.label().to_string(),
            desc: "API 提交网络包格式",
            highlight: config.api_protocol != default_protocol,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_BASE_URL".to_string(),
            value: config.base_url.clone(),
            desc: "网关 base url",
            highlight: !timem_shell::is_default_base_url_for_provider(
                &config.provider,
                &config.base_url,
            ),
        }),
        ConfigTableItem::Section("RUNTIME"),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_MAX_LLM_INPUT".to_string(),
            value: format_token_count(config.max_llm_input_tokens),
            desc: "最大输入 token",
            highlight: false,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_MAX_LLM_OUTPUT".to_string(),
            value: format_token_count(config.max_llm_output_tokens),
            desc: "最大输出 token",
            highlight: false,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_BASH_APPROVAL".to_string(),
            value: bash_approval_mode_label(bash_approval_mode).to_string(),
            desc: "bash 允许策略，approve/ask",
            highlight: false,
        }),
        ConfigTableItem::Section("DATA"),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_SPACE".to_string(),
            value: space.to_string(),
            desc: "记忆空间",
            highlight: false,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_DATA_DIR".to_string(),
            value: absolute_display_path(&data_root()),
            desc: "运行时记忆、日志存储",
            highlight: false,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "local_audit".to_string(),
            value: absolute_display_path(audit_file),
            desc: "payload 记录",
            highlight: false,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "".to_string(),
            value: absolute_display_path(action_audit_file),
            desc: "action 记录",
            highlight: false,
        }),
    ];
    boxed_config_table(&items)
}

fn boxed_config_table(items: &[ConfigTableItem]) -> String {
    boxed_config_table_at_width(items, terminal_width().saturating_sub(1))
}

fn boxed_config_table_at_width(items: &[ConfigTableItem], terminal_width: usize) -> String {
    let table_width = terminal_width.max(50);
    let inner_width = table_width.saturating_sub(2);
    let (key_width, value_width, desc_width) = config_column_widths(inner_width);
    let title = format!(" {TIMEM_LOGO} config ");
    let title_width = display_width(&title);
    let left = (inner_width.saturating_sub(title_width)) / 2;
    let right = inner_width.saturating_sub(title_width + left);
    let top_border = format!("┌{}{}{}┐\n", "─".repeat(left), title, "─".repeat(right));
    let bottom_border = format!("└{}┘\n", "─".repeat(inner_width));
    let mut out = String::new();
    out.push_str(&top_border);
    for item in items {
        match item {
            ConfigTableItem::Section(label) => {
                out.push_str(&section_line(label, inner_width));
            }
            ConfigTableItem::Row(row) => {
                let key_lines = wrap_display(&row.key, key_width);
                let value_lines = wrap_display(&row.value, value_width);
                let desc_lines = wrap_display(row.desc, desc_width);
                let row_height = key_lines
                    .len()
                    .max(value_lines.len())
                    .max(desc_lines.len())
                    .max(1);
                for idx in 0..row_height {
                    let key = key_lines.get(idx).map(String::as_str).unwrap_or("");
                    let value = value_lines.get(idx).map(String::as_str).unwrap_or("");
                    let desc = desc_lines.get(idx).map(String::as_str).unwrap_or("");
                    let value = fit_display(value, value_width);
                    let value = if row.highlight && !value.trim().is_empty() {
                        format!("{ANSI_HIGHLIGHT}{value}{ANSI_RESET}")
                    } else {
                        value
                    };
                    out.push_str(&format!(
                        "│ {} │ {} │ {} │\n",
                        fit_display(key, key_width),
                        value,
                        fit_display(desc, desc_width)
                    ));
                }
            }
        }
    }
    out.push_str(&bottom_border);
    out.push_str(
        "显示的是最终生效值。可先 source /path/to/your/env，或设置左侧 env 变量后启动。\n",
    );
    out.push_str("option 优先于 env，查看：timem --help\n");
    out
}

fn config_column_widths(inner_width: usize) -> (usize, usize, usize) {
    let content_width = inner_width.saturating_sub(8).max(30);
    let key_floor = 22.min(content_width.saturating_sub(20).max(6));
    let key_width = (content_width * 2 / 10).max(key_floor);
    let remaining = content_width.saturating_sub(key_width);
    let value_width = (remaining * 5 / 8).max(12).min(remaining.saturating_sub(8));
    let desc_width = remaining.saturating_sub(value_width).max(8);
    (key_width, value_width, desc_width)
}

fn section_line(label: &str, target_width: usize) -> String {
    let prefix = format!("──── {ANSI_BOLD}{label}{ANSI_RESET} :");
    let prefix_width = display_width(&prefix);
    let content = if target_width <= prefix_width {
        fit_display(&prefix, target_width)
    } else {
        format!("{}{}", prefix, " ".repeat(target_width - prefix_width))
    };
    format!("├{content}│\n")
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(text).as_str())
}

fn strip_ansi(text: &str) -> String {
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

fn fit_display(text: &str, target_width: usize) -> String {
    let width = display_width(text);
    if width == target_width {
        text.to_string()
    } else if width < target_width {
        format!("{}{}", text, " ".repeat(target_width - width))
    } else if target_width == 0 {
        String::new()
    } else {
        let ellipsis = "…";
        let content_width = target_width.saturating_sub(display_width(ellipsis));
        let mut fitted = String::new();
        let mut used = 0;
        for ch in text.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + char_width > content_width {
                break;
            }
            fitted.push(ch);
            used += char_width;
        }
        fitted.push_str(ellipsis);
        let fitted_width = display_width(&fitted);
        if fitted_width < target_width {
            format!("{}{}", fitted, " ".repeat(target_width - fitted_width))
        } else {
            fitted
        }
    }
}

fn wrap_display(text: &str, target_width: usize) -> Vec<String> {
    if target_width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for raw_line in text.lines().default_if_empty() {
        let mut current = String::new();
        let mut current_width = 0usize;
        for ch in raw_line.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width > 0 && current_width + ch_width > target_width {
                lines.push(fit_display(&current, target_width));
                current.clear();
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }
        lines.push(fit_display(&current, target_width));
    }
    if lines.is_empty() {
        lines.push(" ".repeat(target_width));
    }
    lines
}

trait DefaultIfEmpty<'a> {
    fn default_if_empty(self) -> Vec<&'a str>;
}

impl<'a, I> DefaultIfEmpty<'a> for I
where
    I: Iterator<Item = &'a str>,
{
    fn default_if_empty(self) -> Vec<&'a str> {
        let lines = self.collect::<Vec<_>>();
        if lines.is_empty() {
            vec![""]
        } else {
            lines
        }
    }
}

fn absolute_display_path(path: &std::path::Path) -> String {
    if path.is_absolute() {
        path.display().to_string()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
            .display()
            .to_string()
    }
}

fn print_help() {
    print!("{}", help_text());
}

fn help_text() -> &'static str {
    "Usage:\n  timem [options]\n\n\x1b[1mPrecedence:\n  command line options override process env values; process env overrides defaults.\x1b[0m\n\nCreate a private env file from env_template, then load it explicitly:\n  cp env_template env\n  source /path/to/your/env\n\nRecommended run:\n  timem\n\nUseful env values to put in your env file:\n  export TIMEM_GATEWAY_PROVIDER=aliyun\n  export TIMEM_API_KEY=your_api_key_here\n  export TIMEM_MODEL=qwen-plus\n  export TIMEM_SPACE=.test_mem\n\nCommand line override example:\n  timem --data-dir data --space .test_mem --gateway-provider aliyun --model qwen-plus\n\nOptions:\n  --space <name>                 env TIMEM_SPACE; memory/audit space, default .test_mem\n  --gateway-provider <name>      env TIMEM_GATEWAY_PROVIDER; traffic platform / default base URL provider\n  --api-protocol <protocol>      env TIMEM_API_PROTOCOL; openai-compatible|openai-responses|anthropic\n  --base-url <url>               env TIMEM_BASE_URL; override provider default base URL\n  --model <name>                 env TIMEM_MODEL; model name\n  --api-key <key>                env TIMEM_API_KEY; API key, env is safer than shell history\n  --data-dir <path>              env TIMEM_DATA_DIR; data/config/memory/audit root\n  --timeout <seconds>            env TIMEM_TIMEOUT; provider HTTP timeout, default 120\n  --max-llm-input <n|100K>       env TIMEM_MAX_LLM_INPUT; max input context, default 100K\n  --max-llm-output <n|10K>      env TIMEM_MAX_LLM_OUTPUT; max output tokens, default 10K\n  --bash-approval <mode>         env TIMEM_BASH_APPROVAL; ask|approve, default ask\n  --once-json <text>             run one non-interactive turn and print JSON\n  --supporting-context <text>    append extra runtime context for --once-json/debug\n  -h, --help                     show this help\n\nInteractive commands:\n  /config                        edit runtime model and token settings\n  /prof                          show runtime profiling for tokens, model wait/local time, and storage size\n\nInteractive keys:\n  Ctrl+C cancels the current input while editing, and cancels the current turn while Timem is thinking.\n\nProtocol defaults:\n  openai -> openai-responses; anthropic -> anthropic; others -> openai-compatible\n\nVendor fallback key env vars:\n  DASHSCOPE_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN\n"
}

fn format_token_count(value: u32) -> String {
    if value % 1_000 == 0 && value >= 1_000 {
        format!("{}K", value / 1_000)
    } else {
        value.to_string()
    }
}

fn bash_approval_mode_from_options(
    options: &timem_shell::CliOptions,
    env: &HashMap<String, String>,
) -> BashApprovalMode {
    let raw = options
        .bash_approval
        .clone()
        .or_else(|| env.get("TIMEM_BASH_APPROVAL").cloned())
        .unwrap_or_else(|| "ask".to_string())
        .to_lowercase();
    match raw.as_str() {
        "approve" | "approval" | "always" | "never" | "off" | "none" | "false" | "0" => {
            BashApprovalMode::Approve
        }
        "ask" | "prompt" | "true" | "1" => BashApprovalMode::Ask,
        _ => BashApprovalMode::Ask,
    }
}

fn bash_approval_mode_label(mode: BashApprovalMode) -> &'static str {
    match mode {
        BashApprovalMode::Ask => "ask",
        BashApprovalMode::Approve => "approve",
    }
}

fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn session_id() -> String {
    format!("shell_{}", epoch_millis())
}

fn time_label() -> String {
    local_time_label()
}

#[cfg(test)]
mod static_prompt_tests {
    use super::{
        apply_config_value, boxed_config_table_at_width, cancelled_turn_result, config_field_value,
        consume_turn_cancel_request, display_width, epoch_millis, help_text, pasted_line_count,
        random_spinner_tick, read_approval_key, read_menu_key, read_shell_key,
        render_approval_choices, render_config_menu, render_expand_output_choices,
        render_expand_output_prompt, render_paste_recovery_choices, render_paste_recovery_prompt,
        render_round_limit_choices, render_round_limit_prompt, render_shell_input_clear_sequence,
        render_stale_context_choices, render_stale_context_prompt, render_startup_banner,
        render_submitted_user_line_rewrite, render_user_approval_prompt, render_user_input_prompt,
        sanitize_user_input, stale_context_prompt_needed, wrapped_terminal_rows, ApprovalChoice,
        ApprovalKey, ConfigField, ConfigRow, ConfigTableItem, MenuKey, PasteRecoverySummary,
        ShellInputBuffer, ShellInputKey, ANSI_HIGHLIGHT, ANSI_RESET, STALE_CONTEXT_IDLE,
        STALE_CONTEXT_TOKEN_THRESHOLD, STATIC_PROMPT, TURN_CANCEL_REQUESTED,
    };
    use agent_core::{AgentCore, ApprovalRequest, BashApprovalMode, CoreProfile};
    use std::fs;
    use std::io::Cursor;
    use std::process::Command;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use timem_shell::{ApiProtocol, ProviderConfig, SPINNER_ICONS};

    #[test]
    fn static_prompt_uses_full_shared_v1_resource() {
        assert!(STATIC_PROMPT.contains("\"static_prefix_id\": \"static_prefix_v3\""));
        assert!(STATIC_PROMPT.contains("\"General_rule\""));
        assert!(STATIC_PROMPT.contains("\"Mem_rule\""));
        assert!(STATIC_PROMPT.contains("\"Tool_capability\""));
        assert!(STATIC_PROMPT.contains("\"Response_rule\""));
        assert!(STATIC_PROMPT.contains("\"Self_audit\""));
        assert!(STATIC_PROMPT.contains("\"json_schema_summary\""));
        assert!(STATIC_PROMPT.contains("\"acceptance_check.is_satisfied\""));
        assert!(STATIC_PROMPT.contains("\"perspective_policy\""));
        assert!(STATIC_PROMPT.contains("\"tool_claim_policy\""));
        assert!(STATIC_PROMPT.contains("\"storage_style_policy\""));
        assert!(STATIC_PROMPT.contains("\"tool_catalog\""));
        assert!(STATIC_PROMPT.contains("\"chat_history_query\""));
        assert!(STATIC_PROMPT.contains("persisted user/assistant chat records"));
        assert!(STATIC_PROMPT.contains("\"query_memory\""));
        assert!(STATIC_PROMPT.contains("\"memory_schema\""));
        assert!(STATIC_PROMPT.contains("\"memory_sql_query\""));
        assert!(STATIC_PROMPT.contains("\"memory_update\""));
        assert!(STATIC_PROMPT.contains("\"chat_history_delete\""));
        assert!(STATIC_PROMPT.contains("\"scratch_write\""));
        assert!(STATIC_PROMPT.contains("\"scratch_query\""));
        assert!(STATIC_PROMPT.contains("\"scratch_delete\""));
        assert!(STATIC_PROMPT.contains("\"run_bash\""));
        assert!(STATIC_PROMPT.contains("\"shell_job_status\""));
        assert!(STATIC_PROMPT.contains("foreground|background"));
        assert!(STATIC_PROMPT.contains("read_back_command"));
        assert!(STATIC_PROMPT.contains("Never invent a read-only limitation for run_bash"));
        assert!(STATIC_PROMPT.contains("\"durable_ctx_score\""));
        assert!(STATIC_PROMPT.contains("Every model response must score"));
        assert!(STATIC_PROMPT.contains("local machine"));
        assert!(STATIC_PROMPT.contains("\"intent_required\""));
        assert!(STATIC_PROMPT.contains("\"json_protocol\""));
        assert!(STATIC_PROMPT.contains("\"evidence_guard\""));
        assert!(STATIC_PROMPT.contains("\"action_result_guard\""));
        assert!(STATIC_PROMPT.contains("\"thought?\""));
        assert!(STATIC_PROMPT.contains("self_audit"));
        assert!(!STATIC_PROMPT.contains("no_result_terminate"));
        assert!(!STATIC_PROMPT.contains("long_running_shell"));
        assert!(!STATIC_PROMPT.contains("lang_retry"));
        assert!(!STATIC_PROMPT.contains("theme_workflow"));
        assert!(!STATIC_PROMPT.contains("rounds_guard"));
        assert!(!STATIC_PROMPT.contains("perspective_rewrite"));
        assert!(STATIC_PROMPT.len() > 3_000);
    }

    #[test]
    fn public_repo_sources_do_not_contain_private_gateway_markers() {
        let source_text = [
            include_str!("../../README.md"),
            include_str!("../../env_template"),
            include_str!("../../resources/static_v1.json"),
            include_str!("../src/lib.rs"),
            include_str!("../src/main.rs"),
        ]
        .join("\n");
        let markers = [
            ["c", "hj"].concat(),
            ["che", "hejia"].concat(),
            ["inner.", "c", "hj"].concat(),
            ["llm-", "gateway", "-proxy"].concat(),
            ["api-hub.", "inner"].concat(),
            ["X-", "C", "HJ"].concat(),
            ["BCS-", "APIHub"].concat(),
        ];
        for marker in markers {
            assert!(
                !source_text.to_lowercase().contains(&marker.to_lowercase()),
                "private marker leaked into public source: {marker}"
            );
        }
    }

    #[test]
    fn turn_cancel_flag_is_consumed_once() {
        TURN_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
        assert!(consume_turn_cancel_request());
        assert!(!consume_turn_cancel_request());
    }

    #[test]
    fn cancelled_turn_message_does_not_look_like_model_failure() {
        let (text, stats, issue) = cancelled_turn_result();
        assert_eq!(text, "已取消本轮。");
        assert!(!text.contains("模型调用失败"));
        assert_eq!(stats.llm_calls, 0);
        assert_eq!(issue.as_deref(), Some("cancelled_by_user"));
    }

    #[test]
    fn wrapped_input_rerender_does_not_clear_screen_below_prompt() {
        let sequence = render_shell_input_clear_sequence(3);
        assert!(sequence.contains("\x1b[2K"));
        assert!(sequence.contains("\x1b[1E"));
        assert!(!sequence.contains("\x1b[J"));
    }

    #[test]
    fn expand_output_prompt_is_keyboard_driven_and_mentions_retry() {
        let prompt = render_expand_output_prompt(10_000);
        assert!(prompt.contains("10K"));
        assert!(prompt.contains("TIMEM_MAX_LLM_OUTPUT"));
        assert!(prompt.contains("自动重试"));
        assert!(prompt.contains("←/→"));
        assert!(render_expand_output_choices(ApprovalChoice::Allow).contains("\x1b[7m"));
    }

    #[test]
    fn stale_context_prompt_requires_three_hours_and_large_dynamic_context() {
        assert!(!stale_context_prompt_needed(
            STALE_CONTEXT_IDLE - Duration::from_secs(1),
            STALE_CONTEXT_TOKEN_THRESHOLD + 1
        ));
        assert!(!stale_context_prompt_needed(
            STALE_CONTEXT_IDLE,
            STALE_CONTEXT_TOKEN_THRESHOLD
        ));
        assert!(stale_context_prompt_needed(
            STALE_CONTEXT_IDLE,
            STALE_CONTEXT_TOKEN_THRESHOLD + 1
        ));

        let prompt = render_stale_context_prompt(Duration::from_secs(3 * 60 * 60 + 5 * 60), 12_300);
        assert!(prompt.contains("3 小时 5 分钟"));
        assert!(prompt.contains("12.3K"));
        assert!(prompt.contains("是否继续使用上次对话任务上下文"));
        assert!(prompt.contains("选择 NO 会清空旧动态上下文"));
        assert_eq!(
            render_stale_context_choices(ApprovalChoice::Allow),
            "\x1b[7m[ YES ]\x1b[0m   NO"
        );
        assert_eq!(
            render_stale_context_choices(ApprovalChoice::Deny),
            "  YES   \x1b[7m[ NO ]\x1b[0m"
        );
    }

    #[test]
    fn config_menu_renders_effective_values_and_can_apply_updates() {
        let mut config = ProviderConfig {
            provider: "aliyun".to_string(),
            api_protocol: ApiProtocol::OpenAiCompatible,
            api_key: "secret".to_string(),
            model: "qwen-plus".to_string(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            timeout_secs: 120,
            max_llm_output_tokens: 10_000,
            max_llm_input_tokens: 100_000,
        };
        let mut core = AgentCore::new(
            "STATIC",
            CoreProfile {
                name: "aliyun".to_string(),
                provider: "aliyun".to_string(),
                model: "qwen-plus".to_string(),
            },
            std::env::temp_dir().join(format!("timem_config_test_{}", epoch_millis())),
        );
        let mut bash = BashApprovalMode::Ask;

        let menu = render_config_menu(&config, bash, 5);
        assert!(menu.contains("TIMEM_MAX_LLM_OUTPUT"));
        assert!(menu.contains("10K"));
        assert!(menu.contains("\x1b[7m"));
        assert_eq!(
            config_field_value(&config, bash, ConfigField::MaxInput),
            "100K"
        );

        apply_config_value(
            &mut config,
            &mut core,
            &mut bash,
            ConfigField::MaxOutput,
            "20K",
        )
        .unwrap();
        apply_config_value(
            &mut config,
            &mut core,
            &mut bash,
            ConfigField::MaxInput,
            "120K",
        )
        .unwrap();
        apply_config_value(
            &mut config,
            &mut core,
            &mut bash,
            ConfigField::BashApproval,
            "approve",
        )
        .unwrap();

        assert_eq!(config.max_llm_output_tokens, 20_000);
        assert_eq!(config.max_llm_input_tokens, 120_000);
        assert_eq!(bash, BashApprovalMode::Approve);
    }

    #[test]
    fn config_menu_key_reader_accepts_arrows_enter_and_cancel() {
        assert_eq!(
            read_menu_key(&mut Cursor::new(vec![27, b'[', b'A'])),
            MenuKey::Up
        );
        assert_eq!(
            read_menu_key(&mut Cursor::new(vec![27, b'[', b'B'])),
            MenuKey::Down
        );
        assert_eq!(read_menu_key(&mut Cursor::new(vec![b'\n'])), MenuKey::Enter);
        assert_eq!(read_menu_key(&mut Cursor::new(vec![3])), MenuKey::Cancel);
    }

    #[test]
    fn random_spinner_tick_maps_to_valid_icon_slot() {
        for _ in 0..32 {
            let tick = random_spinner_tick();
            assert_eq!(tick % 4, 0);
            assert!(tick / 4 < SPINNER_ICONS.len());
        }
    }

    #[test]
    fn startup_banner_lists_env_overrides_on_separate_lines() {
        let config = ProviderConfig {
            provider: "aliyun".to_string(),
            api_protocol: ApiProtocol::OpenAiCompatible,
            api_key: "secret".to_string(),
            model: "qwen-plus".to_string(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            timeout_secs: 120,
            max_llm_output_tokens: 4096,
            max_llm_input_tokens: 100_000,
        };
        let banner = render_startup_banner(
            ".xxx_mem",
            &config,
            std::path::Path::new(".xxx_mem/audit/api_audit.jsonl"),
            std::path::Path::new(".xxx_mem/audit/action_audit.json"),
            BashApprovalMode::Approve,
        );

        assert!(banner.starts_with('┌'));
        assert!(banner.lines().next().unwrap_or("").starts_with("┌─"));
        assert!(banner.lines().any(|line| line.starts_with("├──── ")
            && line.contains("MODEL")
            && line.contains(":")));
        assert!(banner.lines().any(|line| line.starts_with("├──── ")
            && line.contains("RUNTIME")
            && line.contains(":")));
        assert!(banner
            .lines()
            .any(|line| line.starts_with("├──── ") && line.contains("DATA") && line.contains(":")));
        assert!(banner.contains("MODEL"));
        assert!(banner.contains("RUNTIME"));
        assert!(banner.contains("DATA"));
        assert!(banner.contains("显示的是最终生效值"));
        assert!(banner.contains("source /path/to/your/env"));
        assert!(banner.contains("option 优先于 env"));
        assert!(banner.contains("TIMEM_SPACE"));
        assert!(banner.contains(".xxx_mem"));
        assert!(!banner.contains("session="));
        assert!(!banner.contains("TIMEM_PROFILE"));
        assert!(!banner
            .lines()
            .any(|line| line.trim_start().starts_with("│ TIMEM_SPACE=")));
        assert!(banner.contains("TIMEM_GATEWAY_PROVIDER"));
        assert!(banner.contains("流量平台"));
        assert!(banner.contains("aliyun"));
        assert!(banner.contains("TIMEM_API_PROTOCOL"));
        assert!(banner.contains("openai-compatible"));
        assert!(banner.contains("TIMEM_BASE_URL"));
        assert!(banner.contains("dashscope.aliyuncs"));
        assert!(banner.contains("compatible-mode/v1"));
        assert!(banner.contains("TIMEM_MODEL"));
        assert!(banner.contains("qwen-plus"));
        assert!(banner.contains("TIMEM_MAX_LLM_INPUT"));
        assert!(banner.contains("TIMEM_MAX_LLM_OUTPUT"));
        assert!(!banner.contains("TIMEM_MAX_LLM_INPUT / TIMEM_MAX_LLM_OUTPUT"));
        assert!(banner.contains("100K"));
        assert!(banner.contains("TIMEM_BASH_APPROVAL"));
        assert!(banner.contains("approve"));
        assert!(banner.contains("TIMEM_DATA_DIR"));
        assert!(banner.contains("/data"));
        assert!(banner.contains("local_audit"));
        assert!(banner.contains("api_audit.jsonl"));
        assert!(banner.contains("payload 记录"));
        assert!(banner.contains("action_audit.json"));
        assert!(banner.contains("action 记录"));
        assert!(!banner.contains("TIMEM_API_KEY=secret"));
        let table_lines: Vec<&str> = banner
            .lines()
            .filter(|line| line.starts_with('│') || line.starts_with('├'))
            .collect();
        let first_width = display_width(table_lines[0]);
        assert!(table_lines
            .iter()
            .all(|line| display_width(line) == first_width));
        let model_idx = banner.find("TIMEM_MODEL").unwrap();
        let provider_idx = banner.find("TIMEM_GATEWAY_PROVIDER").unwrap();
        let runtime_idx = banner.find("TIMEM_MAX_LLM_INPUT").unwrap();
        let bash_idx = banner.find("TIMEM_BASH_APPROVAL").unwrap();
        let space_idx = banner.find("TIMEM_SPACE").unwrap();
        let data_idx = banner.find("TIMEM_DATA_DIR").unwrap();
        assert!(model_idx < provider_idx);
        assert!(provider_idx < runtime_idx);
        assert!(runtime_idx < bash_idx);
        assert!(bash_idx < space_idx);
        assert!(space_idx < data_idx);
    }

    #[test]
    fn config_table_uses_window_width_ratio_and_wraps_long_values() {
        let items = [
            ConfigTableItem::Section("MODEL"),
            ConfigTableItem::Row(ConfigRow {
                key: "TIMEM_GATEWAY_PROVIDER".to_string(),
                value: "aliyun".to_string(),
                desc: "流量平台，决定默认 base url",
                highlight: false,
            }),
            ConfigTableItem::Row(ConfigRow {
                key: "TIMEM_BASE_URL".to_string(),
                value: "https://very-long-provider.example.com/compatible-mode/v1/with/a/path/that/wraps".to_string(),
                desc: "网关 base url",
                highlight: false,
            }),
        ];
        let banner = boxed_config_table_at_width(&items, 80);
        let table_lines = banner
            .lines()
            .filter(|line| line.starts_with('│') || line.starts_with('├'))
            .collect::<Vec<_>>();
        assert!(!table_lines.is_empty());
        assert!(table_lines.iter().all(|line| display_width(line) == 80));
        assert!(banner
            .lines()
            .next()
            .is_some_and(|line| display_width(line) == 80));
        assert!(banner.contains("very-long-provider"));
        let data_rows = banner
            .lines()
            .filter(|line| line.starts_with('│'))
            .collect::<Vec<_>>();
        assert!(
            data_rows.len() > 2,
            "long value should wrap into extra table rows:\n{banner}"
        );
        assert_eq!(
            data_rows[1].matches('│').count(),
            data_rows[2].matches('│').count()
        );
    }

    #[test]
    fn startup_banner_highlights_values_outside_provider_defaults() {
        let default_config = ProviderConfig {
            provider: "aliyun".to_string(),
            api_protocol: ApiProtocol::OpenAiCompatible,
            api_key: "secret".to_string(),
            model: "qwen-max".to_string(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            timeout_secs: 120,
            max_llm_output_tokens: 4096,
            max_llm_input_tokens: 100_000,
        };
        let default_banner = render_startup_banner(
            ".test_mem",
            &default_config,
            std::path::Path::new(".test_mem/audit/api_audit.jsonl"),
            std::path::Path::new(".test_mem/audit/action_audit.json"),
            BashApprovalMode::Ask,
        );
        assert!(!default_banner.contains(ANSI_HIGHLIGHT));

        let override_config = ProviderConfig {
            provider: "aliyun".to_string(),
            api_protocol: ApiProtocol::Anthropic,
            api_key: "secret".to_string(),
            model: "aws-claude-sonnet-4-6".to_string(),
            base_url: "https://example.com/v1".to_string(),
            timeout_secs: 120,
            max_llm_output_tokens: 4096,
            max_llm_input_tokens: 100_000,
        };
        let override_banner = render_startup_banner(
            ".test_mem",
            &override_config,
            std::path::Path::new(".test_mem/audit/api_audit.jsonl"),
            std::path::Path::new(".test_mem/audit/action_audit.json"),
            BashApprovalMode::Ask,
        );
        assert!(override_banner.contains(&format!("{ANSI_HIGHLIGHT}anthropic")));
        assert!(override_banner.contains(&format!("{ANSI_HIGHLIGHT}https://example.com/v1")));
        assert!(override_banner.contains(&format!("{ANSI_HIGHLIGHT}aws-claude-sonnet-4-6")));
        let table_lines: Vec<&str> = override_banner
            .lines()
            .filter(|line| line.starts_with('│') || line.starts_with('├'))
            .collect();
        let first_width = display_width(table_lines[0]);
        assert!(table_lines
            .iter()
            .all(|line| display_width(line) == first_width));
    }

    #[test]
    fn startup_banner_does_not_highlight_custom_provider_model_or_base_url() {
        let config = ProviderConfig {
            provider: "private".to_string(),
            api_protocol: ApiProtocol::Anthropic,
            api_key: "secret".to_string(),
            model: "aws-claude-sonnet-4-6".to_string(),
            base_url: "https://your-private-gateway.example/v1".to_string(),
            timeout_secs: 120,
            max_llm_output_tokens: 4096,
            max_llm_input_tokens: 100_000,
        };
        let banner = render_startup_banner(
            ".test_mem",
            &config,
            std::path::Path::new(".test_mem/audit/api_audit.jsonl"),
            std::path::Path::new(".test_mem/audit/action_audit.json"),
            BashApprovalMode::Ask,
        );

        assert!(!banner.contains(&format!(
            "{ANSI_HIGHLIGHT}https://your-private-gateway.example/v1"
        )));
        assert!(!banner.contains(&format!("{ANSI_HIGHLIGHT}aws-claude-sonnet-4-6")));
    }

    #[test]
    fn help_lists_all_env_backed_options() {
        let help = help_text();
        for expected in [
            "\x1b[1mPrecedence:",
            "command line options override process env values; process env overrides defaults.\x1b[0m",
            "cp env_template env",
            "source /path/to/your/env",
            "--space",
            "TIMEM_SPACE",
            "--gateway-provider",
            "TIMEM_GATEWAY_PROVIDER",
            "--api-protocol",
            "TIMEM_API_PROTOCOL",
            "--base-url",
            "TIMEM_BASE_URL",
            "--model",
            "TIMEM_MODEL",
            "--api-key",
            "TIMEM_API_KEY",
            "--data-dir",
            "TIMEM_DATA_DIR",
            "--timeout",
            "TIMEM_TIMEOUT",
            "--max-llm-output",
            "TIMEM_MAX_LLM_OUTPUT",
            "--max-llm-input",
            "TIMEM_MAX_LLM_INPUT",
            "--bash-approval",
            "TIMEM_BASH_APPROVAL",
            "Interactive commands:",
            "/prof",
            "Ctrl+C cancels the current input",
            "cancels the current turn",
        ] {
            assert!(help.contains(expected), "missing help item: {expected}");
        }
        assert!(!help.contains("--profile"));
        assert!(!help.contains("TIMEM_PROFILE"));
    }

    #[test]
    fn env_template_exports_values_for_plain_source() {
        let template = include_str!("../../env_template");
        for key in [
            "TIMEM_GATEWAY_PROVIDER",
            "TIMEM_API_PROTOCOL",
            "TIMEM_API_KEY",
            "TIMEM_BASE_URL",
            "TIMEM_MODEL",
            "TIMEM_SPACE",
            "TIMEM_DATA_DIR",
            "TIMEM_TIMEOUT",
            "TIMEM_MAX_LLM_OUTPUT",
            "TIMEM_MAX_LLM_INPUT",
            "TIMEM_BASH_APPROVAL",
            "DASHSCOPE_API_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
        ] {
            assert!(
                template.contains(&format!("# export {key}=")),
                "env_template must export {key} so `source env` reaches child processes"
            );
            assert!(
                !template.contains(&format!("# {key}=")),
                "env_template must not use non-exported assignments for {key}"
            );
        }
    }

    #[test]
    fn sourced_env_file_reaches_child_process_without_set_a() {
        let mut path = std::env::temp_dir();
        path.push(format!("timem_source_env_{}.sh", epoch_millis()));
        fs::write(
            &path,
            "export TIMEM_GATEWAY_PROVIDER=custom\nexport TIMEM_API_PROTOCOL=anthropic\n",
        )
        .unwrap();
        let script = format!(
            ". \"{}\"; printf '%s|%s' \"$TIMEM_GATEWAY_PROVIDER\" \"$TIMEM_API_PROTOCOL\"",
            path.display()
        );
        let output = Command::new("sh").arg("-c").arg(script).output().unwrap();
        let _ = fs::remove_file(&path);

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "custom|anthropic");
    }

    #[test]
    fn approval_prompt_shows_risk_command_and_keyboard_choices() {
        let prompt = render_user_approval_prompt(&ApprovalRequest {
            approval_id: "approval_test".to_string(),
            action: "run_bash".to_string(),
            command: "uname -s".to_string(),
            read_back_command: "pwd".to_string(),
            reason: "run_bash_requires_user_approval".to_string(),
            risk: "local_shell_command".to_string(),
            intent: "Inspect OS identity.".to_string(),
        });

        assert!(prompt.contains("需要确认执行这个命令"));
        assert!(prompt.contains("command: uname -s"));
        assert!(prompt.contains("intent: Inspect OS identity."));
        assert!(prompt.contains("使用 ←/→ 或 ↑/↓ 选择"));
        assert!(!prompt.contains("输入 yes"));
        assert!(!prompt.contains("action: run_bash"));
        assert!(!prompt.contains("risk: local_shell_command"));
        assert!(!prompt.contains("read_back_command"));
    }

    #[test]
    fn approval_choices_highlight_current_selection() {
        assert_eq!(
            render_approval_choices(ApprovalChoice::Deny),
            "  执行一次   \x1b[7m[ 取消 ]\x1b[0m"
        );
        assert_eq!(
            render_approval_choices(ApprovalChoice::Allow),
            "\x1b[7m[ 执行一次 ]\x1b[0m   取消"
        );
    }

    #[test]
    fn round_limit_prompt_uses_keyboard_choices_and_defaults_to_continue() {
        let prompt = render_round_limit_prompt(20);
        assert!(prompt.contains("本轮已达到最大交互次数 20"));
        assert!(prompt.contains("重新充值 rounds_remaining 为 20"));
        assert!(prompt.contains("使用 ←/→ 或 ↑/↓ 选择"));
        assert!(!prompt.contains("输入 yes"));
        assert_eq!(
            render_round_limit_choices(ApprovalChoice::Allow),
            "\x1b[7m[ 继续 ]\x1b[0m   停止"
        );
        assert_eq!(
            render_round_limit_choices(ApprovalChoice::Deny),
            "  继续   \x1b[7m[ 停止 ]\x1b[0m"
        );
    }

    #[test]
    fn approval_key_reader_accepts_arrow_enter_and_shortcuts() {
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![b'\r'])),
            ApprovalKey::Enter
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![b'\n'])),
            ApprovalKey::Enter
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![27, b'[', b'D'])),
            ApprovalKey::Select(ApprovalChoice::Allow)
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![27, b'[', b'C'])),
            ApprovalKey::Select(ApprovalChoice::Deny)
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![27, b'[', b'A'])),
            ApprovalKey::Toggle
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![27, b'[', b'B'])),
            ApprovalKey::Toggle
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![b' '])),
            ApprovalKey::Toggle
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![b'y'])),
            ApprovalKey::Select(ApprovalChoice::Allow)
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(vec![b'n'])),
            ApprovalKey::Select(ApprovalChoice::Deny)
        );
    }

    #[test]
    fn approval_key_reader_consumes_long_escape_sequences() {
        assert_eq!(
            read_approval_key(&mut Cursor::new(b"\x1b[200~pasted".to_vec())),
            ApprovalKey::Other
        );
        assert_eq!(
            read_approval_key(&mut Cursor::new(b"\x1b[201~".to_vec())),
            ApprovalKey::Other
        );
    }

    #[test]
    fn submitted_user_input_strips_terminal_control_sequences() {
        let input = "把\x1b[200~20260623-211820.mp4\x1b[201~ 的分辨率降低一些\r\r\x1b[A\x1b[B\x1b[C\x1b[D\x03";
        assert_eq!(
            sanitize_user_input(input),
            "把20260623-211820.mp4 的分辨率降低一些"
        );
    }

    #[test]
    fn multiline_paste_displays_bold_placeholder_but_submits_full_text() {
        let mut buffer = ShellInputBuffer::default();
        buffer.insert_text("请处理 ");
        buffer.insert_paste("alpha\nbeta\ngamma");
        buffer.insert_text(" 谢谢");

        assert_eq!(buffer.display_plain(), "请处理 [ pasted 3 lines ] 谢谢");
        assert_eq!(
            buffer.display_styled(),
            format!("请处理 \x1b[1m[ pasted 3 lines ]{ANSI_RESET} 谢谢")
        );
        assert_eq!(buffer.submit_text(false), "请处理 alpha\nbeta\ngamma 谢谢");
        assert_eq!(buffer.dirty_paste_summary(), None);
    }

    #[test]
    fn editing_multiline_placeholder_prompts_and_can_recover_backing_content() {
        let mut buffer = ShellInputBuffer::default();
        buffer.insert_paste("alpha\nbeta\ngamma");
        buffer.move_left();
        buffer.move_left();
        buffer.delete_before_cursor();

        assert_eq!(buffer.display_plain(), "[ pasted 3 line ]");
        assert_eq!(
            buffer.dirty_paste_summary(),
            Some(PasteRecoverySummary {
                dirty_count: 1,
                total_lines: 3,
            })
        );
        assert_eq!(buffer.submit_text(false), "[ pasted 3 line ]");
        assert_eq!(buffer.submit_text(true), "alpha\nbeta\ngamma");
        assert_eq!(
            buffer.display_styled(),
            format!("\x1b[1m[ pasted 3 line ]{ANSI_RESET}")
        );
    }

    #[test]
    fn deleting_next_placeholder_char_prompts_and_can_submit_literal() {
        let mut buffer = ShellInputBuffer::default();
        buffer.insert_text("x");
        buffer.insert_paste("a\nb");
        buffer.insert_text("y");
        while buffer.display_prefix_before_cursor() != "x" {
            buffer.move_left();
        }
        buffer.delete_at_cursor();

        assert_eq!(buffer.display_plain(), "x pasted 2 lines ]y");
        assert_eq!(
            buffer.dirty_paste_summary(),
            Some(PasteRecoverySummary {
                dirty_count: 1,
                total_lines: 2,
            })
        );
        assert_eq!(buffer.submit_text(false), "x pasted 2 lines ]y");
        assert_eq!(buffer.submit_text(true), "xa\nby");
    }

    #[test]
    fn editing_text_around_placeholder_keeps_paste_association() {
        let mut buffer = ShellInputBuffer::default();
        buffer.insert_text("this is ");
        buffer.insert_paste("a\nb\nc\nd\ne");
        buffer.insert_text(" done");
        while buffer.display_prefix_before_cursor() != "this is " {
            buffer.move_left();
        }
        buffer.delete_before_cursor();
        buffer.delete_before_cursor();
        buffer.delete_before_cursor();
        buffer.insert_text("was ");

        assert_eq!(buffer.display_plain(), "this was [ pasted 5 lines ] done");
        assert_eq!(buffer.dirty_paste_summary(), None);
        assert_eq!(buffer.submit_text(false), "this was a\nb\nc\nd\ne done");
    }

    #[test]
    fn single_line_paste_stays_plain_text() {
        let mut buffer = ShellInputBuffer::default();
        buffer.insert_paste("just one line");

        assert_eq!(buffer.display_plain(), "just one line");
        assert_eq!(buffer.display_styled(), "just one line");
        assert_eq!(buffer.submit_text(false), "just one line");
    }

    #[test]
    fn paste_recovery_prompt_and_choices_are_keyboard_driven() {
        let summary = PasteRecoverySummary {
            dirty_count: 1,
            total_lines: 5,
        };
        let prompt = render_paste_recovery_prompt(&summary);
        assert!(prompt.contains("粘贴关联标签可能被误编辑"));
        assert!(prompt.contains("原始粘贴内容共 5 行"));
        assert!(prompt.contains("使用 ←/→ 或 ↑/↓ 选择"));
        assert_eq!(
            render_paste_recovery_choices(ApprovalChoice::Allow),
            "\x1b[7m[ Yes ]\x1b[0m   No"
        );
        assert_eq!(
            render_paste_recovery_choices(ApprovalChoice::Deny),
            "  Yes   \x1b[7m[ No ]\x1b[0m"
        );
    }

    #[test]
    fn pasted_line_count_handles_common_newline_shapes() {
        assert_eq!(pasted_line_count("a"), 1);
        assert_eq!(pasted_line_count("a\nb"), 2);
        assert_eq!(pasted_line_count("a\r\nb\r\nc"), 3);
        assert_eq!(pasted_line_count("a\nb\n"), 3);
    }

    #[test]
    fn shell_key_reader_converts_bracketed_paste_to_paste_event() {
        let mut input = Cursor::new(b"\x1b[200~a\nb\nc\x1b[201~".to_vec());
        assert_eq!(
            read_shell_key(&mut input),
            ShellInputKey::Paste("a\nb\nc".to_string())
        );
    }

    #[test]
    fn shell_key_reader_handles_navigation_and_delete_keys() {
        assert_eq!(
            read_shell_key(&mut Cursor::new(vec![27, b'[', b'D'])),
            ShellInputKey::Left
        );
        assert_eq!(
            read_shell_key(&mut Cursor::new(vec![27, b'[', b'C'])),
            ShellInputKey::Right
        );
        assert_eq!(
            read_shell_key(&mut Cursor::new(vec![27, b'[', b'A'])),
            ShellInputKey::HistoryPrev
        );
        assert_eq!(
            read_shell_key(&mut Cursor::new(vec![27, b'[', b'B'])),
            ShellInputKey::HistoryNext
        );
        assert_eq!(
            read_shell_key(&mut Cursor::new(vec![27, b'[', b'3', b'~'])),
            ShellInputKey::Delete
        );
    }

    #[test]
    fn submitted_user_line_rewrite_clears_wrapped_input_rows() {
        let rendered = render_submitted_user_line_rewrite("abcdef", false, 10, "12:00:00");
        assert!(rendered.starts_with("\x1b[3F\r\x1b[J"));
        assert!(rendered.ends_with("[12:00:00] \x1b[1mYou\x1b[0m ❯❯ abcdef\n"));
    }

    #[test]
    fn user_input_prompt_uses_bold_you_and_double_arrow() {
        assert_eq!(
            render_user_input_prompt("12:00:00"),
            "[12:00:00] \x1b[1mYou\x1b[0m ❯❯ "
        );
    }

    #[test]
    fn submitted_user_line_rewrite_clears_status_and_wrapped_rows() {
        let rendered = render_submitted_user_line_rewrite("abcdef", true, 10, "12:00:00");
        assert!(rendered.starts_with("\x1b[4F\r\x1b[J"));
    }

    #[test]
    fn wrapped_terminal_rows_counts_cjk_display_width() {
        assert_eq!(
            wrapped_terminal_rows(display_width("[12:00:00] You ❯❯ 你好"), 20),
            2
        );
    }
}
