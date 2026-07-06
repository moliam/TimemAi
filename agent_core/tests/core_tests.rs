use agent_core::capability::{CapabilityHostProfile, CapabilityRegistry};
use agent_core::self_tool::SelfToolPaths;
use agent_core::{
    read_audit_doc, AgentCore, BashApprovalMode, CoreProfile, CoreStep, LlmResponse, MemGuard,
    OutputExpansionRequest, OutputExpansionResolution, ProviderConfig, ResponseProtocolKind,
    RoundLimitDecisionRequest, RoundLimitResolution, RuntimeConfigField, TurnFinal, TurnStopDetail,
    TurnStopReason, UsageStats,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_dir(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.push(format!(
        "timem_agent_core_test_{}_{}_{}_{}",
        name,
        std::process::id(),
        now_ms(),
        seq
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn release_quality_skill_overlay(name: &str) -> PathBuf {
    let dir = tmp_dir(name);
    let skill_dir = dir.join("skills").join("release_quality_gate");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("skill.yaml"),
        r#"kind: skill
id: release_quality_gate
title: Release quality gate
summary: Verify tests, CI, release notes, sensitive information, and version state before publishing a release.
entry: instructions.md
when_to_use: |
  Use when preparing, auditing, or deciding whether to publish a Timem release.
"#,
    )
    .unwrap();
    fs::write(
        skill_dir.join("instructions.md"),
        "# Release Quality Gate\n\nRun the relevant local tests.\n",
    )
    .unwrap();
    dir
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn profile(provider: &str, model: &str) -> CoreProfile {
    CoreProfile {
        name: provider.to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
    }
}

fn usage() -> UsageStats {
    UsageStats {
        llm_calls: 1,
        prompt_tokens: 10,
        completion_tokens: 2,
        total_tokens: 12,
        cached_tokens: 4,
        ..UsageStats::zero()
    }
}

fn audit_doc(events: Vec<Value>) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({"version": 1, "events": events})).unwrap()
    )
}

fn write_audit_doc(path: &std::path::Path, events: Vec<Value>) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, audit_doc(events)).unwrap();
}

fn core_with_builtin_capabilities(name: &str) -> AgentCore {
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), tmp_dir(name));
    core.set_capability_registry(CapabilityRegistry::builtin());
    core
}

fn usage_with_prompt_tokens(prompt_tokens: u32) -> UsageStats {
    UsageStats {
        prompt_tokens,
        total_tokens: prompt_tokens + 2,
        ..usage()
    }
}

fn scored(content: impl Into<String>) -> String {
    content.into()
}

fn first_field_value(prompt: &str, field: &str) -> String {
    let prefix = format!("{field}: ");
    prompt
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or("")
        .to_string()
}

fn action_result_field(prompt: &str, field: &str) -> String {
    first_field_value(prompt, field)
}

fn field_values(prompt: &str, field: &str) -> Vec<String> {
    let prefix = format!("{field}: ");
    prompt
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .map(ToString::to_string)
        .collect()
}

#[test]
fn prompt_is_append_only_and_segmented() {
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), tmp_dir("append"));
    let first = match core.begin_turn("你好", Some("runtime_time: now")) {
        CoreStep::NeedModel {
            prompt,
            rounds_remaining,
        } => {
            assert_eq!(rounds_remaining, 50);
            prompt
        }
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(first.contains("[BEGIN SYSTEM PROMPT]"));
    assert!(!first.contains("________"));
    assert!(first.contains("[END SYSTEM PROMPT]\n[BEGIN DELTA]"));
    assert!(first.contains("delta_id: pd_"));
    assert!(first.contains("## USER"));
    assert!(!first.contains("slice_id: ps_"));
    assert!(!first.contains("prompt_type: user_question"));
    assert!(first.contains("\ntime: "));
    assert!(!first.contains("{\"segment_type\""));

    let second = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"你好"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(_) => core.render_prompt(),
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(second.contains("[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]"));
    assert!(second.contains("User question:\n你好"));
    assert!(second.contains("## TIMEM_ASSISTANT"));
    assert!(second.contains("final_answer:\n你好"));
}

#[test]
fn default_max_rounds_is_fifty() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("default_rounds"),
    );
    let step = core.begin_turn("你好", None);
    let CoreStep::NeedModel {
        prompt,
        rounds_remaining,
    } = step
    else {
        panic!("unexpected step: {step:?}");
    };
    assert_eq!(rounds_remaining, 50);
    assert!(prompt.contains("rounds_remaining: 50"));
}

#[test]
fn round_limit_can_be_continued_without_model_visible_task_reset() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("round_limit_continue"),
    );
    core.set_max_rounds(1);
    let _ = core.begin_turn("需要两步完成", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Need evidence.","args":{"type":"durable","op":"query","query":"x","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let CoreStep::RoundLimitReached { max_rounds } = step else {
        panic!("unexpected step: {step:?}");
    };
    assert_eq!(max_rounds, 1);
    let limited_prompt = core.render_prompt();
    assert!(limited_prompt.contains("Action result: memmgr"));

    let audit_file = tmp_dir("round_limit_continue_audit").join("audit.json");
    let resolution = core.resolve_round_limit_with_audit(
        RoundLimitDecisionRequest::new(max_rounds),
        true,
        None,
        &audit_file,
        "session_1",
        "turn_1",
    );
    let RoundLimitResolution::Continue(step) = resolution else {
        panic!("unexpected round limit resolution");
    };
    let CoreStep::NeedModel {
        prompt,
        rounds_remaining,
    } = step
    else {
        panic!("unexpected step: {step:?}");
    };
    let audit = read_audit_doc(&audit_file).unwrap();
    let events = audit["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "round_limit");
    assert_eq!(events[0]["session"], "session_1");
    assert_eq!(events[0]["turn_id"], "turn_1");
    assert_eq!(events[0]["max_rounds"], 1);
    assert_eq!(events[0]["continued"], true);
    assert_eq!(rounds_remaining, 50);
    assert!(prompt.contains("User question:\n需要两步完成"));
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("Runtime round budget continued by user."));
    assert!(prompt.contains("rounds_remaining: 50"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Need evidence after continuation.","args":{"type":"durable","op":"query","query":"x","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let CoreStep::NeedModel {
        prompt,
        rounds_remaining,
    } = step
    else {
        panic!("unexpected step: {step:?}");
    };
    assert_eq!(rounds_remaining, 49);
    assert!(prompt.contains("Action result: memmgr"));
}

#[test]
fn round_limit_stop_resolution_is_core_owned() {
    let dir = tmp_dir("round_limit_stop_resolution");
    let audit_file = dir.join("audit.json");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    core.set_max_rounds(1);
    let _ = core.begin_turn("需要两步完成", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Need evidence.","args":{"type":"durable","op":"query","query":"x","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let CoreStep::RoundLimitReached { max_rounds } = step else {
        panic!("unexpected step: {step:?}");
    };
    let latest = UsageStats {
        prompt_tokens: 12,
        completion_tokens: 3,
        ..UsageStats::zero()
    };

    let resolution = core.resolve_round_limit_with_audit(
        RoundLimitDecisionRequest::new(max_rounds),
        false,
        Some(latest.clone()),
        &audit_file,
        "session_1",
        "turn_1",
    );
    let RoundLimitResolution::Stop(stop) = resolution else {
        panic!("unexpected round limit resolution");
    };
    assert_eq!(
        stop.stop_reason,
        agent_core::TurnStopReason::RoundLimitReached
    );
    assert_eq!(stop.latest_usage, Some(latest));
    assert_eq!(stop.repair_issue.as_deref(), Some("round_limit_reached"));
    assert!(stop.stats.llm_calls > 0);
    let audit = read_audit_doc(&audit_file).unwrap();
    let events = audit["events"].as_array().unwrap();
    assert_eq!(events[0]["type"], "round_limit");
    assert_eq!(events[0]["continued"], false);
}

#[test]
fn output_expansion_resolution_is_core_owned() {
    let dir = tmp_dir("output_expansion_resolution");
    let audit_file = dir.join("audit.json");
    let core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let mut config = ProviderConfig {
        provider: "aliyun".to_string(),
        model: "qwen-plus".to_string(),
        base_url: "https://example.test/v1".to_string(),
        api_key: "test-key".to_string(),
        timeout_secs: 30,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
        response_protocol: agent_core::ResponseProtocolKind::Markdown,
    };

    let resolution = core.resolve_output_expansion_with_audit(
        &mut config,
        OutputExpansionRequest::new(10_000),
        true,
        UsageStats::zero(),
        &audit_file,
        "session_1",
        "turn_1",
    );
    let OutputExpansionResolution::RetryWithExpandedLimit {
        max_llm_output_tokens,
    } = resolution
    else {
        panic!("unexpected output expansion resolution");
    };
    assert_eq!(max_llm_output_tokens, 20_000);
    assert_eq!(config.max_llm_output_tokens, 20_000);
    let audit = read_audit_doc(&audit_file).unwrap();
    let events = audit["events"].as_array().unwrap();
    assert_eq!(events[0]["type"], "max_llm_output_increased");
    assert_eq!(events[0]["session"], "session_1");
    assert_eq!(events[0]["turn_id"], "turn_1");
    assert_eq!(events[0]["max_llm_output_tokens"], 20_000);
}

#[test]
fn output_expansion_decline_returns_core_stop_summary() {
    let dir = tmp_dir("output_expansion_decline");
    let core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let mut config = ProviderConfig {
        provider: "aliyun".to_string(),
        model: "qwen-plus".to_string(),
        base_url: "https://example.test/v1".to_string(),
        api_key: "test-key".to_string(),
        timeout_secs: 30,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
        response_protocol: agent_core::ResponseProtocolKind::Markdown,
    };
    let usage = UsageStats {
        prompt_tokens: 80,
        completion_tokens: 10_000,
        ..UsageStats::zero()
    };

    let resolution = core.resolve_output_expansion_with_audit(
        &mut config,
        OutputExpansionRequest::new(10_000),
        false,
        usage.clone(),
        &dir.join("audit.json"),
        "session_1",
        "turn_1",
    );
    let OutputExpansionResolution::Stop(stop) = resolution else {
        panic!("unexpected output expansion resolution");
    };
    assert_eq!(
        stop.stop_reason,
        agent_core::TurnStopReason::OutputLimitStoppedByUser
    );
    assert_eq!(stop.latest_usage, Some(usage));
    assert_eq!(
        stop.repair_issue.as_deref(),
        Some("truncated_output_stopped_by_user")
    );
    assert_eq!(config.max_llm_output_tokens, 10_000);
}

#[test]
fn runtime_config_update_is_core_owned_and_updates_runtime_state() {
    let dir = tmp_dir("runtime_config_update_core_owned");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let mut config = ProviderConfig {
        provider: "aliyun".to_string(),
        model: "qwen-plus".to_string(),
        base_url: "https://example.test/v1".to_string(),
        api_key: "test-key".to_string(),
        timeout_secs: 30,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
        response_protocol: agent_core::ResponseProtocolKind::Markdown,
    };
    let mut bash = BashApprovalMode::Ask;
    let mut work = agent_core::WorkInstructionLoadMode::Silent;

    let report = core
        .apply_runtime_config_update(
            &mut config,
            &mut bash,
            &mut work,
            RuntimeConfigField::MaxInput,
            "3K",
        )
        .unwrap();

    assert_eq!(report.key, "TIMEM_MAX_LLM_INPUT");
    assert_eq!(report.value, "3000");
    assert_eq!(config.max_llm_input_tokens, 3_000);

    let _ = core.begin_turn("seed", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"seeded"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(2_700),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));

    let prompt = match core.begin_turn("next", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("max_llm_input_tokens=3000"));
    assert!(prompt.contains("force_shrink_threshold_tokens=2700"));

    let report = core
        .apply_runtime_config_update(
            &mut config,
            &mut bash,
            &mut work,
            RuntimeConfigField::BashApproval,
            "approve",
        )
        .unwrap();
    assert_eq!(report.key, "TIMEM_BASH_APPROVAL");
    assert_eq!(report.value, "approve");
    assert_eq!(bash, BashApprovalMode::Approve);

    let report = core
        .apply_runtime_config_update(
            &mut config,
            &mut bash,
            &mut work,
            RuntimeConfigField::WorkInstructions,
            "off",
        )
        .unwrap();
    assert_eq!(report.key, "TIMEM_WORK_INSTRUCTIONS");
    assert_eq!(report.value, "off");
    assert_eq!(work, agent_core::WorkInstructionLoadMode::Off);
}

#[test]
fn runtime_host_configuration_sync_is_core_owned() {
    let dir = tmp_dir("runtime_host_configuration_sync");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let config = ProviderConfig {
        provider: "aliyun".to_string(),
        model: "qwen-plus".to_string(),
        base_url: "https://example.test/v1".to_string(),
        api_key: "test-key".to_string(),
        timeout_secs: 30,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 3_000,
        api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
        response_protocol: agent_core::ResponseProtocolKind::Markdown,
    };

    core.configure_runtime_from_host(&config, BashApprovalMode::Approve);

    let _ = core.begin_turn("seed", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"seeded"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(2_700),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = match core.begin_turn("next", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("max_llm_input_tokens=3000"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Run local command under configured approval policy.","args":{"cmd":"printf configured"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("configured"));
}

#[test]
fn one_prompt_delta_can_render_to_multiple_slices() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("multi_slice_delta"),
    );
    let long_input = "你好".repeat(7000);
    let prompt = match core.begin_turn(&long_input, Some("runtime_time: now")) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };

    assert!(prompt.contains("[BEGIN DELTA]"));
    assert!(prompt.contains("delta_id: pd_"));
    assert!(!prompt.contains("slice_id: ps_"));
    assert!(!prompt.contains("prompt_type: user_question"));
    assert_eq!(prompt.matches("[BEGIN DELTA]").count(), 1);
    assert!(prompt.contains("## USER"));
}

#[test]
fn one_runtime_increment_can_contain_multiple_slices_in_one_delta() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("multi_slice_runtime_delta"),
    );
    let _ = core.begin_turn("需要推理一下", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"free_talk":"先分析","status":"finished","final_answer":"结论"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = core.render_prompt();
    let delta_ids = field_values(&prompt, "delta_id");

    assert_eq!(delta_ids.len(), 2);
    assert_ne!(delta_ids[0], delta_ids[1]);
    assert!(prompt.contains("## TIMEM_ASSISTANT"));
    assert!(prompt.contains("Free_talk:\n先分析"));
    assert!(prompt.contains("final_answer:\n结论"));
}

#[test]
fn user_supplement_appends_to_latest_delta_as_slice() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("user_supplement_slice"),
    );
    let first_prompt = match core.begin_turn("先分析这个问题", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let original_delta = first_field_value(&first_prompt, "delta_id");
    assert!(first_prompt.contains("## USER"));

    let step = core
        .append_user_supplement("补充：优先考虑跨平台实现")
        .expect("non-empty supplement should produce prompt");
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let delta_ids = field_values(&prompt, "delta_id");

    assert_eq!(delta_ids, vec![original_delta]);
    assert!(prompt.contains("## USER"));
    assert!(prompt.contains("User supplement during current turn:"));
    assert!(prompt.contains("补充：优先考虑跨平台实现"));
}

#[test]
fn user_supplements_with_audit_are_core_owned_turn_updates() {
    let dir = tmp_dir("user_supplement_with_audit");
    let audit_file = dir.join("audit/action_audit.json");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("先分析这个问题", None);

    let step = core
        .append_user_supplements_with_audit(
            vec![
                "  ".to_string(),
                "补充：优先考虑跨平台实现".to_string(),
                "补充：保持 UI 无关的数据结构".to_string(),
            ],
            &audit_file,
            "session_1",
            "turn_1",
        )
        .expect("non-empty supplements should produce prompt");
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };

    assert!(prompt.contains("补充：优先考虑跨平台实现"));
    assert!(prompt.contains("补充：保持 UI 无关的数据结构"));
    let audit = read_audit_doc(&audit_file).unwrap();
    let events = audit["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "user_supplement");
    assert_eq!(events[0]["session"], "session_1");
    assert_eq!(events[0]["turn_id"], "turn_1");
    assert_eq!(events[0]["text"], "补充：优先考虑跨平台实现");
    assert_eq!(events[1]["text"], "补充：保持 UI 无关的数据结构");
}

#[test]
fn missing_durable_score_does_not_block_valid_actions() {
    let dir = tmp_dir("durable_ctx_score_not_required");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"user_name","created_at_ms":1,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我的测试代号是什么？", None);

    let step = core.apply_model_response(LlmResponse {
        content: r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"查找已确认的测试代号记忆。","args":{"type":"durable","op":"query","query":"测试代号","limit":5}}]}"#.to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("测试代号是 ALPHA-42"));
    assert!(!prompt.contains("Protocol repair request"));
}

#[test]
fn prompt_rendering_does_not_expose_durable_ctx_score() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("durable_ctx_not_rendered"),
    );
    let prompt = match core.begin_turn("不要记住：纪念日这个词只是测试", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("User question:\n不要记住：纪念日这个词只是测试"));
    assert!(!prompt.contains("durable_ctx_score"));
}

#[test]
fn prompt_shrink_can_remove_whole_delta_by_delta_id() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("shrink_delta_id"),
    );
    let prompt = match core.begin_turn("REMOVE_THIS_DELTA", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let delta_id = first_field_value(&prompt, "delta_id");
    assert!(!delta_id.is_empty());

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Remove stale user question delta.","args":{{"type":"context","op":"shrink","delta_ids":["{}"]}}}}]}}"#,
            delta_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("removed_delta_count: 1"));
    let shrunk_tokens_estimate = first_field_value(&prompt, "shrunk_tokens_estimate")
        .parse::<u32>()
        .unwrap();
    assert!(shrunk_tokens_estimate > 1);
    assert!(!prompt.contains("REMOVE_THIS_DELTA"));

    let final_step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"done"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let CoreStep::Final(final_turn) = final_step else {
        panic!("unexpected step: {final_step:?}");
    };
    assert_eq!(final_turn.stats.shrunk_tokens, shrunk_tokens_estimate);
}

#[test]
fn memmgr_context_shrink_removes_whole_delta_by_delta_id() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("memmgr_context_shrink"),
    );
    let prompt = match core.begin_turn("REMOVE_THIS_MEMMGR_DELTA", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let delta_id = first_field_value(&prompt, "delta_id");
    assert!(!delta_id.is_empty());

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Remove stale user question delta.","args":{{"type":"context","op":"shrink","delta_ids":["{}"]}}}}]}}"#,
            delta_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("type: context"));
    assert!(prompt.contains("op: shrink"));
    assert!(prompt.contains("removed_delta_count: 1"));
    assert!(!prompt.contains("REMOVE_THIS_MEMMGR_DELTA"));
}

#[test]
fn response_context_compact_hides_refs_and_appends_summary_slice() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("response_context_compact"),
    );
    let prompt = match core.begin_turn("OLD_DYNAMIC_CONTEXT_TO_COMPACT", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let delta_id = first_field_value(&prompt, "delta_id");
    assert!(!delta_id.is_empty());

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            "## Progress\n整理旧上下文。\n\n## Context Compact\ndelta_ids: {delta_id}\nsummary:\n旧任务已经完成，只保留 compact 后的测试摘要。"
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };

    assert!(prompt.contains("## SYSTEM"));
    assert!(prompt.contains("旧任务已经完成，只保留 compact 后的测试摘要"));
    assert!(prompt.contains("Action result: context_compact"));
    assert!(prompt.contains("removed_delta_count: 1"));
    assert!(!prompt.contains("OLD_DYNAMIC_CONTEXT_TO_COMPACT"));
}

