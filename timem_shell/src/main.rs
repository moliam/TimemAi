use agent_core::{AgentCore, ApprovalRequest, BashApprovalMode, CoreStep, UsageStats};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use serde_json::json;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use timem_shell::{
    action_status_hint, append_audit, audit_path, call_model, data_root, local_time_label,
    observation_events_from_model_response, parse_cli_args, provider_config_from_env,
    render_final_response_at, render_prof_report, render_shell_status_bar, render_thinking_view_at,
    supporting_context, ModelDirection, ObservationEvent, ObservationPanel, RuntimeProfiler,
    ShellStatusMessage, ShellStatusSnapshot, ShellStatusTone, ThinkingViewSnapshot, SPINNER_ICONS,
    TIMEM_LOGO,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const STATIC_PROMPT: &str = include_str!("../../resources/static_v1.json");
const ANSI_RESET: &str = timem_shell::ANSI_RESET;
const ANSI_HIGHLIGHT: &str = "\x1b[1;33m";
static TURN_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

struct ConfigRow {
    key: &'static str,
    value: String,
    desc: &'static str,
    highlight: bool,
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
    let config = match provider_config_from_env(&options, &env) {
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
    let memory_dir = audit_file
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("memory");
    let bash_approval_mode = bash_approval_mode_from_options(&options, &env);
    let mut profiler = RuntimeProfiler::default();
    let mut core = AgentCore::new(STATIC_PROMPT, config.core_profile(), &memory_dir);
    core.set_bash_approval_mode(bash_approval_mode);
    core.set_max_llm_context_tokens(config.max_llm_context_tokens);
    let session = session_id();

    if let Some(input) = options.once_json_input.as_deref() {
        let context = options.supporting_context.as_deref();
        let (text, stats, elapsed) = run_turn(
            &mut core,
            input,
            &session,
            &audit_file,
            &config,
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
            "max_llm_context_tokens":config.max_llm_context_tokens,
            "bash_approval":bash_approval_mode_label(bash_approval_mode)
        }),
    );

    println!("Timem native shell");
    print!(
        "{}",
        render_startup_banner(&space, &config, &audit_file, bash_approval_mode)
    );
    println!("输入 /prof 查看运行 profiling；输入 /exit 退出；Ctrl+C 取消当前输入。\n");

    let history_file = audit_file.with_file_name("shell_history.txt");
    let mut editor = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(err) => {
            eprintln!("[input_error] failed to initialize line editor: {err}");
            return;
        }
    };
    let _ = editor.load_history(&history_file);
    let mut prompt_status = PromptStatusBar::default();

    loop {
        let prompt = format!("[{}] 你 > ", time_label());
        let input = match editor.readline(&prompt) {
            Ok(input) => {
                if !input.trim().is_empty() {
                    let _ = editor.add_history_entry(input.as_str());
                    let _ = editor.save_history(&history_file);
                }
                input
            }
            Err(ReadlineError::Interrupted) => {
                prompt_status.show_info("已取消当前输入。Ctrl+D 退出。");
                continue;
            }
            Err(ReadlineError::Eof) => {
                prompt_status.clear_before_exit();
                println!("Bye.");
                break;
            }
            Err(err) => {
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
                render_prof_report(&profiler, &memory_dir, &audit_file)
            );
            continue;
        }

        rewrite_submitted_user_line(&input, prompt_status.take_visible());

        let mut status = ThinkingStatus::start(&config.provider, &config.model);
        let (text, stats, elapsed) = run_turn(
            &mut core,
            &input,
            &session,
            &audit_file,
            &config,
            None,
            Some(&mut status),
            true,
            Some(&mut profiler),
        );
        status.finish();
        print_final_response(&text, &stats, &config.provider, &config.model, elapsed);
    }
}

