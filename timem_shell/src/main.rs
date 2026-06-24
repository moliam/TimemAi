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
    parse_cli_args, provider_config_from_env, render_final_response_at, render_shell_status_bar,
    render_thinking_block_at, supporting_context, ModelDirection, ShellStatusMessage,
    ShellStatusSnapshot, ShellStatusTone, SPINNER_ICONS, TIMEM_LOGO,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const STATIC_PROMPT: &str = include_str!("../../resources/static_v1.json");

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
    println!("输入 /exit 退出；Ctrl+C 取消当前输入。\n");

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
) -> (String, UsageStats, Duration) {
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

    let (text, stats, repair_issue) = loop {
        match step {
            CoreStep::NeedModel { prompt, .. } => {
                rounds += 1;
                if let Some(status) = status.as_deref_mut() {
                    status.set_model_direction(rounds, ModelDirection::Upstream);
                }
                match call_model(config, &prompt, audit_file) {
                    Ok(response) => {
                        if let Some(status) = status.as_deref_mut() {
                            status.set_usage(response.usage.clone());
                            status.set_model_direction(rounds, ModelDirection::Downstream);
                            if let Some(hint) = action_status_hint(&response.content) {
                                status.set_intent(&hint.intent, &hint.memory_marker);
                            }
                        }
                        step = core.apply_model_response(response);
                    }
                    Err(err) => {
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
            CoreStep::Final(turn) => {
                break (turn.response_to_user, turn.stats, turn.repair_issue);
            }
        }
    };
    let elapsed = start.elapsed();
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

struct ThinkingStatus {
    state: Arc<Mutex<ShellStatusSnapshot>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ThinkingStatus {
    fn start(provider: &str, model: &str) -> Self {
        let state = Arc::new(Mutex::new(ShellStatusSnapshot {
            provider: provider.to_string(),
            model: model.to_string(),
            intent: "思考中".to_string(),
            memory_marker: String::new(),
            model_round: 1,
            direction: ModelDirection::Upstream,
            usage: UsageStats::zero(),
            tick: random_spinner_tick(),
        }));
        let running = Arc::new(AtomicBool::new(true));
        render_thinking(&state.lock().unwrap());
        let thread_state = Arc::clone(&state);
        let thread_running = Arc::clone(&running);
        let handle = thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1500));
                if let Ok(mut snapshot) = thread_state.lock() {
                    snapshot.tick = random_spinner_tick();
                    rerender_thinking(&snapshot);
                }
            }
        });
        Self {
            state,
            running,
            handle: Some(handle),
        }
    }

    fn set_model_direction(&mut self, round: u32, direction: ModelDirection) {
        if let Ok(mut state) = self.state.lock() {
            state.model_round = round;
            state.direction = direction;
            rerender_thinking(&state);
        }
    }

    fn set_usage(&mut self, usage: UsageStats) {
        if let Ok(mut state) = self.state.lock() {
            state.usage = usage;
            rerender_thinking(&state);
        }
    }

    fn set_intent(&mut self, intent: &str, memory_marker: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.intent = intent
                .trim_end_matches('…')
                .trim_end_matches("...")
                .to_string();
            state.memory_marker = memory_marker.to_string();
            rerender_thinking(&state);
        }
    }

    fn finish(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        clear_thinking_block();
    }

    fn pause_for_user_approval(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        clear_thinking_block();
    }

    fn resume_after_user_approval(&mut self) {
        if self.handle.is_some() {
            return;
        }
        self.running.store(true, Ordering::Relaxed);
        render_thinking(&self.state.lock().unwrap());
        let thread_state = Arc::clone(&self.state);
        let thread_running = Arc::clone(&self.running);
        self.handle = Some(thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1500));
                if let Ok(mut snapshot) = thread_state.lock() {
                    snapshot.tick = random_spinner_tick();
                    rerender_thinking(&snapshot);
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

fn choose_user_approval(request: &ApprovalRequest) -> ApprovalChoice {
    print!("{}", render_user_approval_prompt(request));
    let mut selected = ApprovalChoice::Deny;
    print!("{}", render_approval_choices(selected));
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
                print!("\r\x1b[2K{}", render_approval_choices(selected));
                let _ = io::stdout().flush();
            }
            ApprovalKey::Select(choice) => {
                selected = choice;
                print!("\r\x1b[2K{}", render_approval_choices(selected));
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

fn render_thinking(snapshot: &ShellStatusSnapshot) {
    print!("{}", render_thinking_block_at(snapshot, &time_label()));
    let _ = io::stdout().flush();
}

fn rerender_thinking(snapshot: &ShellStatusSnapshot) {
    print!("\x1b[2F\x1b[J");
    print!("{}", render_thinking_block_at(snapshot, &time_label()));
    let _ = io::stdout().flush();
}

fn clear_thinking_block() {
    print!("\x1b[2F\x1b[J");
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
    if status_line_visible {
        print!("\x1b[2A\r\x1b[J[{}] 你 > {}\n", time_label(), input);
    } else {
        print!("\x1b[1A\r\x1b[2K[{}] 你 > {}\n", time_label(), input);
    }
    let _ = io::stdout().flush();
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
    let rows = [
        ("TIMEM_SPACE", space.to_string(), "记忆空间"),
        (
            "TIMEM_GATEWAY_PROVIDER",
            config.provider.clone(),
            "流量平台，决定默认 base url",
        ),
        (
            "TIMEM_API_PROTOCOL",
            config.api_protocol.label().to_string(),
            "API 提交网络包格式",
        ),
        ("TIMEM_BASE_URL", config.base_url.clone(), "网关 base url"),
        ("TIMEM_MODEL", config.model.clone(), "模型名称"),
        (
            "TIMEM_MAX_LLM_CONTEXT",
            format_token_count(config.max_llm_context_tokens),
            "最大上下文窗口",
        ),
        (
            "TIMEM_BASH_APPROVAL",
            bash_approval_mode_label(bash_approval_mode).to_string(),
            "bash 允许策略，approve/ask",
        ),
        (
            "TIMEM_DATA_DIR",
            absolute_display_path(&data_root()),
            "运行时记忆、日志存储",
        ),
        (
            "local_audit",
            absolute_display_path(audit_file),
            "原始流量审计存储",
        ),
    ];
    boxed_config_table(&rows)
}

fn boxed_config_table(rows: &[(&str, String, &str)]) -> String {
    let key_width = rows
        .iter()
        .map(|row| display_width(row.0))
        .max()
        .unwrap_or(0)
        .max("TIMEM_GATEWAY_PROVIDER".len());
    let value_width = rows
        .iter()
        .map(|row| display_width(&row.1))
        .max()
        .unwrap_or(0)
        .clamp(12, 72);
    let desc_width = rows
        .iter()
        .map(|row| display_width(row.2))
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
    for (key, value, desc) in rows {
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            fit_display(key, key_width),
            fit_display(value, value_width),
            fit_display(desc, desc_width)
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
    UnicodeWidthStr::width(text)
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
    "Usage:\n  timem [options]\n\n\x1b[1mPrecedence:\n  command line options override process env values; process env overrides defaults.\x1b[0m\n\nCreate a private env file from env_template, then load it explicitly:\n  cp env_template env\n  source /path/to/your/env\n\nExamples:\n  timem --space .test_mem --gateway-provider aliyun --model qwen-plus\n  timem --space .test_mem --gateway-provider custom --api-protocol anthropic --base-url https://your-gateway.example/v1 --model aws-claude-sonnet-4-6\n  timem --data-dir data --space .test_mem --max-llm-context 100K\n\nOptions:\n  --space <name>                 env TIMEM_SPACE; memory/audit space, default .test_mem\n  --gateway-provider <name>      env TIMEM_GATEWAY_PROVIDER; traffic platform / default base URL provider\n  --api-protocol <protocol>      env TIMEM_API_PROTOCOL; openai-compatible|anthropic\n  --base-url <url>               env TIMEM_BASE_URL; override provider default base URL\n  --model <name>                 env TIMEM_MODEL; model name\n  --api-key <key>                env TIMEM_API_KEY; API key, env is safer than shell history\n  --data-dir <path>              env TIMEM_DATA_DIR; data/config/memory/audit root\n  --timeout <seconds>            env TIMEM_TIMEOUT; provider HTTP timeout, default 120\n  --max-tokens <n>               env TIMEM_MAX_TOKENS; max response tokens, default 2048\n  --max-llm-context <n|100K>     env TIMEM_MAX_LLM_CONTEXT; context window, default 100K\n  --bash-approval <mode>         env TIMEM_BASH_APPROVAL; ask|approve, default ask\n  --once-json <text>             run one non-interactive turn and print JSON\n  --supporting-context <text>    append extra runtime context for --once-json/debug\n  -h, --help                     show this help\n\nVendor fallback key env vars:\n  DASHSCOPE_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN\n"
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
        display_width, epoch_millis, help_text, random_spinner_tick, read_approval_key,
        render_approval_choices, render_startup_banner, render_user_approval_prompt,
        sanitize_user_input, ApprovalChoice, ApprovalKey, STATIC_PROMPT,
    };
    use agent_core::{ApprovalRequest, BashApprovalMode};
    use std::fs;
    use std::io::Cursor;
    use std::process::Command;
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
        assert!(STATIC_PROMPT.contains("\"run_bash\""));
        assert!(STATIC_PROMPT.contains("OS/system info"));
        assert!(STATIC_PROMPT.contains("\"intent_required\""));
        assert!(STATIC_PROMPT.contains("ask prompts before bash"));
        assert!(STATIC_PROMPT.contains("\"thought?\""));
        assert!(STATIC_PROMPT.contains("no_result_terminate"));
        assert!(STATIC_PROMPT.contains("self_audit"));
        assert!(STATIC_PROMPT.len() > 3_000);
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
}