#[test]
fn prompt_shrink_can_remove_visible_delta_by_delta_id() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("shrink_slice_id"),
    );
    let long_input = format!("SLICE_ONE_ONLY{}", "a".repeat(13_000));
    let prompt = match core.begin_turn(&long_input, None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let delta_id = first_field_value(&prompt, "delta_id");
    assert!(!delta_id.is_empty());
    assert!(prompt.contains("SLICE_ONE_ONLY"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Hide one rendered prompt delta.","args":{{"type":"context","op":"shrink","delta_ids":["{}"]}}}}]}}"#,
            delta_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("removed_delta_count: 1"));
    let shrunk_tokens_estimate = first_field_value(&prompt, "shrunk_tokens_estimate")
        .parse::<u32>()
        .unwrap();
    assert!(shrunk_tokens_estimate >= 3000);
    assert!(!prompt.contains(&format!("[BEGIN DELTA]\ndelta_id: {}", delta_id)));
    assert!(!prompt.contains("SLICE_ONE_ONLY"));

    let final_step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"done"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let CoreStep::Final(final_turn) = final_step else {
        panic!("unexpected step: {final_step:?}");
    };
    assert_eq!(final_turn.stats.shrunk_tokens, shrunk_tokens_estimate);
}

#[test]
fn prompt0_is_static_global_only() {
    let mut core = AgentCore::new(
        "STATIC_GLOBAL",
        profile("aliyun", "qwen-plus"),
        tmp_dir("prompt0_static"),
    );
    let prompt = match core.begin_turn("secret user question", Some("runtime_time: now")) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let prompt0 = prompt.split("[END SYSTEM PROMPT]").next().unwrap_or("");

    assert!(prompt0.contains("STATIC_GLOBAL"));
    assert!(!prompt0.contains("secret user question"));
    assert!(!prompt0.contains("runtime_time: now"));
    assert!(prompt.contains("User question:\nsecret user question"));
    assert!(prompt.contains("Supporting context:\nruntime_time: now"));
}

#[test]
fn dynamic_context_can_be_estimated_and_cleared_without_touching_static_prompt() {
    let mut core = AgentCore::new(
        "STATIC_GLOBAL",
        profile("aliyun", "qwen-plus"),
        tmp_dir("clear_dynamic_context"),
    );
    assert_eq!(core.dynamic_context_estimated_tokens(), 0);
    let _ = core.begin_turn(&"old task context ".repeat(400), None);
    assert!(core.dynamic_context_estimated_tokens() > 1_000);
    assert!(core.render_prompt().contains("old task context"));

    core.clear_dynamic_context();

    assert_eq!(core.dynamic_context_estimated_tokens(), 0);
    let prompt = core.render_prompt();
    assert!(prompt.contains("[BEGIN SYSTEM PROMPT]\nSTATIC_GLOBAL\n[END SYSTEM PROMPT]"));
    assert!(!prompt.contains("old task context"));
    assert!(!prompt.contains("[BEGIN DELTA]"));
}

#[test]
fn stale_context_decision_resolution_is_core_owned() {
    let dir = tmp_dir("stale_context_resolution");
    let audit_file = dir.join("audit.json");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("seed stale context", None);
    assert!(core.dynamic_context_estimated_tokens() > 0);
    let request = agent_core::StaleContextDecisionRequest {
        idle: Duration::from_secs(3 * 60 * 60 + 1),
        dynamic_context_tokens: core.dynamic_context_estimated_tokens(),
        continue_keeps_dynamic_context: true,
        decline_clears_dynamic_context: true,
    };

    assert!(!core.resolve_stale_context_with_audit(request, false, &audit_file, "session_1"));
    assert_eq!(core.dynamic_context_estimated_tokens(), 0);

    let audit = read_audit_doc(&audit_file).unwrap();
    let event = &audit["events"][0];
    assert_eq!(event["type"], "stale_context_choice");
    assert_eq!(event["session"], "session_1");
    assert_eq!(event["continue_old_context"], false);
    assert!(event["dynamic_context_tokens"].as_u64().unwrap() > 0);
}

#[test]
fn stale_context_continue_keeps_dynamic_context() {
    let dir = tmp_dir("stale_context_continue");
    let audit_file = dir.join("audit.json");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("seed stale context", None);
    let before = core.dynamic_context_estimated_tokens();
    assert!(before > 0);
    let request = agent_core::StaleContextDecisionRequest {
        idle: Duration::from_secs(3 * 60 * 60 + 1),
        dynamic_context_tokens: before,
        continue_keeps_dynamic_context: true,
        decline_clears_dynamic_context: true,
    };

    assert!(core.resolve_stale_context_with_audit(request, true, &audit_file, "session_1"));
    assert_eq!(core.dynamic_context_estimated_tokens(), before);
    let audit = read_audit_doc(&audit_file).unwrap();
    assert_eq!(audit["events"][0]["continue_old_context"], true);
}

#[test]
fn long_context_does_not_inject_shrink_review_below_ninety_percent_window() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("shrink_below_force_threshold"),
    );
    core.set_max_llm_input_tokens(3_000);
    let _ = core.begin_turn("seed", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"seeded"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(2_600),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));

    let prompt = match core.begin_turn("below ninety percent", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(!prompt.contains("Long-context maintenance:"));
}

#[test]
fn long_context_uses_observed_provider_prompt_tokens_plus_new_delta_estimate() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("shrink_observed_tokens"),
    );
    core.set_max_llm_input_tokens(3_000);
    let _ = core.begin_turn("seed", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"seeded"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(2_700),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));

    let prompt = match core.begin_turn("next", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Long-context maintenance:"));
    assert!(prompt.contains("mode=force_shrink_required"));
    assert!(prompt.contains("max_llm_input_tokens=3000"));
    assert!(prompt.contains("force_shrink_threshold_tokens=2700"));
}

#[test]
fn long_context_forces_shrink_at_ninety_percent_window_with_compaction_instruction() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("shrink_force"),
    );
    core.set_max_llm_input_tokens(3_000);
    let _ = core.begin_turn("seed", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"seeded"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(2_700),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));

    let prompt = match core.begin_turn("force review", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("mode=force_shrink_required"));
    assert!(prompt.contains("force_shrink_threshold_tokens=2700"));
    assert!(prompt.contains("target_dynamic_context_ratio=10%-20%"));
    assert!(prompt.contains("summarize all dynamic prompt deltas into about 10%-20%"));
    assert!(prompt.contains("task description"));
    assert!(prompt.contains("working environment facts"));
    assert!(prompt.contains("current progress"));
    assert!(prompt.contains("todo/next steps"));
    assert!(prompt.contains("high-level work principles"));
    assert!(prompt.contains("memmgr type=scratch op=write kind=context_offload"));
    assert!(prompt.contains("kind=notes"));
    assert!(prompt.contains("memmgr type=context op=shrink"));
    assert!(!prompt.contains("use scratch_write"));
    assert!(!prompt.contains("use prompt_shrink"));
    assert!(!prompt.contains("shrink_review_threshold_tokens"));
    assert!(!prompt.contains("first_shrink_review_threshold_tokens"));
    assert!(!prompt.contains("next_shrink_review_step_tokens"));
    assert!(!prompt.contains("durable_ctx_score"));
}

