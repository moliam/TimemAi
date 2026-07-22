use agent_core::capability::CapabilityRegistry;
use agent_core::self_tool::SelfToolPaths;
use agent_core::session_store::{
    ChatHistoryRecord, ChatHistoryRole, SessionResumeNotice, SessionStore, StoredSession,
    StoredSessionProfile, StoredSessionState,
};
use agent_core::{AgentCore, ApprovalRequest, BashApprovalMode, ResponseProtocolKind, UsageStats};
use crossterm::event::Event;
use reedline::{
    default_emacs_keybindings, EditCommand, EditMode, Emacs, FileBackedHistory, Highlighter,
    KeyCode, KeyModifiers, Keybindings, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, ReedlineEvent, ReedlineRawEvent, Signal, StyledText,
};
use serde_json::json;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ffi::CStr;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use timem_shell::{
    append_audit, apply_workspace_command_to_path, bash_approval_mode_from_sources,
    capabilities_dir_from_sources, combine_additional_contexts, default_data_root,
    estimate_prompt_context_tokens, format_token_count, host_start_audit_event, layout_for_space,
    load_workspace_dirs_from_path, local_time_label, observation_events_from_core_topic_events,
    observation_panel_width_for_terminal, parse_cli_args, provider_config_from_env,
    render_final_response_at, render_prof_report_data, render_shell_status_bar,
    render_thinking_view_at, render_turn_outcome_text, run_session_turn,
    runtime_active_elapsed_secs, runtime_info_context, runtime_profile_report,
    shell_status_message_from_core_topic, stale_context_decision_request, topic_event_status_hint,
    work_instruction_load_report, work_instruction_load_request, work_instruction_load_topic_event,
    work_instruction_mode_from_sources, workspace_config_file, workspace_reference_context,
    CoreMemoryActivity, CoreTopicEvent, HostDecision, HostDecisionRequest, HostStatusMessage,
    ModelDirection, NoopTurnUi, ObservationEvent, ObservationPanel, OutputExpansionRequest,
    RoundLimitDecisionRequest, RuntimeConfigApplyError, RuntimeConfigApplyMessageKind,
    RuntimeConfigApplyReport, RuntimeConfigField, RuntimeConfigMenuReport, RuntimeProfiler,
    RuntimeRetryStatus, ShellStatusSnapshot, StaleContextDecisionRequest, ThinkingViewSnapshot,
    TurnInput, TurnUi, WorkInstructionLoadMessageKind, WorkInstructionLoadMode,
    WorkInstructionLoadReport, WorkInstructionLoadRequest, WorkspaceCommand,
    WorkspaceCommandMessageKind, WorkspaceCommandOutcome, WorkspaceCommandReport,
    WorkspaceMenuReport, SPINNER_ICONS, TIMEM_LOGO,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const STATIC_PROMPT: &str = include_str!("../../resources/system_prompt/system_prompt.md");
const ANSI_RESET: &str = timem_shell::ANSI_RESET;
const ANSI_BOLD: &str = timem_shell::ANSI_BOLD;
const ANSI_HIGHLIGHT: &str = "\x1b[1;33m";
const PASTE_START_MARKER: char = '\u{2063}';
const PASTE_END_MARKER: char = '\u{2064}';
static TURN_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
struct ConfigRow {
    key: String,
    value: String,
    desc: String,
    highlight: bool,
}

enum ConfigTableItem {
    Section(String),
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
    let data_root = default_data_root();
    let layout = layout_for_space(&space);
    let audit_file = layout.api_audit_file();
    let action_audit_file = layout.action_audit_file();
    let memory_dir = layout.memory_dir();
    let session_store = SessionStore::new(&memory_dir);
    let workspace_config = workspace_config_file(&data_root);
    let mut bash_approval_mode =
        bash_approval_mode_from_sources(options.bash_approval.as_deref(), &env);
    let mut work_instruction_mode =
        work_instruction_mode_from_sources(options.work_instructions.as_deref(), &env);
    let current_work_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut profiler = RuntimeProfiler::default();
    let mut stored_session = load_or_create_shell_session(
        &session_store,
        &config,
        bash_approval_mode,
        work_instruction_mode,
        &current_work_dir,
    );
    let session_env = shell_session_effective_env(&env, &stored_session);
    config = match provider_config_from_env(&options, &session_env) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("[config_error] restored_session_config_invalid: {err}");
            std::process::exit(2);
        }
    };
    bash_approval_mode =
        bash_approval_mode_from_sources(options.bash_approval.as_deref(), &session_env);
    work_instruction_mode =
        work_instruction_mode_from_sources(options.work_instructions.as_deref(), &session_env);
    let response_protocol = options
        .response_protocol
        .as_deref()
        .or_else(|| {
            session_env
                .get("TIMEM_RESPONSE_PROTOCOL")
                .map(String::as_str)
        })
        .map(ResponseProtocolKind::from_name)
        .unwrap_or_default();
    config.response_protocol = response_protocol;
    let mut core = AgentCore::new(STATIC_PROMPT, config.core_profile(), &memory_dir);
    core.set_response_protocol(response_protocol);
    core.configure_self_tool_runtime(
        session_env.clone().into_iter().collect(),
        SelfToolPaths {
            space_dir: absolute_path(layout.space_dir()),
            memory_dir: absolute_path(memory_dir.clone()),
            memory_file: absolute_path(memory_dir.join("memory.jsonl")),
            scratch_file: absolute_path(memory_dir.join("scratch_notes.jsonl")),
            api_audit_file: absolute_path(audit_file.clone()),
            action_audit_file: absolute_path(action_audit_file.clone()),
        },
    );
    if let Some(capabilities_dir) =
        capabilities_dir_from_sources(options.capabilities_dir.as_deref(), &session_env)
    {
        match CapabilityRegistry::builtin_with_overlay_dir(&capabilities_dir) {
            Ok(registry) => core.set_capability_registry(registry),
            Err(err) => {
                eprintln!("[config_error] capability_overlay_failed: {err}");
                std::process::exit(2);
            }
        }
    }
    core.configure_runtime_from_host(&config, bash_approval_mode);
    let session_runtime_info = runtime_info_context(&shell_runtime_info_entries(&core));
    let session_work_dir = shell_session_work_dir(&stored_session, &current_work_dir);
    let _ = core.change_prompt_cwd(session_work_dir.to_string_lossy());
    let (mut work_instruction_context, work_instruction_notice) = match work_instruction_mode {
        WorkInstructionLoadMode::Silent => load_work_instructions_for_shell(&session_work_dir),
        WorkInstructionLoadMode::Ask | WorkInstructionLoadMode::Off => (None, None),
    };
    let session = stored_session.session_id.clone();
    let mut resume_notice_pending = true;
    let mut workspace_pending = !load_workspace_dirs_from_path(&workspace_config).is_empty();

    if let Some(input) = options.once_json_input.as_deref() {
        let turn_id = shell_turn_id();
        append_shell_history_message(
            &session_store,
            &session,
            &turn_id,
            ChatHistoryRole::User,
            input,
        );
        let resume_notice = take_shell_resume_notice(
            &session_store,
            &session,
            &session_work_dir,
            &mut resume_notice_pending,
        );
        let context = combine_additional_contexts([
            session_runtime_info.as_deref(),
            resume_notice.as_deref(),
            work_instruction_context.as_deref(),
            options.supporting_context.as_deref(),
        ]);
        let mut ui = NoopTurnUi;
        let outcome = run_session_turn(
            &mut core,
            &mut config,
            TurnInput {
                input,
                session: &session,
                audit_file: &audit_file,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: context.as_deref(),
            },
            &mut ui,
            Some(&mut profiler),
        );
        println!(
            "{}",
            json!({
                "output": render_turn_outcome_text(&outcome),
                "session_id": session,
                "stats": outcome.stats,
                "status": "done",
                "elapsed_ms": outcome.elapsed.as_millis()
            })
        );
        append_shell_turn_result(
            &session_store,
            &mut stored_session,
            &session,
            &turn_id,
            &render_turn_outcome_text(&outcome),
            &outcome,
            &config,
            bash_approval_mode,
            work_instruction_mode,
            core.current_prompt_cwd(),
        );
        return;
    }

    let _ = append_audit(
        &audit_file,
        &host_start_audit_event(
            "shell",
            &session,
            &space,
            &config.provider,
            &config.base_url,
            &config.api_protocol,
            &config.model,
            config.max_llm_input_tokens,
            bash_approval_mode,
        ),
    );

    println!("Timem native shell");
    print!(
        "{}",
        render_startup_banner(
            &space,
            &config,
            &data_root,
            &audit_file,
            &action_audit_file,
            bash_approval_mode,
            work_instruction_mode,
        )
    );
    let mut startup_messages = Vec::new();
    let init_event = core.init_lifecycle_topic_event(&session);
    if let Some(message) = shell_status_message_from_core_topic(&init_event) {
        startup_messages.push(message);
    }
    if let Some(message) = work_instruction_notice.as_ref() {
        startup_messages.push(message.clone());
    }
    if !startup_messages.is_empty() {
        print!("{}", render_startup_status_block(&startup_messages));
    }
    println!("{}\n", startup_control_hint());

    let history_file = audit_file.with_file_name("shell_history.txt");
    let mut editor = ShellLineEditor::new(history_file);
    let mut prompt_status = PromptStatusBar::default();
    let mut last_dialog_activity = Instant::now();

    loop {
        let prompt = render_user_input_prompt(&time_label());
        let (input, submitted_display) = match editor.readline(&prompt) {
            ShellReadline::Line { text, display } => (text, display),
            ShellReadline::PendingPaste {
                text,
                display,
                line_count,
            } => {
                if matches!(
                    choose_raw_multiline_paste_submit(line_count),
                    ApprovalChoice::Allow
                ) {
                    (text, display)
                } else {
                    prompt_status.show_info("已取消多行粘贴。");
                    continue;
                }
            }
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
        if input == "/help" {
            prompt_status.clear_before_exit();
            print!("{}", runtime_help_text());
            continue;
        }
        if input == "/prof" {
            prompt_status.clear_before_exit();
            let report =
                runtime_profile_report(&profiler, &memory_dir, &audit_file, &action_audit_file);
            println!("{}", render_prof_report_data(&report));
            continue;
        }
        if input == "/config" {
            prompt_status.clear_before_exit();
            let prompt_cwd = core.current_prompt_cwd().to_path_buf();
            if run_config_menu(
                &mut config,
                &mut core,
                &mut bash_approval_mode,
                &mut work_instruction_mode,
                &mut work_instruction_context,
                &prompt_cwd,
            ) {
                println!(
                    "{}",
                    render_startup_banner(
                        &space,
                        &config,
                        &data_root,
                        &audit_file,
                        &action_audit_file,
                        bash_approval_mode,
                        work_instruction_mode,
                    )
                );
            }
            continue;
        }

        if input == "/workspace" {
            prompt_status.clear_before_exit();
            if run_workspace_menu(&workspace_config) {
                workspace_pending = true;
                println!("工作区已更新。");
            }
            continue;
        }

        rewrite_submitted_user_line(&submitted_display, prompt_status.take_visible());
        let turn_id = shell_turn_id();
        append_shell_history_message(
            &session_store,
            &session,
            &turn_id,
            ChatHistoryRole::User,
            &input,
        );

        let idle = last_dialog_activity.elapsed();
        let dynamic_context_tokens = core.dynamic_context_estimated_tokens();
        if let Some(stale_request) = stale_context_decision_request(idle, dynamic_context_tokens) {
            let continue_old_context = request_stale_context_continue(stale_request);
            core.resolve_stale_context_with_audit(
                stale_request,
                continue_old_context,
                &audit_file,
                &session,
            );
        }

        let workspace_ctx: Option<String> = if workspace_pending {
            workspace_pending = false;
            workspace_reference_context(&load_workspace_dirs_from_path(&workspace_config))
        } else {
            None
        };
        let mut status =
            ThinkingStatus::start(&config.provider, &config.model, config.max_llm_input_tokens);
        TURN_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
        let _sigint_guard = SigintGuard::install();
        let mut turn_ui = CliTurnUi {
            status: Some(&mut status),
            interactive_approval: true,
            supplement_input: ThinkingSupplementInput::new(),
        };
        let prompt_cwd = core.current_prompt_cwd().to_path_buf();
        let turn_work_instruction_context = resolve_work_instruction_context_for_turn(
            work_instruction_mode,
            &prompt_cwd,
            &session,
            &mut turn_ui,
        );
        let resume_notice = take_shell_resume_notice(
            &session_store,
            &session,
            &session_work_dir,
            &mut resume_notice_pending,
        );
        let turn_additional_context = combine_additional_contexts([
            session_runtime_info.as_deref(),
            resume_notice.as_deref(),
            turn_work_instruction_context.as_deref(),
            workspace_ctx.as_deref(),
        ]);
        let outcome = run_session_turn(
            &mut core,
            &mut config,
            TurnInput {
                input: &input,
                session: &session,
                audit_file: &audit_file,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: turn_additional_context.as_deref(),
            },
            &mut turn_ui,
            Some(&mut profiler),
        );
        drop(turn_ui);
        let is_cancelled =
            outcome.stop_reason == Some(timem_shell::TurnStopReason::CancelledByUser);
        if is_cancelled {
            let stats = status.accumulated_stats();
            let latest = status.accumulated_latest_usage();
            status.finish_cancelled();
            println!();
            print_final_response(
                &render_turn_outcome_text(&outcome),
                &stats,
                latest.as_ref(),
                &config.provider,
                &config.model,
                outcome.elapsed,
                config.max_llm_input_tokens,
            );
        } else {
            status.finish();
            print_final_response(
                &render_turn_outcome_text(&outcome),
                &outcome.stats,
                outcome.latest_usage.as_ref(),
                &config.provider,
                &config.model,
                outcome.elapsed,
                config.max_llm_input_tokens,
            );
        }
        append_shell_turn_result(
            &session_store,
            &mut stored_session,
            &session,
            &turn_id,
            &render_turn_outcome_text(&outcome),
            &outcome,
            &config,
            bash_approval_mode,
            work_instruction_mode,
            core.current_prompt_cwd(),
        );
        last_dialog_activity = Instant::now();
    }
}

