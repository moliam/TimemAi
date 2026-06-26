use agent_core::{AgentCore, ApprovalRequest, BashApprovalMode, CoreStep, UsageStats};
use crossterm::event::Event;
use reedline::{
    default_emacs_keybindings, EditCommand, EditMode, Emacs, FileBackedHistory, Highlighter,
    KeyCode, KeyModifiers, Keybindings, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, ReedlineEvent, ReedlineRawEvent, Signal, StyledText,
};
use serde_json::json;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use timem_shell::{
    action_audit_path, action_status_hint, append_audit, audit_path, call_model_with_cancel,
    data_root, load_workspace_dirs, local_time_label, memory_path,
    observation_events_from_model_response, parse_cli_args, provider_config_from_env,
    render_final_response_at, render_prof_report, render_shell_status_bar, render_thinking_view_at,
    save_workspace_dirs, supporting_context, ApiProtocol, ModelDirection, ObservationEvent,
    ObservationPanel, RuntimeProfiler, ShellStatusMessage, ShellStatusSnapshot, ShellStatusTone,
    ThinkingViewSnapshot, SPINNER_ICONS, TIMEM_LOGO,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const STATIC_PROMPT: &str = include_str!("../../resources/static_v1.json");
const ANSI_RESET: &str = timem_shell::ANSI_RESET;
const ANSI_BOLD: &str = timem_shell::ANSI_BOLD;
const ANSI_HIGHLIGHT: &str = "\x1b[1;33m";
const PASTE_START_MARKER: char = '\u{2063}';
const PASTE_END_MARKER: char = '\u{2064}';
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
    let mut workspace_pending = !load_workspace_dirs().is_empty();

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
    println!("输入 /prof 查看运行 profiling；输入 /workspace 管理工作区；输入 /exit 退出；Ctrl+C 取消当前输入。\n");

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

        if input == "/workspace" {
            prompt_status.clear_before_exit();
            if run_workspace_menu() {
                workspace_pending = true;
                println!("工作区已更新。");
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

        let workspace_ctx: Option<String> = if workspace_pending {
            workspace_pending = false;
            let dirs = load_workspace_dirs();
            if dirs.is_empty() {
                None
            } else {
                let lines: Vec<String> = dirs.iter().map(|d| format!("- {d}")).collect();
                Some(format!(
                    "workspace_dirs (model reference; not a shell restriction):\n{}",
                    lines.join("\n")
                ))
            }
        } else {
            None
        };
        let mut status = ThinkingStatus::start(&config.provider, &config.model);
        let (text, stats, elapsed) = run_turn(
            &mut core,
            &input,
            &session,
            &audit_file,
            &mut config,
            workspace_ctx.as_deref(),
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
    let mut user_wait_this_turn = Duration::ZERO;
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
                match call_model_with_cancel(config, &prompt, audit_file, || {
                    TURN_CANCEL_REQUESTED.load(Ordering::SeqCst)
                }) {
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
                let user_wait_start = Instant::now();
                let approved = if interactive_approval {
                    request_user_approval(&request)
                } else {
                    false
                };
                user_wait_this_turn = user_wait_this_turn.saturating_add(user_wait_start.elapsed());
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
                let user_wait_start = Instant::now();
                let should_continue =
                    interactive_approval && request_round_limit_continue(max_rounds);
                user_wait_this_turn = user_wait_this_turn.saturating_add(user_wait_start.elapsed());
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
    let elapsed = start.elapsed().saturating_sub(user_wait_this_turn);
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
    started_at: Instant,
    paused_total: Arc<Mutex<Duration>>,
    paused_since: Option<Instant>,
}

impl ThinkingStatus {
    fn start(provider: &str, model: &str) -> Self {
        let started_at = Instant::now();
        let paused_total = Arc::new(Mutex::new(Duration::ZERO));
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
                elapsed_secs: 0,
            },
            observations: ObservationPanel::default(),
        }));
        let running = Arc::new(AtomicBool::new(true));
        let rendered_lines = Arc::new(Mutex::new(0));
        render_thinking(&state.lock().unwrap(), &rendered_lines);
        let thread_state = Arc::clone(&state);
        let thread_running = Arc::clone(&running);
        let thread_rendered_lines = Arc::clone(&rendered_lines);
        let thread_paused_total = Arc::clone(&paused_total);
        let handle = thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1000));
                if let Ok(mut snapshot) = thread_state.lock() {
                    snapshot.status.tick = snapshot.status.tick.wrapping_add(1);
                    snapshot.status.elapsed_secs =
                        active_elapsed_secs(started_at, &thread_paused_total);
                    rerender_thinking(&snapshot, &thread_rendered_lines);
                }
            }
        });
        Self {
            state,
            running,
            rendered_lines,
            handle: Some(handle),
            started_at,
            paused_total,
            paused_since: None,
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
        self.paused_since = Some(Instant::now());
        clear_thinking_block(&self.rendered_lines);
    }

    fn resume_after_user_approval(&mut self) {
        if self.handle.is_some() {
            return;
        }
        if let Some(paused_since) = self.paused_since.take() {
            if let Ok(mut paused_total) = self.paused_total.lock() {
                *paused_total = paused_total.saturating_add(paused_since.elapsed());
            }
        }
        self.running.store(true, Ordering::Relaxed);
        if let Ok(mut state) = self.state.lock() {
            state.status.elapsed_secs = active_elapsed_secs(self.started_at, &self.paused_total);
            render_thinking(&state, &self.rendered_lines);
        }
        let thread_state = Arc::clone(&self.state);
        let thread_running = Arc::clone(&self.running);
        let thread_rendered_lines = Arc::clone(&self.rendered_lines);
        let thread_paused_total = Arc::clone(&self.paused_total);
        let started_at = self.started_at;
        self.handle = Some(thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1000));
                if let Ok(mut snapshot) = thread_state.lock() {
                    snapshot.status.tick = snapshot.status.tick.wrapping_add(1);
                    snapshot.status.elapsed_secs =
                        active_elapsed_secs(started_at, &thread_paused_total);
                    rerender_thinking(&snapshot, &thread_rendered_lines);
                }
            }
        }));
    }
}