#[test]
fn successful_prompt_shrink_invalidates_stale_observed_prompt_tokens() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("shrink_invalidates_observed_tokens"),
    );
    core.set_max_llm_input_tokens(10_000);
    let _ = core.begin_turn(&"old dynamic context ".repeat(1_500), None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"seeded"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(13_253),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));

    let shrink_prompt = match core.begin_turn("compact now", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(shrink_prompt.contains("mode=force_shrink_required"));
    let mut delta_ids = field_values(&shrink_prompt, "delta_id");
    delta_ids.sort();
    delta_ids.dedup();
    assert!(!delta_ids.is_empty());

    let shrink_response = format!(
        r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Remove visible dynamic context after checkpointing.","args":{{"type":"context","op":"shrink","delta_ids":{}}}}}]}}"#,
        serde_json::to_string(&delta_ids).unwrap()
    );
    let step = core.apply_model_response(LlmResponse {
        content: scored(shrink_response),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(13_253),
        truncated: false,
    });
    let next_prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };

    assert!(next_prompt.contains("Action result: memmgr"));
    assert!(next_prompt.contains("type: context"));
    assert!(next_prompt.contains("op: shrink"));
    assert!(!next_prompt.contains("mode=force_shrink_required"));

    let final_step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"压缩已完成，可以继续对话。"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(1_200),
        truncated: false,
    });
    let final_turn = match final_step {
        CoreStep::Final(final_turn) => final_turn,
        other => panic!("unexpected step after shrink follow-up: {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "压缩已完成，可以继续对话。");
}

#[test]
fn forced_shrink_is_not_reissued_when_dynamic_context_cannot_reduce_enough() {
    let mut core = AgentCore::new(
        &"STATIC_PROMPT ".repeat(9_500),
        profile("aliyun", "qwen-plus"),
        tmp_dir("shrink_static_dominant"),
    );
    core.set_max_llm_input_tokens(3_000);

    let prompt = match core.begin_turn("short question", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(!prompt.contains("mode=force_shrink_required"));
}

#[test]
fn memory_candidates_are_persisted() {
    let dir = tmp_dir("memory_write");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我的测试代号是 ALPHA-42", None);
    let final_step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"记住了","memory_candidates":[{"content":"测试代号是 ALPHA-42"}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match final_step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.stats.mem_writes, 1);
    let stored = fs::read_to_string(core.memory_file()).unwrap();
    assert!(stored.contains("测试代号是 ALPHA-42"));
}

#[test]
fn query_memory_action_returns_action_result_delta() {
    let dir = tmp_dir("memory_query");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"测试项目纪念日是 2099-06-12"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("测试项目纪念日是什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"query","query":"测试项目 纪念日","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("2099-06-12"));
}

#[test]
fn memmgr_durable_query_returns_action_result_delta() {
    let dir = tmp_dir("memmgr_durable_query");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"updated_at_ms":1,"version":1,"content":"测试项目纪念日是 2099-06-12"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("测试项目纪念日是什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test durable query","args":{"type":"durable","op":"query","query":"测试项目 纪念日","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("type: durable"));
    assert!(prompt.contains("op: query"));
    assert!(prompt.contains("2099-06-12"));
}

#[test]
fn canonical_tools_accept_json_object_args() {
    let dir = tmp_dir("json_object_tool_args");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"updated_at_ms":1,"version":1,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("用 JSON object args 跑工具", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"测试 JSON object args。","next_actions":[{"action":"memmgr","intent":"查询测试代号记忆。","args":{"type":"durable","op":"query","query":"测试代号","limit":5}},{"action":"run_bash","intent":"验证 shell JSON object args。","args":{"cmd":"printf kv-ok","timeout_ms":5000}},{"action":"self_tool","intent":"查看 Timem 信息。","args":{"type":"about_me","op":"read"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("测试代号是 ALPHA-42"));
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("kv-ok"));
    assert!(prompt.contains("Action result: self_tool"));
    assert!(prompt.contains("TimemAi"));
}

#[test]
fn action_input_field_is_rejected_instead_of_compatibly_executed() {
    let mut core = core_with_builtin_capabilities("old_input_rejected");
    let _ = core.begin_turn("查一下记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"旧协议测试。","next_actions":[{"action":"memmgr","intent":"查询测试代号记忆","input":{"type":"durable","op":"query","query":"测试代号","limit":5}}]}"#, // allow_legacy_input_negative_test
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("## SYSTEM"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("next_actions[0].args_required"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn protocol_examples_cover_normal_and_corner_flows() {
    let dir = tmp_dir("protocol_valid_examples");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"fact_992","created_at_ms":1,"updated_at_ms":1,"version":3,"content":"项目代号旧值"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    core.set_capability_registry(CapabilityRegistry::builtin());
    core.set_bash_approval_mode(BashApprovalMode::Approve);

    let _ = core.begin_turn("路径在哪里", None);
    let final_turn = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"根据测试配置，当前数据存储路径位于 `/tmp/timem_fixture`。"}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("expected direct final answer, got {other:?}"),
    };
    assert!(final_turn.final_answer.contains("/tmp/timem_fixture"));

    let _ = core.begin_turn("查项目代号并统计文件", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"正在检索长期记忆，并统计当前目录的文件总数...","free_talk":{"content":"并行查询记忆和本地文件数量。","keep_in_context":true},"next_actions":[{"action":"memmgr","intent":"查询持久化记忆中与项目代号相关的信息","args":{"type":"durable","op":"query","query":"project codename","limit":5}},{"action":"run_bash","intent":"统计当前工作目录的文件数量以辅助确认上下文","args":{"cmd":"rg --files | wc -l","timeout_ms":5000}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected action results, got {other:?}"),
    };
    assert!(prompt.contains("Free_talk:"));
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("Action result: run_bash"));

    let _ = core.begin_turn("最终确认发布包", None);
    let final_turn = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"项目编译并打包成功，目标文件已生成在 `target/timem_protocol_examples/release.tar.gz` 路径下。"}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final answer, got {other:?}"),
    };
    assert!(final_turn
        .final_answer
        .contains("target/timem_protocol_examples/release.tar.gz"));

    let _ = core.begin_turn("更新冲突后的事实", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"检测到持久化记忆版本冲突，正在尝试带版本号重写...","free_talk":{"content":"使用查询得到的 expected_version=3 更新测试事实。","keep_in_context":false},"next_actions":[{"action":"memmgr","intent":"更新测试事实，指定预期版本号以防止并发冲突","args":{"type":"durable","op":"update","id":"fact_992","expected_version":3,"content":"测试项目代号为：Project-Alpha"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected memory update action result, got {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("Project-Alpha"));

    let _ = core.begin_turn("读取受保护路径", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"尝试修改启动变量被系统拦截，正在转换为只读查询...","free_talk":{"content":"启动变量不可运行期修改，改为读取路径。","keep_in_context":false},"next_actions":[{"action":"self_tool","intent":"既然无法直接修改启动环境，先读取当前内存路径以便向用户说明现状","args":{"type":"mem_path","op":"read"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected self_tool action result, got {other:?}"),
    };
    assert!(prompt.contains("Action result: self_tool"));
    assert!(prompt.contains("type: mem_path"));

    let _ = core.begin_turn("上下文收缩", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"为了保证响应效率，正在对历史冗余上下文进行清理和暂存...","free_talk":{"content":"将测试 delta ids 移出活跃上下文。","keep_in_context":true},"next_actions":[{"action":"memmgr","intent":"将过期的 Prompt Delta 离线暂存到临时便签中","args":{"type":"scratch","op":"write","kind":"context_offload","label":"test offload","delta_ids":"pd_001,pd_002"}},{"action":"memmgr","intent":"从当前活跃 Prompt Context 中裁剪掉已暂存的 delta_ids","args":{"type":"context","op":"shrink","delta_ids":"pd_001,pd_002"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected context maintenance action results, got {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("pd_001"));
    assert!(prompt.contains("pd_002"));

    let _ = core.begin_turn("读取错误日志", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"正在调取系统构建日志以分析故障原因...","next_actions":[{"action":"run_bash","intent":"拉取异常日志摘要。","args":{"cmd":"printf ERROR","timeout_ms":8000}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected bash action result, got {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("ERROR"));

    let _ = core.begin_turn("最小动作", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"next_actions":[{"action":"run_bash","intent":"Check status","args":{"cmd":"printf minimal"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected minimal action result, got {other:?}"),
    };
    assert!(prompt.contains("minimal"));
}

#[test]
fn protocol_examples_repair_malformed_and_conflicting_responses() {
    let invalid_cases = [
        (
            "comment_and_trailing_comma",
            r#"{
  "status": "working",
  "report_job_progress": "正在处理...", // 非法注释
  "next_actions": [
    {
      "action": "run_bash",
      "intent": "清理缓存",
      "args":{"cmd":"rm -rf .cache"},
    }
  ]
}"#,
            "invalid_json",
        ),
        (
            "truncated_json",
            r#"{"status":"working","report_job_progress":"正在调取长期记忆...","next_actions":[{"action":"memmgr","intent":"查询"#,
            "invalid_json",
        ),
        (
            "missing_intent_and_args",
            r#"{"status":"working","next_actions":[{"action":"run_bash"}]}"#,
            "next_actions[0].args_required",
        ),
        (
            "invented_tool",
            r#"{"status":"working","report_job_progress":"尝试调用未授权的第三方工具...","next_actions":[{"action":"fetch_web_page","intent":"去外网抓取数据（系统并未提供此工具）","args":{"url":"https://example.com"}}]}"#,
            "unsupported_action:fetch_web_page",
        ),
        (
            "extra_top_level_fields",
            r#"{"status":"finished","final_answer":"处理完毕。","custom_debug_token":"fixture-token","model_confidence_score":0.98}"#,
            "unexpected_top_level_field",
        ),
    ];

    for (name, content, expected_issue) in invalid_cases {
        let mut core = core_with_builtin_capabilities(name);
        let _ = core.begin_turn("协议鲁棒性测试", None);
        let prompt = match core.apply_model_response(LlmResponse {
            content: scored(content),
            model_name: "qwen-plus".to_string(),
            usage: usage(),
            truncated: false,
        }) {
            CoreStep::NeedModel { prompt, .. } => prompt,
            other => panic!("{name}: expected response repair, got {other:?}"),
        };
        assert!(
            prompt.contains("## SYSTEM") && prompt.contains("Protocol repair request"),
            "{name}: missing repair system block"
        );
        assert!(
            prompt.contains(expected_issue),
            "{name}: missing expected issue {expected_issue}\n{prompt}"
        );
        assert!(
            !prompt.contains("Action result: run_bash"),
            "{name}: invalid response must not execute bash"
        );
    }
}

#[test]
fn protocol_examples_repair_finished_with_action_and_reject_string_args() {
    let mut core = core_with_builtin_capabilities("protocol_semantic_edges");
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("既结束又工作", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"这是给用户的最终回答。","next_actions":[{"action":"run_bash","intent":"但我这里还想执行一个后台命令","args":{"cmd":"printf downgraded","background":true}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected repair, got {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("status_finished_must_not_include_next_actions"));
    assert!(!prompt.contains("job_id:"));
    assert!(!prompt.contains("Action result: run_bash"));
    assert!(!prompt.contains("final_answer:\n这是给用户的最终回答。"));

    let _ = core.begin_turn("字符串 args 应被拒绝", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"正在检查内存...","next_actions":[{"action":"memmgr","intent":"查询记忆","args":"type=durable op=query query='project codename' limit=5"}]}"#, // allow_string_args_negative_test
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected string args to request protocol repair, got {other:?}"),
    };
    assert!(prompt.contains("## SYSTEM"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("next_actions[0].args_must_be_object"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn memmgr_raw_chat_query_reads_persisted_chat_records() {
    let root = tmp_dir("memmgr_raw_chat");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    write_audit_doc(
        &root.join("audit").join("api_audit.json"),
        vec![
            json!({"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"我昨天提到了测试物品 BLUE-17"}),
            json!({"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"我记下了测试物品 BLUE-17这个说法。"}),
        ],
    );
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我之前说过什么物品", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test raw chat query","args":{"type":"raw_chat","op":"query","query":"测试物品 BLUE-17","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("type: raw_chat"));
    assert!(prompt.contains("op: query"));
    assert!(prompt.contains("chat_records"));
    assert!(prompt.contains("测试物品 BLUE-17"));
}

#[test]
fn plain_text_after_repair_failure_is_shown_as_final_answer() {
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), tmp_dir("repair"));
    let _ = core.begin_turn("你好", None);
    // Pure prose (no braces) is auto-wrapped as final_answer
    let step = core.apply_model_response(LlmResponse {
        content: "not json".to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "not json");
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn status_finished_uses_final_answer_as_host_final_answer() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("status_finished_final_answer"),
    );
    let _ = core.begin_turn("总结", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"这是最终结论。"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final, got {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "这是最终结论。");
}

#[test]
fn final_turn_wire_shape_uses_semantic_final_answer_field() {
    let step = CoreStep::Final(TurnFinal {
        final_answer: "这是最终结论。".to_string(),
        stats: UsageStats::zero(),
        profile_label: "aliyun:qwen-plus".to_string(),
        repair_issue: None,
        stop_summary: None,
    });

    let payload = serde_json::to_value(&step).unwrap();
    assert_eq!(payload["Final"]["final_answer"], "这是最终结论。");
    assert!(
        payload["Final"].get("response_to_user").is_none()
            && payload["Final"].get("text").is_none()
    );
}

#[test]
fn fields_wrapped_finished_answer_is_accepted_without_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("fields_wrapped_finished_answer"),
    );
    let _ = core.begin_turn("hello", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"fields":{"status":"finished","final_answer":"你好。"}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final without repair, got {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "你好。");
    assert_eq!(core.current_stats().repair_calls, 0);
    assert!(!core.render_prompt().contains("Protocol repair request"));
}

#[test]
fn final_answer_without_finished_status_requests_protocol_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("final_answer_without_status"),
    );
    core.set_response_protocol(ResponseProtocolKind::Markdown);
    let _ = core.begin_turn("总结", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"final_answer":"这是最终结论。"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel repair, got {other:?}"),
    };
    assert!(prompt.contains("## SYSTEM"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("shrink_priority: discard_first"));
    assert!(prompt.contains("issue: final_answer_requires_status_finished"));
    assert!(prompt.contains("没有明确完成状态"));
    assert!(prompt.contains("请写 `## Status` 为 `finished`"));
    assert!(prompt.contains("并写 `## Final_Answer`"));
    assert!(prompt.contains("Previous model response to repair:"));
    assert!(prompt.contains(r#"{"final_answer":"这是最终结论。"}"#));
    assert_eq!(core.current_stats().repair_calls, 1);
}

#[test]
fn finished_status_without_final_answer_requests_protocol_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("finished_without_final_answer"),
    );
    core.set_response_protocol(ResponseProtocolKind::Markdown);
    let _ = core.begin_turn("总结", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","report_job_progress":"完成了"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel repair, got {other:?}"),
    };
    assert!(prompt.contains("## SYSTEM"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("shrink_priority: discard_first"));
    assert!(prompt.contains("issue: final_answer_required_when_status_finished"));
    assert!(prompt.contains("缺少 `## Final_Answer`"));
    assert!(prompt.contains("同时提供 `## Status` 和 `## Final_Answer`"));
    assert!(prompt.contains(r#"{"status":"finished","report_job_progress":"完成了"}"#));
}

#[test]
fn protocol_repair_slice_focuses_previous_response_around_error() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("repair_focus_previous_response"),
    );
    core.set_response_protocol(ResponseProtocolKind::Json);
    let _ = core.begin_turn("总结", None);
    let raw = format!(
        "BEGIN_SHOULD_NOT_APPEAR{}{{\"report_job_progress\":\"BAD_JSON_FOCUS\nTAIL_NEAR_FOCUS\"}}{}END_SHOULD_NOT_APPEAR",
        "x".repeat(8_000),
        "y".repeat(8_000)
    );
    let step = core.apply_model_response(LlmResponse {
        content: raw,
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel repair, got {other:?}"),
    };
    assert!(prompt.contains("## SYSTEM"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("[BEGIN PREVIOUS_LLM_RESPONSE]"));
    assert!(prompt.contains("[FOCUSED previous response: chars"));
    assert!(prompt.contains("BAD_JSON_FOCUS"));
    assert!(prompt.contains("TAIL_NEAR_FOCUS"));
    assert!(!prompt.contains("BEGIN_SHOULD_NOT_APPEAR"));
    assert!(!prompt.contains("END_SHOULD_NOT_APPEAR"));
    assert!(prompt.contains("[END PREVIOUS_LLM_RESPONSE]"));
}

#[test]
fn status_working_requires_next_actions_and_keeps_progress_separate() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("status_working_progress"),
    );
    let _ = core.begin_turn("查一下", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"working","report_job_progress":"正在查询。","next_actions":[{"action":"memmgr","intent":"Find evidence.","args":{"type":"durable","op":"query","query":"x","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert!(!prompt.contains("prompt_type: llm_progress"));
    assert!(!prompt.contains("report_job_progress:\n正在查询。"));
    assert!(!prompt.contains("正在查询。"));
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("intent: Find evidence."));
}

#[test]
fn omitted_status_bare_action_defaults_to_working_next_action() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bare_action_defaults_working"),
    );
    let _ = core.begin_turn("继续修复", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"action":"run_bash","intent":"Check repository status.","args":{"cmd":"git status --short","timeout_ms":1000}}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("expected NeedsUserApproval, got {other:?}"),
    };
    assert_eq!(request.action, "run_bash");
    assert_eq!(request.command, "git status --short");
}

#[test]
fn final_answer_with_runtime_progress_marker_requests_protocol_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("final_progress_marker_repair"),
    );
    let _ = core.begin_turn("继续汇报", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"◉ 分析完成，汇报结果..."}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected protocol repair, got {other:?}"),
    };
    assert!(prompt.contains("## SYSTEM"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("final_answer_must_not_start_with_runtime_progress_marker"));
}

#[test]
fn malformed_action_like_response_still_gets_protocol_error_after_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("repair_action_like"),
    );
    core.set_response_protocol(ResponseProtocolKind::Json);
    let _ = core.begin_turn("你好", None);
    // Contains braces but invalid JSON -> triggers repair
    let step = core.apply_model_response(LlmResponse {
        content: "{not valid json}".to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::NeedModel { .. }));

    let step = core.apply_model_response(LlmResponse {
        content: r#"next_actions: [{"action":"run_bash","args":{"cmd":"git commit"}}]"#.to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "");
    assert_eq!(final_turn.repair_issue.as_deref(), Some("invalid_json"));
    let stop = final_turn
        .stop_summary
        .as_ref()
        .expect("protocol repair failure should be structured stop data");
    assert_eq!(stop.stop_reason, TurnStopReason::ProtocolRepairFailed);
    assert_eq!(
        stop.detail,
        TurnStopDetail::ProtocolRepairFailure {
            first_issue: "invalid_json".to_string(),
            final_issue: "invalid_json".to_string(),
            truncated: false,
        }
    );
}

#[test]
fn truncated_response_requests_output_limit_repair_in_noninteractive_path() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("truncated_repair"),
    );
    core.set_response_protocol(ResponseProtocolKind::Markdown);
    let _ = core.begin_turn("写一个很长的报告", None);
    let step = core.apply_model_response(LlmResponse {
        content: "{\"report_job_progress\":\"partial".to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: true,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("## SYSTEM"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("truncated_model_output"));
    assert!(prompt.contains("max output token"));
    assert!(prompt.contains("Markdown response protocol"));
    assert!(prompt.contains(r#"{"report_job_progress":"partial"#));
}

#[test]
fn model_repair_audit_is_core_owned_when_applying_response() {
    let dir = tmp_dir("model_repair_audit_core_owned");
    let audit_file = dir.join("audit").join("api_audit.json");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("写一个很长的报告", None);

    let step = core.apply_model_response_with_repair_audit(
        LlmResponse {
            content: "{\"report_job_progress\":\"partial".to_string(),
            model_name: "qwen-plus".to_string(),
            usage: usage(),
            truncated: true,
        },
        &audit_file,
        "session_1",
        "turn_1",
    );

    assert!(matches!(step, CoreStep::NeedModel { .. }));
    let audit = read_audit_doc(&audit_file).unwrap();
    let events = audit["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "model_repair_request");
    assert_eq!(events[0]["session"], "session_1");
    assert_eq!(events[0]["turn_id"], "turn_1");
    assert_eq!(events[0]["issue"], "truncated_model_output");
    assert_eq!(events[0]["model"], "qwen-plus");
    assert_eq!(events[0]["truncated"], true);
    assert_eq!(events[0]["repair_calls"], 1);
    assert_eq!(events[0]["repair_calls_delta"], 1);
    assert_eq!(events[0]["usage"]["prompt_tokens"], 10);
}

#[test]
fn turn_lifecycle_audit_is_core_owned() {
    let dir = tmp_dir("turn_lifecycle_audit_core_owned");
    let audit_file = dir.join("audit").join("api_audit.json");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let outcome = agent_core::TurnOutcome::final_response(
        "done",
        usage(),
        Some(usage_with_prompt_tokens(25)),
        Some("invalid_json".to_string()),
        Duration::from_millis(1234),
    );

    core.record_turn_start_audit(&audit_file, "session_1", "turn_1", "hello");
    core.record_turn_error_audit(&audit_file, "session_1", "turn_1", "provider_network_error");
    core.record_turn_final_audit(&audit_file, "session_1", "turn_1", &outcome);

    let audit = read_audit_doc(&audit_file).unwrap();
    let events = audit["events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["type"], "turn_start");
    assert_eq!(events[0]["user_input"], "hello");
    assert_eq!(events[1]["type"], "turn_error");
    assert_eq!(events[1]["error"], "provider_network_error");
    assert_eq!(events[2]["type"], "turn_final");
    assert_eq!(events[2]["assistant_output"], "done");
    assert_eq!(events[2]["repair_issue"], "invalid_json");
    assert_eq!(events[2]["stop_summary"], Value::Null);
    assert_eq!(events[2]["elapsed_ms"], 1234);
    assert_eq!(events[2]["latest_usage"]["prompt_tokens"], 25);
}

#[test]
fn truncated_repair_failure_explains_provider_max_token_reason() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("truncated_repair_failure"),
    );
    let _ = core.begin_turn("写一个很长的报告", None);
    let step = core.apply_model_response(LlmResponse {
        content: "{\"report_job_progress\":\"partial".to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: true,
    });
    assert!(matches!(step, CoreStep::NeedModel { .. }));

    let step = core.apply_model_response(LlmResponse {
        content: "{\"report_job_progress\":\"still partial".to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: true,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "");
    assert_eq!(final_turn.repair_issue.as_deref(), Some("invalid_json"));
    let stop = final_turn
        .stop_summary
        .as_ref()
        .expect("truncated repair failure should be structured stop data");
    assert_eq!(stop.stop_reason, TurnStopReason::ProtocolRepairFailed);
    assert_eq!(
        stop.detail,
        TurnStopDetail::ProtocolRepairFailure {
            first_issue: "truncated_model_output".to_string(),
            final_issue: "invalid_json".to_string(),
            truncated: true,
        }
    );
}

#[test]
fn mixed_protocol_transcript_extracts_final_json_without_leaking_raw_segments() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("mixed_protocol_transcript"),
    );
    let _ = core.begin_turn("展示一个耗尽 8 步交互的操作", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"free_talk":"Round 7","report_job_progress":"","next_actions":[{"action":"run_bash","intent":"old action","args":{"cmd":"uptime"}}]}

[BEGIN DELTA]
delta_id: pd_18

## ACTIONS
Action result: run_bash
command: uptime
status: 0
output:
ok
[END DELTA]

{
  "free_talk": "Final summary",
  "status": "finished",
  "final_answer": "只展示最终摘要。"
}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "只展示最终摘要。");
    assert!(!final_turn.final_answer.contains("[BEGIN SEGMENT"));
    assert!(!final_turn.final_answer.contains("next_actions"));
}

#[test]
fn prose_then_markdown_fenced_json_extracts_payload() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("prose_then_fenced_json"),
    );
    core.set_response_protocol(ResponseProtocolKind::Json);
    let _ = core.begin_turn("把下载目录视频做 3 倍加速", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"转码已在后台顺利运行，进度正常。

```json
{
  "status": "finished",
  "final_answer": "转码已在后台顺利运行，输出文件：`~/Videos/example_3x.mp4`。"
}
```"#,
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.final_answer.contains("example_3x.mp4"));
    assert!(!final_turn.final_answer.contains("```json"));
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn response_text_with_unescaped_inner_quotes_is_repaired() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("unescaped_response_quotes"),
    );
    let _ = core.begin_turn(
        "当前目录的代码量，rust 代码有多少行？  ---> 这个是几点和你聊的？",
        None,
    );
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{
  "free_talk": "The answer is available from chat history.",
  "status": "finished",
  "final_answer": "根据聊天记录，你问"当前目录的代码量，rust 代码有多少行？"这个问题的时间是今天（2026-06-23）17:46:36 左右。"
}"#),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.final_answer.contains("17:46:36"));
    assert!(final_turn.final_answer.contains("\"当前目录的代码量"));
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn response_text_preserves_valid_complex_symbols_and_quotes() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("valid_complex_symbols"),
    );
    let _ = core.begin_turn("展示各种符号", None);
    let text = r#"中文“引号”、English 'single quotes'、escaped \"double quotes\"、`code`、```fence```、JSON-ish {a:1} [x] (y)、路径 C:\\tmp\\file、URL https://a.example?q=1&x="y"、箭头 -> => --->、emoji 🤖、换行
第二行。"#;
    let payload = serde_json::json!({
        "free_talk": "Symbols should remain normal text.",
        "status": "finished",
        "final_answer": text
    });
    let step = core.apply_model_response(LlmResponse {
        content: payload.to_string(),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.final_answer, text);
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn response_text_decodes_common_json_escape_sequences() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("json_escape_response"),
    );
    let _ = core.begin_turn("展示 escape 符号", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"tab:\tend\nline2\r\nunicode:\u4f60\u597d path:C:\\Users\\me\\file quote:\"ok\" slash:\/ regex:\\d+"}"#),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(
        final_turn.final_answer,
        "tab:\tend\nline2\r\nunicode:你好 path:C:\\Users\\me\\file quote:\"ok\" slash:/ regex:\\d+"
    );
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn action_input_decodes_common_json_escape_sequences() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("json_escape_action_input"),
    );
    let _ = core.begin_turn("记住一段 escape 文本", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Store escaped text exactly after JSON decoding.","args":{"type":"durable","op":"insert","content":"tab:\tend\nline2\r\nunicode:\u4f60\u597d path:C:\\Users\\me\\file quote:\"ok\" slash:\/ regex:\\d+"}}]}"#),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("stored: tab:\tend\nline2\r\nunicode:你好"));
    assert!(prompt.contains("path:C:\\Users\\me\\file"));
    assert!(prompt.contains("quote:\"ok\" slash:/ regex:\\d+"));
}

#[test]
fn action_fields_with_unescaped_inner_quotes_are_repaired() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("unescaped_action_quotes"),
    );
    let _ = core.begin_turn("查刚才那句话", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{
  "report_job_progress": "",
  "next_actions": [
    {
      "action": "memmgr",
      "intent": "查找用户说过的"当前目录"相关问题",
      "args":{"type":"raw_chat","op":"query","query":"当前目录的代码量，\"rust\" 代码有多少行？","limit":5}
    }
  ]
}"#,
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("当前目录"));
}

#[test]
fn malformed_complex_protocol_is_blocked_without_raw_leak() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("malformed_complex_protocol"),
    );
    core.set_response_protocol(ResponseProtocolKind::Json);
    let _ = core.begin_turn("展示各种奇怪符号", None);
    let step = core.apply_model_response(LlmResponse {
        content: "```json\n{\"report_job_progress\":\"bad dangling \\ path and raw \n newline"
            .to_string(),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));

    let step = core.apply_model_response(LlmResponse {
        content: "still ``` not { valid \\ json".to_string(),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "");
    assert!(!final_turn.final_answer.contains("dangling"));
    assert!(!final_turn.final_answer.contains("```"));
    let stop = final_turn
        .stop_summary
        .as_ref()
        .expect("malformed protocol failure should be structured stop data");
    assert_eq!(stop.stop_reason, TurnStopReason::ProtocolRepairFailed);
    assert!(matches!(
        stop.detail,
        TurnStopDetail::ProtocolRepairFailure { .. }
    ));
}

#[test]
fn profile_label_keeps_provider_and_model_distinct() {
    let qwen_openai = profile("openai", "qwen-plus");
    let qwen_aliyun = profile("aliyun", "qwen-plus");
    assert_ne!(qwen_openai.label(), qwen_aliyun.label());
    assert!(qwen_aliyun.label().contains("aliyun:qwen-plus"));
}

#[test]
fn invalid_action_shape_requests_protocol_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bad_action"),
    );
    let _ = core.begin_turn("查记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"query"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(
        prompt.contains("issue: next_actions[0].input.query_required_when_op=query,type=durable")
    );
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn progress_and_next_actions_continue_with_implicit_continue_note() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("progress_action_continue"),
    );
    let _ = core.begin_turn("请一直完成任务，不要停止", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"备份完成。现在继续查证。","next_actions":[{"action":"memmgr","intent":"查找相关记忆。","args":{"type":"durable","op":"query","query":"项目状态","limit":1}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(!prompt.contains("prompt_type: llm_progress"));
    assert!(!prompt.contains("备份完成。现在继续查证。"));
    assert!(!prompt.contains("上轮回复没有写 status"));
    assert!(prompt.contains("Action result: memmgr"));
}

#[test]
fn next_action_requires_intent_for_user_visible_action_status() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("missing_intent"),
    );
    let _ = core.begin_turn("查记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","args":{"type":"durable","op":"query","query":"测试代号","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("intent_required"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn unsupported_action_is_not_executed_silently() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("unsupported_action"),
    );
    let _ = core.begin_turn("打开文件", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"delete_file","intent":"test action","args":{"path":"/tmp/x"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("unsupported_action:delete_file"));
}

#[test]
fn scratch_notes_can_be_written_queried_and_deleted() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scratch_notes"),
    );
    let _ = core.begin_turn("先把这个长期任务记到草稿区", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Create a task checkpoint.","args":{"type":"scratch","op":"write","kind":"notes","label":"release checkpoint","content":"continue this task later"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("label: release checkpoint"));
    assert!(prompt.contains("type: notes"));
    assert!(prompt.contains("content_preview: continue this task later"));
    let stored = fs::read_to_string(core.scratch_file()).unwrap();
    let scratch_id = stored
        .lines()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|value| {
            value
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .expect("scratch id should exist");

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Read saved checkpoint by id.","args":{{"type":"scratch","op":"read","id":"{}"}}}}]}}"#, scratch_id)),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("found: true"));
    assert!(prompt.contains("continue this task later"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Remove completed checkpoint.","args":{{"type":"scratch","op":"delete","id":"{}"}}}}]}}"#, scratch_id)),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("deleted: true"));
    assert!(!fs::read_to_string(core.scratch_file())
        .unwrap()
        .contains("continue this task later"));
}

#[test]
fn memmgr_scratch_write_and_read_notes() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("memmgr_scratch_notes"),
    );
    let _ = core.begin_turn("先把这个长期任务记到草稿区", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Create a task checkpoint.","args":{"type":"scratch","op":"write","kind":"notes","label":"release checkpoint","content":"continue this task later"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("type: scratch"));
    assert!(prompt.contains("op: write"));
    assert!(prompt.contains("label: release checkpoint"));
    assert!(prompt.contains("content_preview: continue this task later"));
    let stored = fs::read_to_string(core.scratch_file()).unwrap();
    let scratch_id = stored
        .lines()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|value| {
            value
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .expect("scratch id should exist");

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Read saved checkpoint by id.","args":{{"type":"scratch","op":"read","id":"{}"}}}}]}}"#, scratch_id)),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("op: read"));
    assert!(prompt.contains("found: true"));
    assert!(prompt.contains("continue this task later"));
}

#[test]
fn memmgr_missing_op_requests_protocol_repair_from_manifest_idl() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("memmgr_missing_op"),
    );
    let _ = core.begin_turn("查一下记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Broken memory action","args":{"type":"durable","query":"测试代号"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.op_required"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn scratch_query_empty_query_lists_recent_notes_with_limit() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scratch_query_recent"),
    );
    fs::write(
        core.scratch_file(),
        r#"{"id":"scratch_old","created_at_ms":1,"scratch_type":"notes","label":"old label","content":"old checkpoint","prompt_delta_ids":[],"prompt_slice_ids":[]}
{"id":"scratch_new","created_at_ms":2,"scratch_type":"notes","label":"new label","content":"new checkpoint","prompt_delta_ids":[],"prompt_slice_ids":[]}
"#,
    )
    .unwrap();

    let _ = core.begin_turn("列出最近一条草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"List recent checkpoints.","args":{"type":"scratch","op":"query","query":"","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("scratch_new"));
    assert!(prompt.contains("label=new label"));
    assert!(prompt.contains("new checkpoint"));
    assert!(!prompt.contains("old checkpoint"));
}

#[test]
fn scratch_actions_request_protocol_repair_for_missing_required_fields() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scratch_protocol_repair"),
    );

    let _ = core.begin_turn("写一条空草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Create empty checkpoint.","args":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.type_required"));
    assert!(!prompt.contains("Action result: memmgr"));
    assert!(!core.scratch_file().exists());

    let _ = core.begin_turn("写一条没有标签的草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Create unlabeled checkpoint.","args":{"type":"scratch","op":"write","kind":"notes","content":"x"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(
        prompt.contains("issue: next_actions[0].input.label_required_when_op=write,type=scratch")
    );
    assert!(!prompt.contains("Action result: memmgr"));

    let _ = core.begin_turn("读取一条没有 id 的草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Read checkpoint.","args":{"type":"scratch","op":"read"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.id_required_when_op=read,type=scratch"));
    assert!(!prompt.contains("Action result: memmgr"));

    let _ = core.begin_turn("删除一条没有 id 的草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Delete checkpoint.","args":{"type":"scratch","op":"delete"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.id_required_when_op=delete,type=scratch"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn scratch_delete_missing_id_is_non_destructive() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scratch_delete_missing"),
    );
    fs::write(
        core.scratch_file(),
        r#"{"id":"scratch_keep","created_at_ms":1,"scratch_type":"notes","label":"keep","content":"keep this checkpoint","prompt_delta_ids":[],"prompt_slice_ids":[]}
"#,
    )
    .unwrap();

    let _ = core.begin_turn("删除不存在的草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Delete missing checkpoint.","args":{"type":"scratch","op":"delete","id":"scratch_missing"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("deleted: false"));
    assert!(fs::read_to_string(core.scratch_file())
        .unwrap()
        .contains("keep this checkpoint"));
}

#[test]
fn scratch_write_context_offload_stores_runtime_prompt_delta_by_id() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scratch_context_offload"),
    );
    let prompt = match core.begin_turn(
        "large investigation context that should move to scratch",
        None,
    ) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let delta_id = first_field_value(&prompt, "delta_id");
    assert!(delta_id.starts_with("pd_"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Offload visible prompt delta by id.","args":{{"type":"scratch","op":"write","kind":"context_offload","label":"large investigation context","delta_ids":["{}"]}}}}]}}"#,
            delta_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("label: large investigation context"));
    assert!(prompt.contains("type: context_offload"));
    assert!(prompt.contains(&format!("prompt_delta_ids: {delta_id}")));
    assert!(prompt.contains("content_preview: [BEGIN SCRATCH OFFLOAD DELTA"));
    let scratch_id = action_result_field(&prompt, "id");
    assert!(scratch_id.starts_with("scratch_"));

    let stored = fs::read_to_string(core.scratch_file()).unwrap();
    assert!(stored.contains("\"scratch_type\":\"context_offload\""));
    assert!(stored.contains("\"label\":\"large investigation context\""));
    assert!(stored.contains("large investigation context that should move to scratch"));
    assert!(stored.contains(&delta_id));

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Read offloaded prompt context.","args":{{"type":"scratch","op":"read","id":"{}"}}}}]}}"#,
            scratch_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("found: true"));
    assert!(prompt.contains("large investigation context that should move to scratch"));
}

#[test]
fn scratch_context_offload_rejects_invalid_prompt_refs_without_writing() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scratch_context_offload_invalid"),
    );
    let _ = core.begin_turn("seed context", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Attempt invalid offload.","args":{"type":"scratch","op":"write","kind":"context_offload","label":"bad refs","delta_ids":["pd_missing"]}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("error: invalid_prompt_refs missing_ids=pd_missing"));
    assert!(!core.scratch_file().exists());
}

#[test]
fn scratch_context_offload_requires_prompt_refs_in_protocol() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scratch_context_offload_refs_required"),
    );
    let _ = core.begin_turn("seed context", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Missing refs.","args":{"type":"scratch","op":"write","kind":"context_offload","label":"missing refs"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.any_required_when_delta_ids"));
}

#[test]
fn memory_write_action_requires_content_or_query() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("empty_write"),
    );
    let _ = core.begin_turn("记住", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"insert"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt
        .contains("issue: next_actions[0].input.content_required_when_op=insert,type=durable"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn query_memory_does_not_expand_semantic_aliases() {
    let dir = tmp_dir("no_semantic_alias");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我的测试代号是什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"query","query":"user's name","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("results: none"));
    assert!(!prompt.contains("测试代号是 ALPHA-42"));
}

#[test]
fn query_memory_exposes_version_for_conflict_safe_updates() {
    let dir = tmp_dir("query_memory_version");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":11,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("查测试代号记忆版本", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Find versioned durable memory before update.","args":{"type":"durable","op":"query","query":"测试代号","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("id=m1"));
    assert!(prompt.contains("version=1"));
    assert!(prompt.contains("created_at_ms=11"));
    assert!(prompt.contains("updated_at_ms=11"));
}

#[test]
fn memory_lookup_context_triggers_runtime_precheck_before_model_reply() {
    let dir = tmp_dir("runtime_memory_precheck");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let prompt = match core.begin_turn(
        "我是谁",
        Some("runtime_time: now\nmemory_lookup_hint: stored personal fact likely needed"),
    ) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("## USER"));
    assert!(prompt.contains("## ACTIONS"));
    assert!(prompt.contains("Action result: runtime_memory_precheck"));
    assert!(prompt.contains("lexical_results: none"));
    assert!(prompt.contains("recent_memory_evidence"));
    assert!(prompt.contains("测试代号是 ALPHA-42"));
}

#[test]
fn memory_lookup_precheck_is_not_added_without_runtime_marker() {
    let dir = tmp_dir("no_runtime_memory_precheck");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let prompt = match core.begin_turn("我是谁", Some("runtime_time: now")) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(!prompt.contains("runtime_memory_precheck"));
}

#[test]
fn sql_read_action_returns_rows() {
    let dir = tmp_dir("sql_read_rows");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":11,"content":"测试代号是 ALPHA-42"}
{"id":"m2","created_at_ms":22,"content":"测试项目纪念日是 2099-06-12"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我最早什么时候告诉你测试代号的", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"SELECT content, created_at_ms FROM memories WHERE content LIKE ? ORDER BY created_at_ms ASC LIMIT 5","params":["%测试代号%"]}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("content=测试代号是 ALPHA-42"));
    assert!(prompt.contains("created_at_ms=11"));
}

#[test]
fn memory_sql_query_reads_memory_versions_and_normalizes_legacy_rows() {
    let dir = tmp_dir("sql_memory_versions");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":11,"content":"测试代号是 ALPHA-42"}
{"id":"m2","created_at_ms":22,"updated_at_ms":33,"version":4,"content":"用户喜欢 Rust"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("查记忆版本", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Read durable memory versions for safe update.","args":{"type":"durable","op":"sql","sql":"SELECT id, version, updated_at_ms, content FROM memories ORDER BY created_at_ms ASC","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("id=m1, version=1, updated_at_ms=11"));
    assert!(prompt.contains("id=m2, version=4, updated_at_ms=33"));
}

#[test]
fn sql_read_allows_with_cte_reads() {
    let dir = tmp_dir("sql_with_cte");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":11,"content":"测试代号是 ALPHA-42"}
{"id":"m2","created_at_ms":22,"content":"测试项目纪念日是 2099-06-12"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("按时间查测试代号", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"WITH\nmatched AS (SELECT content, created_at_ms FROM memories WHERE content LIKE ?) SELECT content, created_at_ms FROM matched ORDER BY created_at_ms ASC LIMIT 5","params":["%测试代号%"]}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("content=测试代号是 ALPHA-42"));
    assert!(prompt.contains("created_at_ms=11"));
}

#[test]
fn sql_read_rejects_write_statement() {
    let dir = tmp_dir("sql_reject_write");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("改记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"UPDATE memories SET content='x' LIMIT 1"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("error: read_only_sql_required"));
}

#[test]
fn memory_sql_query_uses_action_limit_without_sql_limit() {
    let dir = tmp_dir("sql_action_limit");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"第一条记忆"}
{"id":"m2","created_at_ms":2,"content":"第二条记忆"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("查记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"SELECT content FROM memories ORDER BY created_at_ms ASC","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("content=第一条记忆"));
    assert!(!prompt.contains("content=第二条记忆"));
}

#[test]
fn sql_read_rejects_other_tables() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("sql_other_tables"),
    );
    let _ = core.begin_turn("列出表", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"SELECT name FROM sqlite_master LIMIT 5"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("error: only_declared_tables_are_allowed"));
}

#[test]
fn memory_schema_action_returns_native_schema_contract() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("schema_action"),
    );
    let _ = core.begin_turn("有哪些记忆表", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"查看记忆结构","args":{"type":"durable","op":"schema"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains(
        "memories(id TEXT, created_at_ms INTEGER, updated_at_ms INTEGER, version INTEGER, content TEXT)"
    ));
    assert!(prompt.contains("expected_version"));
    assert!(prompt.contains("safe_interface: memmgr"));
    assert!(prompt.contains("durable: query|schema|sql|insert|update|upsert|delete"));
}

#[test]
fn memory_sql_query_allows_pragma_table_info() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("pragma_schema"),
    );
    let _ = core.begin_turn("查看 schema", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"PRAGMA table_info(memories)","limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("name=content"));
    assert!(prompt.contains("name=created_at_ms"));
}

#[test]
fn memory_sql_query_allows_chat_messages_table_info() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("pragma_chat_messages_schema"),
    );
    let _ = core.begin_turn("查看聊天 schema", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"PRAGMA table_info(chat_messages)","limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("name=content"));
    assert!(prompt.contains("name=session_id"));
    assert!(prompt.contains("name=created_at_ms"));
}

#[test]
fn memory_sql_query_rejects_non_memories_pragma() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bad_pragma"),
    );
    let _ = core.begin_turn("查看 schema", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"PRAGMA table_info(sqlite_master)","limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("error: only_declared_tables_are_allowed"));
}

#[test]
fn sql_read_action_requires_sql_for_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("sql_missing"),
    );
    let _ = core.begin_turn("查记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.sql_required_when_op=sql,type=durable"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn memory_sql_query_requires_params_for_placeholders() {
    let dir = tmp_dir("sql_missing_params");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我的测试代号是什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"SELECT content FROM memories WHERE content LIKE ? ORDER BY created_at_ms ASC","limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(!prompt.contains("Protocol repair request"));
    assert!(prompt.contains("SQL placeholder count does not match `params`"));
    assert!(!prompt.contains("sql_query_failed"));
}

#[test]
fn memory_sql_query_rejects_extra_params_for_placeholders() {
    let dir = tmp_dir("sql_extra_params");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"测试代号是 ALPHA-42"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我的测试代号是什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"SELECT content FROM memories WHERE content LIKE ? ORDER BY created_at_ms ASC","params":["%name:%","%mynameis%","%Iam%"],"limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(!prompt.contains("Protocol repair request"));
    assert!(prompt.contains("SQL placeholder count does not match `params`"));
    assert!(!prompt.contains("sql_query_failed"));
}

#[test]
fn chat_history_query_reads_persisted_chat_records() {
    let root = tmp_dir("chat_history_persisted");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("audit").join("api_audit.json");
    write_audit_doc(
        &audit_file,
        vec![
            json!({"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"我昨天提到了测试物品 BLUE-17"}),
            json!({"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"我记下了测试物品 BLUE-17这个说法。"}),
        ],
    );
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我之前说过什么物品", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"查询聊天记录","args":{"type":"raw_chat","op":"query","query":"测试物品 BLUE-17","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("chat_records"));
    assert!(prompt.contains("source=chat_record"));
    assert!(prompt.contains("shell_old"));
    assert!(prompt.contains("测试物品 BLUE-17"));
    assert!(prompt.contains("我记下了测试物品 BLUE-17这个说法"));
}

#[test]
fn chat_history_query_reads_legacy_jsonl_audit_records() {
    let root = tmp_dir("chat_history_legacy_jsonl");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("api_audit.jsonl");
    fs::write(
        &audit_file,
        r#"{"type":"turn_start","session":"legacy_shell","turn_id":"turn_1781760000000","user_input":"旧格式提到了测试物品 GREEN-29"}
{"type":"turn_final","session":"legacy_shell","turn_id":"turn_1781760000000","assistant_output":"我记下了测试物品 GREEN-29。"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("旧格式里说过什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"查询旧格式聊天记录","args":{"type":"raw_chat","op":"query","query":"测试物品 GREEN-29","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("source=chat_record"));
    assert!(prompt.contains("legacy_shell"));
    assert!(prompt.contains("测试物品 GREEN-29"));
}

#[test]
fn chat_history_query_keeps_current_prompt_delta_fallback() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("chat_history"),
    );
    let _ = core.begin_turn("第一轮我说了测试物品 BLUE-17", None);
    let _ = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"收到"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let _ = core.begin_turn("我刚才说了什么物品", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"查询聊天记录","args":{"type":"raw_chat","op":"query","query":"测试物品 BLUE-17","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("测试物品 BLUE-17"));
    assert!(prompt.contains("current_prompt_deltas"));
    assert!(prompt.contains("source=prompt_delta"));
}

#[test]
fn chat_history_query_empty_query_lists_recent_records() {
    let root = tmp_dir("chat_history_recent_empty");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("audit").join("api_audit.json");
    write_audit_doc(
        &audit_file,
        vec![
            json!({"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"第一条历史"}),
            json!({"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"第一条回复"}),
            json!({"type":"turn_start","session":"shell_old","turn_id":"turn_1781846400000","user_input":"第二条历史"}),
            json!({"type":"turn_final","session":"shell_old","turn_id":"turn_1781846400000","assistant_output":"第二条回复"}),
        ],
    );
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("列最近聊天", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"列出最近聊天记录","args":{"type":"raw_chat","op":"query","query":"","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("source=chat_record"));
    assert!(prompt.contains("第二条历史"));
    assert!(!prompt.contains("第一条历史"));
}

#[test]
fn memory_sql_query_reads_chat_messages_with_time_window() {
    let root = tmp_dir("chat_messages_sql");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("audit").join("api_audit.json");
    write_audit_doc(
        &audit_file,
        vec![
            json!({"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"旧聊天"}),
            json!({"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"旧回复"}),
            json!({"type":"turn_start","session":"shell_new","turn_id":"turn_1781846400000","user_input":"新聊天"}),
            json!({"type":"turn_final","session":"shell_new","turn_id":"turn_1781846400000","assistant_output":"新回复"}),
        ],
    );
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("查最近窗口聊天", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"按时间窗口查询聊天记录","args":{"type":"durable","op":"sql","sql":"SELECT session_id, role, content, created_at_ms FROM chat_messages WHERE created_at_ms >= ? AND created_at_ms < ? ORDER BY created_at_ms DESC","params":["1781840000000","1781850000000"],"limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("session_id=shell_new"));
    assert!(prompt.contains("content=新聊天"));
    assert!(prompt.contains("content=新回复"));
    assert!(!prompt.contains("content=旧聊天"));
}

#[test]
fn memory_sql_query_accepts_common_llm_param_shapes() {
    let sql = "SELECT role, content, created_at_ms FROM chat_messages WHERE created_at_ms >= ? AND created_at_ms < ? ORDER BY created_at_ms ASC";
    let sql_json = serde_json::to_string(sql).unwrap();
    let cases = [
        (
            "string_params_inside_input",
            format!(
                r#""args":{{"type":"durable","op":"sql","sql":{},"params":["1782200000000","1782210000000"],"limit":50}}"#,
                sql_json
            ),
        ),
        (
            "integer_params_inside_input",
            format!(
                r#""args":{{"type":"durable","op":"sql","sql":{},"params":[1782200000000,1782210000000],"limit":50}}"#,
                sql_json
            ),
        ),
        (
            "float_params_inside_input",
            format!(
                r#""args":{{"type":"durable","op":"sql","sql":{},"params":[1782200000000,1782210000000],"limit":50}}"#,
                sql_json
            ),
        ),
    ];

    for (case_name, action_fields) in cases {
        let root = tmp_dir(case_name);
        let dir = root.join("memory");
        fs::create_dir_all(&dir).unwrap();
        let audit_file = root.join("audit").join("api_audit.json");
        write_audit_doc(
            &audit_file,
            vec![
                json!({"type":"turn_start","session":"shell_today","turn_id":"turn_1782203922467","user_input":"我今天和你聊过什么？"}),
                json!({"type":"turn_final","session":"shell_today","turn_id":"turn_1782203922467","assistant_output":"今天聊过 shell 记忆查询。"}),
            ],
        );
        let mut core = AgentCore::new("STATIC", profile("custom", "aws-claude-sonnet-4-6"), &dir);
        let _ = core.begin_turn("我今天和你聊过什么？", None);
        let content = scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"查询今天的聊天记录",{}}}]}}"#,
            action_fields
        ));
        let step = core.apply_model_response(LlmResponse {
            content,
            model_name: "aws-claude-sonnet-4-6".to_string(),
            usage: usage(),
            truncated: false,
        });
        let prompt = match step {
            CoreStep::NeedModel { prompt, .. } => prompt,
            other => panic!("{case_name} unexpected step: {other:?}"),
        };
        assert!(prompt.contains("Action result: memmgr"), "{case_name}");
        assert!(
            prompt.contains("content=我今天和你聊过什么？"),
            "{case_name}"
        );
        assert!(
            prompt.contains("content=今天聊过 shell 记忆查询。"),
            "{case_name}"
        );
        assert!(
            !prompt.contains("params_count_mismatch"),
            "{case_name}: {prompt}"
        );
    }
}

#[test]
fn memory_sql_query_rejects_raw_update_sql() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("raw_sql_write"),
    );
    let _ = core.begin_turn("更新记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"sql","sql":"UPDATE memories SET content='bad'","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("error: read_only_sql_required"));
}

#[test]
fn memory_sql_query_rejects_chat_history_delete_sql() {
    let root = tmp_dir("chat_delete_rejected");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("audit").join("api_audit.json");
    write_audit_doc(
        &audit_file,
        vec![
            json!({"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"需要保留的聊天"}),
            json!({"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"这条聊天仍应只读。"}),
        ],
    );
    let before = fs::read_to_string(&audit_file).unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("删除聊天记录", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Attempt to delete chat history through SQL.","args":{"type":"durable","op":"sql","sql":"DELETE FROM chat_messages WHERE content LIKE '%保留%'","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("error: read_only_sql_required"));
    assert_eq!(fs::read_to_string(&audit_file).unwrap(), before);
}

#[test]
fn chat_history_delete_removes_matching_turn_from_audit_log() {
    let root = tmp_dir("chat_delete_action");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("audit").join("api_audit.json");
    write_audit_doc(
        &audit_file,
        vec![
            json!({"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"删除目标聊天"}),
            json!({"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"删除目标回复"}),
            json!({"type":"turn_start","session":"shell_old","turn_id":"turn_1781846400000","user_input":"保留聊天"}),
            json!({"type":"turn_final","session":"shell_old","turn_id":"turn_1781846400000","assistant_output":"保留回复"}),
        ],
    );
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("删除包含目标的聊天记录", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Delete matching chat record.","args":{"type":"raw_chat","op":"delete","query":"删除目标","limit":10}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("deleted_count: 1"));
    let stored = fs::read_to_string(&audit_file).unwrap();
    assert!(!stored.contains("删除目标"));
    assert!(stored.contains("保留聊天"));
    assert!(stored.contains("保留回复"));
}

#[test]
fn memory_update_insert_update_and_delete_are_wrapped() {
    let dir = tmp_dir("memory_update_wrapped");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("记住我的测试代号", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"upsert","id":"user_name","content":"测试代号是 ALPHA-42"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("id: user_name"));
    assert!(core.memory_git_commit_count() >= 1);
    assert!(fs::read_to_string(core.memory_file())
        .unwrap()
        .contains("测试代号是 ALPHA-42"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"update","id":"user_name","content":"测试代号是 BETA-43"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("missing_expected_version"));
    assert!(fs::read_to_string(core.memory_file())
        .unwrap()
        .contains("测试代号是 ALPHA-42\""));

    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"update","id":"user_name","expected_version":1,"content":"测试代号是 BETA-43"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("op: update"));
    assert!(prompt.contains("version: 2"));
    let stored = fs::read_to_string(core.memory_file()).unwrap();
    assert!(stored.contains("测试代号是 BETA-43"));
    assert!(!stored.contains("测试代号是 ALPHA-42\""));
    assert!(core.memory_git_commit_count() >= 2);

    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"delete","id":"user_name","expected_version":2}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("op: delete"));
    assert!(!fs::read_to_string(core.memory_file())
        .unwrap()
        .contains("user_name"));
    assert!(core.memory_git_commit_count() >= 3);
}

#[test]
fn memory_update_detects_stale_version_conflict_without_overwrite() {
    let dir = tmp_dir("memory_update_conflict");
    let mut core_a = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let mut core_b = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);

    let _ = core_a.begin_turn("创建共享记忆", None);
    let step = core_a.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Insert shared row.","args":{"type":"durable","op":"upsert","id":"shared_fact","content":"版本1"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::NeedModel { .. }));

    let _ = core_a.begin_turn("A 更新", None);
    let step = core_a.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Update shared row from A.","args":{"type":"durable","op":"update","id":"shared_fact","expected_version":1,"content":"版本2 from A"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("version: 2"));

    let _ = core_b.begin_turn("B 用旧版本更新", None);
    let step = core_b.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Update shared row from B with stale version.","args":{"type":"durable","op":"update","id":"shared_fact","expected_version":1,"content":"版本2 from B"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("memory_conflict"));
    assert!(prompt.contains("expected_version=1"));
    assert!(prompt.contains("current_version=2"));

    let stored = fs::read_to_string(core_a.memory_file()).unwrap();
    assert!(stored.contains("版本2 from A"));
    assert!(!stored.contains("版本2 from B"));
}

#[test]
fn memory_update_concurrent_same_version_conflicts_allow_only_one_winner() {
    let dir = tmp_dir("memory_update_parallel_conflict");
    let mut seed_core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = seed_core.begin_turn("创建共享记忆", None);
    let step = seed_core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Insert shared conflict row.","args":{"type":"durable","op":"upsert","id":"shared_conflict","content":"初始值"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::NeedModel { .. }));

    let contenders = 8;
    let barrier = Arc::new(Barrier::new(contenders));
    let mut handles = Vec::new();
    for idx in 0..contenders {
        let dir = dir.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
            let _ = core.begin_turn(&format!("并发冲突更新 {idx}"), None);
            barrier.wait();
            let step = core.apply_model_response(LlmResponse {
                content: scored(format!(
                    r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Concurrent same-version update.","args":{{"type":"durable","op":"update","id":"shared_conflict","expected_version":1,"content":"winner candidate {idx}"}}}}]}}"#
                )),
                model_name: "qwen-plus".to_string(),
                usage: usage(),
                truncated: false,
            });
            match step {
                CoreStep::NeedModel { prompt, .. } => prompt,
                other => panic!("unexpected step: {other:?}"),
            }
        }));
    }

    let prompts = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    let success_count = prompts
        .iter()
        .filter(|prompt| prompt.contains("op: update") && prompt.contains("version: 2"))
        .count();
    let conflict_count = prompts
        .iter()
        .filter(|prompt| prompt.contains("memory_conflict"))
        .count();
    assert_eq!(success_count, 1);
    assert_eq!(conflict_count, contenders - 1);

    let stored = fs::read_to_string(dir.join("memory.jsonl")).unwrap();
    let rows = stored
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "shared_conflict");
    assert_eq!(rows[0]["version"], 2);
    let content = rows[0]["content"].as_str().unwrap();
    assert!(content.starts_with("winner candidate "));
}

#[test]
fn mem_guard_blocks_second_writer_until_first_writer_releases_lock() {
    let dir = tmp_dir("mem_guard_blocks").join("memory");
    fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("guard_marker.txt");
    let guard = MemGuard::for_memory_dir(&dir);
    let guard_for_thread = guard.clone();
    let marker_for_thread = marker.clone();

    let handle = guard
        .with_write(|| {
            let handle = thread::spawn(move || {
                guard_for_thread
                    .with_write(|| fs::write(&marker_for_thread, "done"))
                    .unwrap()
                    .unwrap();
            });
            thread::sleep(Duration::from_millis(120));
            assert!(
                !marker.exists(),
                "second writer should wait for the first lock holder"
            );
            handle
        })
        .unwrap();
    handle.join().unwrap();
    assert_eq!(fs::read_to_string(&marker).unwrap(), "done");
}

#[test]
fn mem_guard_child_process_holds_lock_helper() {
    let Ok(dir) = std::env::var("TIMEM_MEM_GUARD_CHILD_DIR") else {
        return;
    };
    let marker = PathBuf::from(std::env::var("TIMEM_MEM_GUARD_CHILD_MARKER").unwrap());
    let release = std::env::var("TIMEM_MEM_GUARD_CHILD_RELEASE")
        .ok()
        .map(PathBuf::from);
    let guard = MemGuard::for_memory_dir(dir);
    guard
        .with_write(|| {
            fs::write(&marker, "locked").unwrap();
            if let Some(release) = release {
                let started = std::time::Instant::now();
                while !release.exists() {
                    assert!(started.elapsed() < Duration::from_secs(10));
                    thread::sleep(Duration::from_millis(20));
                }
            } else {
                thread::sleep(Duration::from_millis(350));
            }
        })
        .unwrap();
}

#[test]
fn mem_guard_serializes_writes_across_processes() {
    if std::env::var("TIMEM_MEM_GUARD_CHILD_DIR").is_ok() {
        return;
    }
    let dir = tmp_dir("mem_guard_process").join("memory");
    fs::create_dir_all(&dir).unwrap();
    let child_marker = dir.join("child_locked.txt");
    let parent_marker = dir.join("parent_after_child.txt");
    let release_marker = dir.join("release_child.txt");
    let current_exe = std::env::current_exe().unwrap();
    let mut child = Command::new(current_exe)
        .arg("--exact")
        .arg("mem_guard_child_process_holds_lock_helper")
        .arg("--nocapture")
        .env("TIMEM_MEM_GUARD_CHILD_DIR", &dir)
        .env("TIMEM_MEM_GUARD_CHILD_MARKER", &child_marker)
        .env("TIMEM_MEM_GUARD_CHILD_RELEASE", &release_marker)
        .spawn()
        .unwrap();

    let started = std::time::Instant::now();
    while !child_marker.exists() {
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(20));
    }

    let parent_dir = dir.clone();
    let parent_marker_for_thread = parent_marker.clone();
    let parent = thread::spawn(move || {
        MemGuard::for_memory_dir(&parent_dir)
            .with_write(|| fs::write(&parent_marker_for_thread, "done"))
            .unwrap()
            .unwrap();
    });
    thread::sleep(Duration::from_millis(200));
    assert!(
        !parent_marker.exists(),
        "parent should wait for child process guard"
    );
    fs::write(&release_marker, "release").unwrap();
    parent.join().unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());
    assert_eq!(fs::read_to_string(parent_marker).unwrap(), "done");
}

#[test]
fn mem_guard_keeps_concurrent_memory_updates_from_losing_records() {
    let dir = tmp_dir("mem_guard_concurrent_updates");
    let writers = 12;
    let barrier = Arc::new(Barrier::new(writers));
    let mut handles = Vec::new();
    for idx in 0..writers {
        let dir = dir.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
            let _ = core.begin_turn(&format!("并发写入 {idx}"), None);
            barrier.wait();
            let step = core.apply_model_response(LlmResponse {
                content: scored(format!(
                    r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Concurrent durable write.","args":{{"type":"durable","op":"upsert","id":"guard_id_{idx}","content":"guard content {idx}"}}}}]}}"#
                )),
                model_name: "qwen-plus".to_string(),
                usage: usage(),
                truncated: false,
            });
            assert!(matches!(step, CoreStep::NeedModel { .. }));
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let stored = fs::read_to_string(dir.join("memory.jsonl")).unwrap();
    let rows = stored
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), writers);
    for idx in 0..writers {
        assert!(
            rows.iter()
                .any(|row| row.get("id").and_then(|id| id.as_str())
                    == Some(format!("guard_id_{idx}").as_str())),
            "missing concurrent memory id {idx}"
        );
    }
}

#[test]
fn memory_update_requires_protocol_fields() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("memory_update_repair"),
    );
    let _ = core.begin_turn("更新记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test action","args":{"type":"durable","op":"update","content":"x"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.id_required_when_op=update,type=durable"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn run_bash_allows_readonly_count_command() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_readonly"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("count cwd lines", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"count output lines","args":{"cmd":"pwd | wc -l","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("status: 0"));
    assert!(prompt.contains("output:"));
}

#[test]
fn action_audit_groups_actions_by_user_turn_and_round() {
    let dir = tmp_dir("action_audit_grouping");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("整理这个任务", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"记录任务计划","args":{"type":"scratch","op":"write","kind":"notes","label":"任务计划","content":"step one"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    match step {
        CoreStep::NeedModel { .. } => {}
        other => panic!("unexpected step: {other:?}"),
    }
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"查询任务计划","args":{"type":"scratch","op":"query","query":"step","limit":3}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    match step {
        CoreStep::NeedModel { .. } => {}
        other => panic!("unexpected step: {other:?}"),
    }

    let audit_path = dir.join("audit").join("action_audit.json");
    let audit_text = fs::read_to_string(audit_path).unwrap();
    let audit: serde_json::Value = serde_json::from_str(&audit_text).unwrap();
    let turns = audit["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["user_question"], "整理这个任务");
    let interactions = turns[0]["interactions"].as_array().unwrap();
    assert_eq!(interactions.len(), 2);
    assert_eq!(interactions[0]["round"], 1);
    assert_eq!(interactions[0]["actions"][0]["action"], "memmgr");
    assert_eq!(interactions[0]["actions"][0]["intent"], "记录任务计划");
    assert_eq!(interactions[0]["actions"][0]["status"], "completed");
    assert_eq!(
        interactions[0]["actions"][0]["input"]["content"],
        "step one"
    );
    assert_eq!(interactions[0]["actions"][0]["input"]["type"], "scratch");
    assert_eq!(interactions[0]["actions"][0]["input"]["kind"], "notes");
    assert_eq!(interactions[0]["actions"][0]["input"]["label"], "任务计划");
    assert_eq!(interactions[1]["round"], 2);
    assert_eq!(interactions[1]["actions"][0]["action"], "memmgr");
    assert_eq!(interactions[1]["actions"][0]["intent"], "查询任务计划");
}

#[test]
fn run_bash_rejects_old_timeout_sec_field() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_timeout_sec"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("count cwd lines", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"count output lines","args":{"cmd":"pwd | wc -l","timeout_sec":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.timeout_sec_unsupported"));
    assert!(!prompt.contains("Action result: run_bash"));
}

#[test]
fn run_bash_can_start_and_poll_background_job() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_background"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("run a long task", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Start a background task.","args":{"cmd":"sleep 0.1; printf background-ok","background":true}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("status: background_started"));
    let job_id = action_result_field(&prompt, "job_id");
    assert!(job_id.starts_with("job_"));

    std::thread::sleep(std::time::Duration::from_millis(250));
    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"shell_job_status","intent":"Poll background task.","args":{{"job_id":"{}","timeout_ms":1000}}}}]}}"#,
            job_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: shell_job_status"));
    assert!(prompt.contains("state: finished"));
    assert!(prompt.contains("exit_code: 0"));
    assert!(prompt.contains("background-ok"));
}

#[test]
fn shell_job_status_missing_job_id_returns_tool_input_error() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_background_job_id_required"),
    );
    let _ = core.begin_turn("poll a long task", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"shell_job_status","intent":"Poll background task.","args":{"op":"status"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.job_id_required"));
    assert!(!prompt.contains("Action result: shell_job_status"));
}

#[test]
fn shell_job_status_missing_timeout_uses_immediate_check() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_background_timeout_optional"),
    );
    let _ = core.begin_turn("poll a long task", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"shell_job_status","intent":"Poll background task.","args":{"job_id":"job_1"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(!prompt.contains("Protocol repair request"));
    assert!(prompt.contains("Action result: shell_job_status"));
    assert!(prompt.contains("error: job_not_found"));
}

#[test]
fn shell_job_status_waits_for_model_chosen_timeout_before_running_result() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_background_wait"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("run a long task", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Start a background task.","args":{"cmd":"sleep 0.4; printf waited-ok","background":true}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let job_id = action_result_field(&prompt, "job_id");
    assert!(job_id.starts_with("job_"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"shell_job_status","intent":"Wait for background task.","args":{{"job_id":"{}","timeout_ms":1500}}}}]}}"#,
            job_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: shell_job_status"));
    assert!(prompt.contains("state: finished"));
    assert!(prompt.contains("waited-ok"));
    assert!(prompt.contains("waited_ms:"));
}

#[test]
fn run_bash_rejects_removed_read_back_protocol() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_reject_readback_field"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("count cwd lines", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"read back count","args":{"cmd":"pwd","read_back_command":"pwd | wc -l","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.read_back_command_unsupported"));
    assert!(!prompt.contains("Action result: run_bash"));
}

#[test]
fn run_bash_rejects_removed_large_readback_protocol() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_reject_large_readback_field"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("count cwd lines", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","args":{"cmd":"pwd","large_readback_opt_in":"need full output","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.large_readback_opt_in_unsupported"));
    assert!(!prompt.contains("Action result: run_bash"));
}

#[test]
fn run_bash_requires_approval_for_mutating_commands() {
    let dir = tmp_dir("bash_reject");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("delete something", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","args":{"cmd":"rm not_allowed"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(request.action, "run_bash");
    assert_eq!(request.command, "rm not_allowed");
    assert_eq!(request.reason, "run_bash_requires_user_approval");
    assert_eq!(request.risk, "local_command_execution");

    let turn_audit = dir.join("audit/turn_audit.json");
    let prompt = match core.resolve_user_approval_with_audit(
        &request,
        false,
        &turn_audit,
        "session_1",
        "turn_1",
    ) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("status: denied_by_user"));
    assert!(prompt.contains(&request.approval_id));
    let turn_audit_doc = read_audit_doc(&turn_audit).unwrap();
    let events = turn_audit_doc["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "user_approval");
    assert_eq!(events[0]["session"], "session_1");
    assert_eq!(events[0]["turn_id"], "turn_1");
    assert_eq!(events[0]["approval_id"], request.approval_id);
    assert_eq!(events[0]["approved"], false);

    let audit_text = fs::read_to_string(dir.join("audit").join("action_audit.json")).unwrap();
    let audit: serde_json::Value = serde_json::from_str(&audit_text).unwrap();
    let actions = audit["turns"][0]["interactions"][0]["actions"]
        .as_array()
        .unwrap();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0]["action"], "run_bash");
    assert_eq!(actions[0]["intent"], "test action");
    assert_eq!(actions[0]["status"], "needs_user_approval");
    assert_eq!(actions[0]["input"]["cmd"], "rm not_allowed");
    assert_eq!(actions[1]["status"], "denied_by_user");
    assert_eq!(actions[1]["input"]["approval_id"], request.approval_id);
}

#[test]
fn run_bash_allows_compound_local_write_commands() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_allow_compound_write"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("write local file", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","args":{"cmd":"mkdir -p target/timem_test; printf ok | tee target/timem_test/write_guard.txt; cat target/timem_test/write_guard.txt"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("status: 0"));
    assert!(prompt.contains("ok"));
    let _ = fs::remove_dir_all("target/timem_test");
    let _ = fs::remove_dir("target");
}

#[test]
fn run_bash_requires_approval_for_high_risk_command_inside_compound_command() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_reject_compound_delete"),
    );
    let _ = core.begin_turn("inspect files", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","args":{"cmd":"pwd && rm not_allowed"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(request.command, "pwd && rm not_allowed");
    assert_eq!(request.reason, "run_bash_requires_user_approval");
    assert_eq!(request.risk, "local_command_execution");
}

#[test]
fn run_bash_executes_shell_syntax_after_user_approval() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_shell_syntax_after_approval"),
    );
    let _ = core.begin_turn("test shell syntax", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Run shell syntax after approval.","args":{"cmd":"x=ok; printf $x | tr o O","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(request.command, "x=ok; printf $x | tr o O");

    let prompt = match core.resolve_user_approval(&request.approval_id, true) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("status: 0"));
    assert!(prompt.contains("Ok"));
    assert!(prompt.contains("approval_status: approved_by_user"));
    assert!(!prompt.contains("shell_expansion_not_allowed"));
}

#[test]
fn run_bash_missing_command_returns_tool_input_error() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_missing"),
    );
    let _ = core.begin_turn("inspect files", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","args":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.any_required:cmd|loop_cmd"));
    assert!(!prompt.contains("Action result: run_bash"));
}

