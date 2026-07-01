use agent_core::capability::CapabilityRegistry;
use agent_core::{
    AgentCore, BashApprovalMode, CoreProfile, CoreStep, LlmResponse, MemGuard, UsageStats,
};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;
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
    assert!(first.contains("[BEGIN SEGMENT 0: prompt_0]"));
    assert!(!first.contains("________"));
    assert!(first.contains("[END SEGMENT 0: prompt_0]\n[BEGIN SEGMENT 1: prompt_delta]"));
    assert!(first.contains("delta_id: pd_"));
    assert!(first.contains("slice_id: ps_"));
    assert!(first.contains("_s001"));
    assert!(first.contains("slice: 1/1"));
    assert!(first.contains("prompt_type: user_question\n"));
    assert!(first.contains("\ntime: "));
    assert!(!first.contains("{\"segment_type\""));

    let second = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"你好","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(_) => core.render_prompt(),
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(second.contains("[BEGIN SEGMENT 0: prompt_0]\nSTATIC\n[END SEGMENT 0: prompt_0]"));
    assert!(second.contains("User question:\n你好"));
    assert!(second.contains("prompt_type: llm_response"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","intent":"Need evidence.","input":{"query":"x","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let CoreStep::RoundLimitReached { max_rounds } = step else {
        panic!("unexpected step: {step:?}");
    };
    assert_eq!(max_rounds, 1);
    let limited_prompt = core.render_prompt();
    assert!(limited_prompt.contains("Action result: query_memory"));

    let step = core.continue_after_round_limit();
    let CoreStep::NeedModel {
        prompt,
        rounds_remaining,
    } = step
    else {
        panic!("unexpected step: {step:?}");
    };
    assert_eq!(rounds_remaining, 50);
    assert!(prompt.contains("User question:\n需要两步完成"));
    assert!(prompt.contains("Action result: query_memory"));
    assert!(prompt.contains("Runtime round budget continued by user."));
    assert!(prompt.contains("rounds_remaining: 50"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","intent":"Need evidence after continuation.","input":{"query":"x","limit":1}}]}"#),
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
    assert!(prompt.contains("Action result: query_memory"));
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

    assert!(prompt.contains("[BEGIN SEGMENT 1: prompt_delta]"));
    assert!(prompt.contains("[BEGIN SEGMENT 2: prompt_delta]"));
    assert!(prompt.contains("delta_id: pd_"));
    assert!(prompt.contains("slice_id: ps_"));
    assert!(prompt.contains("slice: 1/"));
    assert!(prompt.contains("slice: 2/"));
    assert!(prompt.matches("prompt_type: user_question").count() > 1);
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
        content: scored(r#"{"thought":"先分析","report_job_progress":"结论","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = core.render_prompt();
    let delta_ids = field_values(&prompt, "delta_id");
    let slice_ids = field_values(&prompt, "slice_id");
    let slice_markers = field_values(&prompt, "slice");

    assert_eq!(delta_ids.len(), 3);
    assert_eq!(slice_ids.len(), 3);
    assert_ne!(delta_ids[0], delta_ids[1]);
    assert_eq!(delta_ids[1], delta_ids[2]);
    assert!(slice_ids[1].ends_with("_s001"));
    assert!(slice_ids[2].ends_with("_s002"));
    assert_eq!(slice_markers[1], "1/2");
    assert_eq!(slice_markers[2], "2/2");
    assert!(prompt.contains("prompt_type: llm_thought"));
    assert!(prompt.contains("prompt_type: llm_response"));
}

#[test]
fn missing_durable_score_does_not_block_valid_actions() {
    let dir = tmp_dir("durable_ctx_score_not_required");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"user_name","created_at_ms":1,"content":"用户的名字是李默"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我叫什么名字？", None);

    let step = core.apply_model_response(LlmResponse {
        content: r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","intent":"查找已确认的用户姓名记忆。","input":{"query":"名字","limit":5}}],"acceptance_check":{"is_satisfied":false,"missing_info":["已确认的用户姓名记忆"]}}"#.to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: query_memory"));
    assert!(prompt.contains("用户的名字是李默"));
    assert!(!prompt.contains("Protocol repair request"));
}

#[test]
fn prompt_rendering_does_not_expose_durable_ctx_score() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("durable_ctx_not_rendered"),
    );
    let prompt = match core.begin_turn("不要记住：生日这个词只是测试", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("User question:\n不要记住：生日这个词只是测试"));
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
            r#"{{"report_job_progress":"","next_actions":[{{"action":"prompt_shrink","intent":"Remove stale user question delta.","input":{{"delta_ids":["{}"]}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["shrink result"]}}}}"#,
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
    assert!(prompt.contains("Action result: prompt_shrink"));
    assert!(prompt.contains("removed_delta_count: 1"));
    let shrunk_tokens_estimate = first_field_value(&prompt, "shrunk_tokens_estimate")
        .parse::<u32>()
        .unwrap();
    assert!(shrunk_tokens_estimate > 1);
    assert!(!prompt.contains("REMOVE_THIS_DELTA"));

    let final_step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"done","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
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
            r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Remove stale user question delta.","input":{{"type":"context","op":"shrink","delta_ids":["{}"]}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["shrink result"]}}}}"#,
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
fn prompt_shrink_can_hide_specific_slice_by_slice_id() {
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
    let slice_id = first_field_value(&prompt, "slice_id");
    assert!(slice_id.ends_with("_s001"));
    assert!(prompt.contains("SLICE_ONE_ONLY"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"prompt_shrink","intent":"Hide one rendered prompt slice.","input":{{"slice_ids":["{}"]}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["shrink result"]}}}}"#,
            slice_id
        )),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: prompt_shrink"));
    assert!(prompt.contains("hidden_slice_count: 1"));
    let shrunk_tokens_estimate = first_field_value(&prompt, "shrunk_tokens_estimate")
        .parse::<u32>()
        .unwrap();
    assert_eq!(shrunk_tokens_estimate, 3000);
    assert!(!prompt.contains(&format!("slice_id: {}", slice_id)));
    assert!(!prompt.contains("SLICE_ONE_ONLY"));

    let final_step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"done","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
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
    let prompt0 = prompt
        .split("[END SEGMENT 0: prompt_0]")
        .next()
        .unwrap_or("");

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
    assert!(
        prompt.contains("[BEGIN SEGMENT 0: prompt_0]\nSTATIC_GLOBAL\n[END SEGMENT 0: prompt_0]")
    );
    assert!(!prompt.contains("old task context"));
    assert!(!prompt.contains("[BEGIN SEGMENT 1: prompt_delta]"));
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
        content: scored(
            r#"{"report_job_progress":"seeded","continue":false,"acceptance_check":{"is_satisfied":true}}"#,
        ),
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
        content: scored(
            r#"{"report_job_progress":"seeded","continue":false,"acceptance_check":{"is_satisfied":true}}"#,
        ),
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
        content: scored(
            r#"{"report_job_progress":"seeded","continue":false,"acceptance_check":{"is_satisfied":true}}"#,
        ),
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
        content: scored(
            r#"{"report_job_progress":"seeded","continue":false,"acceptance_check":{"is_satisfied":true}}"#,
        ),
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
        r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Remove visible dynamic context after checkpointing.","input":{{"type":"context","op":"shrink","delta_ids":{}}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["shrink result"]}}}}"#,
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
        content: scored(
            r#"{"report_job_progress":"压缩已完成，可以继续对话。","continue":false,"acceptance_check":{"is_satisfied":true}}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage_with_prompt_tokens(1_200),
        truncated: false,
    });
    let final_turn = match final_step {
        CoreStep::Final(final_turn) => final_turn,
        other => panic!("unexpected step after shrink follow-up: {other:?}"),
    };
    assert_eq!(final_turn.response_to_user, "压缩已完成，可以继续对话。");
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
    let _ = core.begin_turn("我叫默默", None);
    let final_step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"记住了","continue":false,"memory_candidates":[{"content":"用户叫默默"}],"acceptance_check":{"is_satisfied":true}}"#),
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
    assert!(stored.contains("用户叫默默"));
}