fn run_turn(
    core: &mut AgentCore,
    input: &str,
    session: &str,
    audit_file: &std::path::Path,
    config: &timem_shell::ProviderConfig,
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
            CoreStep::NeedModel { prompt, .. } => {
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

fn choose_user_approval(request: &ApprovalRequest) -> ApprovalChoice {
    print!("{}", render_user_approval_prompt(request));
    choose_with_keyboard(render_approval_choices, ApprovalChoice::Deny)
}

fn choose_round_limit_continue(max_rounds: u32) -> ApprovalChoice {
    print!("{}", render_round_limit_prompt(max_rounds));
    choose_with_keyboard(render_round_limit_choices, ApprovalChoice::Allow)
}

fn choose_with_keyboard(
    render_choices: fn(ApprovalChoice) -> String,
    initial: ApprovalChoice,
) -> ApprovalChoice {
    let mut selected = initial;
    print!("{}", render_choices(selected));
    let _ = io::stdout().flush();

    let Ok(mut tty) = OpenOptions::new().read(true).write(true).open("/dev/tty") else {
        println!();
        return ApprovalChoice::Deny;
    };
    let fd = tty.as_raw_fd();
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        println!();
        return ApprovalChoice::Deny;
    }
    let mut raw = original;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 1;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        println!();
        return ApprovalChoice::Deny;
    }
    let mut terminal_mode = TerminalModeGuard::new(fd, original);

    let result = loop {
        match read_approval_key(&mut tty) {
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

fn render_submitted_user_line_rewrite(
    input: &str,
    status_line_visible: bool,
    terminal_width: usize,
    time_label: &str,
) -> String {
    let prompt_prefix = format!("[{}] 你 > ", time_label);
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
    bash_approval_mode: BashApprovalMode,
) -> String {
    let default_protocol = timem_shell::default_api_protocol_for_provider(&config.provider);
    let default_base_url = timem_shell::default_base_url_for_provider(&config.provider);
    let rows = [
        ConfigRow {
            key: "TIMEM_SPACE",
            value: space.to_string(),
            desc: "记忆空间",
            highlight: false,
        },
        ConfigRow {
            key: "TIMEM_GATEWAY_PROVIDER",
            value: config.provider.clone(),
            desc: "流量平台，决定默认 base url",
            highlight: false,
        },
        ConfigRow {
            key: "TIMEM_API_PROTOCOL",
            value: config.api_protocol.label().to_string(),
            desc: "API 提交网络包格式",
            highlight: config.api_protocol != default_protocol,
        },
        ConfigRow {
            key: "TIMEM_BASE_URL",
            value: config.base_url.clone(),
            desc: "网关 base url",
            highlight: config.base_url.trim_end_matches('/')
                != default_base_url.trim_end_matches('/'),
        },
        ConfigRow {
            key: "TIMEM_MODEL",
            value: config.model.clone(),
            desc: "模型名称",
            highlight: !timem_shell::is_default_model_for_provider(&config.provider, &config.model),
        },
        ConfigRow {
            key: "TIMEM_MAX_LLM_CONTEXT",
            value: format_token_count(config.max_llm_context_tokens),
            desc: "最大上下文窗口",
            highlight: false,
        },
        ConfigRow {
            key: "TIMEM_BASH_APPROVAL",
            value: bash_approval_mode_label(bash_approval_mode).to_string(),
            desc: "bash 允许策略，approve/ask",
            highlight: false,
        },
        ConfigRow {
            key: "TIMEM_DATA_DIR",
            value: absolute_display_path(&data_root()),
            desc: "运行时记忆、日志存储",
            highlight: false,
        },
        ConfigRow {
            key: "local_audit",
            value: absolute_display_path(audit_file),
            desc: "原始流量审计存储",
            highlight: false,
        },
    ];
    boxed_config_table(&rows)
}

fn boxed_config_table(rows: &[ConfigRow]) -> String {
    let key_width = rows
        .iter()
        .map(|row| display_width(row.key))
        .max()
        .unwrap_or(0)
        .max("TIMEM_GATEWAY_PROVIDER".len());
    let value_width = rows
        .iter()
        .map(|row| display_width(&row.value))
        .max()
        .unwrap_or(0)
        .clamp(12, 72);
    let desc_width = rows
        .iter()
        .map(|row| display_width(row.desc))
        .max()
        .unwrap_or(0)
        .clamp(8, 32);
    let inner_width = key_width + value_width + desc_width + 10;
    let title = format!(" {TIMEM_LOGO} config ");
    let title_width = display_width(&title);
    let left = (inner_width.saturating_sub(title_width)) / 2;
    let right = inner_width.saturating_sub(title_width + left);
    let border = format!("+{}{}{}+\n", "-".repeat(left), title, "-".repeat(right));
    let mut out = String::new();
    out.push_str(&border);
    for row in rows {
        let value = fit_display(&row.value, value_width);
        let value = if row.highlight {
            format!("{ANSI_HIGHLIGHT}{value}{ANSI_RESET}")
        } else {
            value
        };
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            fit_display(row.key, key_width),
            value,
            fit_display(row.desc, desc_width)
        ));
    }
    out.push_str(&border);
    out.push_str(
        "显示的是最终生效值。可先 source /path/to/your/env，或设置左侧 env 变量后启动。\n",
    );
    out.push_str("option 优先于 env，查看：timem --help\n");
    out
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
    "Usage:\n  timem [options]\n\n\x1b[1mPrecedence:\n  command line options override process env values; process env overrides defaults.\x1b[0m\n\nCreate a private env file from env_template, then load it explicitly:\n  cp env_template env\n  source /path/to/your/env\n\nRecommended run:\n  timem\n\nUseful env values to put in your env file:\n  export TIMEM_GATEWAY_PROVIDER=aliyun\n  export TIMEM_API_KEY=your_api_key_here\n  export TIMEM_MODEL=qwen-plus\n  export TIMEM_SPACE=.test_mem\n\nCommand line override example:\n  timem --data-dir data --space .test_mem --gateway-provider aliyun --model qwen-plus\n\nOptions:\n  --space <name>                 env TIMEM_SPACE; memory/audit space, default .test_mem\n  --gateway-provider <name>      env TIMEM_GATEWAY_PROVIDER; traffic platform / default base URL provider\n  --api-protocol <protocol>      env TIMEM_API_PROTOCOL; openai-compatible|openai-responses|anthropic\n  --base-url <url>               env TIMEM_BASE_URL; override provider default base URL\n  --model <name>                 env TIMEM_MODEL; model name\n  --api-key <key>                env TIMEM_API_KEY; API key, env is safer than shell history\n  --data-dir <path>              env TIMEM_DATA_DIR; data/config/memory/audit root\n  --timeout <seconds>            env TIMEM_TIMEOUT; provider HTTP timeout, default 120\n  --max-tokens <n>               env TIMEM_MAX_TOKENS; max response tokens, default 2048\n  --max-llm-context <n|100K>     env TIMEM_MAX_LLM_CONTEXT; context window, default 100K\n  --bash-approval <mode>         env TIMEM_BASH_APPROVAL; ask|approve, default ask\n  --once-json <text>             run one non-interactive turn and print JSON\n  --supporting-context <text>    append extra runtime context for --once-json/debug\n  -h, --help                     show this help\n\nInteractive commands:\n  /prof                          show runtime profiling for tokens, model wait/local time, and storage size\n\nInteractive keys:\n  Ctrl+C cancels the current input while editing, and cancels the current turn while Timem is thinking.\n\nProtocol defaults:\n  openai -> openai-responses; anthropic -> anthropic; others -> openai-compatible\n\nVendor fallback key env vars:\n  DASHSCOPE_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN\n"
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
        cancelled_turn_result, consume_turn_cancel_request, display_width, epoch_millis, help_text,
        random_spinner_tick, read_approval_key, render_approval_choices,
        render_round_limit_choices, render_round_limit_prompt, render_startup_banner,
        render_submitted_user_line_rewrite, render_user_approval_prompt, sanitize_user_input,
        wrapped_terminal_rows, ApprovalChoice, ApprovalKey, ANSI_HIGHLIGHT, STATIC_PROMPT,
        TURN_CANCEL_REQUESTED,
    };
    use agent_core::{ApprovalRequest, BashApprovalMode};
    use std::fs;
    use std::io::Cursor;
    use std::process::Command;
    use std::sync::atomic::Ordering;
    use timem_shell::{ApiProtocol, ProviderConfig, SPINNER_ICONS};

    #[test]
    fn static_prompt_uses_full_shared_v1_resource() {
        assert!(STATIC_PROMPT.contains("\"static_prefix_id\": \"static_prefix_v2\""));
        assert!(STATIC_PROMPT.contains("\"General_rule\""));
        assert!(STATIC_PROMPT.contains("\"Mem_rule\""));
        assert!(STATIC_PROMPT.contains("\"Tool_capability\""));
        assert!(STATIC_PROMPT.contains("\"Response_rule\""));
        assert!(STATIC_PROMPT.contains("\"Freq_reflect\""));
        assert!(STATIC_PROMPT.contains("\"json_schema_summary\""));
        assert!(STATIC_PROMPT.contains("\"acceptance_check.is_satisfied\""));
        assert!(STATIC_PROMPT.contains("\"perspective_policy\""));
        assert!(STATIC_PROMPT.contains("\"tool_claim_policy\""));
        assert!(STATIC_PROMPT.contains("Never say you already checked/read/looked up memory"));
        assert!(STATIC_PROMPT.contains("\"storage_style_policy\""));
        assert!(STATIC_PROMPT.contains("personal or family facts"));
        assert!(STATIC_PROMPT.contains("\"perspective_rewrite\""));
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
        assert!(STATIC_PROMPT.contains("background=true"));
        assert!(STATIC_PROMPT.contains("read_back_command"));
        assert!(STATIC_PROMPT.contains("Never invent a read-only limitation for run_bash"));
        assert!(STATIC_PROMPT.contains("\"durable_ctx_score\""));
        assert!(STATIC_PROMPT.contains("Every model response must score"));
        assert!(STATIC_PROMPT.contains("local machine"));
        assert!(STATIC_PROMPT.contains("\"intent_required\""));
        assert!(STATIC_PROMPT.contains("use only the returned action result as evidence"));
        assert!(STATIC_PROMPT.contains("\"thought?\""));
        assert!(STATIC_PROMPT.contains("no_result_terminate"));
        assert!(STATIC_PROMPT.contains("self_audit"));
        assert!(STATIC_PROMPT.len() > 3_000);
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
            max_tokens: 4096,
            max_llm_context_tokens: 100_000,
        };
        let banner = render_startup_banner(
            ".xxx_mem",
            &config,
            std::path::Path::new(".xxx_mem/shell_audit.jsonl"),
            BashApprovalMode::Approve,
        );

        assert!(banner.starts_with('+'));
        assert!(banner.lines().next().unwrap_or("").starts_with("+-"));
        assert!(banner
            .lines()
            .any(|line| line.starts_with("+-") && line.ends_with("-+")));
        assert!(banner.contains("显示的是最终生效值"));
        assert!(banner.contains("source /path/to/your/env"));
        assert!(banner.contains("option 优先于 env"));
        assert!(banner.contains("TIMEM_SPACE"));
        assert!(banner.contains(".xxx_mem"));
        assert!(!banner.contains("session="));
        assert!(!banner.contains("TIMEM_PROFILE"));
        assert!(!banner
            .lines()
            .any(|line| line.trim_start().starts_with("| TIMEM_SPACE=")));
        assert!(banner.contains("TIMEM_GATEWAY_PROVIDER"));
        assert!(banner.contains("流量平台"));
        assert!(banner.contains("aliyun"));
        assert!(banner.contains("TIMEM_API_PROTOCOL"));
        assert!(banner.contains("openai-compatible"));
        assert!(banner.contains("TIMEM_BASE_URL"));
        assert!(banner.contains("https://dashscope.aliyuncs.com/compatible-mode/v1"));
        assert!(banner.contains("TIMEM_MODEL"));
        assert!(banner.contains("qwen-plus"));
        assert!(banner.contains("TIMEM_MAX_LLM_CONTEXT"));
        assert!(banner.contains("100K"));
        assert!(banner.contains("TIMEM_BASH_APPROVAL"));
        assert!(banner.contains("approve"));
        assert!(banner.contains("TIMEM_DATA_DIR"));
        assert!(banner.contains("/data"));
        assert!(!banner.contains("TIMEM_API_KEY=secret"));
        let table_lines: Vec<&str> = banner
            .lines()
            .filter(|line| line.starts_with('|'))
            .collect();
        let first_width = display_width(table_lines[0]);
        assert!(table_lines
            .iter()
            .all(|line| display_width(line) == first_width));
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
            max_tokens: 4096,
            max_llm_context_tokens: 100_000,
        };
        let default_banner = render_startup_banner(
            ".test_mem",
            &default_config,
            std::path::Path::new(".test_mem/shell_audit.jsonl"),
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
            max_tokens: 4096,
            max_llm_context_tokens: 100_000,
        };
        let override_banner = render_startup_banner(
            ".test_mem",
            &override_config,
            std::path::Path::new(".test_mem/shell_audit.jsonl"),
            BashApprovalMode::Ask,
        );
        assert!(override_banner.contains(&format!("{ANSI_HIGHLIGHT}anthropic")));
        assert!(override_banner.contains(&format!("{ANSI_HIGHLIGHT}https://example.com/v1")));
        assert!(override_banner.contains(&format!("{ANSI_HIGHLIGHT}aws-claude-sonnet-4-6")));
        let table_lines: Vec<&str> = override_banner
            .lines()
            .filter(|line| line.starts_with('|'))
            .collect();
        let first_width = display_width(table_lines[0]);
        assert!(table_lines
            .iter()
            .all(|line| display_width(line) == first_width));
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
            "--max-tokens",
            "TIMEM_MAX_TOKENS",
            "--max-llm-context",
            "TIMEM_MAX_LLM_CONTEXT",
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
            "TIMEM_MAX_TOKENS",
            "TIMEM_MAX_LLM_CONTEXT",
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
    fn submitted_user_line_rewrite_clears_wrapped_input_rows() {
        let rendered = render_submitted_user_line_rewrite("abcdef", false, 10, "12:00:00");
        assert!(rendered.starts_with("\x1b[3F\r\x1b[J"));
        assert!(rendered.ends_with("[12:00:00] 你 > abcdef\n"));
    }

    #[test]
    fn submitted_user_line_rewrite_clears_status_and_wrapped_rows() {
        let rendered = render_submitted_user_line_rewrite("abcdef", true, 10, "12:00:00");
        assert!(rendered.starts_with("\x1b[4F\r\x1b[J"));
    }

    #[test]
    fn wrapped_terminal_rows_counts_cjk_display_width() {
        assert_eq!(
            wrapped_terminal_rows(display_width("[12:00:00] 你 > 你好"), 10),
            2
        );
    }
}