fn consume_turn_cancel_request() -> bool {
    TURN_CANCEL_REQUESTED.swap(false, Ordering::SeqCst)
}

fn shell_runtime_info_entries(core: &AgentCore) -> Vec<String> {
    let ui = if std::env::var("ITERM_SESSION_ID").is_ok() {
        "iterm2"
    } else {
        "shell"
    };
    let mut entries = vec![
        format!("ui: {ui}"),
        format!("os: {}", host_os_type()),
        format!("arch: {}", std::env::consts::ARCH),
        format!("os_version: {}", host_os_version()),
    ];
    if core.capability_contains_tool("run_bash") {
        entries.push("run_bash: available; executes on user_local_machine".to_string());
    }
    entries
}

fn host_os_type() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

fn host_os_version() -> String {
    let mut uts = unsafe { std::mem::zeroed::<libc::utsname>() };
    if unsafe { libc::uname(&mut uts) } != 0 {
        return "unknown".to_string();
    }
    unsafe { CStr::from_ptr(uts.release.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string()
}

fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    }
}

fn load_or_create_shell_session(
    session_store: &SessionStore,
    config: &agent_core::ProviderConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
    current_dir: &Path,
) -> StoredSession {
    if let Ok(Some(session)) = session_store
        .list_sessions()
        .map(|sessions| sessions.into_iter().find(stored_session_is_resumable))
    {
        return session;
    }
    let session_id = "shell_default".to_string();
    StoredSession {
        session_id: session_id.clone(),
        display_name: "ShellSession".to_string(),
        created_at_ms: now_ms_i64(),
        updated_at_ms: now_ms_i64(),
        current_dir: current_dir.display().to_string(),
        profile: shell_session_profile(config),
        env: shell_session_env_values(config, bash_approval_mode, work_instruction_mode),
        env_overrides: None,
        state: StoredSessionState::Ready,
        last_turn_id: None,
        raw_chat_history_path: session_store
            .history_path_for_session(&session_id)
            .display()
            .to_string(),
    }
}

fn stored_session_is_resumable(session: &StoredSession) -> bool {
    !session.session_id.trim().is_empty() && Path::new(&session.current_dir).is_dir()
}

fn shell_session_work_dir(session: &StoredSession, fallback: &Path) -> PathBuf {
    let current_dir = PathBuf::from(&session.current_dir);
    if current_dir.is_dir() {
        std::fs::canonicalize(&current_dir).unwrap_or(current_dir)
    } else {
        std::fs::canonicalize(fallback).unwrap_or_else(|_| fallback.to_path_buf())
    }
}

fn shell_session_effective_env(
    launch_env: &HashMap<String, String>,
    session: &StoredSession,
) -> HashMap<String, String> {
    let mut merged = launch_env.clone();
    for (key, value) in &session.env {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

fn shell_session_profile(config: &agent_core::ProviderConfig) -> StoredSessionProfile {
    StoredSessionProfile {
        provider: config.provider.clone(),
        model: config.model.clone(),
        api_protocol: config.api_protocol.label().to_string(),
        response_protocol: config.response_protocol.name().to_string(),
    }
}

fn shell_session_env_values(
    config: &agent_core::ProviderConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([
        (
            "TIMEM_GATEWAY_PROVIDER".to_string(),
            config.provider.clone(),
        ),
        ("TIMEM_MODEL".to_string(), config.model.clone()),
        (
            "TIMEM_API_PROTOCOL".to_string(),
            config.api_protocol.label().to_string(),
        ),
        (
            "TIMEM_RESPONSE_PROTOCOL".to_string(),
            config.response_protocol.name().to_string(),
        ),
        ("TIMEM_BASE_URL".to_string(), config.base_url.clone()),
        ("TIMEM_TIMEOUT".to_string(), config.timeout_secs.to_string()),
        (
            "TIMEM_MAX_LLM_INPUT".to_string(),
            config.max_llm_input_tokens.to_string(),
        ),
        (
            "TIMEM_MAX_LLM_OUTPUT".to_string(),
            config.max_llm_output_tokens.to_string(),
        ),
        (
            "TIMEM_BASH_APPROVAL".to_string(),
            agent_core::bash_approval_mode_label(bash_approval_mode).to_string(),
        ),
        (
            "TIMEM_WORK_INSTRUCTIONS".to_string(),
            agent_core::work_instruction_mode_label(work_instruction_mode).to_string(),
        ),
    ]);
    if let Some(value) = config.openai_compatible.enable_thinking {
        env.insert("TIMEM_ENABLE_THINKING".to_string(), value.to_string());
    }
    if let Some(value) = &config.openai_compatible.reasoning_effort {
        env.insert("TIMEM_REASONING_EFFORT".to_string(), value.clone());
    }
    env.insert(
        "TIMEM_STREAM".to_string(),
        config.openai_compatible.stream.to_string(),
    );
    env
}

fn take_shell_resume_notice(
    session_store: &SessionStore,
    session_id: &str,
    current_dir: &Path,
    pending: &mut bool,
) -> Option<String> {
    if !std::mem::take(pending) {
        return None;
    }
    Some(
        SessionResumeNotice {
            history_path: session_store.history_path_for_session(session_id),
            current_dir: current_dir.to_path_buf(),
        }
        .render(),
    )
}

fn append_shell_history_message(
    session_store: &SessionStore,
    session_id: &str,
    turn_id: &str,
    role: ChatHistoryRole,
    content: &str,
) {
    let _ = session_store.append_history_record(
        session_id,
        &ChatHistoryRecord::Message {
            role,
            turn_id: turn_id.to_string(),
            created_at_ms: now_ms_i64(),
            kind: None,
            content: content.to_string(),
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn append_shell_turn_result(
    session_store: &SessionStore,
    stored_session: &mut StoredSession,
    session_id: &str,
    turn_id: &str,
    assistant_text: &str,
    outcome: &timem_shell::TurnOutcome,
    config: &agent_core::ProviderConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
    current_dir: &Path,
) {
    append_shell_history_message(
        session_store,
        session_id,
        turn_id,
        ChatHistoryRole::Assistant,
        assistant_text,
    );
    let mut extra = BTreeMap::new();
    extra.insert(
        "payload".to_string(),
        json!({
            "stats": outcome.stats,
            "elapsed_ms": outcome.elapsed.as_millis(),
            "repair_issue": outcome.repair_issue,
            "stop_reason": outcome.stop_reason,
        }),
    );
    let _ = session_store.append_history_record(
        session_id,
        &ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: turn_id.to_string(),
            created_at_ms: now_ms_i64(),
            kind: agent_core::session_store::ChatHistoryEventKind::Stats,
            content: "turn stats".to_string(),
            extra,
        },
    );
    stored_session.updated_at_ms = now_ms_i64();
    stored_session.current_dir = current_dir.display().to_string();
    stored_session.profile = shell_session_profile(config);
    stored_session.env =
        shell_session_env_values(config, bash_approval_mode, work_instruction_mode);
    stored_session.state = if outcome.stop_reason.is_some() {
        StoredSessionState::Interrupted
    } else {
        StoredSessionState::Ready
    };
    stored_session.last_turn_id = Some(turn_id.to_string());
    stored_session.raw_chat_history_path = session_store
        .history_path_for_session(session_id)
        .display()
        .to_string();
    let _ = session_store.upsert_session(stored_session);
}

fn shell_turn_id() -> String {
    format!("shell_turn_{}", now_ms_i64())
}

fn now_ms_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

struct CliTurnUi<'a> {
    status: Option<&'a mut ThinkingStatus>,
    interactive_approval: bool,
    supplement_input: Option<ThinkingSupplementInput>,
}

impl TurnUi for CliTurnUi<'_> {
    fn is_cancel_requested(&mut self) -> bool {
        if let Some(input) = self.supplement_input.as_mut() {
            let _ = input.poll();
        }
        TURN_CANCEL_REQUESTED.load(Ordering::SeqCst)
    }

    fn take_cancel_request(&mut self) -> bool {
        consume_turn_cancel_request()
    }

    fn drain_user_supplements(&mut self) -> Vec<String> {
        let supplements = self
            .supplement_input
            .as_mut()
            .map(ThinkingSupplementInput::drain)
            .unwrap_or_default();
        if !supplements.is_empty() {
            if let Some(status) = self.status.as_deref_mut() {
                status.add_user_supplement_notice(supplements.len());
            }
        }
        supplements
    }

    fn on_model_request(&mut self, round: u32, prompt: &str) {
        if let Some(status) = self.status.as_deref_mut() {
            status.settle_active_observations();
            status.set_model_direction(round, ModelDirection::Upstream);
            status.set_pending_request_usage(estimate_prompt_context_tokens(prompt));
            status.set_transient_observation("思考中...");
        }
    }

    fn on_model_response(&mut self, round: u32, usage: &UsageStats, _content: &str) {
        if let Some(status) = self.status.as_deref_mut() {
            status.clear_transient_observation();
            status.set_usage(usage.clone());
            status.set_model_direction(round, ModelDirection::Downstream);
        }
    }

    fn on_model_response_discarded(&mut self, _round: u32, _reason: &str) {
        if let Some(status) = self.status.as_deref_mut() {
            status.clear_transient_observation();
        }
    }

    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        if let Some(status) = self.status.as_deref_mut() {
            if let Some(hint) = topic_event_status_hint(events) {
                status.set_intent(&hint.action, hint.memory_activity);
            }
            status.apply_observation_events(observation_events_from_core_topic_events(events));
        }
    }

    fn on_model_error(&mut self, _error: &str) {
        if let Some(status) = self.status.as_deref_mut() {
            status.clear_transient_observation();
        }
    }

    fn on_model_retry(&mut self, attempt: u32, max_attempts: u32, delay: Duration, error: &str) {
        if let Some(status) = self.status.as_deref_mut() {
            status.set_network_retry(attempt, max_attempts, delay, error);
        }
    }

    fn pause_for_user_decision(&mut self) {
        self.supplement_input = None;
        if let Some(status) = self.status.as_deref_mut() {
            status.pause_for_user_approval();
        }
    }

    fn resume_after_user_decision(&mut self) {
        if self.supplement_input.is_none() {
            self.supplement_input = ThinkingSupplementInput::new();
        }
        if let Some(status) = self.status.as_deref_mut() {
            status.resume_after_user_approval();
        }
    }

    fn can_request_output_expansion(&mut self) -> bool {
        self.interactive_approval
    }

    fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
        if !self.interactive_approval {
            return request.safe_default().into();
        }
        let accepted = match request {
            HostDecisionRequest::UserApproval(request) => request_user_approval(&request),
            HostDecisionRequest::RoundLimitContinue(request) => {
                request_round_limit_continue(request)
            }
            HostDecisionRequest::OutputExpansion(request) => request_expand_output_tokens(request),
            HostDecisionRequest::StaleContextContinue(request) => {
                request_stale_context_continue(request)
            }
            HostDecisionRequest::WorkInstructionLoad(request) => {
                choose_work_instructions_load(&request) == ApprovalChoice::Allow
            }
            HostDecisionRequest::LongRunningCommandContinue(_) => true,
        };
        if accepted {
            HostDecision::Accept
        } else {
            HostDecision::Decline
        }
    }
}