#[test]
fn query_memory_action_returns_action_result_delta() {
    let dir = tmp_dir("memory_query");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"用户儿子的生日是6月12日"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我儿子的生日是什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","intent":"test action","input":{"query":"儿子 生日","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: query_memory"));
    assert!(prompt.contains("6月12日"));
}

#[test]
fn memmgr_durable_query_returns_action_result_delta() {
    let dir = tmp_dir("memmgr_durable_query");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"updated_at_ms":1,"version":1,"content":"用户儿子的生日是6月12日"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我儿子的生日是什么", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test durable query","input":{"type":"durable","op":"query","query":"儿子 生日","limit":5}}]}"#),
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
    assert!(prompt.contains("6月12日"));
}

#[test]
fn memmgr_raw_chat_query_reads_persisted_chat_records() {
    let root = tmp_dir("memmgr_raw_chat");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        root.join("api_audit.jsonl"),
        r#"{"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"我昨天提到了蓝色雨伞"}
{"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"我记下了蓝色雨伞这个说法。"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我之前说过什么物品", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"test raw chat query","input":{"type":"raw_chat","op":"query","query":"蓝色雨伞","limit":5}}]}"#),
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
    assert!(prompt.contains("蓝色雨伞"));
}

#[test]
fn plain_text_after_repair_failure_is_shown_as_final_answer() {
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), tmp_dir("repair"));
    let _ = core.begin_turn("你好", None);
    let step = core.apply_model_response(LlmResponse {
        content: "not json".to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));

    let step = core.apply_model_response(LlmResponse {
        content: "提交成功！\n\n**commit `a91a7b8`** — `refactor: simplify app_context_policy`"
            .to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(
        final_turn.response_to_user,
        "提交成功！\n\n**commit `a91a7b8`** — `refactor: simplify app_context_policy`"
    );
    assert_eq!(
        final_turn.repair_issue.as_deref(),
        Some("invalid_json_plain_text_fallback")
    );
}

#[test]
fn malformed_action_like_response_still_gets_protocol_error_after_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("repair_action_like"),
    );
    let _ = core.begin_turn("你好", None);
    let step = core.apply_model_response(LlmResponse {
        content: "not json".to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::NeedModel { .. }));

    let step = core.apply_model_response(LlmResponse {
        content: r#"next_actions: [{"action":"run_bash","input":{"command":"git commit"}}]"#
            .to_string(),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(
        final_turn.response_to_user,
        "模型的回复不符合本地协议，已拦截原始报文展示。原因：invalid_json。请重试或换一个更具体的问题。"
    );
    assert_eq!(final_turn.repair_issue.as_deref(), Some("invalid_json"));
}

#[test]
fn truncated_response_requests_output_limit_repair_in_noninteractive_path() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("truncated_repair"),
    );
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
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("truncated_model_output"));
    assert!(prompt.contains("max output token limit"));
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
    assert!(final_turn.response_to_user.contains("API 提供商"));
    assert!(final_turn
        .response_to_user
        .contains("stop_reason=max_tokens"));
    assert!(final_turn.response_to_user.contains("TIMEM_MAX_LLM_OUTPUT"));
    assert_eq!(final_turn.repair_issue.as_deref(), Some("invalid_json"));
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
        content: scored(r#"{"thought":"Round 7","report_job_progress":"","next_actions":[{"action":"run_bash","intent":"old action","input":{"command":"uptime"}}],"acceptance_check":{"is_satisfied":false,"missing_info":["final"]}}

[BEGIN SEGMENT 18: prompt_delta]
prompt_type: result_of_llm_action
Action result: run_bash
command: uptime
status: 0
output:
ok
[END SEGMENT 18: prompt_delta]

{
  "thought": "Final summary",
  "report_job_progress": "只展示最终摘要。","continue":false,"acceptance_check": {
    "is_satisfied": true
  }
}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.response_to_user, "只展示最终摘要。");
    assert!(!final_turn.response_to_user.contains("[BEGIN SEGMENT"));
    assert!(!final_turn.response_to_user.contains("next_actions"));
}

#[test]
fn prose_then_markdown_fenced_json_extracts_payload() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("prose_then_fenced_json"),
    );
    let _ = core.begin_turn("把下载目录视频做 3 倍加速", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"转码已在后台顺利运行，进度正常。

```json
{
  "report_job_progress": "转码已在后台顺利运行，输出文件：`~/Videos/example_3x.mp4`。","continue":false,"acceptance_check": {
    "is_satisfied": true
  }
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
    assert!(final_turn.response_to_user.contains("example_3x.mp4"));
    assert!(!final_turn.response_to_user.contains("```json"));
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
  "thought": "The answer is available from chat history.",
  "report_job_progress": "根据聊天记录，你问"当前目录的代码量，rust 代码有多少行？"这个问题的时间是今天（2026-06-23）17:46:36 左右。",
  "continue": false,
  "acceptance_check": {
    "is_satisfied": true
  }
}"#),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(final_turn.response_to_user.contains("17:46:36"));
    assert!(final_turn.response_to_user.contains("\"当前目录的代码量"));
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
        "thought": "Symbols should remain normal text.",
        "report_job_progress": text,
        "continue": false,
        "acceptance_check": {"is_satisfied": true}
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
    assert_eq!(final_turn.response_to_user, text);
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
        content: scored(r#"{"report_job_progress":"tab:\tend\nline2\r\nunicode:\u4f60\u597d path:C:\\Users\\me\\file quote:\"ok\" slash:\/ regex:\\d+","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(
        final_turn.response_to_user,
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_write","intent":"Store escaped text exactly after JSON decoding.","input":{"content":"tab:\tend\nline2\r\nunicode:\u4f60\u597d path:C:\\Users\\me\\file quote:\"ok\" slash:\/ regex:\\d+"}}],"acceptance_check":{"is_satisfied":false,"missing_info":["memory write result"]}}"#),
        model_name: "aws-claude-sonnet-4-6".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_write"));
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
      "action": "chat_history_query",
      "intent": "查找用户说过的"当前目录"相关问题",
      "input": {
        "query": "当前目录的代码量，"rust" 代码有多少行？",
        "limit": 5
      }
    }
  ],
  "acceptance_check": {
    "is_satisfied": false,
    "missing_info": ["chat history evidence"]
  }
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
    assert!(prompt.contains("Action result: chat_history_query"));
    assert!(prompt.contains("当前目录"));
}