fn active_elapsed_secs(started_at: Instant, paused_total: &Arc<Mutex<Duration>>) -> u64 {
    let paused = paused_total
        .lock()
        .map(|duration| *duration)
        .unwrap_or(Duration::ZERO);
    started_at.elapsed().saturating_sub(paused).as_secs()
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
    let Some(raw_value) = read_tty_line_cancelable() else {
        println!("\n已取消配置修改。");
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

fn run_workspace_menu() -> bool {
    loop {
        let mut dirs = load_workspace_dirs();
        let Some(selection) = choose_workspace_item(&dirs) else {
            println!("已退出 workspace 配置。");
            return false;
        };
        match selection {
            WorkspaceSelection::Add => {
                print!("\n请输入要加入 workspace 的目录（留空取消）：");
                let _ = io::stdout().flush();
                let Some(raw_value) = read_tty_line_cancelable() else {
                    println!("\n已取消 workspace 修改。");
                    return false;
                };
                let value = raw_value.trim();
                if value.is_empty() {
                    continue;
                }
                let normalized = normalize_workspace_dir(value);
                if dirs.iter().any(|dir| dir == &normalized) {
                    println!("目录已存在：{normalized}");
                    continue;
                }
                dirs.push(normalized.clone());
                dirs.sort();
                if let Err(err) = save_workspace_dirs(&dirs) {
                    println!("保存 workspace 失败：{err}");
                    return false;
                }
                println!("已加入 workspace：{normalized}");
                return true;
            }
            WorkspaceSelection::Dir(index) => {
                if index >= dirs.len() {
                    continue;
                }
                let dir = dirs[index].clone();
                if confirm_workspace_delete(&dir) {
                    dirs.remove(index);
                    if let Err(err) = save_workspace_dirs(&dirs) {
                        println!("保存 workspace 失败：{err}");
                        return false;
                    }
                    println!("已从 workspace 移除：{dir}");
                    return true;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceSelection {
    Dir(usize),
    Add,
}

fn choose_workspace_item(dirs: &[String]) -> Option<WorkspaceSelection> {
    println!("\nWorkspace 目录用于提示模型参考资料位置，不限制模型只能在这些目录工作。");
    println!("使用 ↑/↓ 选择，回车确认，Esc/Ctrl+C 返回。\n");
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
    print!("{}", render_workspace_menu(dirs, selected));
    let _ = io::stdout().flush();
    let item_count = dirs.len() + 1;
    let rendered_line_count = workspace_menu_line_count(dirs);
    let result = loop {
        match read_menu_key(&mut input) {
            MenuKey::Up => selected = selected.saturating_sub(1),
            MenuKey::Down => selected = (selected + 1).min(item_count.saturating_sub(1)),
            MenuKey::Enter => {
                break if selected < dirs.len() {
                    Some(WorkspaceSelection::Dir(selected))
                } else {
                    Some(WorkspaceSelection::Add)
                };
            }
            MenuKey::Cancel => break None,
            MenuKey::Other => {}
        }
        print!(
            "\x1b[{}F{}",
            rendered_line_count,
            render_workspace_menu(dirs, selected)
        );
        let _ = io::stdout().flush();
    };
    terminal_mode.restore();
    println!();
    result
}

fn workspace_menu_line_count(dirs: &[String]) -> usize {
    dirs.len().max(1) + 1
}

fn render_workspace_menu(dirs: &[String], selected: usize) -> String {
    let mut lines = Vec::new();
    if dirs.is_empty() {
        lines.push("  （暂无 workspace 目录）".to_string());
    } else {
        for (idx, dir) in dirs.iter().enumerate() {
            let marker = if idx == selected { "▶" } else { " " };
            let line = format!("{marker} {dir}");
            if idx == selected {
                lines.push(format!("\x1b[7m{line}\x1b[0m"));
            } else {
                lines.push(line);
            }
        }
    }
    let add_idx = dirs.len();
    let add_line = if add_idx == selected {
        "\x1b[7m▶ Add...\x1b[0m".to_string()
    } else {
        "  Add...".to_string()
    };
    lines.push(add_line);
    lines.join("\n") + "\n"
}

fn confirm_workspace_delete(dir: &str) -> bool {
    println!("\n已选择：{dir}");
    println!("是否从 workspace 列表中移除？");
    match choose_with_keyboard(render_workspace_delete_choices, ApprovalChoice::Deny) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

fn render_workspace_delete_choices(selected: ApprovalChoice) -> String {
    match selected {
        ApprovalChoice::Allow => "\x1b[7m[ 删除 ]\x1b[0m   保留".to_string(),
        ApprovalChoice::Deny => "  删除   \x1b[7m[ 保留 ]\x1b[0m".to_string(),
    }
}

fn normalize_workspace_dir(value: &str) -> String {
    let expanded = expand_tilde(value.trim());
    std::fs::canonicalize(&expanded)
        .unwrap_or(expanded)
        .to_string_lossy()
        .to_string()
}

fn expand_tilde(value: &str) -> PathBuf {
    if value == "~" {
        return home_dir();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    Path::new(value).to_path_buf()
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
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

fn read_tty_line_cancelable() -> Option<String> {
    TURN_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    let _sigint_guard = SigintGuard::install();
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
    let mut out = String::new();
    let result = loop {
        if TURN_CANCEL_REQUESTED.load(Ordering::SeqCst) {
            break None;
        }
        let Some(byte) = read_cancelable_byte(&mut input, fd) else {
            break None;
        };
        match byte {
            b'\r' | b'\n' => {
                println!();
                break Some(out);
            }
            3 | 4 | 27 => {
                break None;
            }
            8 | 127 => {
                if let Some(ch) = out.pop() {
                    erase_rendered_char(ch);
                }
            }
            byte if byte.is_ascii_control() => {}
            first => {
                let mut bytes = vec![first];
                if first >= 0x80 {
                    let expected_len = utf8_expected_len(first);
                    while bytes.len() < expected_len {
                        let Some(next) = read_cancelable_byte(&mut input, fd) else {
                            break;
                        };
                        bytes.push(next);
                    }
                }
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    out.push_str(text);
                    print!("{text}");
                    let _ = io::stdout().flush();
                }
            }
        }
    };
    terminal_mode.restore();
    let _ = consume_turn_cancel_request();
    result
}

fn read_cancelable_byte(input: &mut impl Read, fd: i32) -> Option<u8> {
    loop {
        if TURN_CANCEL_REQUESTED.load(Ordering::SeqCst) {
            return None;
        }
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let poll_result = unsafe { libc::poll(&mut poll_fd, 1, 100) };
        if poll_result == 0 {
            continue;
        }
        if poll_result < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if poll_fd.revents & libc::POLLIN == 0 {
            continue;
        }
        let mut buf = [0u8; 1];
        match input.read(&mut buf) {
            Ok(1) => return Some(buf[0]),
            Ok(_) => return None,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}

fn utf8_expected_len(first: u8) -> usize {
    if first & 0b1111_1000 == 0b1111_0000 {
        4
    } else if first & 0b1111_0000 == 0b1110_0000 {
        3
    } else if first & 0b1110_0000 == 0b1100_0000 {
        2
    } else {
        1
    }
}

fn erase_rendered_char(ch: char) {
    let width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
    for _ in 0..width {
        print!("\x08 \x08");
    }
    let _ = io::stdout().flush();
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
        .filter(|ch| {
            *ch == '\t'
                || *ch == '\n'
                || *ch == PASTE_START_MARKER
                || *ch == PASTE_END_MARKER
                || (!ch.is_control() && *ch != '\r')
        })
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

fn normalize_newlines(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

fn pasted_line_count(text: &str) -> usize {
    normalize_newlines(text).split('\n').count()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PasteRecoverySummary {
    dirty_count: usize,
    total_lines: usize,
}

fn strip_paste_markers(input: &str) -> String {
    input
        .chars()
        .filter(|ch| *ch != PASTE_START_MARKER && *ch != PASTE_END_MARKER)
        .collect()
}

fn paste_recovery_summary_from_markers(
    input: &str,
    records: &[PasteRecord],
) -> Option<PasteRecoverySummary> {
    let mut dirty_count = 0usize;
    let mut total_lines = 0usize;
    for (idx, marked) in paste_marker_segments(input).into_iter().enumerate() {
        let Some(record) = records.get(idx) else {
            continue;
        };
        if marked != record.placeholder {
            dirty_count += 1;
            total_lines += pasted_line_count(&record.content);
        }
    }
    (dirty_count > 0).then_some(PasteRecoverySummary {
        dirty_count,
        total_lines,
    })
}

fn resolve_paste_markers(input: &str, records: &[PasteRecord], restore_dirty: bool) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    let mut record_idx = 0usize;
    while let Some(ch) = chars.next() {
        if ch != PASTE_START_MARKER {
            if ch != PASTE_END_MARKER {
                out.push(ch);
            }
            continue;
        }
        let mut marked = String::new();
        for next in chars.by_ref() {
            if next == PASTE_END_MARKER {
                break;
            }
            marked.push(next);
        }
        if let Some(record) = records.get(record_idx) {
            if restore_dirty || marked == record.placeholder {
                out.push_str(&record.content);
            } else {
                out.push_str(&marked);
            }
        } else {
            out.push_str(&marked);
        }
        record_idx += 1;
    }
    out
}

fn paste_marker_segments(input: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != PASTE_START_MARKER {
            continue;
        }
        let mut marked = String::new();
        for next in chars.by_ref() {
            if next == PASTE_END_MARKER {
                break;
            }
            marked.push(next);
        }
        segments.push(marked);
    }
    segments
}

enum ShellReadline {
    Line { text: String, display: String },
    Interrupted,
    Eof,
    Error(String),
}

struct ShellLineEditor {
    editor: Reedline,
    paste_records: SharedPasteRecords,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PasteRecord {
    placeholder: String,
    content: String,
}

type SharedPasteRecords = Arc<Mutex<Vec<PasteRecord>>>;

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
        let history = FileBackedHistory::with_file(500, history_file).ok();
        let paste_records = Arc::new(Mutex::new(Vec::new()));
        let edit_mode = Box::new(TimemEditMode::new(paste_records.clone()));
        let mut editor = Reedline::create()
            .with_edit_mode(edit_mode)
            .with_highlighter(Box::new(TimemPasteHighlighter))
            .with_ansi_colors(true);
        if let Some(history) = history {
            editor = editor.with_history(Box::new(history));
        }
        Self {
            editor,
            paste_records,
        }
    }

    fn readline(&mut self, prompt: &str) -> ShellReadline {
        if let Ok(mut records) = self.paste_records.lock() {
            records.clear();
        }
        let prompt = TimemReedlinePrompt {
            indicator: prompt.to_string(),
        };
        let _keyboard_guard = ReedlineKeyboardProtocolGuard::enter();
        match self.editor.read_line(&prompt) {
            Ok(Signal::Success(text)) => {
                let queued = drain_queued_tty_input();
                if queued.interrupted {
                    TURN_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
                    return ShellReadline::Interrupted;
                }
                let raw_input = merge_queued_input(text, &queued);
                let raw = sanitize_user_input(&raw_input);
                let records = self
                    .paste_records
                    .lock()
                    .map(|records| records.clone())
                    .unwrap_or_default();
                let summary = paste_recovery_summary_from_markers(&raw, &records);
                let restore_dirty = summary
                    .as_ref()
                    .map(|summary| matches!(choose_paste_recovery(summary), ApprovalChoice::Allow))
                    .unwrap_or(false);
                let text = resolve_paste_markers(&raw, &records, restore_dirty);
                let display = strip_paste_markers(&raw);
                ShellReadline::Line { display, text }
            }
            Ok(Signal::CtrlC) => ShellReadline::Interrupted,
            Ok(Signal::CtrlD) => ShellReadline::Eof,
            Ok(_) => ShellReadline::Interrupted,
            Err(err) => ShellReadline::Error(format!("readline_failed: {err}")),
        }
    }
}

#[derive(Default)]
struct QueuedInputDrain {
    text: String,
    interrupted: bool,
}

fn merge_queued_input(mut submitted: String, queued: &QueuedInputDrain) -> String {
    if !queued.text.is_empty() {
        submitted.push('\n');
        submitted.push_str(&queued.text);
    }
    submitted
}

fn drain_queued_tty_input() -> QueuedInputDrain {
    let Ok(mut tty) = OpenOptions::new().read(true).open("/dev/tty") else {
        return QueuedInputDrain::default();
    };
    let fd = tty.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return QueuedInputDrain::default();
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return QueuedInputDrain::default();
    }

    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    let first = tty.read(&mut buf);
    match first {
        Ok(0) => {
            let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
            return QueuedInputDrain::default();
        }
        Ok(n) => bytes.extend_from_slice(&buf[..n]),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
            let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
            return QueuedInputDrain::default();
        }
        Err(_) => {
            let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
            return QueuedInputDrain::default();
        }
    }

    let started = Instant::now();
    let mut quiet_deadline = Instant::now() + Duration::from_millis(20);
    let hard_deadline = started + Duration::from_millis(200);
    while Instant::now() < quiet_deadline && Instant::now() < hard_deadline {
        match tty.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&buf[..n]);
                quiet_deadline = Instant::now() + Duration::from_millis(20);
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => break,
        }
    }
    let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };

    queued_input_drain_from_bytes(&bytes)
}

fn queued_input_drain_from_bytes(bytes: &[u8]) -> QueuedInputDrain {
    QueuedInputDrain {
        interrupted: bytes.contains(&3),
        text: String::from_utf8_lossy(bytes).replace('\x03', ""),
    }
}

struct ReedlineKeyboardProtocolGuard;

impl ReedlineKeyboardProtocolGuard {
    fn enter() -> Self {
        print!("{}", reedline_keyboard_protocol_enter_sequence());
        let _ = io::stdout().flush();
        Self
    }
}

impl Drop for ReedlineKeyboardProtocolGuard {
    fn drop(&mut self) {
        print!("{}", reedline_keyboard_protocol_exit_sequence());
        let _ = io::stdout().flush();
    }
}

fn reedline_keyboard_protocol_enter_sequence() -> &'static str {
    "\x1b[>4;2m\x1b[>1u"
}

fn reedline_keyboard_protocol_exit_sequence() -> &'static str {
    "\x1b[>4;0m\x1b[<u"
}

fn timem_reedline_keybindings() -> Keybindings {
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::Enter,
        ReedlineEvent::Edit(vec![EditCommand::InsertNewline]),
    );
    keybindings
}

struct TimemEditMode {
    inner: Emacs,
    paste_records: SharedPasteRecords,
}

impl TimemEditMode {
    fn new(paste_records: SharedPasteRecords) -> Self {
        Self {
            inner: Emacs::new(timem_reedline_keybindings()),
            paste_records,
        }
    }
}

impl EditMode for TimemEditMode {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let event: Event = event.into();
        if let Event::Key(key) = &event {
            if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
                return ReedlineEvent::Edit(vec![EditCommand::InsertNewline]);
            }
            if key.code == KeyCode::Char(' ') {
                return ReedlineEvent::Edit(vec![EditCommand::InsertString(" ".to_string())]);
            }
        }
        if let Event::Paste(body) = event {
            let content = normalize_newlines(&body);
            let line_count = pasted_line_count(&content);
            if line_count > 1 {
                let placeholder = format!("[ pasted {line_count} lines ]");
                if let Ok(mut records) = self.paste_records.lock() {
                    records.push(PasteRecord {
                        placeholder: placeholder.clone(),
                        content,
                    });
                }
                return ReedlineEvent::Edit(vec![EditCommand::InsertString(format!(
                    "{PASTE_START_MARKER}{placeholder}{PASTE_END_MARKER}"
                ))]);
            }
            return ReedlineEvent::Edit(vec![EditCommand::InsertString(content)]);
        }
        match ReedlineRawEvent::try_from(event) {
            Ok(event) => self.inner.parse_event(event),
            Err(_) => ReedlineEvent::None,
        }
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.inner.edit_mode()
    }
}

struct TimemPasteHighlighter;

impl Highlighter for TimemPasteHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        styled.push((nu_ansi_term::Style::new(), line.to_string()));
        for (start, end) in paste_marker_ranges(line) {
            styled.style_range(start, end, nu_ansi_term::Style::new().bold());
        }
        styled
    }
}

