use super::{
    active_elapsed_secs, apply_config_value, boxed_config_table_at_width, cli_help_text,
    config_field_value, consume_turn_cancel_request, display_width, load_or_create_shell_session,
    merge_queued_input, next_paste_recovery_choice, normalize_newlines, paste_marker_ranges,
    paste_marker_segments, paste_recovery_return_edit_clear_lines,
    paste_recovery_summary_from_markers, pasted_line_count, prev_paste_recovery_choice,
    push_thinking_supplement_bytes, queued_input_drain_from_bytes, queued_text_to_supplements,
    random_spinner_tick, raw_multiline_paste_display, raw_multiline_paste_needs_confirmation,
    read_approval_key, read_approval_key_until, read_menu_key, read_paste_recovery_key,
    reedline_keyboard_protocol_enter_sequence, reedline_keyboard_protocol_exit_sequence,
    render_approval_choices, render_config_apply_report, render_config_menu,
    render_expand_output_choices, render_expand_output_prompt, render_note_box_at_width,
    render_paste_recovery_choices, render_paste_recovery_prompt,
    render_raw_multiline_paste_submit_choices, render_raw_multiline_paste_submit_prompt,
    render_round_limit_choices, render_round_limit_prompt, render_stale_context_choices,
    render_stale_context_prompt, render_startup_banner, render_startup_status_block,
    render_submitted_user_line_rewrite, render_user_approval_prompt, render_user_input_prompt,
    render_work_instructions_load_choices, render_work_instructions_load_prompt,
    render_workspace_command_report, render_workspace_delete_choices, render_workspace_menu,
    rendered_terminal_rows, resolve_paste_markers, resolve_work_instruction_context_for_turn,
    runtime_help_text, sanitize_user_input, shell_runtime_info_entries, shell_session_env_values,
    shell_session_profile, startup_control_hint, strip_ansi, strip_paste_markers,
    submitted_input_rows, take_shell_resume_notice, thinking_supplement_terminal_mode,
    timem_reedline_keybindings, utf8_expected_len, work_instruction_shell_load_result,
    workspace_menu_line_count, wrapped_terminal_rows, ApprovalChoice, ApprovalKey, ConfigField,
    ConfigRow, ConfigTableItem, CoreTopicEvent, HostDecision, HostDecisionRequest, MenuKey,
    PasteRecord, PasteRecoveryChoice, PasteRecoveryKey, PasteRecoverySummary, QueuedInputDrain,
    SharedPasteRecords, SharedPrefillInput, ThinkingStatus, TimemEditMode, TimemPasteHighlighter,
    TimemReedlinePrompt, TurnUi, ANSI_HIGHLIGHT, PASTE_END_MARKER, PASTE_START_MARKER,
    STATIC_PROMPT, TURN_CANCEL_REQUESTED,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
use agent_core::{
    session_store::{
        ChatHistoryEventKind, ChatHistoryRecord, ChatHistoryRole, SessionStore, StoredSession,
        StoredSessionState,
    },
    stale_context_prompt_needed, AgentCore, ApprovalRequest, BashApprovalMode, CoreProfile,
    OutputExpansionRequest, ResponseProtocolKind, RoundLimitDecisionRequest,
    RuntimeConfigApplyError, StaleContextDecisionRequest, WorkInstructionLoadMode,
    WorkInstructionLoadReport, WorkInstructionLoadRequest, WorkInstructionLoadStatus,
    WorkspaceChange, WorkspaceCommandOutcome, WorkspaceCommandReport,
    DEFAULT_STALE_CONTEXT_IDLE as STALE_CONTEXT_IDLE,
    DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD as STALE_CONTEXT_TOKEN_THRESHOLD,
};
use crossterm::event::Event;
use crossterm::event::KeyEvent;
use reedline::{
    EditCommand, EditMode, Highlighter, KeyCode, KeyModifiers, Prompt, ReedlineEvent,
    ReedlineRawEvent,
};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use timem_shell::{
    workspace_menu_report, ApiProtocol, HostStatusLevel, ProviderConfig, SPINNER_ICONS,
};
use unicode_width::UnicodeWidthChar;

#[test]
fn static_prompt_uses_full_shared_v1_resource() {
    assert!(STATIC_PROMPT.contains("# Timem System Prompt"));
    assert!(STATIC_PROMPT.contains("## Role"));
    assert!(STATIC_PROMPT.contains("## Memory"));
    assert!(STATIC_PROMPT.contains("## Tools And Skills"));
    assert!(STATIC_PROMPT.contains("{{RESPONSE_PROTOCOL_SECTION}}"));
    // Response schema is now inside protocol section file
    assert!(STATIC_PROMPT.contains("{{TOOL_CATALOG}}"));
    assert!(STATIC_PROMPT.contains("{{SKILL_HEADERS}}"));
    assert!(!STATIC_PROMPT.contains("resources/response_v1_summary.json"));
    assert!(!STATIC_PROMPT.contains("response_v1` schema summary"));
    assert!(!STATIC_PROMPT.contains("\"acceptance_check?\""));
    assert!(!STATIC_PROMPT.contains("\"perspective_policy\""));
    assert!(!STATIC_PROMPT.contains("\"tool_claim_policy\""));
    assert!(!STATIC_PROMPT.contains("\"storage_style_policy\""));
    assert!(STATIC_PROMPT.contains("persisted user/assistant chat records"));
    assert!(!STATIC_PROMPT.contains("\"durable|raw_chat|scratch|context\""));
    assert!(!STATIC_PROMPT.contains("\"durable: query|schema|sql|insert|update|upsert|delete; raw_chat: query|sql|delete; scratch: query|write|read|delete; context: shrink\""));
    assert!(!STATIC_PROMPT.contains("\"query\": {\"type\": \"string\""));
    assert!(!STATIC_PROMPT.contains("\"tool_policy\""));
    assert!(!STATIC_PROMPT.contains("\"query_memory\""));
    assert!(!STATIC_PROMPT.contains("\"memory_schema\""));
    assert!(!STATIC_PROMPT.contains("\"memory_sql_query\""));
    assert!(!STATIC_PROMPT.contains("\"memory_update\""));
    assert!(!STATIC_PROMPT.contains("\"chat_history_query\""));
    assert!(!STATIC_PROMPT.contains("\"chat_history_delete\""));
    assert!(!STATIC_PROMPT.contains("\"scratch_write\""));
    assert!(!STATIC_PROMPT.contains("\"scratch_read\""));
    assert!(!STATIC_PROMPT.contains("\"scratch_query\""));
    assert!(!STATIC_PROMPT.contains("\"scratch_delete\""));
    assert!(!STATIC_PROMPT.contains("\"prompt_shrink\""));
    assert!(!STATIC_PROMPT.contains("\"shell_job_status\""));
    assert!(!STATIC_PROMPT.contains("foreground|background"));
    assert!(!STATIC_PROMPT.contains("\"durable_ctx_score\""));
    assert!(!STATIC_PROMPT.contains("Every model response must score"));
    assert!(STATIC_PROMPT.contains("Context maintenance"));
    assert!(STATIC_PROMPT.contains("response protocol's context compact branch"));
    assert!(STATIC_PROMPT.contains("do not target this system prompt"));
    assert!(!STATIC_PROMPT.contains("\"json_protocol\""));
    assert!(!STATIC_PROMPT.contains("\"evidence_guard\""));
    assert!(!STATIC_PROMPT.contains("\"action_result_guard\""));
    assert!(!STATIC_PROMPT.contains("\"thought?\""));
    assert!(!STATIC_PROMPT.contains("\"Self_audit\""));
    assert!(!STATIC_PROMPT.contains("self_audit"));
    assert!(!STATIC_PROMPT.contains("no_result_terminate"));
    assert!(!STATIC_PROMPT.contains("long_running_shell"));
    assert!(!STATIC_PROMPT.contains("lang_retry"));
    assert!(!STATIC_PROMPT.contains("theme_workflow"));
    assert!(!STATIC_PROMPT.contains("rounds_guard"));
    assert!(!STATIC_PROMPT.contains("perspective_rewrite"));
    assert!(!STATIC_PROMPT.contains("continue:false"));
    assert!(STATIC_PROMPT.len() > 3_000);
}

#[test]
fn thinking_supplement_collects_utf8_lines() {
    let mut buffer = Vec::new();
    let mut pending = Vec::new();

    push_thinking_supplement_bytes(
        &mut buffer,
        &mut pending,
        "补充：请优先修复 UI\r".as_bytes(),
    );

    assert!(buffer.is_empty());
    assert_eq!(pending, vec!["补充：请优先修复 UI"]);
}

#[test]
fn thinking_supplement_backspace_removes_one_utf8_char() {
    let mut buffer = Vec::new();
    let mut pending = Vec::new();

    push_thinking_supplement_bytes(&mut buffer, &mut pending, "中文".as_bytes());
    push_thinking_supplement_bytes(&mut buffer, &mut pending, &[127]);
    push_thinking_supplement_bytes(&mut buffer, &mut pending, b"\n");

    assert_eq!(pending, vec!["中"]);
}

#[test]
fn thinking_supplement_ignores_empty_and_control_lines() {
    let mut buffer = Vec::new();
    let mut pending = Vec::new();

    push_thinking_supplement_bytes(&mut buffer, &mut pending, b"   \n");
    push_thinking_supplement_bytes(&mut buffer, &mut pending, &[3, 4, 27]);
    push_thinking_supplement_bytes(&mut buffer, &mut pending, b" keep \n");

    assert_eq!(pending, vec!["keep"]);
}

#[test]
fn queued_thinking_supplement_text_splits_nonempty_lines() {
    assert_eq!(
        queued_text_to_supplements("\n 补充一 \r\n\n补充二\n"),
        vec!["补充一", "补充二"]
    );
}

#[test]
fn thinking_supplement_terminal_mode_is_noncanonical_but_keeps_sigint() {
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    original.c_lflag = libc::ICANON | libc::ECHO | libc::ISIG;
    original.c_cc[libc::VMIN] = 1;
    original.c_cc[libc::VTIME] = 1;

    let mode = thinking_supplement_terminal_mode(original);

    assert_eq!(mode.c_lflag & libc::ICANON, 0);
    assert_eq!(mode.c_lflag & libc::ECHO, 0);
    assert_ne!(mode.c_lflag & libc::ISIG, 0);
    assert_eq!(mode.c_cc[libc::VMIN], 0);
    assert_eq!(mode.c_cc[libc::VTIME], 0);
}

#[test]
fn public_repo_sources_do_not_contain_private_gateway_markers() {
    let source_text = [
        include_str!("../../../README.md"),
        include_str!("../../../env_template"),
        include_str!("../../../resources/system_prompt/system_prompt.md"),
        include_str!("../../src/lib.rs"),
        include_str!("../../src/main.rs"),
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
fn thinking_status_finish_wakes_renderer_without_waiting_for_tick() {
    let mut status = ThinkingStatus::start("aliyun", "qwen-plus", 100_000);

    let start = Instant::now();
    status.finish();

    assert!(
        start.elapsed() < Duration::from_millis(250),
        "finish waited for renderer tick: {:?}",
        start.elapsed()
    );
}

#[test]
fn thinking_status_initial_frame_includes_thought_panel() {
    let mut status = ThinkingStatus::start("aliyun", "qwen-plus", 100_000);
    let snapshot = status.state.lock().unwrap().clone();
    let rendered = timem_shell::render_thinking_view_at(&snapshot, "12:00:00");

    assert!(rendered.contains("Thought / Action"));
    assert!(rendered.contains("思考中"));
    assert!(!rendered.contains("x2"));

    status.finish();
}

#[test]
fn thinking_status_model_request_does_not_duplicate_initial_thinking_line() {
    let mut status = ThinkingStatus::start("aliyun", "qwen-plus", 100_000);
    status.set_transient_observation("思考中...");
    let snapshot = status.state.lock().unwrap().clone();
    let rendered = timem_shell::render_thinking_view_at(&snapshot, "12:00:00");

    assert_eq!(rendered.matches("思考中...").count(), 1);
    assert!(!rendered.contains("x2"));

    status.finish();
}

#[test]
fn thinking_status_finish_cancelled_preserves_display_and_stats() {
    let mut status = ThinkingStatus::start("aliyun", "qwen-plus", 100_000);
    status.set_usage(super::UsageStats {
        llm_calls: 1,
        prompt_tokens: 5000,
        completion_tokens: 200,
        total_tokens: 5200,
        ..super::UsageStats::zero()
    });
    status.finish_cancelled();
    let rendered_lines = *status.rendered_lines.lock().unwrap();
    assert!(
        rendered_lines > 0,
        "finish_cancelled should keep rendered lines"
    );
    let snapshot = status.state.lock().unwrap();
    assert_eq!(snapshot.status.intent, "已取消");
    assert_eq!(snapshot.status.usage.prompt_tokens, 5000);
    assert_eq!(snapshot.status.usage.completion_tokens, 200);
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
    let (text, stats, latest_usage, issue, stop_reason) = timem_shell::cancelled_turn_result();
    assert!(text.is_empty());
    assert_eq!(stats.llm_calls, 0);
    assert!(latest_usage.is_none());
    assert_eq!(issue, None);
    assert_eq!(
        stop_reason,
        Some(timem_shell::TurnStopReason::CancelledByUser)
    );
    let rendered =
        timem_shell::render_turn_stop_summary(&timem_shell::TurnStopSummary::cancelled_by_user());
    assert_eq!(rendered, "已取消本轮。");
    assert!(!rendered.contains("模型调用失败"));
}

#[test]
fn expand_output_prompt_is_keyboard_driven_and_mentions_retry() {
    let prompt = render_expand_output_prompt(OutputExpansionRequest::new(10_000));
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

    let prompt = render_stale_context_prompt(StaleContextDecisionRequest {
        idle: Duration::from_secs(3 * 60 * 60 + 5 * 60),
        dynamic_context_tokens: 12_300,
        continue_keeps_dynamic_context: true,
        decline_clears_dynamic_context: true,
    });
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
    let empty_report = workspace_menu_report(&[]);
    let empty = render_workspace_menu(&empty_report, 0);
    assert!(empty.contains("（暂无 workspace 目录）"));
    assert!(empty.contains("\x1b[7m▶ Add...\x1b[0m"));
    assert_eq!(workspace_menu_line_count(&empty_report), 2);

    let dirs = vec![
        "/tmp/timem_shell_fixture".to_string(),
        "/tmp/other".to_string(),
    ];
    let report = workspace_menu_report(&dirs);
    assert_eq!(
        report.dirs,
        vec![
            "/tmp/other".to_string(),
            "/tmp/timem_shell_fixture".to_string()
        ]
    );
    let selected_dir = render_workspace_menu(&report, 1);
    assert!(selected_dir.contains("/tmp/timem_shell_fixture"));
    assert!(selected_dir.contains("\x1b[7m▶ /tmp/timem_shell_fixture\x1b[0m"));
    assert!(selected_dir.contains("  Add..."));
    assert_eq!(workspace_menu_line_count(&report), 3);

    let selected_add = render_workspace_menu(&report, 2);
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
fn workspace_command_report_renderer_uses_core_outcome() {
    assert_eq!(
        render_workspace_command_report(&WorkspaceCommandReport {
            outcome: WorkspaceCommandOutcome::Added("/tmp/project".to_string()),
            dirs: vec!["/tmp/project".to_string()],
            changed: true,
        }),
        "已加入 workspace：/tmp/project"
    );
    assert_eq!(
        render_workspace_command_report(&WorkspaceCommandReport {
            outcome: WorkspaceCommandOutcome::Duplicate,
            dirs: vec!["/tmp/project".to_string()],
            changed: false,
        }),
        "目录已存在。"
    );
    assert_eq!(
        render_workspace_command_report(&WorkspaceCommandReport {
            outcome: WorkspaceCommandOutcome::SaveFailed {
                attempted_change: WorkspaceChange::Removed("/tmp/project".to_string()),
                error: "disk_full".to_string(),
            },
            dirs: Vec::new(),
            changed: false,
        }),
        "保存 workspace 失败：disk_full"
    );
}

#[test]
fn workspace_path_normalization_canonicalizes_existing_paths() {
    let dir = std::env::temp_dir().join(format!("timem_workspace_{}", epoch_millis()));
    fs::create_dir_all(&dir).unwrap();
    let nested = dir.join(".").join("child").join("..");
    fs::create_dir_all(dir.join("child")).unwrap();

    let normalized =
        timem_shell::normalize_workspace_dir(nested.to_str().unwrap(), std::path::Path::new("/"));
    assert_eq!(
        normalized,
        dir.canonicalize().unwrap().to_string_lossy().to_string()
    );

    let missing = dir.join("missing").join("path");
    assert_eq!(
        timem_shell::normalize_workspace_dir(missing.to_str().unwrap(), std::path::Path::new("/")),
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
        response_protocol: ResponseProtocolKind::Markdown,
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
    let mut work = WorkInstructionLoadMode::Silent;

    let menu_report = timem_shell::runtime_config_menu_report(&config, bash, work);
    let menu = render_config_menu(&menu_report, 5);
    assert!(menu.contains("TIMEM_MAX_LLM_OUTPUT"));
    assert!(menu.contains("TIMEM_WORK_INSTRUCTIONS"));
    assert!(menu.contains("10K"));
    assert!(menu.contains("\x1b[7m"));
    assert_eq!(
        config_field_value(&config, bash, work, ConfigField::MaxInput),
        "100K"
    );

    let output_report = apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::MaxOutput,
        "20K",
    )
    .unwrap();
    assert_eq!(output_report.key, "TIMEM_MAX_LLM_OUTPUT");
    assert_eq!(output_report.value, "20000");
    assert_eq!(
        render_config_apply_report(&output_report),
        "已更新 TIMEM_MAX_LLM_OUTPUT。"
    );
    let input_report = apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::MaxInput,
        "120K",
    )
    .unwrap();
    assert_eq!(input_report.key, "TIMEM_MAX_LLM_INPUT");
    assert_eq!(input_report.value, "120000");
    apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::WorkInstructions,
        "off",
    )
    .unwrap();
    apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::BashApproval,
        "approve",
    )
    .unwrap();

    assert_eq!(config.max_llm_output_tokens, 20_000);
    assert_eq!(config.max_llm_input_tokens, 120_000);
    assert_eq!(bash, BashApprovalMode::Approve);
    assert_eq!(work, WorkInstructionLoadMode::Off);
}

#[test]
fn config_provider_update_keeps_dependent_defaults_consistent() {
    let mut config = ProviderConfig {
        provider: "aliyun".to_string(),
        api_protocol: ApiProtocol::OpenAiCompatible,
        api_key: "secret".to_string(),
        model: "qwen-plus".to_string(),
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
        timeout_secs: 120,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        response_protocol: ResponseProtocolKind::Markdown,
    };
    let mut core = AgentCore::new(
        "STATIC",
        CoreProfile {
            name: "aliyun".to_string(),
            provider: "aliyun".to_string(),
            model: "qwen-plus".to_string(),
        },
        std::env::temp_dir().join(format!("timem_config_provider_test_{}", epoch_millis())),
    );
    let mut bash = BashApprovalMode::Ask;
    let mut work = WorkInstructionLoadMode::Silent;

    apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::GatewayProvider,
        "anthropic",
    )
    .unwrap();

    assert_eq!(config.provider, "anthropic");
    assert_eq!(config.api_protocol, ApiProtocol::Anthropic);
    assert_eq!(config.base_url, "https://api.anthropic.com");

    let err = apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::GatewayProvider,
        "private",
    )
    .unwrap_err();
    assert_eq!(err, RuntimeConfigApplyError::CustomGatewayRequiresBaseUrl);

    apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::BaseUrl,
        "https://private.example/v1",
    )
    .unwrap();
    apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::GatewayProvider,
        "private",
    )
    .unwrap();
    assert_eq!(config.provider, "private");
    assert_eq!(config.base_url, "https://private.example/v1");
}