#[test]
fn malformed_complex_protocol_is_blocked_without_raw_leak() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("custom", "aws-claude-sonnet-4-6"),
        tmp_dir("malformed_complex_protocol"),
    );
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
    assert!(final_turn.response_to_user.contains("已拦截原始报文展示"));
    assert!(!final_turn.response_to_user.contains("dangling"));
    assert!(!final_turn.response_to_user.contains("```"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","intent":"test action","input":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.query_required"));
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
            r#"{"report_job_progress":"备份完成。现在继续查证。","next_actions":[{"action":"query_memory","intent":"查找相关记忆。","input":{"query":"项目状态","limit":1}}],"acceptance_check":{"is_satisfied":false,"missing_info":["memory evidence"]}}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("prompt_type: llm_progress"));
    assert!(prompt.contains("备份完成。现在继续查证。"));
    assert!(prompt.contains("上轮回复没有写 continue"));
    assert!(prompt.contains("Action result: query_memory"));
}

#[test]
fn next_action_requires_intent_for_ui_status() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("missing_intent"),
    );
    let _ = core.begin_turn("查记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","input":{"query":"名字","limit":5}}]}"#),
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
    assert!(!prompt.contains("Action result: query_memory"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"delete_file","intent":"test action","input":{"path":"/tmp/x"}}]}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_write","intent":"Create a task checkpoint.","input":{"type":"notes","label":"release checkpoint","content":"continue this task later"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: scratch_write"));
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
        content: scored(format!(r#"{{"report_job_progress":"","next_actions":[{{"action":"scratch_read","intent":"Read saved checkpoint by id.","input":{{"id":"{}"}}}}]}}"#, scratch_id)),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: scratch_read"));
    assert!(prompt.contains("found: true"));
    assert!(prompt.contains("continue this task later"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(format!(r#"{{"report_job_progress":"","next_actions":[{{"action":"scratch_delete","intent":"Remove completed checkpoint.","input":{{"id":"{}"}}}}]}}"#, scratch_id)),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: scratch_delete"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Create a task checkpoint.","input":{"type":"scratch","op":"write","kind":"notes","label":"release checkpoint","content":"continue this task later"}}]}"#),
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
        content: scored(format!(r#"{{"report_job_progress":"","next_actions":[{{"action":"memmgr","intent":"Read saved checkpoint by id.","input":{{"type":"scratch","op":"read","id":"{}"}}}}]}}"#, scratch_id)),
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
fn memmgr_requires_type_and_op_for_protocol_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("memmgr_missing_op"),
    );
    let _ = core.begin_turn("查一下记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memmgr","intent":"Broken memory action","input":{"type":"durable","query":"名字"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("next_actions[0].input.op_required"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_query","intent":"List recent checkpoints.","input":{"query":"","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: scratch_query"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_write","intent":"Create empty checkpoint.","input":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.type_required"));
    assert!(!core.scratch_file().exists());

    let _ = core.begin_turn("写一条没有标签的草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_write","intent":"Create unlabeled checkpoint.","input":{"type":"notes","content":"x"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.label_required"));

    let _ = core.begin_turn("读取一条没有 id 的草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_read","intent":"Read checkpoint.","input":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.id_required"));

    let _ = core.begin_turn("删除一条没有 id 的草稿", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_delete","intent":"Delete checkpoint.","input":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.id_required"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_delete","intent":"Delete missing checkpoint.","input":{"id":"scratch_missing"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: scratch_delete"));
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
            r#"{{"report_job_progress":"","next_actions":[{{"action":"scratch_write","intent":"Offload visible prompt delta by id.","input":{{"type":"context_offload","label":"large investigation context","delta_ids":["{}"]}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["scratch write result"]}}}}"#,
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
    assert!(prompt.contains("Action result: scratch_write"));
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
            r#"{{"report_job_progress":"","next_actions":[{{"action":"scratch_read","intent":"Read offloaded prompt context.","input":{{"id":"{}"}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["scratch content"]}}}}"#,
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
    assert!(prompt.contains("Action result: scratch_read"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_write","intent":"Attempt invalid offload.","input":{"type":"context_offload","label":"bad refs","delta_ids":["pd_missing"]}}],"acceptance_check":{"is_satisfied":false,"missing_info":["scratch write result"]}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: scratch_write"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_write","intent":"Missing refs.","input":{"type":"context_offload","label":"missing refs"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.prompt_refs_required"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_write","intent":"test action","input":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("input.content_required"));
}

#[test]
fn query_memory_does_not_expand_semantic_aliases() {
    let dir = tmp_dir("no_semantic_alias");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"用户的名字是默默"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我叫什么名字", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","intent":"test action","input":{"query":"user's name","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: query_memory"));
    assert!(prompt.contains("results: none"));
    assert!(!prompt.contains("用户的名字是默默"));
}

#[test]
fn query_memory_exposes_version_for_conflict_safe_updates() {
    let dir = tmp_dir("query_memory_version");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":11,"content":"用户的名字是默默"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("查名字记忆版本", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","intent":"Find versioned durable memory before update.","input":{"query":"名字","limit":5}}]}"#),
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
        r#"{"id":"m1","created_at_ms":1,"content":"用户的名字是默默"}
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
    assert!(prompt.contains("prompt_type: user_question"));
    assert!(prompt.contains("prompt_type: result_of_llm_action"));
    assert!(prompt.contains("Action result: runtime_memory_precheck"));
    assert!(prompt.contains("lexical_results: none"));
    assert!(prompt.contains("recent_memory_evidence"));
    assert!(prompt.contains("用户的名字是默默"));
}

#[test]
fn memory_lookup_precheck_is_not_added_without_runtime_marker() {
    let dir = tmp_dir("no_runtime_memory_precheck");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"用户的名字是默默"}
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
        r#"{"id":"m1","created_at_ms":11,"content":"用户的名字是默默"}
{"id":"m2","created_at_ms":22,"content":"用户儿子的生日是6月12日"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我最早什么时候告诉你名字的", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"sql_read","intent":"test action","input":{"sql":"SELECT content, created_at_ms FROM memories WHERE content LIKE ? ORDER BY created_at_ms ASC LIMIT 5","params":["%名字%"]}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: sql_read"));
    assert!(prompt.contains("content=用户的名字是默默"));
    assert!(prompt.contains("created_at_ms=11"));
}

#[test]
fn memory_sql_query_reads_memory_versions_and_normalizes_legacy_rows() {
    let dir = tmp_dir("sql_memory_versions");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":11,"content":"用户的名字是默默"}
{"id":"m2","created_at_ms":22,"updated_at_ms":33,"version":4,"content":"用户喜欢 Rust"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("查记忆版本", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"Read durable memory versions for safe update.","input":{"sql":"SELECT id, version, updated_at_ms, content FROM memories ORDER BY created_at_ms ASC","limit":5}}]}"#),
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
        r#"{"id":"m1","created_at_ms":11,"content":"用户的名字是默默"}
{"id":"m2","created_at_ms":22,"content":"用户儿子的生日是6月12日"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("按时间查名字", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"sql_read","intent":"test action","input":{"sql":"WITH\nmatched AS (SELECT content, created_at_ms FROM memories WHERE content LIKE ?) SELECT content, created_at_ms FROM matched ORDER BY created_at_ms ASC LIMIT 5","params":["%名字%"]}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: sql_read"));
    assert!(prompt.contains("content=用户的名字是默默"));
    assert!(prompt.contains("created_at_ms=11"));
}

#[test]
fn sql_read_rejects_write_statement() {
    let dir = tmp_dir("sql_reject_write");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"用户的名字是默默"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("改记忆", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"sql_read","intent":"test action","input":{"sql":"UPDATE memories SET content='x' LIMIT 1"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: sql_read"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"test action","input":{"sql":"SELECT content FROM memories ORDER BY created_at_ms ASC","limit":1}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_sql_query"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"sql_read","intent":"test action","input":{"sql":"SELECT name FROM sqlite_master LIMIT 5"}}]}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_schema","intent":"查看记忆结构"}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_schema"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"test action","input":{"sql":"PRAGMA table_info(memories)","limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_sql_query"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"test action","input":{"sql":"PRAGMA table_info(chat_messages)","limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_sql_query"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"test action","input":{"sql":"PRAGMA table_info(sqlite_master)","limit":20}}]}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"sql_read","intent":"test action","input":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.sql_required"));
}

#[test]
fn memory_sql_query_requires_params_for_placeholders() {
    let dir = tmp_dir("sql_missing_params");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"用户的名字是默默"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我叫什么名字", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"test action","input":{"sql":"SELECT content FROM memories WHERE content LIKE ? ORDER BY created_at_ms ASC","limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.params_count_mismatch"));
    assert!(!prompt.contains("sql_query_failed"));
}

#[test]
fn memory_sql_query_rejects_extra_params_for_placeholders() {
    let dir = tmp_dir("sql_extra_params");
    fs::write(
        dir.join("memory.jsonl"),
        r#"{"id":"m1","created_at_ms":1,"content":"用户的名字是默默"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我叫什么名字", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"test action","input":{"sql":"SELECT content FROM memories WHERE content LIKE ? ORDER BY created_at_ms ASC","params":["%name:%","%my name is%","%I am%"],"limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.params_count_mismatch expected=1 actual=3"));
    assert!(!prompt.contains("sql_query_failed"));
}

#[test]
fn chat_history_query_reads_persisted_chat_records() {
    let root = tmp_dir("chat_history_persisted");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("api_audit.jsonl");
    fs::write(
        &audit_file,
        r#"{"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"我昨天提到了蓝色雨伞"}
{"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"我记下了蓝色雨伞这个说法。"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("我之前说过什么物品", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"chat_history_query","intent":"查询聊天记录","input":{"query":"蓝色雨伞","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: chat_history_query"));
    assert!(prompt.contains("chat_records"));
    assert!(prompt.contains("source=chat_record"));
    assert!(prompt.contains("shell_old"));
    assert!(prompt.contains("蓝色雨伞"));
    assert!(prompt.contains("我记下了蓝色雨伞这个说法"));
}

#[test]
fn chat_history_query_keeps_current_prompt_delta_fallback() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("chat_history"),
    );
    let _ = core.begin_turn("第一轮我说了蓝色雨伞", None);
    let _ = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"收到","continue":false}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let _ = core.begin_turn("我刚才说了什么物品", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"chat_history_query","intent":"查询聊天记录","input":{"query":"蓝色雨伞","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: chat_history_query"));
    assert!(prompt.contains("蓝色雨伞"));
    assert!(prompt.contains("current_prompt_deltas"));
    assert!(prompt.contains("source=prompt_delta"));
}

#[test]
fn chat_history_query_empty_query_lists_recent_records() {
    let root = tmp_dir("chat_history_recent_empty");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("api_audit.jsonl");
    fs::write(
        &audit_file,
        r#"{"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"第一条历史"}
{"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"第一条回复"}
{"type":"turn_start","session":"shell_old","turn_id":"turn_1781846400000","user_input":"第二条历史"}
{"type":"turn_final","session":"shell_old","turn_id":"turn_1781846400000","assistant_output":"第二条回复"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("列最近聊天", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"chat_history_query","intent":"列出最近聊天记录","input":{"query":"","limit":1}}]}"#),
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
    let audit_file = root.join("api_audit.jsonl");
    fs::write(
        &audit_file,
        r#"{"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"旧聊天"}
{"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"旧回复"}
{"type":"turn_start","session":"shell_new","turn_id":"turn_1781846400000","user_input":"新聊天"}
{"type":"turn_final","session":"shell_new","turn_id":"turn_1781846400000","assistant_output":"新回复"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("查最近窗口聊天", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"按时间窗口查询聊天记录","input":{"sql":"SELECT session_id, role, content, created_at_ms FROM chat_messages WHERE created_at_ms >= ? AND created_at_ms < ? ORDER BY created_at_ms DESC","params":["1781840000000","1781850000000"],"limit":20}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_sql_query"));
    assert!(prompt.contains("session_id=shell_new"));
    assert!(prompt.contains("content=新聊天"));
    assert!(prompt.contains("content=新回复"));
    assert!(!prompt.contains("content=旧聊天"));
}

#[test]
fn memory_sql_query_accepts_common_llm_param_shapes() {
    let sql = "SELECT role, content, created_at_ms FROM chat_messages WHERE created_at_ms >= ? AND created_at_ms < ? ORDER BY created_at_ms ASC";
    let cases = [
        (
            "string_params_inside_input",
            format!(
                r#""input":{{"sql":"{}","params":["1782200000000","1782210000000"],"limit":50}}"#,
                sql
            ),
        ),
        (
            "integer_params_inside_input",
            format!(
                r#""input":{{"sql":"{}","params":[1782200000000,1782210000000],"limit":50}}"#,
                sql
            ),
        ),
        (
            "float_params_inside_input",
            format!(
                r#""input":{{"sql":"{}","params":[1782200000000.0,1782210000000.0],"limit":50}}"#,
                sql
            ),
        ),
        (
            "top_level_integer_params",
            format!(
                r#""input":{{"sql":"{}","limit":50}},"params":[1782200000000,1782210000000]"#,
                sql
            ),
        ),
    ];

    for (case_name, action_fields) in cases {
        let root = tmp_dir(case_name);
        let dir = root.join("memory");
        fs::create_dir_all(&dir).unwrap();
        let audit_file = root.join("api_audit.jsonl");
        fs::write(
            &audit_file,
            r#"{"type":"turn_start","session":"shell_today","turn_id":"turn_1782203922467","user_input":"我今天和你聊过什么？"}
{"type":"turn_final","session":"shell_today","turn_id":"turn_1782203922467","assistant_output":"今天聊过 shell 记忆查询。"}
"#,
        )
        .unwrap();
        let mut core = AgentCore::new("STATIC", profile("custom", "aws-claude-sonnet-4-6"), &dir);
        let _ = core.begin_turn("我今天和你聊过什么？", None);
        let content = scored(format!(
            r#"{{"report_job_progress":"","next_actions":[{{"action":"memory_sql_query","intent":"查询今天的聊天记录",{}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["今天的聊天记录内容"]}}}}"#,
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
        assert!(
            prompt.contains("Action result: memory_sql_query"),
            "{case_name}"
        );
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"test action","input":{"sql":"UPDATE memories SET content='bad'","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_sql_query"));
    assert!(prompt.contains("error: read_only_sql_required"));
}

#[test]
fn memory_sql_query_rejects_chat_history_delete_sql() {
    let root = tmp_dir("chat_delete_rejected");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("api_audit.jsonl");
    fs::write(
        &audit_file,
        r#"{"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"需要保留的聊天"}
{"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"这条聊天仍应只读。"}
"#,
    )
    .unwrap();
    let before = fs::read_to_string(&audit_file).unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("删除聊天记录", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"Attempt to delete chat history through SQL.","input":{"sql":"DELETE FROM chat_messages WHERE content LIKE '%保留%'","limit":5}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_sql_query"));
    assert!(prompt.contains("error: read_only_sql_required"));
    assert_eq!(fs::read_to_string(&audit_file).unwrap(), before);
}

#[test]
fn chat_history_delete_removes_matching_turn_from_audit_log() {
    let root = tmp_dir("chat_delete_action");
    let dir = root.join("memory");
    fs::create_dir_all(&dir).unwrap();
    let audit_file = root.join("api_audit.jsonl");
    fs::write(
        &audit_file,
        r#"{"type":"turn_start","session":"shell_old","turn_id":"turn_1781760000000","user_input":"删除目标聊天"}
{"type":"turn_final","session":"shell_old","turn_id":"turn_1781760000000","assistant_output":"删除目标回复"}
{"type":"turn_start","session":"shell_old","turn_id":"turn_1781846400000","user_input":"保留聊天"}
{"type":"turn_final","session":"shell_old","turn_id":"turn_1781846400000","assistant_output":"保留回复"}
"#,
    )
    .unwrap();
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("删除包含目标的聊天记录", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"chat_history_delete","intent":"Delete matching chat record.","input":{"query":"删除目标","limit":10}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: chat_history_delete"));
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
    let _ = core.begin_turn("记住我的名字", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"test action","input":{"operation":"upsert","id":"user_name","content":"用户的名字是默默"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: memory_update"));
    assert!(prompt.contains("id: user_name"));
    assert!(core.memory_git_commit_count() >= 1);
    assert!(fs::read_to_string(core.memory_file())
        .unwrap()
        .contains("用户的名字是默默"));

    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"test action","input":{"operation":"update","id":"user_name","content":"用户的名字是默默2"}}]}"#),
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
        .contains("用户的名字是默默\""));

    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"test action","input":{"operation":"update","id":"user_name","expected_version":1,"content":"用户的名字是默默2"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("operation: update"));
    assert!(prompt.contains("version: 2"));
    let stored = fs::read_to_string(core.memory_file()).unwrap();
    assert!(stored.contains("用户的名字是默默2"));
    assert!(!stored.contains("用户的名字是默默\""));
    assert!(core.memory_git_commit_count() >= 2);

    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"test action","input":{"operation":"delete","id":"user_name","expected_version":2}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("operation: delete"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"Insert shared row.","input":{"operation":"upsert","id":"shared_fact","content":"版本1"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::NeedModel { .. }));

    let _ = core_a.begin_turn("A 更新", None);
    let step = core_a.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"Update shared row from A.","input":{"operation":"update","id":"shared_fact","expected_version":1,"content":"版本2 from A"}}]}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"Update shared row from B with stale version.","input":{"operation":"update","id":"shared_fact","expected_version":1,"content":"版本2 from B"}}]}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"Insert shared conflict row.","input":{"operation":"upsert","id":"shared_conflict","content":"初始值"}}]}"#),
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
                    r#"{{"report_job_progress":"","next_actions":[{{"action":"memory_update","intent":"Concurrent same-version update.","input":{{"operation":"update","id":"shared_conflict","expected_version":1,"content":"winner candidate {idx}"}}}}]}}"#
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
        .filter(|prompt| prompt.contains("operation: update") && prompt.contains("version: 2"))
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
    let guard = MemGuard::for_memory_dir(dir);
    guard
        .with_write(|| {
            fs::write(&marker, "locked").unwrap();
            thread::sleep(Duration::from_millis(350));
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
    let current_exe = std::env::current_exe().unwrap();
    let mut child = Command::new(current_exe)
        .arg("--exact")
        .arg("mem_guard_child_process_holds_lock_helper")
        .arg("--nocapture")
        .env("TIMEM_MEM_GUARD_CHILD_DIR", &dir)
        .env("TIMEM_MEM_GUARD_CHILD_MARKER", &child_marker)
        .spawn()
        .unwrap();

    let started = std::time::Instant::now();
    while !child_marker.exists() {
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(20));
    }

    let waited = std::time::Instant::now();
    MemGuard::for_memory_dir(&dir)
        .with_write(|| fs::write(&parent_marker, "done"))
        .unwrap()
        .unwrap();
    assert!(
        waited.elapsed() >= Duration::from_millis(250),
        "parent should wait for child process guard"
    );
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
                    r#"{{"report_job_progress":"","next_actions":[{{"action":"memory_update","intent":"Concurrent durable write.","input":{{"operation":"upsert","id":"guard_id_{idx}","content":"guard content {idx}"}}}}]}}"#
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"test action","input":{"operation":"update","content":"x"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.id_required"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"count output lines","input":{"command":"pwd | wc -l","timeout_ms":5000}}]}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_write","intent":"记录任务计划","input":{"type":"notes","label":"任务计划","content":"step one"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    match step {
        CoreStep::NeedModel { .. } => {}
        other => panic!("unexpected step: {other:?}"),
    }
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"scratch_query","intent":"查询任务计划","input":{"query":"step","limit":3}}]}"#),
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
    assert_eq!(interactions[0]["actions"][0]["action"], "scratch_write");
    assert_eq!(interactions[0]["actions"][0]["intent"], "记录任务计划");
    assert_eq!(interactions[0]["actions"][0]["status"], "completed");
    assert_eq!(
        interactions[0]["actions"][0]["input"]["content"],
        "step one"
    );
    assert_eq!(interactions[0]["actions"][0]["input"]["type"], "notes");
    assert_eq!(interactions[0]["actions"][0]["input"]["label"], "任务计划");
    assert_eq!(interactions[1]["round"], 2);
    assert_eq!(interactions[1]["actions"][0]["action"], "scratch_query");
    assert_eq!(interactions[1]["actions"][0]["intent"], "查询任务计划");
}

#[test]
fn run_bash_accepts_old_timeout_sec_field() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_timeout_sec"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("count cwd lines", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"count output lines","input":{"command":"pwd | wc -l","timeout_sec":1}}]}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Start a background task.","input":{"command":"sleep 0.1; printf background-ok","background":true}}]}"#),
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
            r#"{{"report_job_progress":"","next_actions":[{{"action":"shell_job_status","intent":"Poll background task.","input":{{"job_id":"{}","timeout_ms":1000}}}}]}}"#,
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
fn shell_job_status_requires_model_chosen_timeout() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_background_timeout_required"),
    );
    let _ = core.begin_turn("poll a long task", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"shell_job_status","intent":"Poll background task.","input":{"job_id":"job_1"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("input.timeout_ms_required"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Start a background task.","input":{"command":"sleep 0.4; printf waited-ok","background":true}}]}"#),
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
            r#"{{"report_job_progress":"","next_actions":[{{"action":"shell_job_status","intent":"Wait for background task.","input":{{"job_id":"{}","timeout_ms":1500}}}}]}}"#,
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
fn run_bash_accepts_old_read_back_protocol() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_readback"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("count cwd lines", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"read back count","input":{"command":"pwd","read_back_command":"pwd | wc -l","large_readback_opt_in":{"protocol":"unbounded_v1","reason":"verify line count"},"timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("Read-back result:"));
    assert!(prompt.contains("command: pwd | wc -l"));
    assert!(prompt
        .contains("read_back_policy: unbounded_v1_requested_but_native_output_is_still_bounded"));
}

#[test]
fn run_bash_accepts_read_back_without_primary_command() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_readback_only"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("count cwd lines", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","input":{"read_back_command":"pwd | wc -l","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("command: pwd | wc -l"));
    assert!(prompt.contains("status: 0"));
}

#[test]
fn run_bash_requires_approval_for_mutating_commands() {
    let dir = tmp_dir("bash_reject");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &dir);
    let _ = core.begin_turn("delete something", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","input":{"command":"rm not_allowed"}}]}"#),
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
    assert_eq!(request.risk, "local_shell_command");

    let prompt = match core.resolve_user_approval(&request.approval_id, false) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("status: denied_by_user"));
    assert!(prompt.contains(&request.approval_id));

    let audit_text = fs::read_to_string(dir.join("audit").join("action_audit.json")).unwrap();
    let audit: serde_json::Value = serde_json::from_str(&audit_text).unwrap();
    let actions = audit["turns"][0]["interactions"][0]["actions"]
        .as_array()
        .unwrap();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0]["action"], "run_bash");
    assert_eq!(actions[0]["intent"], "test action");
    assert_eq!(actions[0]["status"], "needs_user_approval");
    assert_eq!(actions[0]["input"]["command"], "rm not_allowed");
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","input":{"command":"mkdir -p target/timem_test; printf ok | tee target/timem_test/write_guard.txt; cat target/timem_test/write_guard.txt"}}]}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","input":{"command":"pwd && rm not_allowed"}}]}"#),
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
    assert_eq!(request.risk, "local_shell_command");
}