#[test]
fn run_bash_requires_approval_for_absolute_paths() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_path_reject"),
    );
    let _ = core.begin_turn("read passwd", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","args":{"cmd":"cat /etc/passwd"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(request.command, "cat /etc/passwd");
    assert_eq!(request.reason, "run_bash_requires_user_approval");
    assert_eq!(request.risk, "local_command_execution");
}

#[test]
fn run_bash_allows_low_risk_system_identity_commands() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_system_identity"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("inspect system identity", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Read system identity.","args":{"cmd":"uname -s","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("intent: Read system identity."));
    assert!(prompt.contains("command: uname -s"));
    assert!(prompt.contains("status: 0"));
    assert!(!prompt.contains("approval_status: approved_by_user"));
}

#[test]
fn ci_realistic_multiturn_memory_tools_security_and_shrink_story() {
    let dir = tmp_dir("ci_realistic_story");
    let mut core = AgentCore::new("STATIC_GLOBAL_RULES", profile("aliyun", "qwen-plus"), &dir);

    let first_prompt = match core.begin_turn(
        "测试项目纪念日是 2099-06-12",
        Some("runtime_time: 2026-06-19T12:00:00+08:00"),
    ) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(first_prompt.contains("User question:\n测试项目纪念日是 2099-06-12"));
    assert!(first_prompt.contains("Supporting context:\nruntime_time:"));
    let write_final = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"已记录。","memory_candidates":[{"content":"测试项目纪念日是 2099-06-12"}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(write_final.stats.mem_writes, 1);

    let _ = core.begin_turn("2099-06-12 是什么测试日期", None);
    let recall_prompt = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Find durable birthday memory before answering.","args":{"type":"durable","op":"query","query":"测试项目 纪念日 2099-06-12","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(recall_prompt.contains("Action result: memmgr"));
    assert!(recall_prompt.contains("测试项目纪念日是 2099-06-12"));
    let recall_final = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"2099-06-12是测试项目纪念日。"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(recall_final.final_answer.contains("测试项目"));
    assert!(recall_final.stats.mem_reads >= 1);

    let _ = core.begin_turn("删除测试项目纪念日这条记忆", None);
    let delete_prompt = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Delete the user-requested birthday memory.","args":{"type":"durable","op":"delete","id":"mem_0"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(delete_prompt.contains("Action result: memmgr"));
    assert!(delete_prompt.contains("error: id_not_found"));

    let delete_prompt = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Find exact memory id before deleting.","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? ORDER BY created_at_ms DESC","params":["%测试项目纪念日%"],"limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(delete_prompt.contains("Action result: memmgr"));
    assert!(delete_prompt.contains("content=测试项目纪念日是 2099-06-12"));
    assert!(delete_prompt.contains("version=1"));

    let stored = fs::read_to_string(core.memory_file()).unwrap();
    let memory_id = stored
        .lines()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|value| {
            value
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .expect("memory id should exist");
    let delete_final_prompt = match core.apply_model_response(LlmResponse {
        content: scored(format!(r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Delete exact durable birthday memory.","args":{{"type":"durable","op":"delete","id":"{}","expected_version":1}}}}]}}"#, memory_id)),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(delete_final_prompt.contains("op: delete"));
    assert!(!fs::read_to_string(core.memory_file())
        .unwrap()
        .contains("测试项目纪念日"));

    let delete_final = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"已删除。"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(delete_final.stats.mem_writes >= 1);

    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("请统计当前目录文件数量", None);
    let shell_prompt = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Count files in current project folder.","args":{"cmd":"find . -maxdepth 1 -type f | wc -l","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(shell_prompt.contains("Action result: run_bash"));
    assert!(shell_prompt.contains("status: 0"));

    core.set_bash_approval_mode(BashApprovalMode::Ask);
    let _ = core.begin_turn("把 /etc/passwd 读出来", None);
    let security_request = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Attempt forbidden absolute path read.","args":{"cmd":"cat /etc/passwd","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(security_request.reason, "run_bash_requires_user_approval");

    core.set_max_llm_input_tokens(3_000);
    for index in 0..3 {
        let _ = core.begin_turn(
            &format!("无关闲聊 {} {}", index, "长上下文 ".repeat(600)),
            None,
        );
        let step = core.apply_model_response(LlmResponse {
            content: scored(format!(
                r#"{{"status":"finished","final_answer":"ok {}"}}"#,
                index
            )),
            model_name: "qwen-plus".to_string(),
            usage: usage(),
            truncated: false,
        });
        assert!(matches!(step, CoreStep::Final(_)));
    }
    let long_prompt = match core.begin_turn("继续一个新任务", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(long_prompt.starts_with("[BEGIN SYSTEM PROMPT]\nSTATIC_GLOBAL_RULES"));
    assert!(long_prompt.contains("Long-context maintenance:"));
    assert!(long_prompt.contains("mode=force_shrink_required"));
    assert!(long_prompt.contains("force_shrink_threshold_tokens=2700"));
    assert!(long_prompt.contains("target_dynamic_context_ratio=10%-20%"));
    assert!(long_prompt.contains("prompt_delta_count="));
}

#[test]
fn scenario_coding_inspects_project_and_reports_from_shell_evidence() {
    let dir = tmp_dir("scenario_coding");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    core.set_bash_approval_mode(BashApprovalMode::Approve);

    let _ = core.begin_turn(
        "检查这个 Rust 项目的代码入口和测试数量，然后告诉我下一步。",
        None,
    );
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"先查看项目结构和测试入口。","next_actions":[{"action":"run_bash","intent":"Inspect Rust project files and test definitions.","args":{"cmd":"mkdir -p target; printf %s\\n src/lib.rs src/main.rs tests/core_tests.rs > target/timem_scenario_files.txt; printf %s\\n smoke_test regression_test > target/timem_scenario_tests.rs; wc -l target/timem_scenario_files.txt target/timem_scenario_tests.rs","timeout_ms":5000}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected shell evidence prompt, got {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("target/timem_scenario_files.txt"));
    assert!(prompt.contains("target/timem_scenario_tests.rs"));
    assert!(prompt.contains("status: 0"));

    let final_turn = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"根据本地检查结果，项目入口和测试文件已定位；下一步应针对失败点补小范围测试并运行相关 cargo test。"}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final coding answer, got {other:?}"),
    };
    assert!(final_turn.final_answer.contains("本地检查结果"));
    let _ = fs::remove_file("target/timem_scenario_files.txt");
    let _ = fs::remove_file("target/timem_scenario_tests.rs");
}

#[test]
fn scenario_memory_qa_retrieves_durable_and_raw_chat_before_answering() {
    let dir = tmp_dir("scenario_memory_qa");
    let memory_dir = dir.join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(
        memory_dir.join("memory.jsonl"),
        r#"{"id":"mem_name","content":"测试代号是 ALPHA-42","created_at_ms":1780000000000,"updated_at_ms":1780000000000,"version":1}
"#,
    )
    .unwrap();
    let audit_dir = dir.join("audit");
    fs::create_dir_all(&audit_dir).unwrap();
    write_audit_doc(
        &audit_dir.join("api_audit.json"),
        vec![
            json!({"type":"turn_start","session":"scenario_memory","turn_id":"turn_1780000010000_1","created_at":1780000010000u64,"user_input":"测试时段我们聊了 测试发布检查"}),
            json!({"type":"turn_final","session":"scenario_memory","turn_id":"turn_1780000010000_1","created_at":1780000015000u64,"assistant_output":"我建议先跑完整 CI 和真实 TTY smoke。"}),
        ],
    );

    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &memory_dir);
    core.set_capability_registry(CapabilityRegistry::builtin());
    let _ = core.begin_turn("我的测试代号是什么？测试时段我们聊了什么？", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"查询长期记忆和聊天记录后再回答。","next_actions":[{"action":"memmgr","intent":"查询测试代号长期记忆。","args":{"type":"durable","op":"query","query":"用户 测试代号","limit":5}},{"action":"memmgr","intent":"查询测试时段聊天记录。","args":{"type":"raw_chat","op":"query","query":"测试时段 发布检查 CI TTY","limit":5}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected memory evidence prompt, got {other:?}"),
    };
    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("测试代号是 ALPHA-42"));
    assert!(prompt.contains("测试发布检查"));
    assert!(prompt.contains("完整 CI 和真实 TTY smoke"));

    let final_turn = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"测试代号是 ALPHA-42。测试时段我们聊的是 测试发布检查，重点是完整 CI 和真实 TTY smoke。"}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final memory answer, got {other:?}"),
    };
    assert!(final_turn.final_answer.contains("测试代号是 ALPHA-42"));
}

#[test]
fn scenario_self_qa_and_runtime_env_update_stays_bounded() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scenario_self_qa"),
    );
    let _ = core.begin_turn("你是谁？把本轮调试标记设成 enabled，再确认你的路径。", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"查询 Timem 自身信息并设置非敏感运行期标记。","next_actions":[{"action":"self_tool","intent":"查看 Timem 身份和进程。","args":{"type":"about_me","op":"read"}},{"action":"self_tool","intent":"设置非敏感调试标记。","args":{"type":"env","op":"write","key":"TIMEM_SCENARIO_DEBUG","value":"enabled"}},{"action":"self_tool","intent":"查看当前记忆和审计路径。","args":{"type":"mem_path","op":"read"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected self_tool evidence prompt, got {other:?}"),
    };
    assert!(prompt.contains("type: about_me"));
    assert!(prompt.contains("name: TimemAi"));
    assert!(prompt.contains("project: https://github.com/moliam/TimemAi"));
    assert!(prompt.contains("star_message: Please star https://github.com/moliam/TimemAi"));
    assert!(prompt.contains("status: updated_current_process_env"));
    assert!(prompt.contains("key: TIMEM_SCENARIO_DEBUG"));
    assert!(prompt.contains("type: mem_path"));
    assert!(prompt.contains("api_audit_file:"));

    let final_turn = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"我是 TimemAi，当前调试标记已在本进程设置为 enabled；记忆和审计路径也已通过 self_tool 确认。"}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final self answer, got {other:?}"),
    };
    assert!(final_turn.final_answer.contains("TimemAi"));
    assert!(final_turn.final_answer.contains("enabled"));
}

#[test]
fn scenario_file_writing_outputs_artifact_and_verifies_content() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("scenario_file_writing"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);

    let _ = core.begin_turn("帮我写一份简短发布检查 md，并确认文件内容。", None);
    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"写入发布检查文档并读回验证。","next_actions":[{"action":"run_bash","intent":"Create and verify a release checklist markdown file.","args":{"cmd":"mkdir -p target/timem_scenario_output; printf %s\\n Release_Check CI_passed Sensitive_scan_passed Real_TTY_smoke_passed > target/timem_scenario_output/release_check.md; sed -n 1,20p target/timem_scenario_output/release_check.md","timeout_ms":5000}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected file evidence prompt, got {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("Release_Check"));
    assert!(prompt.contains("CI_passed"));
    assert!(prompt.contains("Sensitive_scan_passed"));
    assert!(prompt.contains("status: 0"));
    assert!(
        fs::read_to_string("target/timem_scenario_output/release_check.md")
            .unwrap()
            .contains("Real_TTY_smoke_passed")
    );

    let final_turn = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"已生成并读回验证 `target/timem_scenario_output/release_check.md`，内容包含 CI、敏感扫描和真实 TTY smoke 检查项。"}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final file-writing answer, got {other:?}"),
    };
    assert!(final_turn.final_answer.contains("release_check.md"));
    let _ = fs::remove_dir_all("target/timem_scenario_output");
}

#[test]
fn free_talk_field_is_persisted_as_llm_free_talk_slice() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("thought_slice"),
    );
    let _ = core.begin_turn("需要推理一下", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"free_talk":"推导一下","status":"finished","final_answer":"好的"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = core.render_prompt();
    assert!(prompt.contains("Free_talk:"));
    assert!(prompt.contains("Free_talk:\n推导一下"));
}

#[test]
fn free_talk_field_optional_does_not_trigger_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("thought_absent"),
    );
    let _ = core.begin_turn("简单问答", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"好的"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.final_answer, "好的");
    let prompt = core.render_prompt();
    assert!(!prompt.contains("Free_talk:"));
}