#[test]
fn config_provider_update_resets_custom_settings_when_returning_to_known_provider() {
    let mut config = ProviderConfig {
        provider: "private".to_string(),
        api_protocol: ApiProtocol::Anthropic,
        api_key: "secret".to_string(),
        model: "aws-claude-sonnet-4-6".to_string(),
        base_url: "https://private.example/v1".to_string(),
        timeout_secs: 120,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        response_protocol: ResponseProtocolKind::Markdown,
    };
    let mut core = AgentCore::new(
        "STATIC",
        CoreProfile {
            name: "private".to_string(),
            provider: "private".to_string(),
            model: "aws-claude-sonnet-4-6".to_string(),
        },
        std::env::temp_dir().join(format!(
            "timem_config_provider_reset_test_{}",
            epoch_millis()
        )),
    );
    let mut bash = BashApprovalMode::Ask;
    let mut work = WorkInstructionLoadMode::Silent;

    apply_config_value(
        &mut config,
        &mut core,
        &mut bash,
        &mut work,
        ConfigField::GatewayProvider,
        "aliyun",
    )
    .unwrap();

    assert_eq!(config.provider, "aliyun");
    assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
    assert_eq!(
        config.base_url,
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    );
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
        response_protocol: ResponseProtocolKind::Markdown,
    };
    let banner = render_startup_banner(
        ".xxx_mem",
        &config,
        std::path::Path::new("data"),
        std::path::Path::new(".xxx_mem/audit/api_audit.json"),
        std::path::Path::new(".xxx_mem/audit/action_audit.json"),
        BashApprovalMode::Approve,
        WorkInstructionLoadMode::Silent,
    );

    assert!(banner.starts_with('┌'));
    assert!(banner.lines().next().unwrap_or("").starts_with("┌─"));
    assert!(banner
        .lines()
        .any(|line| line.starts_with("├──── ") && line.contains("MODEL") && line.contains(":")));
    assert!(banner
        .lines()
        .any(|line| line.starts_with("├──── ") && line.contains("RUNTIME") && line.contains(":")));
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
    assert!(banner.contains("api_audit.json"));
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
        ConfigTableItem::Section("MODEL".to_string()),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_GATEWAY_PROVIDER".to_string(),
            value: "aliyun".to_string(),
            desc: "流量平台，决定默认 base url".to_string(),
            highlight: false,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_BASE_URL".to_string(),
            value:
                "https://very-long-provider.example.com/compatible-mode/v1/with/a/path/that/wraps"
                    .to_string(),
            desc: "网关 base url".to_string(),
            highlight: false,
        }),
        ConfigTableItem::Row(ConfigRow {
            key: "TIMEM_WORK_INSTRUCTIONS".to_string(),
            value: "silent".to_string(),
            desc: "AGENTS/CLAUDE 自动加载，silent/ask/off".to_string(),
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
    assert!(banner.contains("TIMEM_WORK_INSTRUCTIONS"));
    assert!(!banner.contains("│ S "));
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
        response_protocol: ResponseProtocolKind::Markdown,
    };
    let default_banner = render_startup_banner(
        ".test_mem",
        &default_config,
        std::path::Path::new("data"),
        std::path::Path::new(".test_mem/audit/api_audit.json"),
        std::path::Path::new(".test_mem/audit/action_audit.json"),
        BashApprovalMode::Ask,
        WorkInstructionLoadMode::Silent,
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
        response_protocol: ResponseProtocolKind::Markdown,
    };
    let override_banner = render_startup_banner(
        ".test_mem",
        &override_config,
        std::path::Path::new("data"),
        std::path::Path::new(".test_mem/audit/api_audit.json"),
        std::path::Path::new(".test_mem/audit/action_audit.json"),
        BashApprovalMode::Ask,
        WorkInstructionLoadMode::Silent,
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
        response_protocol: ResponseProtocolKind::Markdown,
    };
    let banner = render_startup_banner(
        ".test_mem",
        &config,
        std::path::Path::new("data"),
        std::path::Path::new(".test_mem/audit/api_audit.json"),
        std::path::Path::new(".test_mem/audit/action_audit.json"),
        BashApprovalMode::Ask,
        WorkInstructionLoadMode::Silent,
    );

    assert!(!banner.contains(&format!(
        "{ANSI_HIGHLIGHT}https://your-private-gateway.example/v1"
    )));
    assert!(!banner.contains(&format!("{ANSI_HIGHLIGHT}aws-claude-sonnet-4-6")));
}