fn paste_marker_ranges(input: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut search_from = 0usize;
    while let Some(start_rel) = input[search_from..].find(PASTE_START_MARKER) {
        let start_marker = search_from + start_rel;
        let content_start = start_marker + PASTE_START_MARKER.len_utf8();
        let Some(end_rel) = input[content_start..].find(PASTE_END_MARKER) else {
            break;
        };
        let content_end = content_start + end_rel;
        if content_start < content_end {
            ranges.push((content_start, content_end));
        }
        search_from = content_end + PASTE_END_MARKER.len_utf8();
    }
    ranges
}

struct TimemReedlinePrompt {
    indicator: String,
}

impl Prompt for TimemReedlinePrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed(self.indicator.as_str())
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }
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
    "Usage:\n  timem [options]\n\n\x1b[1mPrecedence:\n  command line options override process env values; process env overrides defaults.\x1b[0m\n\nCreate a private env file from env_template, then load it explicitly:\n  cp env_template env\n  source /path/to/your/env\n\nRecommended run:\n  timem\n\nUseful env values to put in your env file:\n  export TIMEM_GATEWAY_PROVIDER=aliyun\n  export TIMEM_API_KEY=your_api_key_here\n  export TIMEM_MODEL=qwen-plus\n  export TIMEM_SPACE=.test_mem\n\nCommand line override example:\n  timem --data-dir data --space .test_mem --gateway-provider aliyun --model qwen-plus\n\nOptions:\n  --space <name>                 env TIMEM_SPACE; memory/audit space, default .test_mem\n  --gateway-provider <name>      env TIMEM_GATEWAY_PROVIDER; traffic platform / default base URL provider\n  --api-protocol <protocol>      env TIMEM_API_PROTOCOL; openai-compatible|openai-responses|anthropic\n  --base-url <url>               env TIMEM_BASE_URL; override provider default base URL\n  --model <name>                 env TIMEM_MODEL; model name\n  --api-key <key>                env TIMEM_API_KEY; API key, env is safer than shell history\n  --data-dir <path>              env TIMEM_DATA_DIR; data/config/memory/audit root\n  --timeout <seconds>            env TIMEM_TIMEOUT; provider HTTP timeout, default 120\n  --max-llm-input <n|100K>       env TIMEM_MAX_LLM_INPUT; max input context, default 100K\n  --max-llm-output <n|10K>       env TIMEM_MAX_LLM_OUTPUT; max output tokens, default 10K\n  --bash-approval <mode>         env TIMEM_BASH_APPROVAL; ask|approve, default ask\n  --once-json <text>             run one non-interactive turn and print JSON\n  --supporting-context <text>    append extra runtime context for --once-json/debug\n  -h, --help                     show this help\n\nInteractive commands:\n  /config                        edit runtime model and token settings\n  /workspace                     manage workspace directories shown to the model as reference context\n  /prof                          show runtime profiling for tokens, model wait/local time, and storage size\n\nInteractive keys:\n  Ctrl+C cancels the current input while editing, and cancels the current turn while Timem is thinking.\n\nProtocol defaults:\n  openai -> openai-responses; anthropic -> anthropic; others -> openai-compatible\n\nVendor fallback key env vars:\n  DASHSCOPE_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN\n"
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
        active_elapsed_secs, apply_config_value, boxed_config_table_at_width,
        cancelled_turn_result, config_field_value, consume_turn_cancel_request, display_width,
        epoch_millis, help_text, merge_queued_input, normalize_newlines, normalize_workspace_dir,
        paste_marker_ranges, paste_marker_segments, paste_recovery_summary_from_markers,
        pasted_line_count, queued_input_drain_from_bytes, random_spinner_tick, read_approval_key,
        read_menu_key, reedline_keyboard_protocol_enter_sequence,
        reedline_keyboard_protocol_exit_sequence, render_approval_choices, render_config_menu,
        render_expand_output_choices, render_expand_output_prompt, render_paste_recovery_choices,
        render_paste_recovery_prompt, render_round_limit_choices, render_round_limit_prompt,
        render_stale_context_choices, render_stale_context_prompt, render_startup_banner,
        render_submitted_user_line_rewrite, render_user_approval_prompt, render_user_input_prompt,
        render_workspace_delete_choices, render_workspace_menu, resolve_paste_markers,
        sanitize_user_input, stale_context_prompt_needed, strip_paste_markers,
        timem_reedline_keybindings, utf8_expected_len, workspace_menu_line_count,
        wrapped_terminal_rows, ApprovalChoice, ApprovalKey, ConfigField, ConfigRow,
        ConfigTableItem, MenuKey, PasteRecord, PasteRecoverySummary, QueuedInputDrain,
        TimemEditMode, TimemPasteHighlighter, TimemReedlinePrompt, ANSI_HIGHLIGHT,
        PASTE_END_MARKER, PASTE_START_MARKER, STALE_CONTEXT_IDLE, STALE_CONTEXT_TOKEN_THRESHOLD,
        STATIC_PROMPT, TURN_CANCEL_REQUESTED,
    };
    use agent_core::{AgentCore, ApprovalRequest, BashApprovalMode, CoreProfile};
    use crossterm::event::Event;
    use crossterm::event::KeyEvent;
    use reedline::{
        EditCommand, EditMode, Highlighter, KeyCode, KeyModifiers, Prompt, ReedlineEvent,
        ReedlineRawEvent,
    };
    use std::fs;
    use std::io::Cursor;
    use std::process::Command;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use timem_shell::{ApiProtocol, ProviderConfig, SPINNER_ICONS};
    use unicode_width::UnicodeWidthChar;

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
    fn active_elapsed_excludes_user_pause_duration() {
        let paused_total = Arc::new(Mutex::new(Duration::from_secs(3)));
        let elapsed = active_elapsed_secs(Instant::now() - Duration::from_secs(10), &paused_total);
        assert!((7..=8).contains(&elapsed), "elapsed={elapsed}");
    }

    #[test]
    fn cancelable_tty_line_reader_understands_utf8_widths() {
        assert_eq!(utf8_expected_len(b'a'), 1);
        assert_eq!(utf8_expected_len("é".as_bytes()[0]), 2);
        assert_eq!(utf8_expected_len("中".as_bytes()[0]), 3);
        assert_eq!(utf8_expected_len("😀".as_bytes()[0]), 4);
        assert_eq!(UnicodeWidthChar::width('中'), Some(2));
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
    fn workspace_menu_renders_dirs_placeholder_and_delete_choices() {
        let empty = render_workspace_menu(&[], 0);
        assert!(empty.contains("（暂无 workspace 目录）"));
        assert!(empty.contains("\x1b[7m▶ Add...\x1b[0m"));
        assert_eq!(workspace_menu_line_count(&[]), 2);

        let dirs = vec![
            "/Users/limo3/my_code/timem_shell".to_string(),
            "/tmp/other".to_string(),
        ];
        let selected_dir = render_workspace_menu(&dirs, 1);
        assert!(selected_dir.contains("/Users/limo3/my_code/timem_shell"));
        assert!(selected_dir.contains("\x1b[7m▶ /tmp/other\x1b[0m"));
        assert!(selected_dir.contains("  Add..."));
        assert_eq!(workspace_menu_line_count(&dirs), 3);

        let selected_add = render_workspace_menu(&dirs, 2);
        assert!(selected_add.contains("\x1b[7m▶ Add...\x1b[0m"));
        assert_eq!(
            render_workspace_delete_choices(ApprovalChoice::Allow),
            "\x1b[7m[ 删除 ]\x1b[0m   保留"
        );
        assert_eq!(
            render_workspace_delete_choices(ApprovalChoice::Deny),
            "  删除   \x1b[7m[ 保留 ]\x1b[0m"
        );
    }

    #[test]
    fn workspace_path_normalization_canonicalizes_existing_paths() {
        let dir = std::env::temp_dir().join(format!("timem_workspace_{}", epoch_millis()));
        fs::create_dir_all(&dir).unwrap();
        let nested = dir.join(".").join("child").join("..");
        fs::create_dir_all(dir.join("child")).unwrap();

        let normalized = normalize_workspace_dir(nested.to_str().unwrap());
        assert_eq!(
            normalized,
            dir.canonicalize().unwrap().to_string_lossy().to_string()
        );

        let missing = dir.join("missing").join("path");
        assert_eq!(
            normalize_workspace_dir(missing.to_str().unwrap()),
            missing.to_string_lossy().to_string()
        );
        fs::remove_dir_all(dir).unwrap();
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
            "/workspace",
            "/prof",
            "Ctrl+C cancels the current input",
            "cancels the current turn",
        ] {
            assert!(help.contains(expected), "missing help item: {expected}");
        }
        assert!(!help.contains("--profile"));
        assert!(!help.contains("TIMEM_PROFILE"));
        assert!(help.contains(
            "  --max-llm-input <n|100K>       env TIMEM_MAX_LLM_INPUT; max input context, default 100K"
        ));
        assert!(help.contains(
            "  --max-llm-output <n|10K>       env TIMEM_MAX_LLM_OUTPUT; max output tokens, default 10K"
        ));
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
    fn queued_paste_fallback_merges_pending_lines_into_one_submission() {
        let merged = merge_queued_input(
            "first line".to_string(),
            &QueuedInputDrain {
                text: "second line\nthird line\n".to_string(),
                interrupted: false,
            },
        );
        assert_eq!(merged, "first line\nsecond line\nthird line\n");
    }

    #[test]
    fn queued_paste_fallback_keeps_multiline_config_table_as_one_user_input() {
        let queued = queued_input_drain_from_bytes(
            "│ TIMEM_MODEL             │ qwen-plus │ 模型名称 │\n│ TIMEM_BASE_URL          │ https://example.com/v1 │ 网关 base url │\n"
                .as_bytes(),
        );
        let merged = merge_queued_input("请分析这段配置".to_string(), &queued);

        assert!(!queued.interrupted);
        assert_eq!(
            merged,
            "请分析这段配置\n│ TIMEM_MODEL             │ qwen-plus │ 模型名称 │\n│ TIMEM_BASE_URL          │ https://example.com/v1 │ 网关 base url │\n"
        );
        assert_eq!(merged.matches("│ TIMEM_").count(), 2);
    }

    #[test]
    fn queued_paste_fallback_marks_ctrl_c_and_removes_control_byte() {
        let queued = queued_input_drain_from_bytes(b"line 2\n\x03line 3\n");

        assert!(queued.interrupted);
        assert_eq!(queued.text, "line 2\nline 3\n");
    }

    #[test]
    fn bracketed_multiline_paste_sanitizes_to_single_multiline_text() {
        let pasted = "\x1b[200~alpha\nbeta\n中文；gamma\x1b[201~";

        assert_eq!(sanitize_user_input(pasted), "alpha\nbeta\n中文；gamma");
    }

    fn marked_placeholder(lines: usize) -> String {
        format!("{PASTE_START_MARKER}[ pasted {lines} lines ]{PASTE_END_MARKER}")
    }

    #[test]
    fn reedline_shift_enter_is_bound_to_insert_newline() {
        let bindings = timem_reedline_keybindings();
        assert_eq!(
            bindings.find_binding(KeyModifiers::SHIFT, KeyCode::Enter),
            Some(ReedlineEvent::Edit(vec![EditCommand::InsertNewline]))
        );
    }

    #[test]
    fn reedline_input_enables_keyboard_modifier_reporting() {
        let enter = reedline_keyboard_protocol_enter_sequence();
        assert!(enter.contains("\x1b[>4;2m"));
        assert!(enter.contains("\x1b[>1u"));

        let exit = reedline_keyboard_protocol_exit_sequence();
        assert!(exit.contains("\x1b[>4;0m"));
        assert!(exit.contains("\x1b[<u"));
    }

    #[test]
    fn reedline_shift_enter_event_inserts_newline_without_submit() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let mut mode = TimemEditMode::new(records);
        let event = ReedlineRawEvent::try_from(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT,
        )))
        .unwrap();

        assert_eq!(
            mode.parse_event(event),
            ReedlineEvent::Edit(vec![EditCommand::InsertNewline])
        );
    }

    #[test]
    fn reedline_multiline_paste_inserts_marked_placeholder_and_records_content() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let mut mode = TimemEditMode::new(records.clone());
        let event =
            ReedlineRawEvent::try_from(Event::Paste("alpha\r\nbeta\ngamma".to_string())).unwrap();

        assert_eq!(
            mode.parse_event(event),
            ReedlineEvent::Edit(vec![EditCommand::InsertString(marked_placeholder(3))])
        );
        assert_eq!(
            *records.lock().unwrap(),
            vec![PasteRecord {
                placeholder: "[ pasted 3 lines ]".to_string(),
                content: "alpha\nbeta\ngamma".to_string(),
            }]
        );
    }

    #[test]
    fn reedline_single_line_paste_stays_plain_text() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let mut mode = TimemEditMode::new(records.clone());
        let event = ReedlineRawEvent::try_from(Event::Paste("just one line".to_string())).unwrap();

        assert_eq!(
            mode.parse_event(event),
            ReedlineEvent::Edit(vec![EditCommand::InsertString("just one line".to_string())])
        );
        assert!(records.lock().unwrap().is_empty());
    }

    #[test]
    fn reedline_space_inserts_string_to_avoid_cjk_abbreviation_boundary_panic() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let mut mode = TimemEditMode::new(records);
        let event = ReedlineRawEvent::try_from(Event::Key(KeyEvent::new(
            KeyCode::Char(' '),
            KeyModifiers::NONE,
        )))
        .unwrap();

        assert_eq!(
            mode.parse_event(event),
            ReedlineEvent::Edit(vec![EditCommand::InsertString(" ".to_string())])
        );
    }

    #[test]
    fn paste_marker_helpers_expand_clean_placeholder_but_display_only_label() {
        let records = vec![PasteRecord {
            placeholder: "[ pasted 3 lines ]".to_string(),
            content: "alpha\nbeta\ngamma".to_string(),
        }];
        let raw = format!("请处理 {} 谢谢", marked_placeholder(3));

        assert_eq!(strip_paste_markers(&raw), "请处理 [ pasted 3 lines ] 谢谢");
        assert_eq!(paste_marker_segments(&raw), vec!["[ pasted 3 lines ]"]);
        assert_eq!(paste_recovery_summary_from_markers(&raw, &records), None);
        assert_eq!(
            resolve_paste_markers(&raw, &records, false),
            "请处理 alpha\nbeta\ngamma 谢谢"
        );
    }

    #[test]
    fn paste_highlighter_bolds_placeholder_inside_invisible_markers() {
        let raw = format!("请处理 {} 谢谢", marked_placeholder(3));
        let ranges = paste_marker_ranges(&raw);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&raw[ranges[0].0..ranges[0].1], "[ pasted 3 lines ]");

        let highlighted = TimemPasteHighlighter.highlight(&raw, raw.len());
        assert!(
            highlighted
                .buffer
                .iter()
                .any(|(style, text)| style.is_bold && text == "[ pasted 3 lines ]"),
            "pasted placeholder should render as bold: {:?}",
            highlighted
                .buffer
                .iter()
                .map(|(style, text)| (style.is_bold, text.as_str()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn dirty_paste_placeholder_can_submit_literal_or_restore_backing_content() {
        let records = vec![PasteRecord {
            placeholder: "[ pasted 3 lines ]".to_string(),
            content: "alpha\nbeta\ngamma".to_string(),
        }];
        let raw = format!("{PASTE_START_MARKER}[ pasted 3 line ]{PASTE_END_MARKER}");

        assert_eq!(
            paste_recovery_summary_from_markers(&raw, &records),
            Some(PasteRecoverySummary {
                dirty_count: 1,
                total_lines: 3,
            })
        );
        assert_eq!(
            resolve_paste_markers(&raw, &records, false),
            "[ pasted 3 line ]"
        );
        assert_eq!(
            resolve_paste_markers(&raw, &records, true),
            "alpha\nbeta\ngamma"
        );
    }

    #[test]
    fn editing_text_around_placeholder_keeps_paste_association() {
        let records = vec![PasteRecord {
            placeholder: "[ pasted 5 lines ]".to_string(),
            content: "a\nb\nc\nd\ne".to_string(),
        }];
        let raw = format!("this was {} done", marked_placeholder(5));

        assert_eq!(paste_recovery_summary_from_markers(&raw, &records), None);
        assert_eq!(
            resolve_paste_markers(&raw, &records, false),
            "this was a\nb\nc\nd\ne done"
        );
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
    fn pasted_line_count_and_normalization_handle_common_newline_shapes() {
        assert_eq!(normalize_newlines("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(pasted_line_count("a"), 1);
        assert_eq!(pasted_line_count("a\nb"), 2);
        assert_eq!(pasted_line_count("a\r\nb\r\nc"), 3);
        assert_eq!(pasted_line_count("a\nb\n"), 3);
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
    fn reedline_multiline_prompt_has_no_visible_prefix() {
        let prompt = TimemReedlinePrompt {
            indicator: "[12:00:00] You ❯❯ ".to_string(),
        };
        assert_eq!(prompt.render_prompt_multiline_indicator(), "");
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