#[test]
fn free_talk_object_is_persisted_as_llm_free_talk_slice() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("thought_obj_keep_in_context"),
    );
    let _ = core.begin_turn("需要推理", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"free_talk":{"content":"对象形式的思考","keep_in_context":true},"status":"finished","final_answer":"好的"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = core.render_prompt();
    assert!(prompt.contains("Free_talk:"));
    assert!(prompt.contains("Free_talk:\n对象形式的思考"));
}

#[test]
fn free_talk_object_keep_in_context_false_is_still_persisted() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("thought_obj_not_kept"),
    );
    let _ = core.begin_turn("需要推理", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"free_talk":{"content":"临时思考不保留","keep_in_context":false},"status":"finished","final_answer":"好的"}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = core.render_prompt();
    assert!(prompt.contains("Free_talk:"));
    assert!(prompt.contains("Free_talk:\n临时思考不保留"));
}

#[test]
fn static_prompt_keeps_contracts_concise() {
    let template = include_str!("../../resources/system_prompt/system_prompt.md");
    let protocol_section = include_str!("../../resources/protocol/json/response_protocol.md");
    // Template-level checks
    assert!(template.contains("# Timem Static Prompt"));
    assert!(template.contains("exactly protocol-compliant response"));
    assert!(template.contains("Answer based on collected evidence"));
    assert!(template.contains("Context maintenance"));
    assert!(template.contains("{{RESPONSE_PROTOCOL_SECTION}}"));
    assert!(template.contains("{{CURRENT_PROTOCOL_LANG}}"));
    assert!(template.contains("{{TOOL_CATALOG}}"));
    assert!(template.contains("{{SKILL_HEADERS}}"));
    assert!(template.contains("Do not expose internal mechanisms"));
    assert!(template.contains("memory/storage structure"));
    assert!(template.contains("tool/capability catalog"));
    assert!(!template.contains("runtime implementation details"));
    assert!(!template.contains("resources/response_v1_summary.json"));
    // Protocol section content checks
    assert!(protocol_section.contains("## Response Protocol"));
    assert!(protocol_section.contains("final_answer"));
    assert!(protocol_section.contains("{{RESPONSE_V1_SCHEMA}}"));
    assert!(protocol_section.contains("DO NOT leave or add anything outside"));
}