#[test]
fn cli_help_lists_all_env_backed_options() {
    let help = cli_help_text();
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
        "--capabilities-dir",
        "TIMEM_CAPABILITIES_DIR",
        "--bash-approval",
        "TIMEM_BASH_APPROVAL",
        "--work-instructions",
        "TIMEM_WORK_INSTRUCTIONS",
        "Interactive commands:",
        "/help",
        "/workspace",
        "/prof",
        "Ctrl+C or Esc cancels the current input, menu, or confirmation prompt",
        "Ctrl+C also cancels an active model turn",
        "one Ctrl+C never exits Timem by itself",
        "Use Ctrl+D or /exit",
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
    assert!(help.contains(
            "  --capabilities-dir <path>      env TIMEM_CAPABILITIES_DIR; runtime capability manifest overlay"
        ));
}

#[test]
fn runtime_help_omits_startup_options_and_highlights_sections() {
    let help = runtime_help_text();
    for expected in [
        "\x1b[1mInteractive commands\x1b[0m",
        "\x1b[1mInteractive keys\x1b[0m",
        "\x1b[1mRuntime system\x1b[0m",
        "/help",
        "/config",
        "/workspace",
        "/prof",
        "/exit",
        "Ctrl+C or Esc cancels",
        "While Timem is thinking",
        "Use /prof to inspect token usage",
        "Use /config for changes that can take effect without restarting",
    ] {
        assert!(
            help.contains(expected),
            "missing runtime help item: {expected}"
        );
    }
    for startup_only in [
        "Usage:",
        "Precedence:",
        "Command line override example:",
        "Create a private env file",
        "cp env_template env",
        "source /path/to/your/env",
        "--space",
        "--gateway-provider",
        "--api-key",
        "TIMEM_API_KEY",
        "Vendor fallback key env vars",
    ] {
        assert!(
            !help.contains(startup_only),
            "runtime help leaked startup help item: {startup_only}"
        );
    }
}