#[test]
fn run_bash_requires_approval_for_mutating_read_back_command() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_reject_readback"),
    );
    let _ = core.begin_turn("inspect files", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","input":{"command":"pwd","read_back_command":"rm not_allowed"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(request.command, "pwd");
    assert_eq!(request.read_back_command, "rm not_allowed");
    assert_eq!(request.reason, "run_bash_requires_user_approval");
}

#[test]
fn run_bash_does_not_execute_primary_command_before_read_back_approval() {
    let marker = PathBuf::from("target/timem_test_approval_preflight_marker.txt");
    let _ = fs::remove_file(&marker);
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_readback_approval_preflight"),
    );
    let _ = core.begin_turn("write then inspect broader environment", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Write local marker then run broader readback.","input":{"command":"touch target/timem_test_approval_preflight_marker.txt","read_back_command":"cat /etc/passwd","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(
        request.command,
        "touch target/timem_test_approval_preflight_marker.txt"
    );
    assert_eq!(request.read_back_command, "cat /etc/passwd");
    assert!(!marker.exists());

    let prompt = match core.resolve_user_approval(&request.approval_id, false) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("status: denied_by_user"));
    assert!(!marker.exists());
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Run shell syntax after approval.","input":{"command":"x=ok; printf \"$x\" | tr o O","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(request.command, r#"x=ok; printf "$x" | tr o O"#);

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
fn run_bash_requires_command_for_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("bash_missing"),
    );
    let _ = core.begin_turn("inspect files", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","input":{}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Protocol repair request"));
    assert!(prompt.contains("input.any_required:command|read_back_command"));
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"test action","input":{"command":"cat /etc/passwd"}}]}"#),
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
    assert_eq!(request.risk, "local_shell_command");
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Read system identity.","input":{"command":"uname -s","timeout_ms":5000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(prompt.contains("Action result: run_bash"));
    assert!(prompt.contains("command: uname -s"));
    assert!(prompt.contains("status: 0"));
    assert!(!prompt.contains("approval_status: approved_by_user"));
}