#[test]
fn rendered_prompt_response_schema_is_injected_from_resource() {
    let mut core = AgentCore::new(
        "## Protocol\n{{RESPONSE_V1_SCHEMA}}",
        profile("aliyun", "qwen-plus"),
        tmp_dir("response_schema_prompt_injection"),
    );
    core.set_response_protocol(ResponseProtocolKind::Markdown);
    let prompt = match core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };

    assert!(!prompt.contains("\"$id\""));
    assert!(prompt.contains("Markdown response sections."));
    assert!(prompt.find("`## Status`").unwrap() < prompt.find("`## Free_talk`").unwrap());
    assert!(prompt.contains("`## Progress`"));
    assert!(prompt.contains("`## Context Compact`"));
    assert!(prompt.contains("`delta_ids`"));
    assert!(!prompt.contains("`slice_ids`"));
    assert!(!prompt.contains("\"context_compact?\""));
    assert!(!prompt.contains("object or array<object>"));
    assert!(!prompt.contains("`## Thought`"));
    assert!(!prompt.contains(
        "\"durable\": \"boolean; optional. Default false. Set true only when this reasoning draft"
    ));
    assert!(prompt.contains("`intent`"));
    assert!(prompt.contains("The top-level response is Markdown, not JSON."));
    assert!(prompt.contains("the individual action blocks"));
    assert!(prompt.contains("inside `## Intermediate_Actions` use JSON objects."));
    assert!(!prompt.contains("`## Thought`"));
    assert!(prompt.contains("`## Intermediate_Actions`"));
    assert!(!prompt.contains("\"json_schema_summary\": \"stale\""));
}

#[test]
fn work_directory_instructions_are_loaded_once_even_if_host_repeats_context() {
    let mut core = AgentCore::new(
        "static",
        profile("aliyun", "qwen-plus"),
        tmp_dir("work_instruction_dedupe"),
    );
    let supporting_context = r#"work_directory_instructions:
These instructions were loaded from files in the current working directory. Follow them while working in that directory.

[BEGIN WORK_DIRECTORY_INSTRUCTION file="AGENTS.md" directory="/tmp/project"]
unique_agents_rule_do_not_repeat_7f9a
[END WORK_DIRECTORY_INSTRUCTION file="AGENTS.md"]

workspace_reference:
unique_workspace_reference_should_remain_visible
"#;

    let first_prompt = match core.begin_turn("first", Some(supporting_context)) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert_eq!(
        first_prompt
            .matches("unique_agents_rule_do_not_repeat_7f9a")
            .count(),
        1
    );
    assert!(first_prompt.contains("unique_workspace_reference_should_remain_visible"));

    let second_prompt = match core.begin_turn("second", Some(supporting_context)) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert_eq!(
        second_prompt
            .matches("unique_agents_rule_do_not_repeat_7f9a")
            .count(),
        1,
        "repeated AGENTS.md/CLAUDE.md content should not be injected into a later prompt delta"
    );
    assert_eq!(
        second_prompt
            .matches("unique_workspace_reference_should_remain_visible")
            .count(),
        2,
        "non-work-instruction supporting context should not be dropped by the dedupe"
    );

    core.clear_dynamic_context();
    let prompt_after_clear = match core.begin_turn("third", Some(supporting_context)) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert_eq!(
        prompt_after_clear
            .matches("unique_agents_rule_do_not_repeat_7f9a")
            .count(),
        1,
        "after clearing dynamic context, work instructions must be visible again"
    );
}

#[test]
fn rendered_static_prompt_preserves_source_rule_order() {
    let mut core = AgentCore::new(
        include_str!("../../resources/system_prompt/system_prompt.md"),
        profile("aliyun", "qwen-plus"),
        tmp_dir("static_prompt_order"),
    );
    let prompt = match core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };

    let role_pos = prompt.find("## Role").expect("role section should render");
    let style_pos = prompt.find("## Soul").expect("Soul section should render");
    let memory_pos = prompt
        .find("## Memory")
        .expect("memory section should render");

    assert!(
        role_pos < style_pos && style_pos < memory_pos,
        "Markdown static prompt should keep source section order"
    );
}

#[test]
fn response_protocol_kind_controls_rendered_protocol_section() {
    let template = include_str!("../../resources/system_prompt/system_prompt.md");
    let mut default_core = AgentCore::new(
        template,
        profile("aliyun", "qwen-plus"),
        tmp_dir("response_protocol_default"),
    );
    let default_prompt = match default_core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert!(default_prompt.contains("The top-level response is XML, not JSON or Markdown."));
    assert!(default_prompt.contains("protocol-compliant response in XML format"));
    assert!(!default_prompt.contains("{{CURRENT_PROTOCOL_LANG}}"));

    let mut markdown_core = AgentCore::new(
        template,
        profile("aliyun", "qwen-plus"),
        tmp_dir("response_protocol_markdown"),
    );
    markdown_core.set_response_protocol(ResponseProtocolKind::Markdown);
    let markdown_prompt = match markdown_core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert!(markdown_prompt.contains("The top-level response is Markdown, not JSON."));
    assert!(markdown_prompt.contains("protocol-compliant response in Markdown format"));
    assert!(markdown_prompt.contains("## Intermediate_Actions"));
    assert!(!markdown_prompt.contains("All your output things MUST BE enclosed"));
    assert!(!markdown_prompt.contains("{{CURRENT_PROTOCOL_LANG}}"));

    let mut json_core = AgentCore::new(
        template,
        profile("aliyun", "qwen-plus"),
        tmp_dir("response_protocol_json"),
    );
    json_core.set_response_protocol(ResponseProtocolKind::Json);
    let json_prompt = match json_core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert!(json_prompt.contains("All your output things MUST BE enclosed"));
    assert!(json_prompt.contains("protocol-compliant response in JSON format"));
    assert!(!json_prompt.contains("{{CURRENT_PROTOCOL_LANG}}"));

    let mut xml_core = AgentCore::new(
        template,
        profile("aliyun", "qwen-plus"),
        tmp_dir("response_protocol_xml"),
    );
    xml_core.set_response_protocol(ResponseProtocolKind::Xml);
    let xml_prompt = match xml_core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert!(xml_prompt.contains("The top-level response is XML, not JSON or Markdown."));
    assert!(xml_prompt.contains("protocol-compliant response in XML format"));
    assert!(xml_prompt.contains("<intermediate_actions>"));
    assert!(!xml_prompt.contains("{{CURRENT_PROTOCOL_LANG}}"));
}