#[test]
fn startup_control_hint_points_to_help_instead_of_listing_commands() {
    let hint = startup_control_hint();
    assert_eq!(hint, "输入 /help 查看控制命令。");
    assert!(!hint.contains("/prof"));
    assert!(!hint.contains("/workspace"));
    assert!(!hint.contains("/exit"));
    assert!(!hint.contains("Ctrl+C"));
}

#[test]
fn env_template_exports_values_for_plain_source() {
    let template = include_str!("../../../env_template");
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
        "TIMEM_CAPABILITIES_DIR",
        "TIMEM_BASH_APPROVAL",
        "TIMEM_WORK_INSTRUCTIONS",
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
fn capabilities_dir_option_overrides_env() {
    let mut env = HashMap::new();
    env.insert(
        "TIMEM_CAPABILITIES_DIR".to_string(),
        "/env/capabilities".to_string(),
    );
    let options = timem_shell::CliOptions {
        capabilities_dir: Some("/cli/capabilities".to_string()),
        ..timem_shell::CliOptions::default()
    };

    assert_eq!(
        timem_shell::capabilities_dir_from_sources(options.capabilities_dir.as_deref(), &env)
            .as_deref(),
        Some(Path::new("/cli/capabilities"))
    );
    assert_eq!(
        timem_shell::capabilities_dir_from_sources(
            timem_shell::CliOptions::default()
                .capabilities_dir
                .as_deref(),
            &env
        )
        .as_deref(),
        Some(Path::new("/env/capabilities"))
    );
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
fn bash_approval_mode_accepts_only_current_documented_values() {
    let options = timem_shell::CliOptions::default();
    let empty = HashMap::new();
    assert_eq!(
        timem_shell::bash_approval_mode_from_sources(options.bash_approval.as_deref(), &empty),
        BashApprovalMode::Ask
    );

    let mut approve_env = HashMap::new();
    approve_env.insert("TIMEM_BASH_APPROVAL".to_string(), " APPROVE ".to_string());
    assert_eq!(
        timem_shell::bash_approval_mode_from_sources(
            options.bash_approval.as_deref(),
            &approve_env
        ),
        BashApprovalMode::Approve
    );

    let mut stale_env = HashMap::new();
    stale_env.insert("TIMEM_BASH_APPROVAL".to_string(), "approval".to_string());
    assert_eq!(
        timem_shell::bash_approval_mode_from_sources(options.bash_approval.as_deref(), &stale_env),
        BashApprovalMode::Ask
    );

    let stale_option = timem_shell::CliOptions {
        bash_approval: Some("never".to_string()),
        ..timem_shell::CliOptions::default()
    };
    assert_eq!(
        timem_shell::bash_approval_mode_from_sources(stale_option.bash_approval.as_deref(), &empty),
        BashApprovalMode::Ask
    );
}

#[test]
fn approval_prompt_shows_risk_command_and_keyboard_choices() {
    let prompt = render_user_approval_prompt(&ApprovalRequest {
        approval_id: "approval_test".to_string(),
        action: "run_bash".to_string(),
        command: "uname -s".to_string(),
        reason: "run_bash_requires_user_approval".to_string(),
        risk: "local_command_execution".to_string(),
    });

    assert!(prompt.contains("需要确认执行这个命令"));
    assert!(prompt.contains("command: uname -s"));
    assert!(prompt.contains("使用 ←/→ 或 ↑/↓ 选择"));
    assert!(!prompt.contains("输入 yes"));
    assert!(!prompt.contains("action: run_bash"));
    assert!(!prompt.contains("risk: local_command_execution"));
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
fn work_instruction_prompt_and_choices_are_keyboard_driven() {
    let prompt = render_work_instructions_load_prompt(&WorkInstructionLoadRequest {
        directory: "/tmp/project".into(),
        file_names: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
    });
    assert!(prompt.contains("AGENTS.md, CLAUDE.md"));
    assert!(prompt.contains("/tmp/project"));
    assert!(prompt.contains("30s 不操作自动跳过"));
    assert_eq!(
        render_work_instructions_load_choices(ApprovalChoice::Allow),
        "\x1b[7m[ 加载 ]\x1b[0m   跳过"
    );
    assert_eq!(
        render_work_instructions_load_choices(ApprovalChoice::Deny),
        "  加载   \x1b[7m[ 跳过 ]\x1b[0m"
    );
}

#[test]
fn work_instruction_shell_adapter_renders_core_report() {
    let (context, notice) = work_instruction_shell_load_result(WorkInstructionLoadReport {
        status: WorkInstructionLoadStatus::Loaded,
        directory: std::path::PathBuf::from("/tmp/project"),
        file_names: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
        context: Some("loaded context".to_string()),
        error: None,
    });
    assert_eq!(context.as_deref(), Some("loaded context"));
    let notice = notice.unwrap();
    assert_eq!(notice.level, HostStatusLevel::Info);
    assert!(notice.text.contains("AGENTS.md, CLAUDE.md"));

    let (context, notice) = work_instruction_shell_load_result(WorkInstructionLoadReport {
        status: WorkInstructionLoadStatus::Failed,
        directory: std::path::PathBuf::from("/tmp/project"),
        file_names: vec!["AGENTS.md".to_string()],
        context: None,
        error: Some("read_work_instruction_failed".to_string()),
    });
    assert_eq!(context, None);
    let notice = notice.unwrap();
    assert_eq!(notice.level, HostStatusLevel::Warning);
    assert!(notice.text.contains("read_work_instruction_failed"));
}

struct WorkInstructionDecisionUi {
    accept: bool,
    topics: Vec<String>,
    requests: usize,
}

impl TurnUi for WorkInstructionDecisionUi {
    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        self.topics
            .extend(events.iter().map(|event| event.topic.name.clone()));
    }

    fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
        assert!(matches!(
            request,
            HostDecisionRequest::WorkInstructionLoad(_)
        ));
        self.requests += 1;
        if self.accept {
            HostDecision::Accept
        } else {
            HostDecision::Decline
        }
    }
}

#[test]
fn work_instruction_context_for_turn_ask_uses_core_request_topic() {
    let dir = std::env::temp_dir().join(format!("timem_work_instruction_turn_{}", epoch_millis()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("AGENTS.md"),
        "Run the focused tests before final answer.\n",
    )
    .unwrap();
    let mut ui = WorkInstructionDecisionUi {
        accept: true,
        topics: Vec::new(),
        requests: 0,
    };

    let context = resolve_work_instruction_context_for_turn(
        WorkInstructionLoadMode::Ask,
        &dir,
        "session_test",
        &mut ui,
    )
    .unwrap();

    assert_eq!(ui.requests, 1);
    assert_eq!(
        ui.topics,
        vec![
            "core.work_instruction_load".to_string(),
            "core.work_instruction_load".to_string()
        ]
    );
    assert!(context.contains("Run the focused tests before final answer."));
    assert!(context.contains("AGENTS.md"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn work_instruction_context_for_turn_respects_decline_silent_and_off_modes() {
    let dir = std::env::temp_dir().join(format!("timem_work_instruction_modes_{}", epoch_millis()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("CLAUDE.md"), "Prefer small commits.\n").unwrap();

    let mut decline_ui = WorkInstructionDecisionUi {
        accept: false,
        topics: Vec::new(),
        requests: 0,
    };
    let declined = resolve_work_instruction_context_for_turn(
        WorkInstructionLoadMode::Ask,
        &dir,
        "session_test",
        &mut decline_ui,
    );
    assert!(declined.is_none());
    assert_eq!(decline_ui.requests, 1);
    assert_eq!(
        decline_ui.topics,
        vec!["core.work_instruction_load".to_string()]
    );

    let mut silent_ui = WorkInstructionDecisionUi {
        accept: false,
        topics: Vec::new(),
        requests: 0,
    };
    let silent = resolve_work_instruction_context_for_turn(
        WorkInstructionLoadMode::Silent,
        &dir,
        "session_test",
        &mut silent_ui,
    )
    .unwrap();
    assert!(silent.contains("Prefer small commits."));
    assert_eq!(
        silent_ui.topics,
        vec!["core.work_instruction_load".to_string()]
    );
    assert_eq!(silent_ui.requests, 0);

    let mut off_ui = WorkInstructionDecisionUi {
        accept: true,
        topics: Vec::new(),
        requests: 0,
    };
    let off = resolve_work_instruction_context_for_turn(
        WorkInstructionLoadMode::Off,
        &dir,
        "session_test",
        &mut off_ui,
    );
    assert!(off.is_none());
    assert!(off_ui.topics.is_empty());
    assert_eq!(off_ui.requests, 0);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn round_limit_prompt_uses_keyboard_choices_and_defaults_to_continue() {
    let prompt = render_round_limit_prompt(RoundLimitDecisionRequest::new(50));
    assert!(prompt.contains("本轮已达到最大交互次数 50"));
    assert!(prompt.contains("重新充值 rounds_remaining 为 50"));
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
    assert_eq!(
        read_approval_key(&mut Cursor::new(vec![3])),
        ApprovalKey::Cancel
    );
    assert_eq!(
        read_approval_key(&mut Cursor::new(vec![27])),
        ApprovalKey::Cancel
    );
    assert_eq!(
        read_approval_key_until(&mut Cursor::new(Vec::<u8>::new()), Instant::now()),
        ApprovalKey::Cancel
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
    let input =
        "把\x1b[200~20260623-211820.mp4\x1b[201~ 的分辨率降低一些\r\r\x1b[A\x1b[B\x1b[C\x1b[D\x03";
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
fn queued_paste_fallback_handles_crlf_boundary_without_extra_blank_line() {
    let queued = queued_input_drain_from_bytes(b"\nsecond line\r\nthird line\r\n");
    let merged = merge_queued_input("first line".to_string(), &queued);

    assert_eq!(queued.text, "\nsecond line\nthird line\n");
    assert_eq!(merged, "first line\nsecond line\nthird line\n");
}

#[test]
fn queued_paste_fallback_preserves_user_blank_line_after_crlf_boundary() {
    let queued = queued_input_drain_from_bytes(b"\n\r\nsecond line\r\n");
    let merged = merge_queued_input("first line".to_string(), &queued);

    assert_eq!(queued.text, "\n\nsecond line\n");
    assert_eq!(merged, "first line\n\nsecond line\n");
}

#[test]
fn raw_multiline_paste_requires_confirmation_before_model_submit() {
    let queued = queued_input_drain_from_bytes(b"\nsecond line\r\nthird line");
    let raw = merge_queued_input("first line".to_string(), &queued);

    assert!(raw_multiline_paste_needs_confirmation(&queued, &raw));
    assert_eq!(raw_multiline_paste_display(&raw), "[ pasted 3 lines ]");
}

#[test]
fn raw_multiline_paste_submit_prompt_is_keyboard_driven() {
    let prompt = render_raw_multiline_paste_submit_prompt(3);
    assert!(prompt.contains("检测到 3 行粘贴内容"));
    assert!(prompt.contains("避免把粘贴中的换行误当成多次提交"));
    assert_eq!(
        render_raw_multiline_paste_submit_choices(ApprovalChoice::Allow),
        "\x1b[7m[ 提交 ]\x1b[0m   取消"
    );
    assert_eq!(
        render_raw_multiline_paste_submit_choices(ApprovalChoice::Deny),
        "  提交   \x1b[7m[ 取消 ]\x1b[0m"
    );
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
    let queued = queued_input_drain_from_bytes(b"line 2\r\n\x03line 3\r\n");

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

fn timem_edit_mode_for_test(records: SharedPasteRecords) -> (TimemEditMode, SharedPrefillInput) {
    let prefill = Arc::new(Mutex::new(None));
    (TimemEditMode::new(records, prefill.clone()), prefill)
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
    let (mut mode, _) = timem_edit_mode_for_test(records);
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
    let (mut mode, _) = timem_edit_mode_for_test(records.clone());
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
    let (mut mode, _) = timem_edit_mode_for_test(records.clone());
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
    let (mut mode, _) = timem_edit_mode_for_test(records);
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
fn reedline_prefill_is_inserted_before_next_user_event() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let (mut mode, prefill) = timem_edit_mode_for_test(records);
    *prefill.lock().unwrap() = Some("已编辑 [ pasted 3 lin ]".to_string());
    let event = ReedlineRawEvent::try_from(Event::Paste("x".to_string())).unwrap();

    assert_eq!(
        mode.parse_event(event),
        ReedlineEvent::Edit(vec![
            EditCommand::InsertString("已编辑 [ pasted 3 lin ]".to_string()),
            EditCommand::InsertString("x".to_string())
        ])
    );
    assert!(prefill.lock().unwrap().is_none());
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
fn paste_marker_matching_ignores_stale_preserved_records_when_placeholder_matches_later_record() {
    let records = vec![
        PasteRecord {
            placeholder: "[ pasted 2 lines ]".to_string(),
            content: "old-a\nold-b".to_string(),
        },
        PasteRecord {
            placeholder: "[ pasted 3 lines ]".to_string(),
            content: "new-a\nnew-b\nnew-c".to_string(),
        },
    ];
    let raw = format!(
        "问题 {}",
        format!("{PASTE_START_MARKER}[ pasted 3 lines ]{PASTE_END_MARKER}")
    );

    assert_eq!(paste_recovery_summary_from_markers(&raw, &records), None);
    assert_eq!(
        resolve_paste_markers(&raw, &records, false),
        "问题 new-a\nnew-b\nnew-c"
    );
}

#[test]
fn paste_highlighter_reverses_placeholder_inside_invisible_markers() {
    let raw = format!("请处理 {} 谢谢", marked_placeholder(3));
    let ranges = paste_marker_ranges(&raw);
    assert_eq!(ranges.len(), 1);
    assert_eq!(&raw[ranges[0].0..ranges[0].1], "[ pasted 3 lines ]");

    let highlighted = TimemPasteHighlighter.highlight(&raw, raw.len());
    assert!(
        highlighted
            .buffer
            .iter()
            .any(|(style, text)| style.is_reverse && text == "[ pasted 3 lines ]"),
        "pasted placeholder should render as reverse video: {:?}",
        highlighted
            .buffer
            .iter()
            .map(|(style, text)| (style.is_reverse, text.as_str()))
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
            first_dirty_marker: "[ pasted 3 line ]".to_string(),
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
        first_dirty_marker: "[ pasted 5 lin ]".to_string(),
    };
    let prompt = render_paste_recovery_prompt(&summary);
    assert!(prompt.contains("粘贴关联标签"));
    assert!(prompt.contains("可能被误编辑"));
    assert!(prompt.contains("\x1b[7m[ pasted 5 lin ]\x1b[0m"));
    assert!(prompt.contains("┏━ Note"));
    assert!(prompt.contains('┗'));
    assert!(prompt.contains("继续/恢复粘贴/返回编辑"));
    assert!(prompt.contains("原始粘贴内容共 5 行"));
    assert!(prompt.contains("使用 ←/→ 或 ↑/↓ 选择"));
    assert!(prompt.contains("Ctrl+C/Esc 取消当前输入"));
    let submit_selected = render_paste_recovery_choices(PasteRecoveryChoice::SubmitEdited);
    assert!(submit_selected.contains("\x1b[7m[ 继续 ]\x1b[0m"));
    assert!(submit_selected.contains("恢复粘贴"));
    assert!(submit_selected.contains("返回编辑"));
    let edit_selected = render_paste_recovery_choices(PasteRecoveryChoice::ReturnToEdit);
    assert!(edit_selected.contains("继续"));
    assert!(edit_selected.contains("恢复粘贴"));
    assert!(edit_selected.contains("\x1b[7m[ 返回编辑 ]\x1b[0m"));
}

#[test]
fn note_box_wraps_lines_and_keeps_reverse_video_marker() {
    let rendered = render_note_box_at_width(
        "Note",
        &[
            format!(
                "检测到 1 个粘贴关联标签 {} 可能被误编辑，请确认：",
                "\x1b[7m[ pasted 9 linas;djf ;j ]\x1b[0m"
            ),
            "继续/恢复粘贴/返回编辑".to_string(),
        ],
        64,
    );

    assert!(rendered.contains("\x1b[1m┏━ Note"));
    assert!(rendered.contains("\x1b[7m[ pasted 9 linas;djf ;j ]\x1b[0m"));
    assert!(rendered.contains("继续/恢复粘贴/返回编辑"));
    assert!(rendered.lines().all(|line| display_width(line) == 62));
}

#[test]
fn startup_status_block_groups_core_topics_away_from_help_text() {
    let rendered = render_startup_status_block(&[
        timem_shell::HostStatusMessage::info(
            "Timem Core 启动成功：aliyun:qwen-plus，response protocol=xml，tools=6，skills=0",
        ),
        timem_shell::HostStatusMessage::info("已加载当前工作目录指令：AGENTS.md"),
    ]);

    assert!(rendered.contains("┏━ Startup"));
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("[INFO] Timem Core 启动成功"));
    assert!(plain.contains("[INFO] 已加载当前工作目录指令：AGENTS.md"));
    assert!(!rendered.contains("Info:"));
    assert!(!rendered.contains("输入 /prof"));
    assert!(rendered.lines().all(|line| display_width(line) <= 88));
}

#[test]
fn return_to_edit_clear_lines_include_note_choices_and_original_input_rows() {
    let prompt = render_user_input_prompt("12:00:00");
    let input = format!("阿萨德；。 {} ；大家按阿萨德；", marked_placeholder(11));
    let prompt_width = display_width(&prompt);
    let input_rows = submitted_input_rows(prompt_width, &strip_paste_markers(&input), 32);

    assert!(
        input_rows > 1,
        "test must cover wrapped/nontrivial input rows"
    );
    assert_eq!(
        paste_recovery_return_edit_clear_lines(8, &prompt, &input, 32),
        8 + input_rows
    );
}

#[test]
fn paste_recovery_choice_order_cycles_across_three_actions() {
    assert_eq!(
        next_paste_recovery_choice(PasteRecoveryChoice::SubmitEdited),
        PasteRecoveryChoice::Restore
    );
    assert_eq!(
        next_paste_recovery_choice(PasteRecoveryChoice::Restore),
        PasteRecoveryChoice::ReturnToEdit
    );
    assert_eq!(
        next_paste_recovery_choice(PasteRecoveryChoice::ReturnToEdit),
        PasteRecoveryChoice::SubmitEdited
    );
    assert_eq!(
        prev_paste_recovery_choice(PasteRecoveryChoice::ReturnToEdit),
        PasteRecoveryChoice::Restore
    );
}

#[test]
fn paste_recovery_key_reader_supports_cancel_and_direct_shortcuts() {
    assert_eq!(
        read_paste_recovery_key(&mut Cursor::new(vec![3])),
        PasteRecoveryKey::Cancel
    );
    assert_eq!(
        read_paste_recovery_key(&mut Cursor::new(vec![27])),
        PasteRecoveryKey::Cancel
    );
    assert_eq!(
        read_paste_recovery_key(&mut Cursor::new(vec![27, 27])),
        PasteRecoveryKey::Cancel
    );
    assert_eq!(
        read_paste_recovery_key(&mut Cursor::new(vec![b'y'])),
        PasteRecoveryKey::Select(PasteRecoveryChoice::Restore)
    );
    assert_eq!(
        read_paste_recovery_key(&mut Cursor::new(vec![b'n'])),
        PasteRecoveryKey::Select(PasteRecoveryChoice::SubmitEdited)
    );
    assert_eq!(
        read_paste_recovery_key(&mut Cursor::new(vec![b'e'])),
        PasteRecoveryKey::Select(PasteRecoveryChoice::ReturnToEdit)
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
    assert!(rendered.ends_with("\x1b[94;1m[12:00:00] You ❯❯\x1b[0m abcdef\n"));
}

#[test]
fn user_input_prompt_uses_bright_blue_prefix_and_double_arrow() {
    assert_eq!(
        render_user_input_prompt("12:00:00"),
        "\x1b[94;1m[12:00:00] You ❯❯\x1b[0m "
    );
    assert_eq!(
        display_width(&render_user_input_prompt("12:00:00")),
        display_width("[12:00:00] You ❯❯ ")
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
fn submitted_user_line_rewrite_clears_actual_multiline_input_rows() {
    let rendered =
        render_submitted_user_line_rewrite("first\n第二行\nthird", false, 80, "12:00:00");
    assert!(rendered.starts_with("\x1b[3F\r\x1b[J"), "{rendered:?}");
    assert!(rendered.contains("first\n第二行\nthird\n"));

    let rendered_with_status =
        render_submitted_user_line_rewrite("first\n第二行\nthird", true, 80, "12:00:00");
    assert!(
        rendered_with_status.starts_with("\x1b[4F\r\x1b[J"),
        "{rendered_with_status:?}"
    );
}

#[test]
fn submitted_input_rows_counts_real_newlines_independently_of_wrapping() {
    let prompt_width = display_width("[12:00:00] You ❯❯ ");

    assert_eq!(
        submitted_input_rows(prompt_width, "short\n中文；line\n", 80),
        3
    );
    assert_eq!(
        submitted_input_rows(prompt_width, "abcdef\n第二行", 20),
        wrapped_terminal_rows(prompt_width + display_width("abcdef"), 20)
            + wrapped_terminal_rows(display_width("第二行"), 20)
    );
    assert_eq!(submitted_input_rows(prompt_width, "", 80), 1);
}

#[test]
fn wrapped_terminal_rows_counts_cjk_display_width() {
    assert_eq!(
        wrapped_terminal_rows(display_width("[12:00:00] You ❯❯ 你好"), 20),
        2
    );
}

#[test]
fn rendered_terminal_rows_counts_soft_wrapped_status_lines() {
    let rendered =
            "[23:55:16] 𝓣𝓲𝓶𝓮𝓶  ⬇\n  └─ 网络错误，10s 后重试（第1/5次）\n  └─ 详情：provider_network_error: curl: (16) Error in the HTTP2 framing layer\n";
    let logical_lines = rendered.lines().count();
    let physical_rows = rendered_terminal_rows(rendered, 40);

    assert!(physical_rows > logical_lines);
    assert_eq!(
        physical_rows,
        rendered
            .lines()
            .map(|line| wrapped_terminal_rows(display_width(line), 40))
            .sum::<usize>()
    );
}

#[test]
fn shell_runtime_info_is_host_supplied_and_has_no_cwd() {
    let core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            name: "test".into(),
            provider: "aliyun".into(),
            model: "qwen-plus".into(),
        },
        std::env::temp_dir().join(format!("timem_shell_runtime_info_{}", epoch_millis())),
    );
    let entries = shell_runtime_info_entries(&core);
    let joined = entries.join("\n");

    assert!(joined.contains("ui:"));
    assert!(
        joined.contains("os: macos")
            || joined.contains("os: linux")
            || joined.contains("os: windows")
            || joined.contains("os: unknown")
    );
    assert!(joined.contains("arch: "));
    assert!(joined.contains("os_version: "));
    assert!(joined.contains("run_bash: available"));
    assert!(!joined.contains("cwd:"));
    assert!(!joined.contains("/Users/"));
}

#[test]
fn shell_session_resume_uses_shared_store_and_notice_format() {
    let root = std::env::temp_dir().join(format!("timem_shell_session_resume_{}", epoch_millis()));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let store = SessionStore::new(root.join("memory"));
    let config = ProviderConfig {
        provider: "aliyun".to_string(),
        api_protocol: ApiProtocol::OpenAiCompatible,
        api_key: "secret".to_string(),
        model: "qwen-plus".to_string(),
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
        timeout_secs: 120,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        response_protocol: ResponseProtocolKind::Xml,
    };
    let stored = StoredSession {
        session_id: "web_session_1".to_string(),
        display_name: "Recovered Web".to_string(),
        created_at_ms: 1,
        updated_at_ms: 2,
        current_dir: workspace.display().to_string(),
        profile: shell_session_profile(&config),
        env: shell_session_env_values(
            &config,
            BashApprovalMode::Approve,
            WorkInstructionLoadMode::Silent,
        ),
        state: StoredSessionState::Ready,
        last_turn_id: None,
        raw_chat_history_path: store
            .history_path_for_session("web_session_1")
            .display()
            .to_string(),
    };
    store.upsert_session(&stored).unwrap();

    let loaded = load_or_create_shell_session(
        &store,
        &config,
        BashApprovalMode::Approve,
        WorkInstructionLoadMode::Silent,
        &workspace,
    );
    assert_eq!(loaded.session_id, "web_session_1");
    assert_eq!(loaded.display_name, "Recovered Web");

    let mut pending = true;
    let notice = take_shell_resume_notice(&store, &loaded.session_id, &workspace, &mut pending)
        .expect("first restored shell turn should include resume notice");
    assert!(notice.contains("This session was restored"));
    assert!(notice.contains("raw_chat_history.jsonl"));
    assert!(notice.contains("format: JSONL, one record per line."));
    assert!(!pending);
    assert!(
        take_shell_resume_notice(&store, &loaded.session_id, &workspace, &mut pending).is_none()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shell_can_resume_web_style_session_history() {
    let root = std::env::temp_dir().join(format!("timem_shell_cross_host_{}", epoch_millis()));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let store = SessionStore::new(root.join("memory"));
    let session_id = "web_session_handoff";
    let config = ProviderConfig {
        provider: "aliyun".to_string(),
        api_protocol: ApiProtocol::OpenAiCompatible,
        api_key: "secret".to_string(),
        model: "qwen-plus".to_string(),
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
        timeout_secs: 120,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        response_protocol: ResponseProtocolKind::Xml,
    };
    store
        .upsert_session(&StoredSession {
            session_id: session_id.to_string(),
            display_name: "Session0".to_string(),
            created_at_ms: 1,
            updated_at_ms: 4,
            current_dir: workspace.display().to_string(),
            profile: shell_session_profile(&config),
            env: shell_session_env_values(
                &config,
                BashApprovalMode::Approve,
                WorkInstructionLoadMode::Silent,
            ),
            state: StoredSessionState::Ready,
            last_turn_id: Some("turn_web_1".to_string()),
            raw_chat_history_path: store
                .history_path_for_session(session_id)
                .display()
                .to_string(),
        })
        .unwrap();
    store
        .append_history_record(
            session_id,
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::User,
                turn_id: "turn_web_1".to_string(),
                created_at_ms: 2,
                content: "web user question".to_string(),
            },
        )
        .unwrap();
    store
        .append_history_record(
            session_id,
            &ChatHistoryRecord::Event {
                role: ChatHistoryRole::System,
                turn_id: "turn_web_1".to_string(),
                created_at_ms: 3,
                kind: ChatHistoryEventKind::ActionResult,
                content: "Action result: run_bash\nok".to_string(),
                extra: BTreeMap::from([(
                    "payload".to_string(),
                    serde_json::json!({"action": "run_bash", "status": "completed"}),
                )]),
            },
        )
        .unwrap();
    store
        .append_history_record(
            session_id,
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::Assistant,
                turn_id: "turn_web_1".to_string(),
                created_at_ms: 4,
                content: "web final answer".to_string(),
            },
        )
        .unwrap();

    let loaded = load_or_create_shell_session(
        &store,
        &config,
        BashApprovalMode::Approve,
        WorkInstructionLoadMode::Silent,
        &workspace,
    );
    assert_eq!(loaded.session_id, session_id);
    assert_eq!(loaded.display_name, "Session0");

    let history = store.read_history_page(session_id, None, 200).unwrap();
    assert_eq!(history.records.len(), 3);
    assert_eq!(history.records[0].turn_id(), "turn_web_1");
    assert!(serde_json::to_string(&history.records[1])
        .unwrap()
        .contains("\"kind\":\"action_result\""));

    let mut pending = true;
    let notice = take_shell_resume_notice(&store, &loaded.session_id, &workspace, &mut pending)
        .expect("shell should inject a cross-host resume notice");
    assert!(notice.contains("Refer to chat history when necessary:"));
    assert!(notice.contains("record types:"));
    assert!(notice.contains("\"type\":\"message\""));
    assert!(notice.contains("\"type\":\"event\""));
    assert!(notice.contains("Current cwd:"));
    assert!(notice.contains("raw_chat_history.jsonl"));

    fs::remove_dir_all(root).unwrap();
}