#[test]
fn ci_realistic_multiturn_memory_tools_security_and_shrink_story() {
    let dir = tmp_dir("ci_realistic_story");
    let mut core = AgentCore::new("STATIC_GLOBAL_RULES", profile("aliyun", "qwen-plus"), &dir);

    let first_prompt = match core.begin_turn(
        "我儿子的生日是6月12日",
        Some("runtime_time: 2026-06-19T12:00:00+08:00"),
    ) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(first_prompt.contains("User question:\n我儿子的生日是6月12日"));
    assert!(first_prompt.contains("Supporting context:\nruntime_time:"));
    let write_final = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"已记录。","continue":false,"memory_candidates":[{"content":"用户的儿子生日是6月12日"}],"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(write_final.stats.mem_writes, 1);

    let _ = core.begin_turn("june 12th 是谁的生日", None);
    let recall_prompt = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"query_memory","intent":"Find durable birthday memory before answering.","input":{"query":"儿子 生日 6月12日","limit":5}}],"acceptance_check":{"is_satisfied":false,"missing_info":["durable memory evidence"]}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(recall_prompt.contains("Action result: query_memory"));
    assert!(recall_prompt.contains("用户的儿子生日是6月12日"));
    let recall_final = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"6月12日是你儿子的生日。","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(recall_final.response_to_user.contains("你儿子"));
    assert!(recall_final.stats.mem_reads >= 1);

    let _ = core.begin_turn("删除我儿子生日这条记忆", None);
    let delete_prompt = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_update","intent":"Delete the user-requested birthday memory.","input":{"operation":"delete","id":"mem_0"}}],"acceptance_check":{"is_satisfied":false,"missing_info":["delete result"]}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(delete_prompt.contains("Action result: memory_update"));
    assert!(delete_prompt.contains("error: id_not_found"));

    let delete_prompt = match core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"memory_sql_query","intent":"Find exact memory id before deleting.","input":{"sql":"SELECT id, version, content FROM memories WHERE content LIKE ? ORDER BY created_at_ms DESC","params":["%儿子生日%"],"limit":5}}],"acceptance_check":{"is_satisfied":false,"missing_info":["memory id"]}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(delete_prompt.contains("Action result: memory_sql_query"));
    assert!(delete_prompt.contains("content=用户的儿子生日是6月12日"));
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
        content: scored(format!(r#"{{"report_job_progress":"","next_actions":[{{"action":"memory_update","intent":"Delete exact durable birthday memory.","input":{{"operation":"delete","id":"{}","expected_version":1}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["delete confirmation"]}}}}"#, memory_id)),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    }) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert!(delete_final_prompt.contains("operation: delete"));
    assert!(!fs::read_to_string(core.memory_file())
        .unwrap()
        .contains("儿子生日"));

    let delete_final = match core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"已删除。","continue":false,"acceptance_check":{"is_satisfied":true}}"#,
        ),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Count files in current project folder.","input":{"command":"find . -maxdepth 1 -type f | wc -l","timeout_ms":5000}}],"acceptance_check":{"is_satisfied":false,"missing_info":["file count"]}}"#),
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
        content: scored(r#"{"report_job_progress":"","next_actions":[{"action":"run_bash","intent":"Attempt forbidden absolute path read.","input":{"command":"cat /etc/passwd","timeout_ms":5000}}],"acceptance_check":{"is_satisfied":false,"missing_info":["file content"]}}"#),
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
                r#"{{"report_job_progress":"ok {}","continue":false,"acceptance_check":{{"is_satisfied":true}}}}"#,
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
    assert!(long_prompt.starts_with("[BEGIN SEGMENT 0: prompt_0]\nSTATIC_GLOBAL_RULES"));
    assert!(long_prompt.contains("Long-context maintenance:"));
    assert!(long_prompt.contains("mode=force_shrink_required"));
    assert!(long_prompt.contains("force_shrink_threshold_tokens=2700"));
    assert!(long_prompt.contains("target_dynamic_context_ratio=10%-20%"));
    assert!(long_prompt.contains("prompt_delta_count="));
}