struct ThinkingStatus {
    state: Arc<Mutex<ThinkingViewSnapshot>>,
    running: Arc<AtomicBool>,
    rendered_lines: Arc<Mutex<usize>>,
    handle: Option<JoinHandle<()>>,
    stop_tx: Option<Sender<()>>,
    started_at: Instant,
    paused_total: Arc<Mutex<Duration>>,
    paused_since: Option<Instant>,
}

impl ThinkingStatus {
    fn start(provider: &str, model: &str, max_llm_input_tokens: u32) -> Self {
        let started_at = Instant::now();
        let paused_total = Arc::new(Mutex::new(Duration::ZERO));
        let state = Arc::new(Mutex::new(ThinkingViewSnapshot {
            status: ShellStatusSnapshot {
                provider: provider.to_string(),
                model: model.to_string(),
                intent: "思考中".to_string(),
                memory_activity: CoreMemoryActivity::None,
                model_round: 1,
                direction: ModelDirection::Upstream,
                usage: UsageStats::zero(),
                latest_usage: None,
                tick: random_spinner_tick(),
                elapsed_secs: 0,
                max_llm_input_tokens,
                retry: None,
            },
            observations: {
                let mut panel = ObservationPanel::default();
                panel.apply(ObservationEvent::EnsureTransient("思考中...".to_string()));
                panel
            },
        }));
        let running = Arc::new(AtomicBool::new(true));
        let rendered_lines = Arc::new(Mutex::new(0));
        render_thinking(&state.lock().unwrap(), &rendered_lines);
        let (handle, stop_tx) = spawn_thinking_renderer(
            Arc::clone(&state),
            Arc::clone(&running),
            Arc::clone(&rendered_lines),
            Arc::clone(&paused_total),
            started_at,
        );
        Self {
            state,
            running,
            rendered_lines,
            handle: Some(handle),
            stop_tx: Some(stop_tx),
            started_at,
            paused_total,
            paused_since: None,
        }
    }