#[test]
fn static_prompt_does_not_handwrite_tool_catalog() {
    let static_prompt = include_str!("../../resources/system_prompt/system_prompt.md");
    assert!(static_prompt.contains("{{TOOL_CATALOG}}"));
    assert!(
        !static_prompt.contains("\"run_bash\":"),
        "static prompt must not hand-maintain executable tool specs; registry injects tool catalog"
    );
}

#[test]
fn no_local_command_host_omits_bash_from_prompt_and_rejects_bash_actions() {
    let mut core = AgentCore::new(
        include_str!("../../resources/system_prompt/system_prompt.md"),
        profile("aliyun", "qwen-plus"),
        tmp_dir("no_local_command_host"),
    );
    core.set_capability_registry(CapabilityRegistry::builtin_for_host(
        CapabilityHostProfile::without_local_command_execution(),
    ));

    let prompt = match core.begin_turn("count files", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert!(!prompt.contains("#### `run_bash`"));
    assert!(!prompt.contains("#### `shell_job_status`"));
    assert!(prompt.contains("#### `memmgr`"));

    let repair_prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"checking files","next_actions":[{"action":"run_bash","intent":"Count files.","args":{"cmd":"rg --files | wc -l","timeout_ms":5000}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected repair NeedModel, got {other:?}"),
    };
    assert!(repair_prompt.contains("Protocol repair request"));
    assert!(repair_prompt.contains("unsupported_action:run_bash"));
    assert!(!repair_prompt.contains("status: 0"));
    assert!(!repair_prompt.contains("output:\n"));
}

#[test]
fn agent_core_stays_terminal_ui_free_for_host_adapters() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let forbidden_ui_crates = [
        "reedline",
        "crossterm",
        "ratatui",
        "termion",
        "dialoguer",
        "indicatif",
        "nu-ansi-term",
        "unicode-width",
    ];
    for forbidden in forbidden_ui_crates {
        assert!(
            !cargo_toml.contains(forbidden),
            "agent_core must not depend on terminal/UI crate `{forbidden}`; keep it reusable by iOS/Web host adapters"
        );
    }

    for entry in fs::read_dir(root.join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(&path).unwrap();
        for forbidden in forbidden_ui_crates {
            assert!(
                !content.contains(forbidden),
                "agent_core source {} references terminal/UI detail `{forbidden}`",
                path.display()
            );
        }
    }

    let lib_rs = fs::read_to_string(root.join("src").join("lib.rs")).unwrap();
    assert!(
        lib_rs.contains("pub extern \"C\" fn timem_core_begin_turn")
            && lib_rs.contains("pub extern \"C\" fn timem_core_apply_model_response"),
        "agent_core should keep a host-adapter ABI for iOS/Web integrations"
    );
    assert!(
        !lib_rs.contains("RuntimeProfileView")
            && !lib_rs.contains("ModelProfileView")
            && !lib_rs.contains("StorageProfileView"),
        "agent_core should export raw profiler data, not shell-specific profiler view strings"
    );

    let profiler_rs = fs::read_to_string(root.join("src").join("profiler.rs")).unwrap();
    for forbidden in [
        "RuntimeProfileView",
        "ModelProfileView",
        "StorageProfileView",
        "format_count(",
        "format_bytes(",
        "format_percent(",
        "format_wait_per_1k_output(",
    ] {
        assert!(
            !profiler_rs.contains(forbidden),
            "agent_core profiler should keep raw structured data; `{forbidden}` belongs in host UI rendering"
        );
    }

    let status_summary_rs = fs::read_to_string(root.join("src").join("status_summary.rs")).unwrap();
    for forbidden in ["compact_token_count", "trim_decimal(", "\"K\"", "\"M\""] {
        assert!(
            !status_summary_rs.contains(forbidden),
            "agent_core status summary should expose raw token values; `{forbidden}` belongs in host UI rendering"
        );
    }

    let lib_rs = fs::read_to_string(root.join("src").join("lib.rs")).unwrap();
    for forbidden in [
        "模型的回复不符合本地协议",
        "请调大 TIMEM_MAX_LLM_OUTPUT",
        "请重试或换一个更具体的问题",
    ] {
        assert!(
            !lib_rs.contains(forbidden),
            "agent_core should keep protocol failure causes structured; terminal copy `{forbidden}` belongs in host UI rendering"
        );
    }
}

#[test]
fn architecture_docs_do_not_bind_bash_capability_to_shell_ui() {
    let docs = include_str!("../../docs/architecture.md");

    assert!(
        !docs.contains("run_bash` is for shell sessions only"),
        "run_bash capability must be gated by host capability, not shell UI type"
    );
    assert!(docs.contains("active host profile"));
    assert!(docs.contains("local command execution"));
    assert!(docs.contains("independent of UI type"));
}

#[test]
fn agent_core_dispatches_owned_structured_topic_events_to_host_sink() {
    let mut core = core_with_builtin_capabilities("notification_sink");
    let _ = core.begin_turn("检查项目", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","free_talk":"先说明一下检查思路。","report_job_progress":"正在检查项目结构","next_actions":[{"action":"run_bash","intent":"列出 Rust 文件","args":{"cmd":"rg --files -g '*.rs'","timeout_ms":5000}}]}"#,
        ),
        usage: usage(),
        model_name: "qwen-plus".to_string(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::NeedsUserApproval { .. }));

    let mut received = Vec::new();
    core.notify_last_topic_events(
        "session_a",
        &mut |events: &[agent_core::CoreTopicEvent]| {
            received.extend_from_slice(events);
        },
    );

    assert_eq!(received.len(), 2);
    assert_eq!(received[0].session_id, "session_a");
    assert_eq!(
        received[0].topic.name,
        agent_core::CORE_TOPIC_MODEL_RESPONSE
    );
    let model_response = received[0].as_model_response().unwrap();
    assert_eq!(model_response.free_talk, "先说明一下检查思路。");
    assert_eq!(model_response.report_job_progress, "正在检查项目结构");
    assert_eq!(model_response.status, "working");
    assert_eq!(model_response.global.working_worker_count, 1);

    let action = received[1].as_action().unwrap();
    assert_eq!(received[1].session_id, "session_a");
    assert_eq!(received[1].topic.name, agent_core::CORE_TOPIC_ACTION);
    assert_eq!(action.intent.as_deref(), Some("列出 Rust 文件"));
    assert_eq!(action.action, "run_bash");
    assert_eq!(action.input["cmd"], "rg --files -g '*.rs'");
    assert_eq!(action.input["timeout_ms"], 5000);
    assert_eq!(
        action.kind,
        agent_core::CoreActionKind::Bash {
            command: "rg --files -g '*.rs'".to_string(),
            mode: "foreground".to_string(),
            interval_ms: None,
            timeout_ms: Some(5000),
            loop_timeout_ms: None,
            once_timeout_ms: None,
        }
    );
    assert!(action.active);
    assert_eq!(action.memory_activity, agent_core::CoreMemoryActivity::None);

    let queued_for_later = received;
    assert_eq!(
        queued_for_later[1].as_action().unwrap().input["cmd"],
        "rg --files -g '*.rs'"
    );
}

#[test]
fn protocol_repair_does_not_publish_invalid_model_response_topic() {
    let mut core = core_with_builtin_capabilities("repair_no_topic");
    let _ = core.begin_turn("检查项目", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"正在检查","next_actions":[{"action":"run_bash","intent":"List.""#,
        ),
        usage: usage(),
        model_name: "qwen-plus".to_string(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::NeedModel { .. }));

    let mut received = Vec::new();
    core.notify_last_topic_events(
        "session_a",
        &mut |events: &[agent_core::CoreTopicEvent]| {
            received.extend_from_slice(events);
        },
    );
    assert!(
        received.is_empty(),
        "repair request should not publish model response topics from malformed output: {received:?}"
    );
}

#[test]
fn external_tool_call_protocol_repairs_without_showing_raw_tool_call() {
    let mut core = core_with_builtin_capabilities("external_tool_call_repair");
    let _ = core.begin_turn("推送远端并检查 CI", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"<tool_call>
{"name": "run_bash", "arguments": {"cmd": "gh run list --limit 1", "timeout_ms": 5000}}
</tool_call>"#,
        ),
        usage: usage(),
        model_name: "qwen-plus".to_string(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected repair NeedModel, got {other:?}"),
    };
    assert!(prompt.contains("external_tool_call_protocol"));
    assert!(prompt.contains("Timem 不能执行这种格式"));

    let mut received = Vec::new();
    core.notify_last_topic_events(
        "session_a",
        &mut |events: &[agent_core::CoreTopicEvent]| {
            received.extend_from_slice(events);
        },
    );
    assert!(
        received.is_empty(),
        "external tool_call repair must not publish raw model response topics: {received:?}"
    );
}

#[test]
fn rendered_static_prompt_examples_avoid_task_like_action_instructions() {
    let mut core = AgentCore::new(
        include_str!("../../resources/system_prompt/system_prompt.md"),
        profile("aliyun", "qwen-plus"),
        tmp_dir("static_prompt_examples_not_task_like"),
    );
    let prompt = match core.begin_turn("请只回答 ok", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };

    assert!(prompt.contains("Examples below are format examples ONLY"));
    for leaked_example_task in [
        "project codename",
        "Get the OS version",
        "Find confirmed memory evidence before answering",
    ] {
        assert!(
            !prompt.contains(leaked_example_task),
            "static prompt example still contains task-like action text: {leaked_example_task}"
        );
    }
}

#[test]
fn rendered_markdown_protocol_examples_do_not_sit_below_protocol_sections() {
    let mut core = AgentCore::new(
        include_str!("../../resources/system_prompt/system_prompt.md"),
        profile("aliyun", "qwen-plus"),
        tmp_dir("static_prompt_example_heading_levels"),
    );
    core.set_response_protocol(ResponseProtocolKind::Markdown);
    let prompt = match core.begin_turn("请只回答 ok", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };

    assert!(prompt.contains("## -------- Example: receive a new input and need actions --------"));
    assert!(prompt
        .contains("## -------- Example:  receive a user task, plan, and start doing --------"));
    assert!(prompt
        .contains("## -------- Example: finish one of user's tasks, compact context --------"));
    assert!(
        !prompt.contains("### Example:"),
        "example headings must not be lower-level than protocol headings such as ## Progress"
    );
    assert!(
        !prompt.contains("\n## Example:"),
        "example headings should have visual separators to avoid ambiguity with protocol sections"
    );
}

#[test]
fn rendered_prompt_tool_catalog_is_generated_from_capability_manifests() {
    let mut core = AgentCore::new(
        "## Tools\n{{TOOL_CATALOG}}\n\n## Skills\n{{SKILL_HEADERS}}",
        profile("aliyun", "qwen-plus"),
        tmp_dir("capability_prompt_catalog"),
    );
    let prompt = match core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };

    assert!(prompt.contains("#### `memmgr`"));
    assert!(prompt.contains("#### `capmgr`"));
    assert!(prompt.contains("#### `run_bash`"));
    assert!(prompt.contains("#### `shell_job_status`"));
    assert!(!prompt.contains("\"release_quality_gate\""));
    assert!(prompt.contains("Unified local memory manager"));
}

#[test]
fn canonical_tool_action_is_validated_through_capability_registry() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("capability_registry_action_parse"),
    );
    let _ = core.begin_turn("查记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"正在查询记忆。","next_actions":[{"action":"memmgr","intent":"Query durable memory through manifest-backed tool.","args":{"type":"durable","op":"query","query":"测试代号","limit":5}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel after action, got {other:?}"),
    };

    assert!(prompt.contains("Action result: memmgr"));
    assert!(prompt.contains("type: durable"));
    assert!(prompt.contains("op: query"));
    assert!(!prompt.contains("Protocol repair request"));
}