#[test]
fn thought_field_is_persisted_as_llm_thought_slice() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("thought_slice"),
    );
    let _ = core.begin_turn("需要推理一下", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"thought":"推导一下","report_job_progress":"好的","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = core.render_prompt();
    assert!(prompt.contains("prompt_type: llm_thought"));
    assert!(prompt.contains("Thought:\n推导一下"));
}

#[test]
fn thought_field_optional_does_not_trigger_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("thought_absent"),
    );
    let _ = core.begin_turn("简单问答", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"好的","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_turn = match step {
        CoreStep::Final(turn) => turn,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(final_turn.response_to_user, "好的");
    let prompt = core.render_prompt();
    assert!(!prompt.contains("prompt_type: llm_thought"));
}

#[test]
fn thought_object_durable_true_is_persisted_as_llm_thought_slice() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("thought_obj_durable"),
    );
    let _ = core.begin_turn("需要推理", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"thought":{"content":"对象形式的思考","durable":true},"report_job_progress":"好的","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = core.render_prompt();
    assert!(prompt.contains("prompt_type: llm_thought"));
    assert!(prompt.contains("Thought:\n对象形式的思考"));
}

#[test]
fn thought_object_durable_false_is_not_persisted() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("thought_obj_not_durable"),
    );
    let _ = core.begin_turn("需要推理", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"thought":{"content":"临时思考不保留","durable":false},"report_job_progress":"好的","continue":false,"acceptance_check":{"is_satisfied":true}}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(matches!(step, CoreStep::Final(_)));
    let prompt = core.render_prompt();
    assert!(!prompt.contains("prompt_type: llm_thought"));
}