    fn set_model_direction(&mut self, round: u32, direction: ModelDirection) {
        if let Ok(mut state) = self.state.lock() {
            state.status.model_round = round;
            state.status.direction = direction;
            state.status.retry = None;
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn set_usage(&mut self, usage: UsageStats) {
        if let Ok(mut state) = self.state.lock() {
            state.status.usage.add(&usage);
            state.status.latest_usage = Some(usage);
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn set_pending_request_usage(&mut self, prompt_tokens: u32) {
        if let Ok(mut state) = self.state.lock() {
            state.status.latest_usage = Some(UsageStats {
                prompt_tokens,
                ..UsageStats::zero()
            });
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn set_intent(&mut self, intent: &str, memory_activity: CoreMemoryActivity) {
        if let Ok(mut state) = self.state.lock() {
            state.status.intent = intent
                .trim_end_matches('…')
                .trim_end_matches("...")
                .trim()
                .to_string();
            state.status.memory_activity = memory_activity;
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn set_transient_observation(&mut self, text: &str) {
        if let Ok(mut state) = self.state.lock() {
            state
                .observations
                .apply(ObservationEvent::EnsureTransient(text.to_string()));
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

    fn add_user_supplement_notice(&mut self, count: usize) {
        let text = if count == 1 {
            "已收到用户补充指示，下一轮会使用。".to_string()
        } else {
            format!("已收到 {count} 条用户补充指示，下一轮会使用。")
        };
        if let Ok(mut state) = self.state.lock() {
            state.observations.apply(ObservationEvent::Persistent(text));
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn set_network_retry(&mut self, attempt: u32, max_attempts: u32, delay: Duration, error: &str) {
        if let Ok(mut state) = self.state.lock() {
            let until_epoch_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .saturating_add(delay)
                .as_millis();
            state.status.retry = Some(RuntimeRetryStatus {
                until_epoch_ms: Some(until_epoch_ms),
                error: Some(error.to_string()),
                attempt: Some(attempt),
                max_attempts: Some(max_attempts),
            });
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
        self.stop_renderer_thread();
        clear_thinking_block(&self.rendered_lines);
    }

    fn finish_cancelled(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.stop_renderer_thread();
        if let Ok(mut state) = self.state.lock() {
            state.status.intent = "已取消".to_string();
            state.status.elapsed_secs = active_elapsed_secs(self.started_at, &self.paused_total);
            rerender_thinking(&state, &self.rendered_lines);
        }
    }

    fn accumulated_stats(&self) -> UsageStats {
        self.state
            .lock()
            .map(|s| s.status.usage.clone())
            .unwrap_or_else(|_| UsageStats::zero())
    }

    fn accumulated_latest_usage(&self) -> Option<UsageStats> {
        self.state
            .lock()
            .ok()
            .and_then(|s| s.status.latest_usage.clone())
    }

    fn pause_for_user_approval(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.stop_renderer_thread();
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
        let (handle, stop_tx) = spawn_thinking_renderer(
            Arc::clone(&self.state),
            Arc::clone(&self.running),
            Arc::clone(&self.rendered_lines),
            Arc::clone(&self.paused_total),
            self.started_at,
        );
        self.handle = Some(handle);
        self.stop_tx = Some(stop_tx);
    }

    fn stop_renderer_thread(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn spawn_thinking_renderer(
    state: Arc<Mutex<ThinkingViewSnapshot>>,
    running: Arc<AtomicBool>,
    rendered_lines: Arc<Mutex<usize>>,
    paused_total: Arc<Mutex<Duration>>,
    started_at: Instant,
) -> (JoinHandle<()>, Sender<()>) {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            match stop_rx.recv_timeout(Duration::from_millis(1000)) {
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if !running.load(Ordering::Relaxed) {
                break;
            }
            if let Ok(mut snapshot) = state.lock() {
                snapshot.status.tick = snapshot.status.tick.wrapping_add(1);
                snapshot.status.elapsed_secs = active_elapsed_secs(started_at, &paused_total);
                rerender_thinking(&snapshot, &rendered_lines);
            }
        }
    });
    (handle, stop_tx)
}

fn active_elapsed_secs(started_at: Instant, paused_total: &Arc<Mutex<Duration>>) -> u64 {
    let paused = paused_total
        .lock()
        .map(|duration| *duration)
        .unwrap_or(Duration::ZERO);
    runtime_active_elapsed_secs(started_at.elapsed(), paused)
}

fn request_user_approval(request: &ApprovalRequest) -> bool {
    match choose_user_approval(request) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

fn request_round_limit_continue(request: RoundLimitDecisionRequest) -> bool {
    match choose_round_limit_continue(request) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

fn request_expand_output_tokens(request: OutputExpansionRequest) -> bool {
    match choose_expand_output_tokens(request) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

fn request_stale_context_continue(request: StaleContextDecisionRequest) -> bool {
    match choose_stale_context_continue(request) {
        ApprovalChoice::Allow => true,
        ApprovalChoice::Deny => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApprovalChoice {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApprovalDecision {
    Choice(ApprovalChoice),
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteRecoveryChoice {
    SubmitEdited,
    Restore,
    ReturnToEdit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteRecoveryDecision {
    Choice(PasteRecoveryChoice),
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PasteRecoveryOutcome {
    decision: PasteRecoveryDecision,
    rendered_lines: usize,
}

fn render_user_approval_prompt(request: &ApprovalRequest) -> String {
    format!(
        "\n需要确认执行这个命令（超出低风险自动执行范围）。\n  command: {}\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n",
        request.command
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

fn render_round_limit_prompt(request: RoundLimitDecisionRequest) -> String {
    let context_text = if request.keep_task_context {
        "当前任务上下文保持不变"
    } else {
        "当前任务上下文不会保持"
    };
    format!(
        "\n本轮已达到最大交互次数 {}。\n继续后会为模型重新充值 rounds_remaining 为 {}，{}。\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n",
        request.max_rounds, request.recharge_rounds, context_text
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

fn render_expand_output_prompt(request: OutputExpansionRequest) -> String {
    let retry_text = if request.retry_same_turn {
        "并自动重试本轮请求"
    } else {
        "但不自动重试本轮请求"
    };
    format!(
        "\n模型输出达到当前上限 {}，导致 JSON 被截断。\n是否将 TIMEM_MAX_LLM_OUTPUT 临时增加 {}{}？\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n",
        format_token_count(request.current_tokens),
        format_token_count(request.increment_tokens),
        retry_text
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

struct ThinkingSupplementInput {
    input: ShellInputSource,
    terminal_mode: TerminalModeGuard,
    nonblocking_mode: NonblockingGuard,
    buffer: Vec<u8>,
    pending: Vec<String>,
}

impl ThinkingSupplementInput {
    fn new() -> Option<Self> {
        let input = ShellInputSource::open().ok()?;
        let fd = input.as_raw_fd();
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return None;
        }
        let mode = thinking_supplement_terminal_mode(original);
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &mode) } != 0 {
            return None;
        }
        let terminal_mode = TerminalModeGuard::new(fd, original);
        let nonblocking_mode = NonblockingGuard::new(fd).ok()?;
        Some(Self {
            input,
            terminal_mode,
            nonblocking_mode,
            buffer: Vec::new(),
            pending: Vec::new(),
        })
    }

    fn poll(&mut self) -> io::Result<()> {
        let mut bytes = [0u8; 256];
        loop {
            match self.input.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => self.push_bytes(&bytes[..n]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    fn drain(&mut self) -> Vec<String> {
        let _ = self.poll();
        let mut supplements = std::mem::take(&mut self.pending);
        let queued = drain_queued_tty_input(
            Duration::from_millis(20),
            Duration::from_millis(20),
            Duration::from_millis(120),
        );
        if queued.interrupted {
            TURN_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
        }
        supplements.extend(queued_text_to_supplements(&queued.text));
        supplements
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        push_thinking_supplement_bytes(&mut self.buffer, &mut self.pending, bytes);
    }
}

impl Drop for ThinkingSupplementInput {
    fn drop(&mut self) {
        let _ = self.poll();
        self.nonblocking_mode.restore();
        self.terminal_mode.restore();
    }
}

fn thinking_supplement_terminal_mode(mut mode: libc::termios) -> libc::termios {
    mode.c_lflag &= !(libc::ICANON | libc::ECHO);
    // Keep ISIG enabled so Ctrl+C still reaches the process-level turn cancel
    // handler while ordinary text can be polled as a supplement line.
    mode.c_cc[libc::VMIN] = 0;
    mode.c_cc[libc::VTIME] = 0;
    mode
}

fn push_thinking_supplement_bytes(buffer: &mut Vec<u8>, pending: &mut Vec<String>, bytes: &[u8]) {
    for &byte in bytes {
        match byte {
            b'\r' | b'\n' => finish_thinking_supplement_line(buffer, pending),
            3 | 4 | 27 => {}
            8 | 127 => pop_last_utf8_char_bytes(buffer),
            byte if byte.is_ascii_control() => {}
            byte => buffer.push(byte),
        }
    }
}

fn finish_thinking_supplement_line(buffer: &mut Vec<u8>, pending: &mut Vec<String>) {
    let bytes = std::mem::take(buffer);
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    if !text.is_empty() {
        pending.push(text);
    }
}

fn queued_text_to_supplements(text: &str) -> Vec<String> {
    normalize_newlines(text)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn pop_last_utf8_char_bytes(buffer: &mut Vec<u8>) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(text) = std::str::from_utf8(buffer) {
        if let Some((idx, _)) = text.char_indices().next_back() {
            buffer.truncate(idx);
            return;
        }
    }
    buffer.pop();
    while !buffer.is_empty() && std::str::from_utf8(buffer).is_err() {
        buffer.pop();
    }
}

fn format_idle_duration(duration: std::time::Duration) -> String {
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
fn render_stale_context_prompt(request: StaleContextDecisionRequest) -> String {
    let no_effect = if request.decline_clears_dynamic_context {
        "选择 NO 会清空旧动态上下文，从当前问题重新开始。"
    } else {
        "选择 NO 不会清空旧动态上下文。"
    };
    format!(
        "\n距离上次对话已经过去 {}，当前旧任务上下文约 {} tokens。\n是否继续使用上次对话任务上下文？{}\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n",
        format_idle_duration(request.idle),
        timem_shell::compact_count(request.dynamic_context_tokens),
        no_effect
    )
}

fn render_stale_context_choices(selected: ApprovalChoice) -> String {
    match selected {
        ApprovalChoice::Allow => "\x1b[7m[ YES ]\x1b[0m   NO".to_string(),
        ApprovalChoice::Deny => "  YES   \x1b[7m[ NO ]\x1b[0m".to_string(),
    }
}

fn render_paste_recovery_prompt(summary: &PasteRecoverySummary) -> String {
    let lines = vec![
        format!(
            "检测到 {} 个粘贴关联标签 {} 可能被误编辑，请确认：",
            summary.dirty_count,
            ansi_inverse(&summary.first_dirty_marker)
        ),
        "继续/恢复粘贴/返回编辑".to_string(),
        "使用 ←/→ 或 ↑/↓ 选择，回车确认，Ctrl+C/Esc 取消当前输入。".to_string(),
        format!("原始粘贴内容共 {} 行。", summary.total_lines),
    ];
    format!("\n{}", render_note_box("Note", &lines))
}

fn render_startup_status_block(messages: &[timem_shell::HostStatusMessage]) -> String {
    let lines = messages
        .iter()
        .map(|message| {
            let label = match message.level {
                timem_shell::HostStatusLevel::Info => "\x1b[1;32m[INFO]\x1b[0m",
                timem_shell::HostStatusLevel::Warning => "\x1b[1;33m[WARN]\x1b[0m",
                timem_shell::HostStatusLevel::Error => "\x1b[1;31m[ERROR]\x1b[0m",
            };
            format!("{label} {}", message.text.trim())
        })
        .collect::<Vec<_>>();
    render_note_box("Startup", &lines)
}

fn ansi_inverse(text: &str) -> String {
    format!("\x1b[7m{text}\x1b[0m")
}

fn render_note_box(title: &str, lines: &[String]) -> String {
    let width = terminal_width().clamp(48, 88).saturating_sub(2);
    render_note_box_at_width(title, lines, width)
}

fn render_note_box_at_width(title: &str, lines: &[String], width: usize) -> String {
    let content_width = width.saturating_sub(4).max(24);
    let title = format!(" {title} ");
    let mut out = String::new();
    out.push_str("\x1b[1m┏━");
    out.push_str(&title);
    out.push_str(&"━".repeat(content_width.saturating_sub(display_width(&title) + 1)));
    out.push_str("┓\x1b[0m\n");
    for line in lines {
        for wrapped in wrap_display_ansi(line, content_width.saturating_sub(2)) {
            let padded = pad_display_width_ansi(&wrapped, content_width.saturating_sub(2));
            out.push('┃');
            out.push(' ');
            out.push_str(&padded);
            out.push(' ');
            out.push_str("┃\n");
        }
    }
    out.push_str("\x1b[1m┗");
    out.push_str(&"━".repeat(content_width));
    out.push_str("┛\x1b[0m\n");
    out
}

fn render_paste_recovery_choices(selected: PasteRecoveryChoice) -> String {
    fn label(choice: PasteRecoveryChoice, text: &str, selected: PasteRecoveryChoice) -> String {
        if choice == selected {
            format!("\x1b[7m[ {text} ]\x1b[0m")
        } else {
            format!("  {text}  ")
        }
    }
    format!(
        "{}   {}   {}",
        label(PasteRecoveryChoice::SubmitEdited, "继续", selected),
        label(PasteRecoveryChoice::Restore, "恢复粘贴", selected),
        label(PasteRecoveryChoice::ReturnToEdit, "返回编辑", selected)
    )
}

fn next_paste_recovery_choice(choice: PasteRecoveryChoice) -> PasteRecoveryChoice {
    match choice {
        PasteRecoveryChoice::SubmitEdited => PasteRecoveryChoice::Restore,
        PasteRecoveryChoice::Restore => PasteRecoveryChoice::ReturnToEdit,
        PasteRecoveryChoice::ReturnToEdit => PasteRecoveryChoice::SubmitEdited,
    }
}

fn prev_paste_recovery_choice(choice: PasteRecoveryChoice) -> PasteRecoveryChoice {
    match choice {
        PasteRecoveryChoice::SubmitEdited => PasteRecoveryChoice::ReturnToEdit,
        PasteRecoveryChoice::Restore => PasteRecoveryChoice::SubmitEdited,
        PasteRecoveryChoice::ReturnToEdit => PasteRecoveryChoice::Restore,
    }
}

fn render_raw_multiline_paste_submit_prompt(line_count: usize) -> String {
    format!(
        "\n检测到 {line_count} 行粘贴内容。为避免把粘贴中的换行误当成多次提交，请确认是否作为一条消息提交。\n使用 ←/→ 或 ↑/↓ 选择，回车确认。\n"
    )
}

fn render_raw_multiline_paste_submit_choices(selected: ApprovalChoice) -> String {
    match selected {
        ApprovalChoice::Allow => "\x1b[7m[ 提交 ]\x1b[0m   取消".to_string(),
        ApprovalChoice::Deny => "  提交   \x1b[7m[ 取消 ]\x1b[0m".to_string(),
    }
}

fn choose_raw_multiline_paste_submit(line_count: usize) -> ApprovalChoice {
    print!("{}", render_raw_multiline_paste_submit_prompt(line_count));
    choose_with_keyboard(
        render_raw_multiline_paste_submit_choices,
        ApprovalChoice::Deny,
    )
}

fn choose_user_approval(request: &ApprovalRequest) -> ApprovalChoice {
    print!("{}", render_user_approval_prompt(request));
    choose_with_keyboard(render_approval_choices, ApprovalChoice::Deny)
}

fn choose_round_limit_continue(request: RoundLimitDecisionRequest) -> ApprovalChoice {
    print!("{}", render_round_limit_prompt(request));
    choose_with_keyboard(render_round_limit_choices, ApprovalChoice::Allow)
}

fn choose_expand_output_tokens(request: OutputExpansionRequest) -> ApprovalChoice {
    print!("{}", render_expand_output_prompt(request));
    choose_with_keyboard(render_expand_output_choices, ApprovalChoice::Allow)
}

fn choose_stale_context_continue(request: StaleContextDecisionRequest) -> ApprovalChoice {
    print!("{}", render_stale_context_prompt(request));
    choose_with_keyboard(render_stale_context_choices, ApprovalChoice::Allow)
}

fn load_work_instructions_for_shell(
    dir: &std::path::Path,
) -> (Option<String>, Option<HostStatusMessage>) {
    work_instruction_shell_load_result(work_instruction_load_report(dir))
}

fn resolve_work_instruction_context_for_turn(
    mode: WorkInstructionLoadMode,
    current_work_dir: &Path,
    session: &str,
    ui: &mut dyn TurnUi,
) -> Option<String> {
    match mode {
        WorkInstructionLoadMode::Silent => {
            let report = work_instruction_load_report(current_work_dir);
            ui.on_core_topic_events(&[work_instruction_load_topic_event(session, &report)]);
            work_instruction_shell_load_result(report).0
        }
        WorkInstructionLoadMode::Ask => {
            let request = work_instruction_load_request(current_work_dir)?;
            if ui
                .request_host_decision_topic(
                    session,
                    HostDecisionRequest::WorkInstructionLoad(request),
                )
                .as_bool()
            {
                let report = work_instruction_load_report(current_work_dir);
                ui.on_core_topic_events(&[work_instruction_load_topic_event(session, &report)]);
                work_instruction_shell_load_result(report).0
            } else {
                None
            }
        }
        WorkInstructionLoadMode::Off => None,
    }
}

fn work_instruction_shell_load_result(
    report: WorkInstructionLoadReport,
) -> (Option<String>, Option<HostStatusMessage>) {
    let message = report.message();
    match message.kind {
        WorkInstructionLoadMessageKind::Loaded => {
            let names = message.file_names.join(", ");
            (
                report.context,
                Some(HostStatusMessage {
                    level: message.level.unwrap_or(timem_shell::HostStatusLevel::Info),
                    text: format!("已加载当前工作目录指令：{names}"),
                }),
            )
        }
        WorkInstructionLoadMessageKind::NotFound => (None, None),
        WorkInstructionLoadMessageKind::Failed => (
            None,
            Some(HostStatusMessage {
                level: message
                    .level
                    .unwrap_or(timem_shell::HostStatusLevel::Warning),
                text: format!(
                    "工作目录指令加载失败：{}",
                    message.error.unwrap_or_else(|| "unknown_error".to_string())
                ),
            }),
        ),
    }
}

fn render_work_instructions_load_prompt(request: &WorkInstructionLoadRequest) -> String {
    let names = request.file_names.join(", ");
    format!(
        "\n发现当前工作目录下存在指令文件：{}\n目录：{}\n是否加载到本轮 agent context？\n使用 ←/→ 或 ↑/↓ 选择，回车确认，Ctrl+C/Esc 跳过；30s 不操作自动跳过。\n",
        names,
        request.directory.display()
    )
}

fn render_work_instructions_load_choices(selected: ApprovalChoice) -> String {
    match selected {
        ApprovalChoice::Allow => "\x1b[7m[ 加载 ]\x1b[0m   跳过".to_string(),
        ApprovalChoice::Deny => "  加载   \x1b[7m[ 跳过 ]\x1b[0m".to_string(),
    }
}

fn choose_work_instructions_load(request: &WorkInstructionLoadRequest) -> ApprovalChoice {
    print!("{}", render_work_instructions_load_prompt(request));
    let timeout = HostDecisionRequest::WorkInstructionLoad(request.clone()).timeout();
    match choose_with_keyboard_decision_timeout(
        render_work_instructions_load_choices,
        ApprovalChoice::Allow,
        timeout,
    ) {
        ApprovalDecision::Choice(choice) => choice,
        ApprovalDecision::Cancel => ApprovalChoice::Deny,
    }
}

fn choose_paste_recovery(summary: &PasteRecoverySummary) -> PasteRecoveryOutcome {
    choose_paste_recovery_with_keyboard(summary, PasteRecoveryChoice::ReturnToEdit)
}

fn choose_paste_recovery_with_keyboard(
    summary: &PasteRecoverySummary,
    initial: PasteRecoveryChoice,
) -> PasteRecoveryOutcome {
    let mut selected = initial;
    let prompt = render_paste_recovery_prompt(summary);
    let rendered_lines = prompt.lines().count() + 1;

    let Ok(mut input) = ShellInputSource::open() else {
        println!();
        return PasteRecoveryOutcome {
            decision: PasteRecoveryDecision::Cancel,
            rendered_lines: 1,
        };
    };
    let fd = input.as_raw_fd();
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        println!();
        return PasteRecoveryOutcome {
            decision: PasteRecoveryDecision::Cancel,
            rendered_lines: 1,
        };
    }
    let mut raw = original;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 1;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        println!();
        return PasteRecoveryOutcome {
            decision: PasteRecoveryDecision::Cancel,
            rendered_lines: 1,
        };
    }
    let mut terminal_mode = TerminalModeGuard::new(fd, original);
    let mut nonblocking_mode = match NonblockingGuard::new(fd) {
        Ok(guard) => guard,
        Err(_) => {
            terminal_mode.restore();
            println!();
            return PasteRecoveryOutcome {
                decision: PasteRecoveryDecision::Cancel,
                rendered_lines: 1,
            };
        }
    };
    print!("{}{}", prompt, render_paste_recovery_choices(selected));
    let _ = io::stdout().flush();

    let result = loop {
        match read_paste_recovery_key(&mut input) {
            PasteRecoveryKey::Previous => {
                selected = prev_paste_recovery_choice(selected);
                print!("\r\x1b[2K{}", render_paste_recovery_choices(selected));
                let _ = io::stdout().flush();
            }
            PasteRecoveryKey::Next => {
                selected = next_paste_recovery_choice(selected);
                print!("\r\x1b[2K{}", render_paste_recovery_choices(selected));
                let _ = io::stdout().flush();
            }
            PasteRecoveryKey::Select(choice) => {
                selected = choice;
                print!("\r\x1b[2K{}", render_paste_recovery_choices(selected));
                let _ = io::stdout().flush();
            }
            PasteRecoveryKey::Enter => break PasteRecoveryDecision::Choice(selected),
            PasteRecoveryKey::Cancel => break PasteRecoveryDecision::Cancel,
            PasteRecoveryKey::Other => {}
        }
    };
    nonblocking_mode.restore();
    terminal_mode.restore();
    println!();
    PasteRecoveryOutcome {
        decision: result,
        rendered_lines,
    }
}

fn clear_previous_terminal_lines(lines: usize) {
    if lines == 0 {
        return;
    }
    print!("\x1b[{lines}F\x1b[J");
    let _ = io::stdout().flush();
}

fn paste_recovery_return_edit_clear_lines(
    rendered_recovery_lines: usize,
    prompt: &str,
    raw_input: &str,
    terminal_width: usize,
) -> usize {
    let prompt_width = display_width(prompt);
    let displayed_input = strip_paste_markers(raw_input);
    rendered_recovery_lines + submitted_input_rows(prompt_width, &displayed_input, terminal_width)
}

type ConfigField = RuntimeConfigField;

fn run_config_menu(
    config: &mut timem_shell::ProviderConfig,
    core: &mut AgentCore,
    bash_approval_mode: &mut BashApprovalMode,
    work_instruction_mode: &mut WorkInstructionLoadMode,
    work_instruction_context: &mut Option<String>,
    current_work_dir: &Path,
) -> bool {
    let Some(field) = choose_config_field(config, *bash_approval_mode, *work_instruction_mode)
    else {
        println!("已取消配置修改。");
        return false;
    };
    let current = config_field_value(config, *bash_approval_mode, *work_instruction_mode, field);
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
    match apply_config_value(
        config,
        core,
        bash_approval_mode,
        work_instruction_mode,
        field,
        value,
    ) {
        Ok(report) => {
            println!("{}", render_config_apply_report(&report));
            if field == RuntimeConfigField::WorkInstructions {
                if let Some(message) = apply_work_instruction_mode_after_config(
                    *work_instruction_mode,
                    current_work_dir,
                    work_instruction_context,
                ) {
                    println!("{}", render_startup_status_block(&[message]));
                }
            }
            true
        }
        Err(err) => {
            println!("{}", render_config_apply_error(err));
            false
        }
    }
}

fn apply_work_instruction_mode_after_config(
    mode: WorkInstructionLoadMode,
    current_work_dir: &Path,
    work_instruction_context: &mut Option<String>,
) -> Option<timem_shell::HostStatusMessage> {
    match mode {
        WorkInstructionLoadMode::Silent => {
            let (context, notice) = load_work_instructions_for_shell(current_work_dir);
            *work_instruction_context = context;
            notice.or_else(|| {
                Some(timem_shell::HostStatusMessage {
                    level: timem_shell::HostStatusLevel::Info,
                    text: "当前工作目录未发现 AGENTS.md/CLAUDE.md 指令。".to_string(),
                })
            })
        }
        WorkInstructionLoadMode::Ask => {
            if let Some(request) = work_instruction_load_request(current_work_dir) {
                if choose_work_instructions_load(&request) == ApprovalChoice::Allow {
                    let (context, notice) = load_work_instructions_for_shell(current_work_dir);
                    *work_instruction_context = context;
                    notice
                } else {
                    *work_instruction_context = None;
                    Some(timem_shell::HostStatusMessage {
                        level: timem_shell::HostStatusLevel::Info,
                        text: "已跳过当前工作目录的 AGENTS.md/CLAUDE.md 指令。".to_string(),
                    })
                }
            } else {
                *work_instruction_context = None;
                Some(timem_shell::HostStatusMessage {
                    level: timem_shell::HostStatusLevel::Info,
                    text: "当前工作目录未发现 AGENTS.md/CLAUDE.md 指令。".to_string(),
                })
            }
        }
        WorkInstructionLoadMode::Off => {
            *work_instruction_context = None;
            Some(timem_shell::HostStatusMessage {
                level: timem_shell::HostStatusLevel::Info,
                text: "已关闭当前工作目录指令的后续注入；历史上下文中的既有内容不会被删除。"
                    .to_string(),
            })
        }
    }
}

fn run_workspace_menu(workspace_config: &Path) -> bool {
    loop {
        let report =
            timem_shell::workspace_menu_report(&load_workspace_dirs_from_path(workspace_config));
        let Some(selection) = choose_workspace_item(&report) else {
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
                let report = apply_workspace_command_to_path(
                    workspace_config,
                    WorkspaceCommand::AddDir {
                        value: raw_value.trim().to_string(),
                        home_dir: home_dir(),
                    },
                );
                println!("{}", render_workspace_command_report(&report));
                match report.outcome {
                    WorkspaceCommandOutcome::EmptyInput | WorkspaceCommandOutcome::Duplicate => {
                        continue
                    }
                    _ => return report.changed,
                }
            }
            WorkspaceSelection::Dir(index) => {
                let Some(dir) = report.dirs.get(index).cloned() else {
                    continue;
                };
                if confirm_workspace_delete(&dir) {
                    let report = apply_workspace_command_to_path(
                        workspace_config,
                        WorkspaceCommand::RemoveIndex(index),
                    );
                    println!("{}", render_workspace_command_report(&report));
                    return report.changed;
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

fn choose_workspace_item(report: &WorkspaceMenuReport) -> Option<WorkspaceSelection> {
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
    let mut nonblocking_mode = NonblockingGuard::new(fd).ok()?;
    println!("\nWorkspace 目录用于提示模型参考资料位置，不限制模型只能在这些目录工作。");
    println!("使用 ↑/↓ 选择，回车确认，Esc/Ctrl+C 返回。\n");
    let mut selected = 0usize;
    print!("{}", render_workspace_menu(report, selected));
    let _ = io::stdout().flush();
    let item_count = report.dirs.len() + 1;
    let rendered_line_count = workspace_menu_line_count(report);
    let result = loop {
        match read_menu_key(&mut input) {
            MenuKey::Up => selected = selected.saturating_sub(1),
            MenuKey::Down => selected = (selected + 1).min(item_count.saturating_sub(1)),
            MenuKey::Enter => {
                break if selected < report.dirs.len() {
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
            render_workspace_menu(report, selected)
        );
        let _ = io::stdout().flush();
    };
    nonblocking_mode.restore();
    terminal_mode.restore();
    println!();
    result
}

fn workspace_menu_line_count(report: &WorkspaceMenuReport) -> usize {
    report.dirs.len().max(1) + 1
}

fn render_workspace_menu(report: &WorkspaceMenuReport, selected: usize) -> String {
    let mut lines = Vec::new();
    if report.is_empty {
        lines.push("  （暂无 workspace 目录）".to_string());
    } else {
        for (idx, dir) in report.dirs.iter().enumerate() {
            let marker = if idx == selected { "▶" } else { " " };
            let line = format!("{marker} {dir}");
            if idx == selected {
                lines.push(format!("\x1b[7m{line}\x1b[0m"));
            } else {
                lines.push(line);
            }
        }
    }
    let add_line = if report.add_index == selected {
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

fn render_workspace_command_report(report: &WorkspaceCommandReport) -> String {
    let message = report.message();
    match message.kind {
        WorkspaceCommandMessageKind::Added => {
            format!(
                "已加入 workspace：{}",
                message.subject.as_deref().unwrap_or("")
            )
        }
        WorkspaceCommandMessageKind::Removed => {
            format!(
                "已从 workspace 移除：{}",
                message.subject.as_deref().unwrap_or("")
            )
        }
        WorkspaceCommandMessageKind::Cancelled => "已取消 workspace 修改。".to_string(),
        WorkspaceCommandMessageKind::Duplicate => "目录已存在。".to_string(),
        WorkspaceCommandMessageKind::SelectionInvalid => "workspace 选择已失效。".to_string(),
        WorkspaceCommandMessageKind::SaveFailed => {
            format!(
                "保存 workspace 失败：{}",
                message.error.as_deref().unwrap_or("unknown error")
            )
        }
    }
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn choose_config_field(
    config: &timem_shell::ProviderConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
) -> Option<ConfigField> {
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
    let mut nonblocking_mode = NonblockingGuard::new(fd).ok()?;
    println!("\n选择要修改的配置，使用 ↑/↓ 选择，回车确认，Esc/Ctrl+C 取消。\n");
    let mut selected = 0usize;
    let report =
        timem_shell::runtime_config_menu_report(config, bash_approval_mode, work_instruction_mode);
    print!("{}", render_config_menu(&report, selected));
    let _ = io::stdout().flush();
    let result = loop {
        match read_menu_key(&mut input) {
            MenuKey::Up => selected = selected.saturating_sub(1),
            MenuKey::Down => selected = (selected + 1).min(report.items.len().saturating_sub(1)),
            MenuKey::Enter => break report.items.get(selected).map(|item| item.field),
            MenuKey::Cancel => break None,
            MenuKey::Other => {}
        }
        print!(
            "\x1b[{}F{}",
            report.items.len(),
            render_config_menu(&report, selected)
        );
        let _ = io::stdout().flush();
    };
    nonblocking_mode.restore();
    terminal_mode.restore();
    println!();
    result
}

fn render_config_menu(report: &RuntimeConfigMenuReport, selected: usize) -> String {
    report
        .items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let marker = if idx == selected { "▶" } else { " " };
            let value = config_display_value(config_field_row_kind(item.field), &item.value);
            let line = format!("{marker} {:<22} {value}", item.key);
            if idx == selected {
                format!("\x1b[7m{line}\x1b[0m\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect()
}

fn render_config_apply_report(report: &RuntimeConfigApplyReport) -> String {
    let message = report.message();
    match message.kind {
        RuntimeConfigApplyMessageKind::Updated => format!("已更新 {}。", message.key),
    }
}

fn render_config_apply_error(error: RuntimeConfigApplyError) -> String {
    match error {
        RuntimeConfigApplyError::EmptyGatewayProvider => {
            "配置无效：TIMEM_GATEWAY_PROVIDER 不能为空。".to_string()
        }
        RuntimeConfigApplyError::CustomGatewayRequiresBaseUrl => {
            "配置无效：自定义 gateway provider 需要先设置 TIMEM_BASE_URL，避免沿用旧平台默认 URL。"
                .to_string()
        }
        RuntimeConfigApplyError::InvalidApiProtocol => {
            "配置无效：API protocol 只能是 openai-compatible、openai-responses 或 anthropic。"
                .to_string()
        }
        RuntimeConfigApplyError::InvalidTokenCount {
            field: RuntimeConfigField::MaxInput,
        } => "配置无效：请输入数字，或 100K/1M 这类格式。".to_string(),
        RuntimeConfigApplyError::InvalidTokenCount {
            field: RuntimeConfigField::MaxOutput,
        } => "配置无效：请输入数字，或 10K 这类格式。".to_string(),
        RuntimeConfigApplyError::InvalidTokenCount { field } => {
            format!("配置无效：{} 不接受 token 数值。", field.label())
        }
        RuntimeConfigApplyError::InvalidBashApproval => {
            "配置无效：bash 允许策略只能是 approve 或 ask。".to_string()
        }
        RuntimeConfigApplyError::InvalidWorkInstructions => {
            "配置无效：工作目录指令加载策略只能是 silent、ask 或 off。".to_string()
        }
    }
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
    let Some(byte) = read_key_byte_wait(input) else {
        return MenuKey::Cancel;
    };
    match byte {
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
    work_instruction_mode: WorkInstructionLoadMode,
    field: ConfigField,
) -> String {
    config_display_value(
        config_field_row_kind(field),
        &timem_shell::runtime_config_field_value(
            config,
            bash_approval_mode,
            work_instruction_mode,
            field,
        ),
    )
}

fn config_display_value(kind: timem_shell::RuntimeConfigRowKind, value: &str) -> String {
    match kind {
        timem_shell::RuntimeConfigRowKind::MaxLlmInput
        | timem_shell::RuntimeConfigRowKind::MaxLlmOutput => value
            .parse::<u32>()
            .map(format_token_count)
            .unwrap_or_else(|_| value.to_string()),
        _ => value.to_string(),
    }
}

fn config_field_row_kind(field: ConfigField) -> timem_shell::RuntimeConfigRowKind {
    match field {
        RuntimeConfigField::Model => timem_shell::RuntimeConfigRowKind::Model,
        RuntimeConfigField::GatewayProvider => timem_shell::RuntimeConfigRowKind::GatewayProvider,
        RuntimeConfigField::ApiProtocol => timem_shell::RuntimeConfigRowKind::ApiProtocol,
        RuntimeConfigField::BaseUrl => timem_shell::RuntimeConfigRowKind::BaseUrl,
        RuntimeConfigField::MaxInput => timem_shell::RuntimeConfigRowKind::MaxLlmInput,
        RuntimeConfigField::MaxOutput => timem_shell::RuntimeConfigRowKind::MaxLlmOutput,
        RuntimeConfigField::BashApproval => timem_shell::RuntimeConfigRowKind::BashApproval,
        RuntimeConfigField::WorkInstructions => timem_shell::RuntimeConfigRowKind::WorkInstructions,
    }
}

fn apply_config_value(
    config: &mut timem_shell::ProviderConfig,
    core: &mut AgentCore,
    bash_approval_mode: &mut BashApprovalMode,
    work_instruction_mode: &mut WorkInstructionLoadMode,
    field: ConfigField,
    value: &str,
) -> Result<RuntimeConfigApplyReport, RuntimeConfigApplyError> {
    core.apply_runtime_config_update(
        config,
        bash_approval_mode,
        work_instruction_mode,
        field,
        value,
    )
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
    let mut nonblocking_mode = NonblockingGuard::new(fd).ok()?;
    let mut out = String::new();
    let result = loop {
        if TURN_CANCEL_REQUESTED.load(Ordering::SeqCst) {
            break None;
        }
        let Some(byte) = read_cancelable_byte(&mut input) else {
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
                        let Some(next) = read_cancelable_byte(&mut input) else {
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
    nonblocking_mode.restore();
    terminal_mode.restore();
    let _ = consume_turn_cancel_request();
    result
}

fn read_cancelable_byte(input: &mut impl Read) -> Option<u8> {
    loop {
        if TURN_CANCEL_REQUESTED.load(Ordering::SeqCst) {
            return None;
        }
        let mut buf = [0u8; 1];
        match input.read(&mut buf) {
            Ok(1) => return Some(buf[0]),
            Ok(_) => return None,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
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
    match choose_with_keyboard_decision(render_choices, initial) {
        ApprovalDecision::Choice(choice) => choice,
        ApprovalDecision::Cancel => ApprovalChoice::Deny,
    }
}

fn choose_with_keyboard_decision(
    render_choices: fn(ApprovalChoice) -> String,
    initial: ApprovalChoice,
) -> ApprovalDecision {
    choose_with_keyboard_decision_timeout(render_choices, initial, None)
}

fn choose_with_keyboard_decision_timeout(
    render_choices: fn(ApprovalChoice) -> String,
    initial: ApprovalChoice,
    timeout: Option<Duration>,
) -> ApprovalDecision {
    let mut selected = initial;

    let Ok(mut input) = ShellInputSource::open() else {
        println!();
        return ApprovalDecision::Cancel;
    };
    let fd = input.as_raw_fd();
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        println!();
        return ApprovalDecision::Cancel;
    }
    let mut raw = original;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 1;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        println!();
        return ApprovalDecision::Cancel;
    }
    let mut terminal_mode = TerminalModeGuard::new(fd, original);
    let mut nonblocking_mode = match NonblockingGuard::new(fd) {
        Ok(guard) => guard,
        Err(_) => {
            terminal_mode.restore();
            println!();
            return ApprovalDecision::Cancel;
        }
    };
    print!("{}", render_choices(selected));
    let _ = io::stdout().flush();
    let deadline = timeout.map(|duration| Instant::now() + duration);

    let result = loop {
        let key = match deadline {
            Some(deadline) => {
                if Instant::now() >= deadline {
                    break ApprovalDecision::Cancel;
                }
                read_approval_key_until(&mut input, deadline)
            }
            None => read_approval_key(&mut input),
        };
        match key {
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
            ApprovalKey::Enter => break ApprovalDecision::Choice(selected),
            ApprovalKey::Cancel => break ApprovalDecision::Cancel,
            ApprovalKey::Other => {}
        }
    };
    nonblocking_mode.restore();
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

struct NonblockingGuard {
    fd: i32,
    original_flags: i32,
    active: bool,
}

impl NonblockingGuard {
    fn new(fd: i32) -> io::Result<Self> {
        let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if original_flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, original_flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            fd,
            original_flags,
            active: true,
        })
    }

    fn restore(&mut self) {
        if self.active {
            let _ = unsafe { libc::fcntl(self.fd, libc::F_SETFL, self.original_flags) };
            self.active = false;
        }
    }
}

impl Drop for NonblockingGuard {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteRecoveryKey {
    Previous,
    Next,
    Select(PasteRecoveryChoice),
    Enter,
    Cancel,
    Other,
}

fn read_approval_key(input: &mut impl Read) -> ApprovalKey {
    let Some(byte) = read_key_byte_wait(input) else {
        return ApprovalKey::Cancel;
    };
    approval_key_from_byte(input, byte)
}

fn read_approval_key_until(input: &mut impl Read, deadline: Instant) -> ApprovalKey {
    let Some(byte) = read_key_byte_until(input, deadline) else {
        return ApprovalKey::Cancel;
    };
    approval_key_from_byte(input, byte)
}

fn approval_key_from_byte(input: &mut impl Read, byte: u8) -> ApprovalKey {
    match byte {
        b'\r' | b'\n' => ApprovalKey::Enter,
        3 | 4 | 27 => {
            if byte != 27 {
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

fn read_paste_recovery_key(input: &mut impl Read) -> PasteRecoveryKey {
    let Some(byte) = read_key_byte_wait(input) else {
        return PasteRecoveryKey::Cancel;
    };
    match byte {
        b'\r' | b'\n' => PasteRecoveryKey::Enter,
        3 | 4 => PasteRecoveryKey::Cancel,
        27 => {
            let Some(seq) = read_escape_sequence(input) else {
                return PasteRecoveryKey::Cancel;
            };
            match seq.as_slice() {
                [] | [27] => PasteRecoveryKey::Cancel,
                [b'[', b'A'] | [b'[', b'D'] => PasteRecoveryKey::Previous,
                [b'[', b'B'] | [b'[', b'C'] => PasteRecoveryKey::Next,
                _ => PasteRecoveryKey::Other,
            }
        }
        b'h' | b'k' => PasteRecoveryKey::Previous,
        b'\t' | b' ' | b'j' | b'l' => PasteRecoveryKey::Next,
        b'y' | b'Y' => PasteRecoveryKey::Select(PasteRecoveryChoice::Restore),
        b'n' | b'N' => PasteRecoveryKey::Select(PasteRecoveryChoice::SubmitEdited),
        b'e' | b'E' => PasteRecoveryKey::Select(PasteRecoveryChoice::ReturnToEdit),
        _ => PasteRecoveryKey::Other,
    }
}

fn read_escape_sequence(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut seq = Vec::with_capacity(8);
    let deadline = Instant::now() + Duration::from_millis(80);
    for _ in 0..16 {
        let Some(byte) = read_key_byte_until(input, deadline) else {
            return if seq.is_empty() { None } else { Some(seq) };
        };
        seq.push(byte);
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'~') {
            break;
        }
    }
    Some(seq)
}

fn read_key_byte_wait(input: &mut impl Read) -> Option<u8> {
    loop {
        if consume_turn_cancel_request() {
            return None;
        }
        match read_key_byte_once(input) {
            KeyByteRead::Byte(byte) => return Some(byte),
            KeyByteRead::Pending => thread::sleep(Duration::from_millis(20)),
            KeyByteRead::Closed => return None,
        }
    }
}

fn read_key_byte_until(input: &mut impl Read, deadline: Instant) -> Option<u8> {
    loop {
        if consume_turn_cancel_request() {
            return None;
        }
        match read_key_byte_once(input) {
            KeyByteRead::Byte(byte) => return Some(byte),
            KeyByteRead::Closed => return None,
            KeyByteRead::Pending => {}
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

enum KeyByteRead {
    Byte(u8),
    Pending,
    Closed,
}

fn read_key_byte_once(input: &mut impl Read) -> KeyByteRead {
    let mut buf = [0u8; 1];
    match input.read(&mut buf) {
        Ok(1) => KeyByteRead::Byte(buf[0]),
        Ok(_) => KeyByteRead::Closed,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            KeyByteRead::Pending
        }
        Err(_) => KeyByteRead::Closed,
    }
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
            next.sa_sigaction = handle_sigint as *const () as usize;
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct PasteRecoverySummary {
    dirty_count: usize,
    total_lines: usize,
    first_dirty_marker: String,
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
    let mut first_dirty_marker = String::new();
    let mut used_records = vec![false; records.len()];
    for (idx, marked) in paste_marker_segments(input).into_iter().enumerate() {
        let Some(record_idx) = paste_record_index_for_marker(&marked, idx, records, &used_records)
        else {
            continue;
        };
        used_records[record_idx] = true;
        let record = &records[record_idx];
        if marked != record.placeholder {
            dirty_count += 1;
            total_lines += pasted_line_count(&record.content);
            if dirty_count == 1 {
                first_dirty_marker = strip_paste_markers(&marked);
            }
        }
    }
    (dirty_count > 0).then_some(PasteRecoverySummary {
        dirty_count,
        total_lines,
        first_dirty_marker,
    })
}

fn resolve_paste_markers(input: &str, records: &[PasteRecord], restore_dirty: bool) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    let mut record_idx = 0usize;
    let mut used_records = vec![false; records.len()];
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
        if let Some(matched_idx) =
            paste_record_index_for_marker(&marked, record_idx, records, &used_records)
        {
            used_records[matched_idx] = true;
            let record = &records[matched_idx];
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

fn paste_record_index_for_marker(
    marked: &str,
    marker_idx: usize,
    records: &[PasteRecord],
    used_records: &[bool],
) -> Option<usize> {
    if let Some((idx, _)) = records.iter().enumerate().find(|(idx, record)| {
        !used_records.get(*idx).copied().unwrap_or(false) && record.placeholder == marked
    }) {
        return Some(idx);
    }
    records
        .iter()
        .enumerate()
        .filter(|(idx, _)| !used_records.get(*idx).copied().unwrap_or(false))
        .nth(marker_idx)
        .map(|(idx, _)| idx)
        .or_else(|| {
            records
                .iter()
                .enumerate()
                .find(|(idx, _)| !used_records.get(*idx).copied().unwrap_or(false))
                .map(|(idx, _)| idx)
        })
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
    Line {
        text: String,
        display: String,
    },
    PendingPaste {
        text: String,
        display: String,
        line_count: usize,
    },
    Interrupted,
    Eof,
    Error(String),
}

struct ShellLineEditor {
    editor: Reedline,
    paste_records: SharedPasteRecords,
    prefill_input: SharedPrefillInput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PasteRecord {
    placeholder: String,
    content: String,
}

type SharedPasteRecords = Arc<Mutex<Vec<PasteRecord>>>;
type SharedPrefillInput = Arc<Mutex<Option<String>>>;

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
        let prefill_input = Arc::new(Mutex::new(None));
        let edit_mode = Box::new(TimemEditMode::new(
            paste_records.clone(),
            prefill_input.clone(),
        ));
        let mut editor = Reedline::create()
            .with_edit_mode(edit_mode)
            .with_highlighter(Box::new(TimemPasteHighlighter))
            .with_ansi_colors(true)
            .use_bracketed_paste(true);
        if let Some(history) = history {
            editor = editor.with_history(Box::new(history));
        }
        Self {
            editor,
            paste_records,
            prefill_input,
        }
    }

    fn readline(&mut self, prompt: &str) -> ShellReadline {
        let mut preserve_paste_records_once = false;
        loop {
            let preserve_records = std::mem::take(&mut preserve_paste_records_once);
            if !preserve_records {
                if let Ok(mut records) = self.paste_records.lock() {
                    records.clear();
                }
            }
            let prompt = TimemReedlinePrompt {
                indicator: prompt.to_string(),
            };
            if let Some(prefill) = self
                .prefill_input
                .lock()
                .ok()
                .and_then(|mut prefill| prefill.take())
            {
                self.editor
                    .run_edit_commands(&[EditCommand::InsertString(prefill)]);
            }
            let _keyboard_guard = ReedlineKeyboardProtocolGuard::enter();
            let result = match self.editor.read_line(&prompt) {
                Ok(Signal::Success(text)) => {
                    let queued = drain_queued_tty_input_for_submission();
                    if queued.interrupted {
                        TURN_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
                        discard_queued_tty_input_after_cancel();
                        return ShellReadline::Interrupted;
                    }
                    let raw_input = merge_queued_input(text, &queued);
                    let raw = sanitize_user_input(&raw_input);
                    if raw_multiline_paste_needs_confirmation(&queued, &raw) {
                        return ShellReadline::PendingPaste {
                            display: raw_multiline_paste_display(&raw),
                            line_count: pasted_line_count(&raw),
                            text: raw,
                        };
                    }
                    let records = self
                        .paste_records
                        .lock()
                        .map(|records| records.clone())
                        .unwrap_or_default();
                    let summary = paste_recovery_summary_from_markers(&raw, &records);
                    let recovery = summary.as_ref().map(choose_paste_recovery);
                    let restore_dirty = match recovery.as_ref().map(|outcome| &outcome.decision) {
                        Some(PasteRecoveryDecision::Choice(PasteRecoveryChoice::Restore)) => true,
                        Some(PasteRecoveryDecision::Choice(PasteRecoveryChoice::SubmitEdited))
                        | None => false,
                        Some(PasteRecoveryDecision::Choice(PasteRecoveryChoice::ReturnToEdit)) => {
                            if let Some(outcome) = recovery.as_ref() {
                                let lines = paste_recovery_return_edit_clear_lines(
                                    outcome.rendered_lines,
                                    prompt.indicator.as_str(),
                                    &raw,
                                    terminal_width(),
                                );
                                clear_previous_terminal_lines(lines);
                            }
                            if let Ok(mut prefill) = self.prefill_input.lock() {
                                *prefill = Some(raw);
                            }
                            preserve_paste_records_once = true;
                            continue;
                        }
                        Some(PasteRecoveryDecision::Cancel) => {
                            discard_queued_tty_input_after_cancel();
                            return ShellReadline::Interrupted;
                        }
                    };
                    let text = resolve_paste_markers(&raw, &records, restore_dirty);
                    let display = strip_paste_markers(&raw);
                    ShellReadline::Line { display, text }
                }
                Ok(Signal::CtrlC) => {
                    discard_queued_tty_input_after_cancel();
                    ShellReadline::Interrupted
                }
                Ok(Signal::CtrlD) => ShellReadline::Eof,
                Ok(_) => {
                    discard_queued_tty_input_after_cancel();
                    ShellReadline::Interrupted
                }
                Err(err) => ShellReadline::Error(format!("readline_failed: {err}")),
            };
            return result;
        }
    }
}

#[derive(Default)]
struct QueuedInputDrain {
    text: String,
    interrupted: bool,
}

fn merge_queued_input(mut submitted: String, queued: &QueuedInputDrain) -> String {
    let queued_text = queued.text.strip_prefix('\n').unwrap_or(&queued.text);
    if !queued_text.is_empty() {
        submitted.push('\n');
        submitted.push_str(queued_text);
    }
    submitted
}

fn raw_multiline_paste_needs_confirmation(queued: &QueuedInputDrain, raw: &str) -> bool {
    !queued.text.is_empty() && pasted_line_count(raw) > 1
}

fn raw_multiline_paste_display(raw: &str) -> String {
    format!("[ pasted {} lines ]", pasted_line_count(raw))
}

fn drain_queued_tty_input_for_submission() -> QueuedInputDrain {
    drain_queued_tty_input(
        Duration::from_millis(90),
        Duration::from_millis(35),
        Duration::from_millis(500),
    )
}

fn discard_queued_tty_input_after_cancel() {
    let _ = drain_queued_tty_input(
        Duration::from_millis(300),
        Duration::from_millis(60),
        Duration::from_millis(1200),
    );
}

fn drain_queued_tty_input(
    initial_wait: Duration,
    quiet_window: Duration,
    hard_window: Duration,
) -> QueuedInputDrain {
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
    let initial_deadline = Instant::now() + initial_wait;
    while Instant::now() < initial_deadline {
        match tty.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&buf[..n]);
                break;
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => break,
        }
    }
    if bytes.is_empty() {
        let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
        return QueuedInputDrain::default();
    }

    let started = Instant::now();
    let mut quiet_deadline = Instant::now() + quiet_window;
    let hard_deadline = started + hard_window;
    while Instant::now() < quiet_deadline && Instant::now() < hard_deadline {
        match tty.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&buf[..n]);
                quiet_deadline = Instant::now() + quiet_window;
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
        text: normalize_newlines(&String::from_utf8_lossy(bytes).replace('\x03', "")),
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
    prefill_input: SharedPrefillInput,
}

impl TimemEditMode {
    fn new(paste_records: SharedPasteRecords, prefill_input: SharedPrefillInput) -> Self {
        Self {
            inner: Emacs::new(timem_reedline_keybindings()),
            paste_records,
            prefill_input,
        }
    }
}

impl EditMode for TimemEditMode {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let prefill = self
            .prefill_input
            .lock()
            .ok()
            .and_then(|mut prefill| prefill.take());
        let parsed = self.parse_event_without_prefill(event);
        if let Some(prefill) = prefill {
            return prepend_prefill_event(prefill, parsed);
        }
        parsed
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.inner.edit_mode()
    }
}

impl TimemEditMode {
    fn parse_event_without_prefill(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
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
}

fn prepend_prefill_event(prefill: String, parsed: ReedlineEvent) -> ReedlineEvent {
    let insert = ReedlineEvent::Edit(vec![EditCommand::InsertString(prefill)]);
    match parsed {
        ReedlineEvent::None => insert,
        ReedlineEvent::Edit(mut edits) => {
            let ReedlineEvent::Edit(mut with_prefill) = insert else {
                unreachable!();
            };
            with_prefill.append(&mut edits);
            ReedlineEvent::Edit(with_prefill)
        }
        other => ReedlineEvent::Multiple(vec![insert, other]),
    }
}

struct TimemPasteHighlighter;

impl Highlighter for TimemPasteHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        styled.push((nu_ansi_term::Style::new(), line.to_string()));
        for (start, end) in paste_marker_ranges(line) {
            styled.style_range(start, end, nu_ansi_term::Style::new().reverse());
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
    let mut snapshot = snapshot.clone();
    snapshot
        .observations
        .set_max_width(observation_panel_width_for_terminal(terminal_width()));
    let width = terminal_width();
    let rendered = render_thinking_view_at(&snapshot, &time_label());
    let line_count = rendered_terminal_rows(&rendered, width);
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

fn rendered_terminal_rows(rendered: &str, terminal_width: usize) -> usize {
    let width = terminal_width.max(1);
    let rows = rendered
        .lines()
        .map(|line| wrapped_terminal_rows(display_width(line), width))
        .sum::<usize>();
    rows.max(1)
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
        self.replace_with(HostStatusMessage {
            level: timem_shell::HostStatusLevel::Info,
            text: text.to_string(),
        });
    }

    fn replace_with(&mut self, message: HostStatusMessage) {
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
    format!("\x1b[94;1m[{time_label}] You ❯❯\x1b[0m ")
}

fn render_submitted_user_line_rewrite(
    input: &str,
    status_line_visible: bool,
    terminal_width: usize,
    time_label: &str,
) -> String {
    let prompt_prefix = render_user_input_prompt(time_label);
    let prompt_width = display_width(&prompt_prefix);
    let input_rows = submitted_input_rows(prompt_width, input, terminal_width);
    let rows_to_clear = input_rows + usize::from(status_line_visible);
    format!(
        "\x1b[{}F\r\x1b[J{}{}\n",
        rows_to_clear, prompt_prefix, input
    )
}

fn submitted_input_rows(prompt_width: usize, input: &str, terminal_width: usize) -> usize {
    let width = terminal_width.max(1);
    let mut rows = 0usize;
    for (idx, line) in input.split('\n').enumerate() {
        let line_width = display_width(line);
        let occupied = if idx == 0 {
            prompt_width + line_width
        } else {
            line_width
        };
        rows += wrapped_terminal_rows(occupied, width);
    }
    rows.max(1)
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
    latest_usage: Option<&UsageStats>,
    provider: &str,
    model: &str,
    elapsed: Duration,
    max_llm_input_tokens: u32,
) {
    let rendered = render_final_response_at(
        text,
        stats,
        latest_usage,
        provider,
        model,
        elapsed.as_secs(),
        max_llm_input_tokens,
        &time_label(),
    );
    print!("{rendered}");
    let _ = io::stdout().flush();
}

fn render_startup_banner(
    space: &str,
    config: &timem_shell::ProviderConfig,
    data_root: &std::path::Path,
    audit_file: &std::path::Path,
    action_audit_file: &std::path::Path,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
) -> String {
    let report = timem_shell::runtime_config_report(
        config,
        timem_shell::RuntimeConfigReportInput {
            space: space.to_string(),
            data_dir: absolute_display_path(data_root),
            api_audit_path: absolute_display_path(audit_file),
            action_audit_path: absolute_display_path(action_audit_file),
            bash_approval_mode,
            work_instruction_mode,
        },
    );
    let items = report
        .items
        .into_iter()
        .map(config_report_item_to_table_item)
        .collect::<Vec<_>>();
    boxed_config_table(&items)
}

fn config_report_item_to_table_item(item: timem_shell::RuntimeConfigReportItem) -> ConfigTableItem {
    match item {
        timem_shell::RuntimeConfigReportItem::Section(section) => {
            ConfigTableItem::Section(config_section_label(section).to_string())
        }
        timem_shell::RuntimeConfigReportItem::Row(row) => ConfigTableItem::Row(ConfigRow {
            desc: config_row_description(row.kind).to_string(),
            key: row.key,
            value: config_display_value(row.kind, &row.value),
            highlight: row.not_default,
        }),
    }
}

fn config_section_label(section: timem_shell::RuntimeConfigSection) -> &'static str {
    match section {
        timem_shell::RuntimeConfigSection::Model => "MODEL",
        timem_shell::RuntimeConfigSection::Runtime => "RUNTIME",
        timem_shell::RuntimeConfigSection::Data => "DATA",
    }
}

fn config_row_description(kind: timem_shell::RuntimeConfigRowKind) -> &'static str {
    match kind {
        timem_shell::RuntimeConfigRowKind::Model => "模型名称",
        timem_shell::RuntimeConfigRowKind::GatewayProvider => "流量平台，决定默认 base url",
        timem_shell::RuntimeConfigRowKind::ApiProtocol => "API 提交网络包格式",
        timem_shell::RuntimeConfigRowKind::BaseUrl => "网关 base url",
        timem_shell::RuntimeConfigRowKind::MaxLlmInput => "最大输入 token",
        timem_shell::RuntimeConfigRowKind::MaxLlmOutput => "最大输出 token",
        timem_shell::RuntimeConfigRowKind::BashApproval => "bash 允许策略，approve/ask",
        timem_shell::RuntimeConfigRowKind::WorkInstructions => {
            "AGENTS/CLAUDE 自动加载，silent/ask/off"
        }
        timem_shell::RuntimeConfigRowKind::Space => "记忆空间",
        timem_shell::RuntimeConfigRowKind::DataDir => "运行时记忆、日志存储",
        timem_shell::RuntimeConfigRowKind::ApiAudit => "payload 记录",
        timem_shell::RuntimeConfigRowKind::ActionAudit => "action 记录",
    }
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
                let desc_lines = wrap_display(&row.desc, desc_width);
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
    let key_floor = 24.min(content_width.saturating_sub(20).max(6));
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

fn pad_display_width_ansi(text: &str, target_width: usize) -> String {
    let width = display_width(text);
    if width >= target_width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(target_width - width))
    }
}

fn wrap_display_ansi(text: &str, target_width: usize) -> Vec<String> {
    if target_width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for raw_line in text.lines().default_if_empty() {
        let mut current = String::new();
        let mut current_width = 0usize;
        let mut chars = raw_line.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                current.push(ch);
                current.push(chars.next().unwrap_or('['));
                for code_ch in chars.by_ref() {
                    current.push(code_ch);
                    if code_ch.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width > 0 && current_width + ch_width > target_width {
                lines.push(pad_display_width_ansi(&current, target_width));
                current.clear();
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }
        lines.push(pad_display_width_ansi(&current, target_width));
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
    print!("{}", cli_help_text());
}

fn cli_help_text() -> &'static str {
    "Usage:\n  timem [options]\n\n\x1b[1mPrecedence:\n  command line options override process env values; process env overrides defaults.\x1b[0m\n\nCreate a private env file from env_template, then load it explicitly:\n  cp env_template env\n  source /path/to/your/env\n\nRecommended run:\n  timem\n\nUseful env values to put in your env file:\n  export TIMEM_GATEWAY_PROVIDER=aliyun\n  export TIMEM_API_KEY=your_api_key_here\n  export TIMEM_MODEL=qwen-plus\n  export TIMEM_SPACE=.test_mem\n\nCommand line override example:\n  timem --data-dir data --space .test_mem --gateway-provider aliyun --model qwen-plus\n\nOptions:\n  --space <name>                 env TIMEM_SPACE; memory/audit space, default .test_mem\n  --gateway-provider <name>      env TIMEM_GATEWAY_PROVIDER; traffic platform / default base URL provider\n  --api-protocol <protocol>      env TIMEM_API_PROTOCOL; provider wire format: openai-compatible|openai-responses|anthropic\n  --response-protocol <protocol> env TIMEM_RESPONSE_PROTOCOL; model response parser: markdown|json|xml, default xml\n  --base-url <url>               env TIMEM_BASE_URL; override provider default base URL\n  --model <name>                 env TIMEM_MODEL; model name\n  --api-key <key>                env TIMEM_API_KEY; API key, env is safer than shell history\n  --data-dir <path>              env TIMEM_DATA_DIR; data/config/memory/audit root\n  --timeout <seconds>            env TIMEM_TIMEOUT; provider HTTP timeout, default 120\n  --max-llm-input <n|100K>       env TIMEM_MAX_LLM_INPUT; max input context, default 100K\n  --max-llm-output <n|10K>       env TIMEM_MAX_LLM_OUTPUT; max output tokens, default 10K\n  --capabilities-dir <path>      env TIMEM_CAPABILITIES_DIR; runtime capability manifest overlay\n  --bash-approval <mode>         env TIMEM_BASH_APPROVAL; ask|approve, default ask\n  --work-instructions <mode>     env TIMEM_WORK_INSTRUCTIONS; silent|ask|off, default silent\n  --once-json <text>             run one non-interactive turn and print JSON\n  --supporting-context <text>    append extra runtime context for --once-json/debug\n  -h, --help                     show this help\n\nInteractive commands:\n  /help                          show these control commands\n  /config                        edit runtime model and token settings\n  /workspace                     manage workspace directories shown to the model as reference context\n  /prof                          show runtime profiling for tokens, model wait/local time, and storage size\n\nInteractive keys:\n  Ctrl+C or Esc cancels the current input, menu, or confirmation prompt.\n  While Timem is thinking, type a supplement and press Enter to add it to the current turn.\n  Ctrl+C also cancels an active model turn; one Ctrl+C never exits Timem by itself.\n  Use Ctrl+D or /exit to leave the shell intentionally.\n\nProtocol defaults:\n  API protocol: openai -> openai-responses; anthropic -> anthropic; others -> openai-compatible\n  Response protocol: xml\n\nVendor fallback key env vars:\n  DASHSCOPE_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN\n"
}

fn runtime_help_text() -> &'static str {
    "\x1b[1mInteractive commands\x1b[0m\n  /help       show runtime help\n  /config     edit runtime model and token settings\n  /workspace  manage workspace directories shown to the model as reference context\n  /prof       show runtime profiling for tokens, model wait/local time, and storage size\n  /exit       leave the shell intentionally\n\n\x1b[1mInteractive keys\x1b[0m\n  Ctrl+C or Esc cancels the current input, menu, or confirmation prompt.\n  While Timem is thinking, type a supplement and press Enter to add it to the current turn.\n  Ctrl+C also cancels an active model turn; one Ctrl+C never exits Timem by itself.\n  Ctrl+D exits the shell intentionally.\n\n\x1b[1mRuntime system\x1b[0m\n  Timem keeps a local memory space, runtime context, action audit, and API audit under the configured data directory.\n  Use /prof to inspect token usage, KVC stats, model wait time, local execution time, and storage size.\n  Use /config for changes that can take effect without restarting this Timem process.\n"
}

fn startup_control_hint() -> &'static str {
    "输入 /help 查看控制命令。"
}

fn time_label() -> String {
    local_time_label()
}

#[cfg(test)]
#[path = "../tests/unit/main_tests.rs"]
mod static_prompt_tests;