#[test]
fn legacy_actions_are_not_visible_or_executable() {
    let mut core = AgentCore::new(
        include_str!("../../resources/system_prompt/system_prompt.md"),
        profile("aliyun", "qwen-plus"),
        tmp_dir("legacy_action_fallback_boundary"),
    );
    let prompt = match core.begin_turn("查旧动作", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert!(prompt.contains("`memmgr`") || prompt.contains("\"memmgr\""));
    assert!(!prompt.contains("\"query_memory\""));

    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"旧动作应被拒绝。","next_actions":[{"action":"query_memory","intent":"Legacy fallback check.","args":{"query":"测试代号","limit":1}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected protocol repair, got {other:?}"),
    };

    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("unsupported_action:query_memory"));
    assert!(!prompt.contains("Action result: memmgr"));
}

#[test]
fn capmgr_load_skill_adds_skill_body_as_action_result() {
    let registry =
        CapabilityRegistry::builtin_with_overlay_dir(release_quality_skill_overlay("capmgr_skill"))
            .unwrap();
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("capmgr_load_skill"),
    );
    core.set_capability_registry(registry);
    let _ = core.begin_turn("准备发布", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"加载发布检查 skill。","next_actions":[{"action":"capmgr","intent":"Load release quality instructions before auditing.","args":{"op":"load","kind":"skill","id":"release_quality_gate"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel after capmgr load, got {other:?}"),
    };

    assert!(prompt.contains("Action result: capmgr"));
    assert!(prompt.contains("kind: skill"));
    assert!(prompt.contains("id: release_quality_gate"));
    assert!(prompt.contains("# Release Quality Gate"));
}

#[test]
fn self_tool_reads_mem_paths_and_about_info() {
    let dir = tmp_dir("self_tool_paths");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("Timem 的记忆路径和版本是什么？", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"查询 Timem 自身信息。","next_actions":[{"action":"self_tool","intent":"查看本次记忆和审计路径。","args":{"type":"mem_path","op":"read"}},{"action":"self_tool","intent":"查看 Timem 软件信息。","args":{"type":"about_me","op":"read"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected model continuation, got {other:?}"),
    };
    assert!(prompt.contains("Action result: self_tool"));
    assert!(prompt.contains("type: mem_path"));
    assert!(prompt.contains("memory_file:"));
    assert!(prompt.contains("api_audit_file:"));
    assert!(prompt.contains("type: about_me"));
    assert!(prompt.contains("name: TimemAi"));
    assert!(prompt.contains("author: TimemAi <phylimo@163.com>"));
    assert!(prompt.contains("project: https://github.com/moliam/TimemAi"));
    assert!(prompt.contains("star_message: Please star https://github.com/moliam/TimemAi"));
    assert!(prompt.contains("pid:"));
    assert!(prompt.contains("current_dir:"));
    assert!(prompt.contains("executable:"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"TimemAi 当前进程信息和记忆路径已确认。"}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final after self_tool evidence, got {other:?}"),
    };
    assert!(final_turn.final_answer.contains("TimemAi"));
}

#[test]
fn self_tool_runtime_configuration_keeps_core_owned_identity() {
    let dir = tmp_dir("self_tool_runtime_config");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let configured_space = dir.join("custom_space");
    let configured_memory = configured_space.join("memory");
    let configured_api_audit = configured_space.join("audit").join("api_audit.json");
    let configured_action_audit = configured_space.join("audit").join("action_audit.json");
    let mut env = BTreeMap::new();
    env.insert("TIMEM_SPACE".to_string(), ".custom_mem".to_string());
    core.configure_self_tool_runtime(
        env,
        SelfToolPaths {
            space_dir: configured_space.clone(),
            memory_dir: configured_memory.clone(),
            memory_file: configured_memory.join("memory.jsonl"),
            scratch_file: configured_memory.join("scratch_notes.jsonl"),
            api_audit_file: configured_api_audit.clone(),
            action_audit_file: configured_action_audit.clone(),
        },
    );

    let _ = core.begin_turn("查看运行时信息", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"查询 Timem 运行时信息。","next_actions":[{"action":"self_tool","intent":"读取运行时路径。","args":{"type":"mem_path","op":"read"}},{"action":"self_tool","intent":"读取 Timem 软件身份。","args":{"type":"about_me","op":"read"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected model continuation, got {other:?}"),
    };

    assert!(prompt.contains(&format!("space_dir: {}", configured_space.display())));
    assert!(prompt.contains(&format!("memory_dir: {}", configured_memory.display())));
    assert!(prompt.contains(&format!(
        "api_audit_file: {}",
        configured_api_audit.display()
    )));
    assert!(prompt.contains(&format!(
        "action_audit_file: {}",
        configured_action_audit.display()
    )));
    assert!(prompt.contains("name: TimemAi"));
    assert!(prompt.contains("author: TimemAi <phylimo@163.com>"));
    assert!(prompt.contains("project: https://github.com/moliam/TimemAi"));
}

#[test]
fn self_tool_env_denies_api_keys_and_allows_non_sensitive_runtime_write() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("self_tool_env"),
    );
    let _ = core.begin_turn("调整 Timem 的运行期环境。", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"检查并更新运行期环境。","next_actions":[{"action":"self_tool","intent":"确认 API key 不会暴露。","args":{"type":"env","op":"read","key":"TIMEM_API_KEY"}},{"action":"self_tool","intent":"设置一个非敏感运行期标记。","args":{"type":"env","op":"write","key":"TIMEM_SELF_TOOL_TEST","value":"enabled"}},{"action":"self_tool","intent":"读取刚设置的运行期标记。","args":{"type":"env","op":"read","key":"TIMEM_SELF_TOOL_TEST"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected model continuation, got {other:?}"),
    };
    assert!(prompt.contains("key: TIMEM_API_KEY"));
    assert!(prompt.contains("error: sensitive_env_denied"));
    assert!(prompt.contains("status: updated_current_process_env"));
    assert!(prompt.contains("key: TIMEM_SELF_TOOL_TEST"));
    assert!(prompt.contains("value: enabled"));
}

#[test]
fn self_tool_env_denies_memory_path_writes_through_core_action() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("self_tool_protected_env"),
    );
    let _ = core.begin_turn("把 Timem 的 data dir 改到另一个目录。", None);
    let attempted_path = "/tmp/timem-should-not-become-data-root";
    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"status":"working","report_job_progress":"尝试更新 Timem 数据目录。","next_actions":[{{"action":"self_tool","intent":"更新 Timem 数据根目录。","args":{{"type":"env","op":"write","key":"TIMEM_DATA_DIR","value":"{attempted_path}"}}}},{{"action":"self_tool","intent":"更新 Timem 记忆空间。","args":{{"type":"env","op":"write","key":"TIMEM_SPACE","value":".other_mem"}}}}]}}"#
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected model continuation, got {other:?}"),
    };
    assert!(prompt.contains("key: TIMEM_DATA_DIR"));
    assert!(prompt.contains("key: TIMEM_SPACE"));
    assert_eq!(prompt.matches("error: protected_env_denied").count(), 2);
    assert!(prompt.contains("reason: memory_path_env_is_startup_only"));
    assert!(!prompt.contains("status: updated_current_process_env"));
    assert!(!prompt.contains(attempted_path));
}

#[test]
fn self_tool_supports_identity_and_process_qa_replay() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("self_tool_identity_process_qa"),
    );
    let _ = core.begin_turn("你是谁？你这个 Timem 进程是什么？", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"working","report_job_progress":"查询 Timem 自身身份和进程。","next_actions":[{"action":"self_tool","intent":"查看 Timem 身份、版本、作者和当前进程。","args":{"type":"about_me","op":"read"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected model continuation, got {other:?}"),
    };
    assert!(prompt.contains("Action result: self_tool"));
    assert!(prompt.contains("type: about_me"));
    assert!(prompt.contains("name: TimemAi"));
    assert!(prompt.contains("version:"));
    assert!(prompt.contains("author: TimemAi <phylimo@163.com>"));
    assert!(prompt.contains("project: https://github.com/moliam/TimemAi"));
    assert!(prompt.contains("star_message: Please star https://github.com/moliam/TimemAi"));
    assert!(prompt.contains("pid:"));
    assert!(prompt.contains("current_dir:"));
    assert!(prompt.contains("executable:"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"我是 TimemAi。当前 self_tool 已返回版本、作者、pid、current_dir 和 executable，可据此说明我正在本机 Timem 进程中运行。"}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("expected final identity answer, got {other:?}"),
    };
    assert!(final_turn.final_answer.contains("TimemAi"));
    assert!(final_turn.final_answer.contains("pid"));
    assert!(final_turn.final_answer.contains("executable"));
}

#[test]
fn capmgr_load_missing_kind_requests_protocol_repair_from_manifest_idl() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("capmgr_missing_fields"),
    );
    let _ = core.begin_turn("准备发布", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"加载 skill。","next_actions":[{"action":"capmgr","intent":"Load missing skill.","args":{"op":"load"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected repair prompt, got {other:?}"),
    };

    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("issue: next_actions[0].input.kind_required_when_op=load"));
    assert!(!prompt.contains("Action result: capmgr"));
}

#[test]
fn capmgr_invalid_values_request_protocol_repair_from_manifest_idl() {
    for (case, payload, expected_issue) in [
        (
            "bad_op",
            r#"{"report_job_progress":"检查 capability。","next_actions":[{"action":"capmgr","intent":"Use an unsupported capability operation.","args":{"op":"remove","kind":"skill","id":"release_quality_gate"}}]}"#,
            "issue: next_actions[0].input.op_unsupported:remove",
        ),
        (
            "bad_kind",
            r#"{"report_job_progress":"检查 capability。","next_actions":[{"action":"capmgr","intent":"Use an unsupported capability kind.","args":{"op":"load","kind":"resource","id":"release_quality_gate"}}]}"#,
            "issue: next_actions[0].input.kind_unsupported:resource",
        ),
    ] {
        let mut core = AgentCore::new(
            "STATIC",
            profile("aliyun", "qwen-plus"),
            tmp_dir(&format!("capmgr_enum_fields_{case}")),
        );
        let _ = core.begin_turn("检查能力", None);
        let step = core.apply_model_response(LlmResponse {
            content: scored(payload),
            model_name: "qwen-plus".to_string(),
            usage: usage(),
            truncated: false,
        });
        let prompt = match step {
            CoreStep::NeedModel { prompt, .. } => prompt,
            other => panic!("expected repair prompt for {case}, got {other:?}"),
        };

        assert!(prompt.contains("Protocol repair request"));
        assert!(!prompt.contains("Action result: capmgr"));
        assert!(prompt.contains(expected_issue));
    }
}

#[test]
fn runtime_overlay_command_tool_executes_with_json_input() {
    let memory_dir = tmp_dir("overlay_command_memory");
    let overlay_dir = tmp_dir("overlay_command_capabilities");
    let tools_dir = overlay_dir.join("tools");
    let scripts_dir = overlay_dir.join("scripts");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::create_dir_all(&scripts_dir).unwrap();
    fs::write(
        tools_dir.join("echo_payload.yaml"),
        r#"kind: tool
id: echo_payload
binding_type: command
binding_name: scripts/echo_payload.sh
summary: Echo the action JSON payload.
description: |
  Use to echo a bounded payload during capability tests.
input_properties:
  text: string
required:
  - text
example_json: |
  {
    "action": "echo_payload",
    "intent": "Echo payload.",
    "args":{"text":"hello"}
  }
"#,
    )
    .unwrap();
    fs::write(
        scripts_dir.join("echo_payload.sh"),
        "payload=$(cat)\nprintf 'overlay_command_ok %s\\n' \"$payload\"\n",
    )
    .unwrap();
    let registry = CapabilityRegistry::builtin_with_overlay_dir(&overlay_dir).unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &memory_dir);
    core.set_capability_registry(registry);
    let _ = core.begin_turn("echo", None);

    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"运行 overlay command。","next_actions":[{"action":"echo_payload","intent":"Echo runtime payload.","args":{"text":"hello"}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected command action result, got {other:?}"),
    };

    assert!(prompt.contains("Action result: echo_payload"));
    assert!(prompt.contains("overlay_command_ok"));
    assert!(prompt.contains("\"text\":\"hello\""));
}

#[test]
fn overlay_command_background_requires_manifest_declared_fields() {
    let memory_dir = tmp_dir("overlay_command_background_reject_memory");
    let overlay_dir = tmp_dir("overlay_command_background_reject_capabilities");
    let tools_dir = overlay_dir.join("tools");
    let scripts_dir = overlay_dir.join("scripts");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::create_dir_all(&scripts_dir).unwrap();
    fs::write(
        tools_dir.join("echo_payload.yaml"),
        r#"kind: tool
id: echo_payload
binding_type: command
binding_name: scripts/echo_payload.sh
summary: Echo the action JSON payload.
description: |
  Use to echo a bounded payload during capability tests.
input_properties:
  text: string
required:
  - text
example_json: |
  {
    "action": "echo_payload",
    "intent": "Echo payload.",
    "args":{"text":"hello"}
  }
"#,
    )
    .unwrap();
    fs::write(scripts_dir.join("echo_payload.sh"), "cat\n").unwrap();
    let registry = CapabilityRegistry::builtin_with_overlay_dir(&overlay_dir).unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &memory_dir);
    core.set_capability_registry(registry);
    let _ = core.begin_turn("echo in background", None);

    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"运行 overlay command。","next_actions":[{"action":"echo_payload","intent":"Echo runtime payload in background.","args":{"text":"hello","background":true}}]}"#
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected manifest repair prompt, got {other:?}"),
    };

    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("next_actions[0].input.background_unsupported"));
    assert!(!prompt.contains("background_started"));
}

#[test]
fn overlay_command_background_job_uses_capmgr_job_status() {
    let memory_dir = tmp_dir("overlay_command_background_memory");
    let overlay_dir = tmp_dir("overlay_command_background_capabilities");
    let tools_dir = overlay_dir.join("tools");
    let scripts_dir = overlay_dir.join("scripts");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::create_dir_all(&scripts_dir).unwrap();
    fs::write(
        tools_dir.join("echo_payload.yaml"),
        r#"kind: tool
id: echo_payload
binding_type: command
binding_name: scripts/echo_payload.sh
summary: Echo the action JSON payload.
description: |
  Use to echo a bounded payload during capability tests.
input_schema: |
  {
    "type": "object",
    "properties": {
      "text": {"type": "string"},
      "background": {"type": "boolean"},
      "mode": {"type": "string", "enum": ["foreground", "background"]},
      "timeout_ms": {"type": "integer"}
    },
    "required": ["text"]
  }
example_json: |
  {
    "action": "echo_payload",
    "intent": "Echo payload.",
    "args":{"text":"hello","background":true}
  }
"#,
    )
    .unwrap();
    fs::write(
        scripts_dir.join("echo_payload.sh"),
        "payload=$(cat)\nprintf 'registered_background_ok %s\\n' \"$payload\"\n",
    )
    .unwrap();
    let registry = CapabilityRegistry::builtin_with_overlay_dir(&overlay_dir).unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &memory_dir);
    core.set_capability_registry(registry);
    let _ = core.begin_turn("echo in background", None);

    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"后台运行 overlay command。","next_actions":[{"action":"echo_payload","intent":"Echo runtime payload in background.","args":{"text":"hello","background":true,"timeout_ms":5000}}]}"#
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected background job result, got {other:?}"),
    };
    assert!(prompt.contains("Action result: echo_payload"));
    assert!(prompt.contains("status: background_started"));
    assert!(prompt.contains("next_action: capmgr op=job_status"));
    let job_id = prompt
        .lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .expect("job id in action result")
        .trim()
        .to_string();

    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"检查后台工具任务。","next_actions":[{{"action":"capmgr","intent":"Wait for registered background tool.","args":{{"op":"job_status","job_id":"{}","timeout_ms":3000}}}}]}}"#,
            job_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected tool job status result, got {other:?}"),
    };
    assert!(prompt.contains("Action result: capmgr"));
    assert!(prompt.contains("op: job_status"));
    assert!(prompt.contains("action: echo_payload"));
    assert!(prompt.contains("state: finished"));
    assert!(prompt.contains("registered_background_ok"));
}

#[test]
fn overlay_command_background_job_can_be_cancelled_through_capmgr() {
    let memory_dir = tmp_dir("overlay_command_cancel_memory");
    let overlay_dir = tmp_dir("overlay_command_cancel_capabilities");
    let tools_dir = overlay_dir.join("tools");
    let scripts_dir = overlay_dir.join("scripts");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::create_dir_all(&scripts_dir).unwrap();
    fs::write(
        tools_dir.join("slow_payload.yaml"),
        r#"kind: tool
id: slow_payload
binding_type: command
binding_name: scripts/slow_payload.sh
summary: Slow payload command.
description: |
  Use to exercise background cancellation in capability tests.
input_schema: |
  {
    "type": "object",
    "properties": {
      "background": {"type": "boolean"},
      "timeout_ms": {"type": "integer"}
    }
  }
example_json: |
  {
    "action": "slow_payload",
    "intent": "Run slowly.",
    "args":{"background":true}
  }
"#,
    )
    .unwrap();
    fs::write(
        scripts_dir.join("slow_payload.sh"),
        "printf 'slow-start\\n'; sleep 10; printf 'slow-done\\n'\n",
    )
    .unwrap();
    let registry = CapabilityRegistry::builtin_with_overlay_dir(&overlay_dir).unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &memory_dir);
    core.set_capability_registry(registry);
    let _ = core.begin_turn("run slow tool in background", None);

    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"后台运行慢工具。","next_actions":[{"action":"slow_payload","intent":"Start slow registered tool.","args":{"background":true,"timeout_ms":5000}}]}"#
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected background job result, got {other:?}"),
    };
    let job_id = prompt
        .lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .expect("job id in action result")
        .trim()
        .to_string();

    let prompt = match core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"取消后台工具任务。","next_actions":[{{"action":"capmgr","intent":"Cancel registered background tool.","args":{{"op":"job_cancel","job_id":"{}"}}}}]}}"#,
            job_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected tool job cancel result, got {other:?}"),
    };
    assert!(prompt.contains("Action result: capmgr"));
    assert!(prompt.contains("op: job_cancel"));
    assert!(prompt.contains("action: slow_payload"));
    assert!(prompt.contains("state: cancelled"));
}

#[test]
fn finished_with_actions_requests_repair_and_executes_nothing() {
    let memory_dir = tmp_dir("finished_actions_repair");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &memory_dir);
    core.set_capability_registry(CapabilityRegistry::builtin());
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("完成任务", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"status":"finished","final_answer":"任务已完成","next_actions":[{"action":"run_bash","intent":"Verify.","args":{"cmd":"true","timeout_ms":2000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected protocol repair, got {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("status_finished_must_not_include_next_actions"));
    assert!(prompt.contains("不能同时包含"));
    assert!(!prompt.contains("Action result: run_bash"));
    let action_audit_text =
        fs::read_to_string(memory_dir.join("audit").join("action_audit.json")).unwrap();
    assert!(!action_audit_text.contains(r#""action":"run_bash""#));
    assert!(!action_audit_text.contains(r#""status":"completed""#));
}

#[test]
fn finished_with_multiple_or_non_bash_actions_requests_same_repair() {
    for (case, payload) in [
        (
            "multiple",
            r#"{"status":"finished","final_answer":"任务已完成","next_actions":[{"action":"run_bash","intent":"First.","args":{"cmd":"true","timeout_ms":2000}},{"action":"run_bash","intent":"Last.","args":{"cmd":"true","timeout_ms":2000}}]}"#,
        ),
        (
            "self_tool",
            r#"{"status":"finished","final_answer":"好的，以下是我的版本信息。","next_actions":[{"action":"self_tool","intent":"获取 Timem 的版本和运行时信息","args":{"type":"about_me","op":"read"}}]}"#,
        ),
    ] {
        let mut core = core_with_builtin_capabilities(&format!("finished_actions_repair_{case}"));
        core.set_bash_approval_mode(BashApprovalMode::Approve);
        let _ = core.begin_turn("完成任务", None);
        let step = core.apply_model_response(LlmResponse {
            content: scored(payload),
            model_name: "qwen-plus".to_string(),
            usage: usage(),
            truncated: false,
        });
        let prompt = match step {
            CoreStep::NeedModel { prompt, .. } => prompt,
            other => panic!("expected protocol repair for {case}, got {other:?}"),
        };
        assert!(prompt.contains("Protocol repair request"));
        assert!(prompt.contains("status_finished_must_not_include_next_actions"));
        assert!(!prompt.contains("Action result: run_bash"));
        assert!(!prompt.contains("Action result: self_tool"));
    }
}

#[test]
fn prose_then_final_answer_only_json_extracts_payload() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("prose_final_answer_only"),
    );
    let _ = core.begin_turn("你叫什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"你叫张三！

{"status":"finished","final_answer":"你叫**张三**！"}
"#,
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.final_answer.contains("张三"));
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn markdown_fenced_final_answer_only_json_extracts_payload() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("fenced_final_answer_only"),
    );
    let _ = core.begin_turn("秘密是什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            "```json\n{\"status\":\"finished\",\"final_answer\":\"ABC = 123456\"}\n```",
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.final_answer.contains("ABC = 123456"));
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn prose_with_json_reference_before_actual_response() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("prose_json_ref"),
    );
    let _ = core.begin_turn("explain json", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            "JSON looks like {\"key\":\"value\"} and is widely used.\n\n{\"status\":\"finished\",\"final_answer\":\"JSON is a data format.\"}",
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.final_answer.contains("JSON is a data format"));
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn final_answer_containing_json_code_example() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("final_answer_json_code"),
    );
    let _ = core.begin_turn("show json example", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"status":"finished","final_answer":"Use this format:\n```json\n{\"name\": \"test\"}\n```"}"#,
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.final_answer.contains("Use this format"));
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn prose_with_fake_envelope_keys_picks_last_valid_json() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("fake_envelope"),
    );
    let _ = core.begin_turn("test", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            "Example:\n{\"status\":\"finished\",\"final_answer\":\"wrong\"}\n\nActual:\n{\"status\":\"finished\",\"final_answer\":\"correct answer\"}",
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.final_answer.contains("correct answer"));
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn prose_with_curly_braces_in_code_does_not_confuse_parser() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("curly_in_code"),
    );
    let _ = core.begin_turn("rust code", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            "In Rust: fn main() { println!(\"hello\"); }\n\n{\"status\":\"finished\",\"final_answer\":\"Rust uses curly braces for blocks.\"}",
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.final_answer.contains("curly braces"));
    assert_eq!(final_turn.repair_issue, None);
}

#[test]
fn array_of_actions_auto_wrapped_as_next_actions() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("array_actions"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("find files", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"[{"action":"run_bash","intent":"Find files.","args":{"cmd":"echo ok","timeout_ms":5000}}]"#,
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel with action results, got: {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("ok"));
}

#[test]
fn array_of_multiple_actions_auto_wrapped() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("array_multi_actions"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("multi", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"[{"action":"run_bash","intent":"First.","args":{"cmd":"echo one","timeout_ms":5000}},{"action":"run_bash","intent":"Second.","args":{"cmd":"echo two","timeout_ms":5000}}]"#,
        ),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got: {other:?}"),
    };
    assert!(prompt.contains("one"));
    assert!(prompt.contains("two"));
}

#[test]
fn array_without_action_key_still_rejected() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("array_no_action"),
    );
    let _ = core.begin_turn("bad", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"[{"foo":"bar"}]"#),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel (repair), got: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair"));
}

fn perf_guard_enabled() -> bool {
    std::env::var("TIMEM_PERF_GUARD").ok().as_deref() == Some("1")
}

fn assert_perf_under(label: &str, started: Instant, budget: Duration) {
    if perf_guard_enabled() {
        let elapsed = started.elapsed();
        assert!(
            elapsed <= budget,
            "{label} took {elapsed:?}, expected <= {budget:?}"
        );
    }
}

#[test]
fn performance_guard_large_context_prompt_render_is_bounded() {
    let mut core = AgentCore::new(
        include_str!("../../resources/system_prompt/system_prompt.md"),
        profile("aliyun", "qwen-plus"),
        tmp_dir("perf_large_prompt_render"),
    );
    let repeated_context = "local evidence ".repeat(120);
    for idx in 0..160 {
        let _ = core.begin_turn(&format!("user turn {idx}: {repeated_context}"), None);
        let step = core.apply_model_response(LlmResponse {
            content: scored(&format!(
                r#"{{"status":"finished","final_answer":"assistant turn {idx}: done"}}"#
            )),
            model_name: "qwen-plus".to_string(),
            usage: usage(),
            truncated: false,
        });
        assert!(matches!(step, CoreStep::Final(_)));
    }

    let started = Instant::now();
    let mut total_len = 0usize;
    for _ in 0..30 {
        total_len += core.render_prompt().len();
    }
    assert!(total_len > 1_000_000);
    assert_perf_under(
        "large context prompt render x30",
        started,
        Duration::from_millis(1500),
    );
}

#[test]
fn performance_guard_topic_generation_for_many_actions_is_bounded() {
    #[derive(Default)]
    struct CountSink {
        count: usize,
    }
    impl agent_core::CoreTopicEventSink for CountSink {
        fn on_core_topic_events(&mut self, events: &[agent_core::CoreTopicEvent]) {
            self.count += events.len();
        }
    }

    let mut core = core_with_builtin_capabilities("perf_many_action_topics");
    let actions = (0..150)
        .map(|idx| {
            format!(
                r#"{{"action":"self_tool","intent":"Read runtime info {idx}.","args":{{"type":"about_me","op":"read"}}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    let _ = core.begin_turn("emit many action topics", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(&format!(
            r#"{{"status":"working","report_job_progress":"checking many actions","next_actions":[{actions}]}}"#
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::NeedModel { .. }));

    let started = Instant::now();
    let mut sink = CountSink::default();
    for _ in 0..200 {
        core.notify_last_topic_events("session_perf", &mut sink);
    }
    assert!(sink.count >= 30_000);
    assert_perf_under(
        "topic generation for many actions x200",
        started,
        Duration::from_millis(500),
    );
}