#[test]
fn static_prompt_keeps_contracts_concise() {
    let static_prompt = include_str!("../../resources/static_v1.json");
    assert!(static_prompt.contains("\"json_protocol\""));
    assert!(static_prompt.contains("\"evidence_guard\""));
    assert!(static_prompt.contains("\"action_result_guard\""));
    assert!(static_prompt.contains("memory_conflict"));
    assert!(static_prompt.contains("row version changed"));
    assert!(static_prompt.contains("Use report_job_progress to report current job progress"));
    assert!(static_prompt.contains("Runtime injects response_v1 schema summary"));
    assert!(
        !static_prompt.contains("\"$id\": \"https://timem.local/schemas/response_v1.schema.json\"")
    );
    assert!(!static_prompt.contains("\"report_job_progress?\""));
    assert!(!static_prompt.contains("durable_ctx_score"));
    assert!(!static_prompt.contains("delta_scores"));
    assert!(!static_prompt.contains("no_result_terminate"));
    assert!(!static_prompt.contains("long_running_shell"));
    assert!(!static_prompt.contains("lang_retry"));
    assert!(!static_prompt.contains("theme_workflow"));
    assert!(!static_prompt.contains("rounds_guard"));
}

#[test]
fn rendered_prompt_response_schema_is_injected_from_resource() {
    let mut core = AgentCore::new(
        r#"{"Response_rule":{"json_schema_summary":"stale"}}"#,
        profile("aliyun", "qwen-plus"),
        tmp_dir("response_schema_prompt_injection"),
    );
    let prompt = match core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };

    assert!(prompt.contains("\"$id\": \"https://timem.local/schemas/response_v1.schema.json\""));
    assert!(prompt.contains("\"report_job_progress?\""));
    assert!(prompt.contains("\"intent\""));
    assert!(!prompt.contains("\"json_schema_summary\": \"stale\""));
}

#[test]
fn static_prompt_does_not_handwrite_tool_catalog() {
    let static_prompt = include_str!("../../resources/static_v1.json");
    let value: serde_json::Value = serde_json::from_str(static_prompt).unwrap();
    let tool_catalog = value
        .get("Tool_capability")
        .and_then(|v| v.get("tool_catalog"))
        .unwrap();

    assert!(
        !tool_catalog.is_object(),
        "static prompt must not hand-maintain tool specs; registry injects tool_catalog"
    );
    assert!(tool_catalog
        .as_str()
        .unwrap_or_default()
        .contains("Runtime injects"));
}

#[test]
fn rendered_prompt_tool_catalog_is_generated_from_capability_manifests() {
    let mut core = AgentCore::new(
        r#"{"Tool_capability":{"tool_catalog":{"stale_tool":{"when":"old"}}}}"#,
        profile("aliyun", "qwen-plus"),
        tmp_dir("capability_prompt_catalog"),
    );
    let prompt = match core.begin_turn("hello", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };

    assert!(prompt.contains("\"memmgr\""));
    assert!(prompt.contains("\"capmgr\""));
    assert!(prompt.contains("\"run_bash\""));
    assert!(prompt.contains("\"shell_job_status\""));
    assert!(prompt.contains("\"skill_headers\""));
    assert!(prompt.contains("\"release_quality_gate\""));
    assert!(prompt.contains("Unified local memory manager"));
    assert!(!prompt.contains("stale_tool"));
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
            r#"{"report_job_progress":"正在查询记忆。","continue":true,"next_actions":[{"action":"memmgr","intent":"Query durable memory through manifest-backed tool.","input":{"type":"durable","op":"query","query":"名字","limit":5}}]}"#,
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
fn legacy_action_fallback_is_not_part_of_manifest_catalog() {
    let mut core = AgentCore::new(
        include_str!("../../resources/static_v1.json"),
        profile("aliyun", "qwen-plus"),
        tmp_dir("legacy_action_fallback_boundary"),
    );
    let prompt = match core.begin_turn("查旧动作", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected NeedModel, got {other:?}"),
    };
    assert!(prompt.contains("\"memmgr\""));
    assert!(!prompt.contains("\"query_memory\""));

    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"兼容旧动作。","continue":true,"next_actions":[{"action":"query_memory","intent":"Legacy fallback check.","input":{"query":"名字","limit":1}}]}"#,
        ),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected legacy action result, got {other:?}"),
    };

    assert!(prompt.contains("Action result: query_memory"));
}

#[test]
fn capmgr_load_skill_adds_skill_body_as_action_result() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("capmgr_load_skill"),
    );
    let _ = core.begin_turn("准备发布", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"加载发布检查 skill。","continue":true,"next_actions":[{"action":"capmgr","intent":"Load release quality instructions before auditing.","input":{"op":"load","kind":"skill","id":"release_quality_gate"}}]}"#,
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
fn capmgr_load_requires_kind_and_id_for_protocol_repair() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("capmgr_missing_fields"),
    );
    let _ = core.begin_turn("准备发布", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(
            r#"{"report_job_progress":"加载 skill。","continue":true,"next_actions":[{"action":"capmgr","intent":"Load missing skill.","input":{"op":"load"}}]}"#,
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
    assert!(prompt.contains("kind_required"));
}

#[test]
fn capmgr_rejects_manifest_enum_violations_before_execution() {
    for (case, payload, expected_issue) in [
        (
            "bad_op",
            r#"{"report_job_progress":"检查 capability。","continue":true,"next_actions":[{"action":"capmgr","intent":"Use an unsupported capability operation.","input":{"op":"remove","kind":"skill","id":"release_quality_gate"}}]}"#,
            "input.op_unsupported:remove",
        ),
        (
            "bad_kind",
            r#"{"report_job_progress":"检查 capability。","continue":true,"next_actions":[{"action":"capmgr","intent":"Use an unsupported capability kind.","input":{"op":"load","kind":"resource","id":"release_quality_gate"}}]}"#,
            "input.kind_unsupported:resource",
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
            other => panic!("expected protocol repair for {case}, got {other:?}"),
        };

        assert!(prompt.contains("Protocol repair request"));
        assert!(prompt.contains(expected_issue));
        assert!(!prompt.contains("Action result: capmgr"));
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
description: Echo the action JSON payload.
prompt_when: |
  Use to echo a bounded payload during capability tests.
input_properties:
  text: string
required:
  - text
example_json: |
  {
    "action": "echo_payload",
    "intent": "Echo payload.",
    "input": {
      "text": "hello"
    }
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
            r#"{"report_job_progress":"运行 overlay command。","continue":true,"next_actions":[{"action":"echo_payload","intent":"Echo runtime payload.","input":{"text":"hello"}}]}"#,
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
fn guarded_finalize_success_finalizes_when_expect_exits_zero() {
    let memory_dir = tmp_dir("guarded_finalize_ok");
    let mut core = AgentCore::new("STATIC", profile("aliyun", "qwen-plus"), &memory_dir);
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("完成任务", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"任务已完成","continue":false,"next_actions":[{"action":"query_memory","intent":"Verify.","input":{"query":"x","limit":1,"expect":"true","expect_timeout_ms":2000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let final_ = match step {
        CoreStep::Final(f) => f,
        other => panic!("expected Final, got {other:?}"),
    };
    assert_eq!(final_.response_to_user, "任务已完成");
    let prompt = core.render_prompt();
    assert!(prompt.contains("prompt_type: llm_response"));
    assert!(prompt.contains("Expect check:"));
    assert!(prompt.contains("verdict: PASS") || prompt.contains("status: 0"));
    let audit = fs::read_to_string(memory_dir.join("audit").join("action_audit.json")).unwrap();
    assert!(audit.contains("guarded_finalize_expect"));
    assert!(audit.contains("guarded_finalize_expect_pass"));
    assert!(audit.contains("\"expect\""));
    assert!(audit.contains("\"true\""));
}

#[test]
fn guarded_finalize_blocks_final_when_expect_nonzero() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("guarded_finalize_fail"),
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("完成任务", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"任务已完成","continue":false,"next_actions":[{"action":"query_memory","intent":"Verify.","input":{"query":"x","limit":1,"expect":"false","expect_timeout_ms":2000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    assert!(
        !matches!(step, CoreStep::Final(_)),
        "expect!=0 must not finalize, got {step:?}"
    );
    let prompt = core.render_prompt();
    assert!(
        prompt.contains("expect 命令 exit!=0"),
        "missing runtime_note about expect failure"
    );
    assert!(
        !prompt.contains("prompt_type: llm_response"),
        "must not emit llm_response slice on expect failure"
    );
}

#[test]
fn guarded_finalize_expect_respects_bash_approval_mode() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("guarded_finalize_approval"),
    );
    let _ = core.begin_turn("完成任务", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"任务已完成","continue":false,"next_actions":[{"action":"query_memory","intent":"Verify.","input":{"query":"x","limit":1,"expect":"true","expect_timeout_ms":2000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let request = match step {
        CoreStep::NeedsUserApproval { request } => request,
        other => panic!("expected approval before expect bash, got {other:?}"),
    };
    assert_eq!(request.command, "true");
    assert_eq!(request.intent, "Verify final answer before showing it.");
}

#[test]
fn guarded_finalize_requires_expect_timeout() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("guarded_finalize_missing_timeout"),
    );
    let _ = core.begin_turn("完成任务", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"任务已完成","continue":false,"next_actions":[{"action":"query_memory","intent":"Verify.","input":{"query":"x","limit":1,"expect":"true"}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected protocol repair, got {other:?}"),
    };
    assert!(prompt.contains("expect_timeout_ms_required"));
}

#[test]
fn guarded_finalize_rejects_expect_on_non_last_action() {
    let mut core = AgentCore::new(
        "STATIC",
        profile("aliyun", "qwen-plus"),
        tmp_dir("guarded_finalize_non_last"),
    );
    let _ = core.begin_turn("完成任务", None);
    let step = core.apply_model_response(LlmResponse {
        content: scored(r#"{"report_job_progress":"任务已完成","continue":false,"next_actions":[{"action":"query_memory","intent":"First.","input":{"query":"x","limit":1,"expect":"true","expect_timeout_ms":2000}},{"action":"query_memory","intent":"Last.","input":{"query":"y","limit":1,"expect":"true","expect_timeout_ms":2000}}]}"#),
        model_name: "qwen-plus".to_string(),
        usage: usage(),
        truncated: false,
    });
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected protocol repair, got {other:?}"),
    };
    assert!(prompt.contains("expect_only_allowed_on_last_action"));
}
