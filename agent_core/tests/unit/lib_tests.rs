use super::*;

#[test]
fn native_interruption_note_is_not_rendered_as_an_action_result() {
    let mut core = test_core("native_interruption_runtime_note");
    core.set_interaction_profile(&InteractionProfile {
        api_protocol: "openai_compatible".to_string(),
        model: "test".to_string(),
        gateway: "test".to_string(),
        requested_mode: ToolCallMode::Native,
        resolved_mode: ToolCallMode::Native,
        active_prompt_protocol: "json".to_string(),
        parallel_supported: true,
        parallel_enabled: true,
        source: CapabilityProbeSource::Explicit,
        reason: "test".to_string(),
        probe_latency_ms: None,
        observed_tool_calls: 1,
    });

    let _ = core.begin_turn("old interrupted work", None);
    core.mark_user_interrupted_work();
    let prompt = match core.begin_turn("继续", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };

    let note_text = "NOTE: User interrupted the above work. Continue it based on the user's new input's intent. If not sure, ask the user.";
    let note = prompt.find(note_text).expect("interruption note");
    let new_user = prompt[note..]
        .find("继续")
        .map(|offset| note + offset)
        .expect("new user input after interruption note");
    assert!(prompt[..note].contains("old interrupted work"), "{prompt}");
    assert!(note < new_user, "{prompt}");
    assert_eq!(prompt.matches(note_text).count(), 1);
    assert!(prompt.ends_with(prompt_render::NATIVE_RESPONSE_TRAILER));

    let note_delta_start = prompt[..note]
        .rfind("[BEGIN DELTA ")
        .expect("interruption delta start");
    let note_delta_end = prompt[note..]
        .find("[BEGIN DELTA ")
        .map(|offset| note + offset)
        .unwrap_or(prompt.len());
    let note_delta = &prompt[note_delta_start..note_delta_end];
    assert!(note_delta.contains("## RUNTIME"), "{note_delta}");
    assert!(note_delta.contains("## USER"), "{note_delta}");
    assert!(
        !note_delta.contains("The following are results of the actions generated in response:"),
        "{note_delta}"
    );
}

#[test]
fn forced_compaction_preserves_native_history_and_restricts_model_request() {
    let mut core = test_core("forced_native_compaction_gate");
    core.set_max_llm_input_tokens(3_000);
    core.set_interaction_profile(&InteractionProfile {
        api_protocol: "openai_compatible".to_string(),
        model: "test".to_string(),
        gateway: "test".to_string(),
        requested_mode: ToolCallMode::Native,
        resolved_mode: ToolCallMode::Native,
        active_prompt_protocol: "json".to_string(),
        parallel_supported: true,
        parallel_enabled: true,
        source: CapabilityProbeSource::Explicit,
        reason: "test".to_string(),
        probe_latency_ms: None,
        observed_tool_calls: 1,
    });
    core.append_delta(vec![("user_question".to_string(), "keep task".to_string())]);
    core.native_exchanges.push(NativeExchange {
        delta_id: "pd_1".to_string(),
        assistant_text: "old tool work".to_string(),
        calls: vec![NativeToolCall {
            id: "call_old".to_string(),
            name: "readfile".to_string(),
            arguments: serde_json::json!({"path":"large.txt"}),
            raw_arguments: r#"{"path":"large.txt"}"#.to_string(),
        }],
        results: vec![NativeToolResult {
            call_id: "call_old".to_string(),
            name: "readfile".to_string(),
            content: "large old result".repeat(100),
            is_error: false,
        }],
    });
    core.last_observed_prompt_tokens = 2_700;

    core.append_in_turn_shrink_review_if_needed();

    assert!(core.context_compact_required);
    assert_eq!(core.native_exchanges.len(), 1);
    assert_eq!(core.native_exchanges[0].delta_id, "pd_1");
    let prompt = core.render_prompt();
    assert!(!prompt.contains("old tool work"));
    assert!(prompt.contains("mode=force_shrink_required"));
    let request_prompt = core.build_model_request_prompt(&prompt);
    assert!(request_prompt
        .ends_with("Context is too long. Your tool calls must start with context_compact:"));
    let request = core.model_interaction_request(request_prompt);
    assert_eq!(request.tool_choice, NativeToolChoice::Required);
    assert!(request
        .tools
        .iter()
        .any(|tool| tool.name == "context_compact"));
    assert!(request.tools.iter().any(|tool| tool.name == "readfile"));
}

#[test]
fn native_final_keeps_structured_tool_history_before_final_replay() {
    let mut core = test_core("native_final_history");
    core.set_interaction_profile(&InteractionProfile {
        api_protocol: "openai_compatible".to_string(),
        model: "test".to_string(),
        gateway: "test".to_string(),
        requested_mode: ToolCallMode::Native,
        resolved_mode: ToolCallMode::Native,
        active_prompt_protocol: "json".to_string(),
        parallel_supported: true,
        parallel_enabled: true,
        source: CapabilityProbeSource::Explicit,
        reason: "test".to_string(),
        probe_latency_ms: None,
        observed_tool_calls: 1,
    });
    core.append_delta(vec![(
        "user_question".to_string(),
        "inspect the project".to_string(),
    )]);
    core.native_exchanges.push(NativeExchange {
        delta_id: "pd_1".to_string(),
        assistant_text: "I will inspect it.".to_string(),
        calls: vec![NativeToolCall {
            id: "call_read".to_string(),
            name: "readfile".to_string(),
            arguments: serde_json::json!({"path":"README.md"}),
            raw_arguments: r#"{"path":"README.md"}"#.to_string(),
        }],
        results: vec![NativeToolResult {
            call_id: "call_read".to_string(),
            name: "readfile".to_string(),
            content: "PROJECT-EVIDENCE-42".to_string(),
            is_error: false,
        }],
    });

    let step = core.apply_model_response(LlmResponse {
        content: "Final answer based on PROJECT-EVIDENCE-42".to_string(),
        tool_calls: Vec::new(),
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });

    assert!(matches!(step, CoreStep::Final(_)));
    assert_eq!(core.native_exchanges.len(), 1);
    assert_eq!(core.native_exchanges[0].delta_id, "pd_1");
    let prompt = core.build_next_prompt();
    assert!(!prompt.contains("Tool calls:"));
    assert!(prompt.contains("Final answer based on PROJECT-EVIDENCE-42"));
    assert_eq!(prompt.matches("PROJECT-EVIDENCE-42").count(), 1);
    assert_eq!(
        prompt
            .matches("Final answer based on PROJECT-EVIDENCE-42")
            .count(),
        1
    );

    core.append_delta(vec![(
        "user_question".to_string(),
        "what did you find?".to_string(),
    )]);
    let next_prompt = core.render_prompt();
    let next_request = core.model_interaction_request(next_prompt.clone());
    assert_eq!(next_request.native_exchanges.len(), 1);
    assert_eq!(next_request.native_exchanges[0].delta_id, "pd_1");
    assert!(!next_prompt.contains("Tool calls:"));
    assert!(!next_prompt.to_ascii_lowercase().contains("native"));
    assert_eq!(next_prompt.matches("PROJECT-EVIDENCE-42").count(), 1);
    assert!(
        next_prompt
            .find("Final answer based on PROJECT-EVIDENCE-42")
            .unwrap()
            < next_prompt.rfind("what did you find?").unwrap()
    );
}

#[test]
fn dynamic_context_estimate_and_shrink_stats_include_native_exchanges() {
    let mut core = test_core("native_dynamic_token_estimate");
    core.append_delta(vec![(
        "user_question".to_string(),
        "small text delta".to_string(),
    )]);
    core.native_exchanges.push(NativeExchange {
        delta_id: "pd_1".to_string(),
        assistant_text: "inspect the large result".to_string(),
        calls: vec![NativeToolCall {
            id: "call_large".to_string(),
            name: "readfile".to_string(),
            arguments: serde_json::json!({"path":"large.txt"}),
            raw_arguments: r#"{"path":"large.txt"}"#.to_string(),
        }],
        results: vec![NativeToolResult {
            call_id: "call_large".to_string(),
            name: "readfile".to_string(),
            content: "NATIVE-EVIDENCE-".repeat(1_000),
            is_error: false,
        }],
    });

    let before = core.dynamic_context_token_estimate();
    assert!(before.text_tokens > 0);
    assert!(before.native_tokens > before.text_tokens);
    assert_eq!(
        core.dynamic_context_summary().estimated_tokens,
        before.total_tokens()
    );

    let result = core.apply_prompt_shrink(
        "context compacted successfully.",
        &["pd_1".to_string()],
        &[],
    );

    assert_eq!(core.dynamic_context_summary().estimated_tokens, 0);
    assert_eq!(core.current_stats.shrunk_tokens, before.total_tokens());
    assert!(result.contains(&format!(
        "shrunk_tokens_estimate: {}",
        before.total_tokens()
    )));
}

#[test]
fn native_exchange_is_discarded_with_its_owning_delta() {
    let mut core = test_core("native_delta_discard");
    core.append_delta(vec![("user_question".to_string(), "Q1".to_string())]);
    core.append_delta(vec![("user_question".to_string(), "Q2".to_string())]);
    for (delta_id, call_id) in [("pd_1", "call_1"), ("pd_2", "call_2")] {
        core.native_exchanges.push(NativeExchange {
            delta_id: delta_id.to_string(),
            assistant_text: format!("work {call_id}"),
            calls: vec![NativeToolCall {
                id: call_id.to_string(),
                name: "readfile".to_string(),
                arguments: serde_json::json!({"path": format!("{call_id}.txt")}),
                raw_arguments: format!(r#"{{"path":"{call_id}.txt"}}"#),
            }],
            results: vec![NativeToolResult {
                call_id: call_id.to_string(),
                name: "readfile".to_string(),
                content: format!("result {call_id}"),
                is_error: false,
            }],
        });
    }
    let result = core.apply_prompt_shrink(
        "context compacted successfully.",
        &["pd_1".to_string()],
        &[],
    );
    assert!(result.contains("context compacted successfully."));
    assert_eq!(core.native_exchanges.len(), 1);
    assert_eq!(core.native_exchanges[0].delta_id, "pd_2");
    assert_eq!(core.native_exchanges[0].calls[0].id, "call_2");
}

#[test]
fn native_exchange_is_included_when_owning_delta_is_offloaded() {
    let mut core = test_core("native_delta_offload");
    core.append_delta(vec![("user_question".to_string(), "Q1".to_string())]);
    core.native_exchanges.push(NativeExchange {
        delta_id: "pd_1".to_string(),
        assistant_text: "inspect evidence".to_string(),
        calls: vec![NativeToolCall {
            id: "call_1".to_string(),
            name: "readfile".to_string(),
            arguments: serde_json::json!({"path":"evidence.txt"}),
            raw_arguments: r#"{"path":"evidence.txt"}"#.to_string(),
        }],
        results: vec![NativeToolResult {
            call_id: "call_1".to_string(),
            name: "readfile".to_string(),
            content: "EVIDENCE-42".to_string(),
            is_error: false,
        }],
    });
    let offload = core
        .collect_prompt_context_for_scratch(&["pd_1".to_string()], &[])
        .expect("owning delta should be offloadable");
    assert_eq!(offload.delta_ids, vec!["pd_1"]);
    assert!(offload.content.contains("assistant_text: inspect evidence"));
    assert!(offload.content.contains(r#""tool_call_id":"call_1""#));
    assert!(offload.content.contains("EVIDENCE-42"));
}

#[test]
fn forced_compaction_ignores_non_compact_output_then_unlocks_after_success() {
    let mut core = test_core("forced_compaction_ignore");
    core.set_response_protocol(ResponseProtocolKind::Json);
    core.context_compact_required = true;
    core.append_delta(vec![(
        "user_question".to_string(),
        "active task".to_string(),
    )]);
    let before = core.render_prompt();
    let round_before = core.current_round;

    let ignored = core.apply_model_response(LlmResponse {
        content: r#"{"final_answer":"must not be shown"}"#.to_string(),
        tool_calls: Vec::new(),
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });
    assert!(matches!(ignored, CoreStep::NeedModel { .. }));
    assert_eq!(core.current_round, round_before);
    assert_eq!(core.render_prompt(), before);
    assert!(core.context_compact_required);

    let ids = core
        .deltas
        .iter()
        .map(|delta| delta.delta_id.clone())
        .collect::<Vec<_>>();
    let completed = core.apply_model_response(LlmResponse {
        content: serde_json::json!({
            "context_compact": {
                "discard": ids,
                "summary": "keep active task and continue"
            }
        })
        .to_string(),
        tool_calls: Vec::new(),
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });
    assert!(matches!(completed, CoreStep::NeedModel { .. }));
    assert!(!core.context_compact_required);
}

#[test]
fn native_context_compact_persists_summary_after_discarding_all_old_deltas() {
    let mut core = test_core("native_compact_summary_all");
    core.set_response_protocol(ResponseProtocolKind::Json);
    core.set_interaction_profile(&InteractionProfile {
        api_protocol: "openai_compatible".to_string(),
        model: "test".to_string(),
        gateway: "test".to_string(),
        requested_mode: ToolCallMode::Native,
        resolved_mode: ToolCallMode::Native,
        active_prompt_protocol: "json".to_string(),
        parallel_supported: true,
        parallel_enabled: true,
        source: CapabilityProbeSource::Explicit,
        reason: "test".to_string(),
        probe_latency_ms: None,
        observed_tool_calls: 1,
    });
    core.append_delta(vec![(
        "user_question".to_string(),
        "OLD NATIVE CONTEXT".to_string(),
    )]);
    let old_delta_id = core.deltas[0].delta_id.clone();
    let summary = "NATIVE COMPACT SUMMARY MUST SURVIVE";
    let arguments = serde_json::json!({
        "discard": [old_delta_id],
        "summary": summary,
    });

    let step = core.apply_model_response(LlmResponse {
        content: String::new(),
        tool_calls: vec![NativeToolCall {
            id: "call_compact_all".to_string(),
            name: "context_compact".to_string(),
            raw_arguments: arguments.to_string(),
            arguments,
        }],
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });
    let CoreStep::NeedModel { prompt, .. } = step else {
        panic!("native context compact should continue with a model request")
    };

    assert!(!prompt.contains("OLD NATIVE CONTEXT"));
    assert_eq!(prompt.matches(summary).count(), 1);
    assert!(prompt.contains("## TIMEM_ASSISTANT (context compaction summary)"));
    assert!(prompt.contains("context compacted successfully."));
    assert_eq!(core.deltas.len(), 1, "summary must live in a fresh delta");
    assert_ne!(core.deltas[0].delta_id, old_delta_id);
    assert!(core.native_exchanges.is_empty());
    let request = core.model_interaction_request(prompt);
    assert_eq!(request.rendered_prompt.matches(summary).count(), 1);
    assert!(request.native_exchanges.is_empty());
    assert_eq!(core.build_next_prompt().matches(summary).count(), 1);
}

#[test]
fn native_context_compact_summary_does_not_depend_on_discarded_owning_delta() {
    let mut core = test_core("native_compact_summary_owner");
    core.set_response_protocol(ResponseProtocolKind::Json);
    core.set_interaction_profile(&InteractionProfile {
        api_protocol: "openai_compatible".to_string(),
        model: "test".to_string(),
        gateway: "test".to_string(),
        requested_mode: ToolCallMode::Native,
        resolved_mode: ToolCallMode::Native,
        active_prompt_protocol: "json".to_string(),
        parallel_supported: true,
        parallel_enabled: true,
        source: CapabilityProbeSource::Explicit,
        reason: "test".to_string(),
        probe_latency_ms: None,
        observed_tool_calls: 1,
    });
    core.append_delta(vec![("user_question".to_string(), "KEEP ME".to_string())]);
    core.append_delta(vec![(
        "result_of_llm_action".to_string(),
        "DISCARD OWNING DELTA".to_string(),
    )]);
    let owning_delta_id = core.deltas[1].delta_id.clone();
    let summary = "SUMMARY HAS AN INDEPENDENT NEW OWNER";
    let arguments = serde_json::json!({
        "discard": [owning_delta_id],
        "summary": summary,
    });

    let step = core.apply_model_response(LlmResponse {
        content: String::new(),
        tool_calls: vec![NativeToolCall {
            id: "call_compact_owner".to_string(),
            name: "context_compact".to_string(),
            raw_arguments: arguments.to_string(),
            arguments,
        }],
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });
    let CoreStep::NeedModel { prompt, .. } = step else {
        panic!("native context compact should continue with a model request")
    };

    assert!(prompt.contains("KEEP ME"));
    assert!(!prompt.contains("DISCARD OWNING DELTA"));
    assert_eq!(prompt.matches(summary).count(), 1);
    assert!(prompt.contains("## TIMEM_ASSISTANT (context compaction summary)"));
    assert!(core
        .deltas
        .iter()
        .any(|delta| delta.delta_id != owning_delta_id
            && delta.slices.iter().any(|slice| slice.text == summary)));
    assert!(core.native_exchanges.is_empty());
    let next_prompt = core.build_next_prompt();
    assert_eq!(next_prompt.matches(summary).count(), 1);
    let request = core.model_interaction_request(next_prompt);
    assert_eq!(request.rendered_prompt.matches(summary).count(), 1);
    assert!(request.native_exchanges.is_empty());
}

fn native_test_profile() -> InteractionProfile {
    InteractionProfile {
        api_protocol: "openai_compatible".to_string(),
        model: "test".to_string(),
        gateway: "test".to_string(),
        requested_mode: ToolCallMode::Native,
        resolved_mode: ToolCallMode::Native,
        active_prompt_protocol: "json".to_string(),
        parallel_supported: true,
        parallel_enabled: true,
        source: CapabilityProbeSource::Explicit,
        reason: "test".to_string(),
        probe_latency_ms: None,
        observed_tool_calls: 2,
    }
}

#[test]
fn native_context_compact_first_then_executes_later_call_with_correct_id() {
    let mut core = test_core("native_compact_then_call");
    core.set_interaction_profile(&native_test_profile());
    core.append_delta(vec![(
        "user_question".to_string(),
        "OLD CONTEXT TO DISCARD".to_string(),
    )]);
    let old_delta_id = core.deltas[0].delta_id.clone();
    let compact_arguments = serde_json::json!({
        "discard": [old_delta_id],
        "summary": "KEEP ACTIVE STATE",
    });
    let cwd_arguments = serde_json::json!({"type": "cwd"});

    let step = core.apply_model_response(LlmResponse {
        content: "compacting before continuing".to_string(),
        tool_calls: vec![
            NativeToolCall {
                id: "call_compact_first".to_string(),
                name: "context_compact".to_string(),
                raw_arguments: compact_arguments.to_string(),
                arguments: compact_arguments,
            },
            NativeToolCall {
                id: "call_after_compact".to_string(),
                name: "self_tool".to_string(),
                raw_arguments: cwd_arguments.to_string(),
                arguments: cwd_arguments,
            },
        ],
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });
    let CoreStep::NeedModel { prompt, .. } = step else {
        panic!("compact followed by a tool call should continue")
    };

    assert!(!prompt.contains("OLD CONTEXT TO DISCARD"));
    assert!(prompt.contains("KEEP ACTIVE STATE"));
    assert_eq!(core.native_exchanges.len(), 1);
    let exchange = &core.native_exchanges[0];
    assert_eq!(exchange.calls.len(), 1);
    assert_eq!(exchange.calls[0].id, "call_after_compact");
    assert_eq!(exchange.results.len(), 1);
    assert_eq!(exchange.results[0].call_id, "call_after_compact");
    assert_eq!(exchange.results[0].name, "self_tool");
    assert!(exchange.results[0].content.contains("CWD:"));
}

#[test]
fn native_context_compact_after_another_call_is_rejected() {
    let mut core = test_core("native_compact_not_first");
    core.set_interaction_profile(&native_test_profile());
    core.append_delta(vec![(
        "user_question".to_string(),
        "KEEP OLD STATE".to_string(),
    )]);
    let old_delta_id = core.deltas[0].delta_id.clone();
    let cwd_arguments = serde_json::json!({"type": "cwd"});
    let compact_arguments = serde_json::json!({
        "discard": [old_delta_id],
        "summary": "SHOULD NOT APPLY",
    });

    let step = core.apply_model_response(LlmResponse {
        content: String::new(),
        tool_calls: vec![
            NativeToolCall {
                id: "call_before_compact".to_string(),
                name: "self_tool".to_string(),
                raw_arguments: cwd_arguments.to_string(),
                arguments: cwd_arguments,
            },
            NativeToolCall {
                id: "call_compact_second".to_string(),
                name: "context_compact".to_string(),
                raw_arguments: compact_arguments.to_string(),
                arguments: compact_arguments,
            },
        ],
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });
    let CoreStep::NeedModel { prompt, .. } = step else {
        panic!("non-first context_compact should request protocol repair")
    };

    assert!(prompt.contains("context_compact_must_be_first"));
    assert!(prompt.contains("KEEP OLD STATE"));
    assert!(!prompt.contains("SHOULD NOT APPLY"));
    assert!(core.native_exchanges.is_empty());
}

#[test]
fn failed_context_compact_blocks_later_native_calls() {
    let mut core = test_core("native_compact_failure_barrier");
    core.set_interaction_profile(&native_test_profile());
    core.append_delta(vec![(
        "user_question".to_string(),
        "ACTIVE STATE".to_string(),
    )]);
    let compact_arguments = serde_json::json!({
        "discard": ["pd_missing"],
        "summary": "INVALID COMPACT",
    });
    let cwd_arguments = serde_json::json!({"type": "cwd"});

    let step = core.apply_model_response(LlmResponse {
        content: String::new(),
        tool_calls: vec![
            NativeToolCall {
                id: "call_bad_compact".to_string(),
                name: "context_compact".to_string(),
                raw_arguments: compact_arguments.to_string(),
                arguments: compact_arguments,
            },
            NativeToolCall {
                id: "call_must_not_run".to_string(),
                name: "self_tool".to_string(),
                raw_arguments: cwd_arguments.to_string(),
                arguments: cwd_arguments,
            },
        ],
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });
    let CoreStep::NeedModel { prompt, .. } = step else {
        panic!("failed context_compact should continue without executing later calls")
    };

    assert!(prompt.contains("error: invalid_prompt_refs"));
    assert!(!prompt.contains("Action result: self_tool"));
    assert!(prompt.contains("ACTIVE STATE"));
    assert!(core.native_exchanges.is_empty());
}

#[test]
fn product_default_and_explicit_unlimited_have_no_round_limit() {
    assert_eq!(configured_round_budget(None), UNLIMITED_ROUND_BUDGET);
    assert_eq!(
        configured_round_budget(Some("unlimited")),
        UNLIMITED_ROUND_BUDGET
    );
}

#[test]
fn benchmark_round_budget_accepts_three_hundred() {
    assert_eq!(configured_round_budget(Some("300")), 300);
}

#[test]
fn invalid_round_budget_uses_product_default() {
    assert_eq!(configured_round_budget(Some("0")), DEFAULT_ROUND_BUDGET);
    assert_eq!(
        configured_round_budget(Some("not-a-number")),
        DEFAULT_ROUND_BUDGET
    );
}

#[test]
fn mem_guard_different_domains_do_not_block_each_other() {
    let dir = std::env::temp_dir().join(format!(
        "timem_mem_guard_domains_{}_{}",
        std::process::id(),
        now_ms()
    ));
    let memory_dir = dir.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();

    let first = MemGuard::for_memory_domain(&memory_dir, "session-index");
    let second = MemGuard::for_memory_domain(&memory_dir, "durable-memory");
    let marker = dir.join("second-domain-finished");
    let marker_for_thread = marker.clone();

    let handle = first
        .with_write(|| {
            let second_thread = std::thread::spawn(move || {
                second
                    .with_write(|| std::fs::write(marker_for_thread, "done"))
                    .unwrap()
                    .unwrap();
            });
            let started = std::time::Instant::now();
            while !marker.exists() && started.elapsed() < std::time::Duration::from_secs(2) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(
                marker.exists(),
                "a writer in another consistency domain must not wait"
            );
            second_thread
        })
        .unwrap();

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mem_guard_reads_do_not_wait_for_same_domain_writer() {
    let dir = std::env::temp_dir().join(format!(
        "timem_mem_guard_read_{}_{}",
        std::process::id(),
        now_ms()
    ));
    let memory_dir = dir.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();

    let writer = MemGuard::for_memory_domain(&memory_dir, "durable-memory");
    let reader = writer.clone();

    writer
        .with_write(|| {
            let started = std::time::Instant::now();
            let observed = reader.with_read(|| "consistent-snapshot").unwrap();
            assert_eq!(observed, "consistent-snapshot");
            assert!(
                started.elapsed() < std::time::Duration::from_millis(100),
                "read path unexpectedly waited for the writer lock"
            );
        })
        .unwrap();

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mem_guard_same_domain_still_serializes_writers() {
    let dir = std::env::temp_dir().join(format!(
        "timem_mem_guard_same_domain_{}_{}",
        std::process::id(),
        now_ms()
    ));
    let memory_dir = dir.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();

    let first = MemGuard::for_memory_domain(&memory_dir, "scratch-notes");
    let second = first.clone();
    let marker = dir.join("second-writer-finished");
    let marker_for_thread = marker.clone();

    let handle = first
        .with_write(|| {
            let second_thread = std::thread::spawn(move || {
                second
                    .with_write(|| std::fs::write(marker_for_thread, "done"))
                    .unwrap()
                    .unwrap();
            });
            std::thread::sleep(std::time::Duration::from_millis(120));
            assert!(
                !marker.exists(),
                "writers in the same consistency domain must remain serialized"
            );
            second_thread
        })
        .unwrap();

    handle.join().unwrap();
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "done");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn atomic_snapshot_readers_only_observe_complete_documents() {
    let dir = std::env::temp_dir().join(format!(
        "timem_atomic_snapshot_{}_{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("snapshot.json");
    let first = br#"{"generation":0,"payload":"first"}"#;
    atomic_write_file(&path, first).unwrap();

    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        for generation in 1..=200 {
            let payload = format!(
                r#"{{"generation":{generation},"payload":"{}"}}"#,
                "x".repeat(4096)
            );
            atomic_write_file(&writer_path, payload.as_bytes()).unwrap();
        }
    });

    while !writer.is_finished() {
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|error| {
            panic!("reader observed an incomplete snapshot: {error}: {text}")
        });
        assert!(value
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            .is_some());
        assert!(value
            .get("payload")
            .and_then(serde_json::Value::as_str)
            .is_some());
    }
    writer.join().unwrap();

    let final_text = std::fs::read_to_string(&path).unwrap();
    let final_value: serde_json::Value = serde_json::from_str(&final_text).unwrap();
    assert_eq!(final_value["generation"], 200);
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn mem_guard_reclaims_a_fresh_lock_owned_by_a_dead_process() {
    let dir = std::env::temp_dir().join(format!(
        "timem_dead_mem_guard_{}_{}",
        std::process::id(),
        now_ms()
    ));
    let memory_dir = dir.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let guard = MemGuard::for_memory_dir(&memory_dir);
    std::fs::create_dir_all(&guard.lock_dir).unwrap();
    std::fs::write(
        guard.lock_dir.join("owner.json"),
        serde_json::json!({"pid": i32::MAX, "created_at_ms": now_ms()}).to_string(),
    )
    .unwrap();

    guard.with_write(|| ()).unwrap();
    assert!(!guard.lock_dir.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn completed_background_bash_emits_terminal_topic_for_original_action() {
    #[derive(Default)]
    struct TopicRecorder(Vec<CoreTopicEvent>);

    impl ActionRuntime for TopicRecorder {
        fn should_cancel(&mut self) -> bool {
            false
        }

        fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
            self.0.extend_from_slice(events);
        }
    }

    let mut core = test_core("background_exit_topic");
    core.set_response_protocol(ResponseProtocolKind::Json);
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let _ = core.begin_turn("start a background command", None);
    let mut runtime = TopicRecorder::default();
    let step = core.apply_model_response_with_action_runtime(
        LlmResponse {
            tool_calls: Vec::new(),
            content: r#"{"status":"working","working_still_action":[{"run_bash":{"cmd":"sleep 0.1; printf done","background":true}}]}"#.to_string(),
            model_name: "test".to_string(),
            usage: UsageStats::zero(),
            truncated: false,
        },
        &mut runtime,
    );
    let prompt = match step {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("expected model continuation, got {other:?}"),
    };
    let background = runtime
        .0
        .iter()
        .find(|event| event.payload["status"] == "background_running")
        .expect("background-running topic");
    let action_id = background.payload["action_id"]
        .as_str()
        .expect("action id")
        .to_string();
    assert!(!action_id.is_empty());

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let _ = core.build_model_request_prompt_with_runtime(&prompt, &mut runtime);
        if runtime.0.iter().any(|event| {
            event.payload["event"] == "finish"
                && event.payload["status"] == "completed"
                && event.payload["action_id"] == action_id
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "missing terminal background topic"
        );
        thread::sleep(Duration::from_millis(25));
    }

    let terminal = runtime
        .0
        .iter()
        .find(|event| {
            event.payload["event"] == "finish"
                && event.payload["status"] == "completed"
                && event.payload["action_id"] == action_id
        })
        .expect("terminal background topic");
    assert_eq!(terminal.payload["action"], "run_bash");
    assert_eq!(terminal.payload["exit_status"], "0");
    assert_eq!(terminal.payload["turn_id"], core.current_action_turn_id());
}

#[test]
fn parallel_sub_answers_are_all_shown_in_declared_order() {
    #[derive(Default)]
    struct TopicRecorder(Vec<CoreTopicEvent>);

    impl ActionRuntime for TopicRecorder {
        fn should_cancel(&mut self) -> bool {
            false
        }

        fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
            self.0.extend_from_slice(events);
        }
    }

    let mut core = test_core("parallel_sub_answers");
    core.set_response_protocol(ResponseProtocolKind::Json);
    let _ = core.begin_turn("answer three independent questions", None);
    let actions = (1..=3)
        .map(|index| ParsedAction {
            action: "sub_answer".to_string(),
            name: None,
            call_id: format!("call_sub_{index}"),
            raw_input: json!({
                "task": format!("Question {index}"),
                "answer": format!("Answer {index}"),
            }),
        })
        .collect();
    let mut runtime = TopicRecorder::default();

    let results = core
        .execute_action_groups(
            vec![ParsedActionGroup {
                order: ActionGroupOrder::Parallel,
                actions,
            }],
            &mut runtime,
        )
        .expect("parallel sub answers should not require approval");

    assert_eq!(results.len(), 3);
    assert!(results
        .iter()
        .all(|result| result.contains("Shown to user successfully.")));
    assert!(results
        .iter()
        .all(|result| !result.contains("parallel_use_not_allowed")));
    let sub_answers = runtime
        .0
        .iter()
        .filter(|event| event.topic.name == CORE_TOPIC_SUB_ANSWER)
        .collect::<Vec<_>>();
    assert_eq!(sub_answers.len(), 3);
    for (index, event) in sub_answers.into_iter().enumerate() {
        let ordinal = u64::try_from(index + 1).unwrap();
        assert_eq!(event.payload["ordinal"], ordinal);
        assert_eq!(event.payload["task"], format!("Question {ordinal}"));
        assert_eq!(event.payload["answer"], format!("Answer {ordinal}"));
    }
}

fn test_core(name: &str) -> AgentCore {
    let dir = std::env::temp_dir().join(format!(
        "timem_prompt_component_test_{}_{}",
        name,
        super::unique_id("tmp")
    ));
    AgentCore::new(
        "static prompt\n{{RESPONSE_PROTOCOL_SECTION}}\n{{TOOL_CATALOG}}\n",
        CoreProfile {
            model: "test".to_string(),
        },
        dir,
    )
}

#[test]
fn build_next_prompt_orders_pending_components_without_role_merging() {
    let mut core = test_core("ordering");
    core.set_assistant_speaker_name("Ai4");

    core.submit_prompt_component_at(
        PromptComponentRole::system(),
        "result_of_llm_action",
        "Action result: run_bash\nold result",
        "previous_model_response",
        10,
    );
    core.submit_prompt_component_at(
        PromptComponentRole::user(),
        "user_question",
        "new input",
        "user_input",
        20,
    );
    core.submit_prompt_component_at(
        PromptComponentRole::system(),
        "runtime_note",
        "found something new",
        "runtime",
        30,
    );
    core.submit_prompt_component_at(
        PromptComponentRole::assistant("Ai4"),
        "free_talk",
        "assistant note",
        "previous_model_response",
        40,
    );

    let prompt = core.build_next_prompt();
    let system_first = prompt.find("<RUNTIME>\n\nAction result: run_bash").unwrap();
    let action_result = prompt.find("Action result: run_bash").unwrap();
    let user = prompt.find("<USER>\n\nnew input").unwrap();
    let system_second = prompt.find("<RUNTIME>\n\nfound something new").unwrap();
    let assistant = prompt.find("<ASSISTANT>\n\nassistant note").unwrap();

    assert!(system_first < user);
    assert!(system_first < action_result);
    assert!(action_result < user);
    assert!(user < system_second);
    assert!(system_second < assistant);
    assert!(prompt.matches("<RUNTIME>").count() >= 2);
    let dynamic_prompt = prompt.split("<prompt_delta ").nth(1).unwrap_or("");
    assert!(!dynamic_prompt.contains("created_at_ms"));
    assert!(!dynamic_prompt.contains("sequence"));
    assert!(!dynamic_prompt.contains("batch_id"));
}

#[test]
fn common_prompt_component_ingress_marks_every_truncated_action_result() {
    let mut core = test_core("action_result_truncation");
    let oversized = format!(
        "Action result: readfile\ncontent:\n{} alpha beta gamma",
        "x".repeat(prompt_render::MAX_ACTION_RESULT_PROMPT_BYTES)
    );
    core.submit_prompt_component(
        PromptComponentRole::system(),
        "action_result",
        oversized,
        "readfile",
    );
    let prompt = core.build_next_prompt();
    assert!(prompt.contains("Action result: readfile"));
    assert!(prompt.contains("words truncated. Generate more actions if necessary !!!"));
    assert!(!prompt.ends_with('…'));
}

#[test]
fn model_result_gate_uses_each_actions_tail_out_policy() {
    let mut core = test_core("tail_result_gate");
    core.set_response_protocol(ResponseProtocolKind::Json);
    let raw = format!("BEGIN_MARKER {} END_MARKER", "内容 ".repeat(20_000));
    let outcome = ActionOutcome::completed(raw);

    let head = core.format_action_outcome(
        &ParsedAction {
            action: "run_bash".to_string(),
            name: None,
            call_id: "test_call".to_string(),
            raw_input: json!({"tail_out": false}),
        },
        &outcome,
    );
    assert!(head.contains("BEGIN_MARKER"));
    assert!(!head.contains("END_MARKER"));
    assert!(head.contains("words truncated."));

    let tail = core.format_action_outcome(
        &ParsedAction {
            action: "run_bash".to_string(),
            name: None,
            call_id: "test_call".to_string(),
            raw_input: json!({"tail_out": true}),
        },
        &outcome,
    );
    assert!(!tail.contains("BEGIN_MARKER"));
    assert!(tail.contains("END_MARKER"));
    assert!(tail.contains("!!!Too long,"));
    assert!(tail.contains("truncated before"));
    assert!(
        head.len() <= tool_result_gate::MAX_MODEL_TOOL_RESULT_BYTES + 64,
        "tool-call correlation metadata stays bounded"
    );
    assert!(
        tail.len() <= tool_result_gate::MAX_MODEL_TOOL_RESULT_BYTES + 64,
        "tool-call correlation metadata stays bounded"
    );
}

#[test]
fn xml_model_result_gate_retains_tail_inside_a_complete_envelope() {
    let mut core = test_core("xml_tail_result_gate");
    core.set_response_protocol(ResponseProtocolKind::Xml);
    let raw = format!("BEGIN_MARKER {} END_MARKER", "内容 ".repeat(20_000));
    let outcome = ActionOutcome::completed("unused").with_bash_result(BashResultEvidence {
        stdout: raw,
        stderr: String::new(),
        exit_code: Some(0),
        signal: None,
        pid: None,
        timed_out: false,
        pid_kind: None,
        error_type: None,
    });
    let result = core.format_action_outcome(
        &ParsedAction {
            action: "run_bash".to_string(),
            name: Some("tail XML".to_string()),
            call_id: "test_call".to_string(),
            raw_input: json!({"tail_out": true}),
        },
        &outcome,
    );

    assert!(result.contains("<bash_result "));
    assert!(result.ends_with("</bash_result>"));
    assert!(result.contains("truncated before"));
    assert!(!result.contains("BEGIN_MARKER"));
    assert!(result.contains("END_MARKER"));
    assert!(
        result.len() <= tool_result_gate::MAX_MODEL_TOOL_RESULT_BYTES + 64,
        "tool-call correlation metadata stays bounded"
    );
}

#[test]
fn previous_model_response_components_share_earliest_logical_time() {
    let mut core = test_core("previous_batch");
    let batch_time = 100;
    core.submit_prompt_components_from_slice_texts(
            vec![
                (
                    "llm_free_talk".to_string(),
                    "previous free talk".to_string(),
                ),
                (
                    "llm_response".to_string(),
                    "All previous pending open tasks are completed. Do not repeat this previous answer unless the user asks to quote it. Final Answer:\nprevious final"
                        .to_string(),
                ),
            ],
            "previous_model_response",
            batch_time,
        );
    core.submit_prompt_component_at(
        PromptComponentRole::user(),
        "user_question",
        "next user input",
        "user_input",
        200,
    );

    assert_eq!(core.pending_prompt_components.len(), 3);
    assert!(core.pending_prompt_components[..2]
        .iter()
        .all(|component| component.created_at_ms == batch_time));
    assert!(
        core.pending_prompt_components[0].sequence < core.pending_prompt_components[1].sequence
    );

    let prompt = core.build_next_prompt();
    let free_talk = prompt.find("previous free talk").unwrap();
    let final_answer = prompt.find("previous final").unwrap();
    let user = prompt.find("next user input").unwrap();
    assert!(free_talk < user);
    assert!(final_answer < user);
}

#[test]
fn sudden_large_action_output_is_replaced_before_crossing_safety_limit() {
    let mut core = test_core("large_action_output_guard");
    core.set_max_llm_input_tokens(10_000);
    core.last_observed_prompt_tokens = 9_400;
    let oversized_marker = "OVERSIZED_ACTION_MARKER";
    let oversized = format!("{oversized_marker}{}", "x".repeat(8_000));

    let rejected = core.append_delta_with_action_output_budget(vec![
        (
            "llm_free_talk".to_string(),
            "I inspected the output.".to_string(),
        ),
        ("result_of_llm_action".to_string(), oversized),
    ]);
    let prompt = core.render_prompt();

    assert!(rejected);
    assert!(!prompt.contains(oversized_marker));
    assert!(prompt.contains("Your action's output is too large:"));
    assert!(prompt.contains("You need to optimize your action or compact context."));
    assert!(!prompt.contains("I inspected the output."));
}

#[test]
fn combined_multi_action_output_is_budgeted_as_one_delta() {
    let mut core = test_core("multi_action_output_guard");
    core.set_max_llm_input_tokens(10_000);
    core.last_observed_prompt_tokens = 9_300;
    let result = [
        format!("Action result: first\nFIRST_BURST{}", "a".repeat(2_000)),
        format!("Action result: second\nSECOND_BURST{}", "b".repeat(2_000)),
    ]
    .join("\n\n");

    assert!(core.append_delta_with_action_output_budget(vec![(
        "result_of_llm_action".to_string(),
        result,
    )]));
    let prompt = core.render_prompt();
    assert!(!prompt.contains("FIRST_BURST"));
    assert!(!prompt.contains("SECOND_BURST"));
    assert_eq!(
        prompt.matches("Your action's output is too large:").count(),
        1
    );
}

#[test]
fn same_batch_pending_action_updates_are_removed_with_oversized_delta() {
    let mut core = test_core("pending_action_update_guard");
    core.set_max_llm_input_tokens(10_000);
    core.last_observed_prompt_tokens = 9_200;
    core.submit_prompt_component(
        PromptComponentRole::system(),
        "running_job_update",
        format!("PENDING_JOB_OUTPUT{}", "z".repeat(3_000)),
        "runtime",
    );

    assert!(core.append_delta_with_action_output_budget(vec![(
        "result_of_llm_action".to_string(),
        "Action result: run_bash\nsmall result".to_string(),
    )]));
    let prompt = core.render_prompt();
    assert!(!prompt.contains("PENDING_JOB_OUTPUT"));
    assert!(!prompt.contains("small result"));
    assert!(prompt.contains("Your action's output is too large:"));
}

#[test]
fn build_next_prompt_guards_pending_precheck_output_without_losing_user_input() {
    let mut core = test_core("pending_precheck_output_guard");
    core.set_max_llm_input_tokens(10_000);
    core.last_observed_prompt_tokens = 9_100;
    core.submit_prompt_component(
        PromptComponentRole::user(),
        "user_question",
        "Keep this new user question",
        "user_input",
    );
    core.submit_prompt_component(
        PromptComponentRole::system(),
        "result_of_llm_action",
        format!("MEMORY_PRECHECK_BURST{}", "记".repeat(1_000)),
        "runtime_memory_precheck",
    );

    let prompt = core.build_next_prompt();
    assert!(prompt.contains("Keep this new user question"));
    assert!(!prompt.contains("MEMORY_PRECHECK_BURST"));
    assert!(prompt.contains("Your action's output is too large:"));
}

#[test]
fn action_output_at_or_below_safety_limit_is_preserved() {
    let mut core = test_core("action_output_below_limit");
    core.set_max_llm_input_tokens(10_000);
    core.last_observed_prompt_tokens = 1_000;

    assert!(!core.append_delta_with_action_output_budget(vec![(
        "result_of_llm_action".to_string(),
        "Action result: run_bash\nSAFE_RESULT".to_string(),
    )]));
    let prompt = core.render_prompt();
    assert!(prompt.contains("SAFE_RESULT"));
    assert!(!prompt.contains("Your action's output is too large:"));
}

#[test]
fn action_output_budget_accepts_exact_95_percent_and_rejects_the_next_token() {
    const MAX_INPUT_TOKENS: u32 = 10_000;
    const SAFETY_LIMIT_TOKENS: u32 = MAX_INPUT_TOKENS * ACTION_OUTPUT_CONTEXT_SAFETY_PERCENT / 100;

    let mut at_limit = test_core("action_output_exact_95");
    at_limit.set_max_llm_input_tokens(MAX_INPUT_TOKENS);
    let current_tokens = estimate_prompt_tokens(&at_limit.render_prompt());
    let available_tokens = SAFETY_LIMIT_TOKENS
        .saturating_sub(current_tokens)
        .saturating_sub(PROMPT_DELTA_RENDER_OVERHEAD_TOKENS);
    assert!(available_tokens > 10);
    let exact_output = "x".repeat(available_tokens as usize * 4);
    assert!(!at_limit.append_delta_with_action_output_budget(vec![(
        "result_of_llm_action".to_string(),
        exact_output,
    )]));

    let mut over_limit = test_core("action_output_over_95");
    over_limit.set_max_llm_input_tokens(MAX_INPUT_TOKENS);
    let current_tokens = estimate_prompt_tokens(&over_limit.render_prompt());
    let available_tokens = SAFETY_LIMIT_TOKENS
        .saturating_sub(current_tokens)
        .saturating_sub(PROMPT_DELTA_RENDER_OVERHEAD_TOKENS);
    let one_token_over = "x".repeat(available_tokens as usize * 4 + 1);
    assert!(over_limit.append_delta_with_action_output_budget(vec![(
        "result_of_llm_action".to_string(),
        one_token_over,
    )]));
}

#[test]
fn non_ascii_action_burst_uses_conservative_token_estimation() {
    let mut core = test_core("non_ascii_action_burst");
    core.set_max_llm_input_tokens(10_000);
    core.last_observed_prompt_tokens = 8_500;
    let chinese_output = format!("中文突发输出标记{}", "数".repeat(1_100));

    assert!(core.append_delta_with_action_output_budget(vec![(
        "result_of_llm_action".to_string(),
        chinese_output,
    )]));
    let prompt = core.render_prompt();
    assert!(!prompt.contains("中文突发输出标记"));
    assert!(prompt.contains("Your action's output is too large:"));
}

#[test]
fn model_input_overflow_recovery_removes_only_latest_action_results() {
    let mut core = test_core("model_input_overflow_recovery");
    core.set_max_llm_input_tokens(20_000);
    core.append_delta(vec![
        (
            "llm_free_talk".to_string(),
            "keep this assistant state".to_string(),
        ),
        (
            "result_of_llm_action".to_string(),
            "Action result: run_bash\nREMOVE_THIS_OUTPUT".to_string(),
        ),
    ]);

    let recovery = core
        .recover_from_model_input_too_large("model_http_400: context_length_exceeded")
        .expect("latest action result should be recoverable");
    let step = recovery.step;
    let CoreStep::NeedModel { prompt, .. } = step else {
        panic!("overflow recovery should continue with a model request");
    };
    assert!(!prompt.contains("keep this assistant state"));
    assert!(!prompt.contains("REMOVE_THIS_OUTPUT"));
    assert!(prompt.contains("Your action's output is too large:"));
    assert!(prompt.contains("context_length_exceeded"));
    assert!(core
        .recover_from_model_input_too_large("model_http_413")
        .is_none());
}

#[test]
fn model_input_overflow_does_not_delete_older_action_history() {
    let mut core = test_core("model_input_overflow_keeps_old_history");
    core.append_delta(vec![(
        "result_of_llm_action".to_string(),
        "Action result: run_bash\nOLDER_RESULT".to_string(),
    )]);
    core.append_delta(vec![(
        "user_question".to_string(),
        "A newer user message that is not an action result".to_string(),
    )]);

    assert!(core
        .recover_from_model_input_too_large("model_http_413")
        .is_none());
    let prompt = core.render_prompt();
    assert!(prompt.contains("OLDER_RESULT"));
    assert!(prompt.contains("A newer user message"));
}

#[test]
fn action_topic_pid_requires_managed_running_bash_evidence() {
    let forged_text = ActionOutcome::new(
        ActionStatus::BackgroundRunning,
        "Action result: run_bash\npid=49189, timeout, but is still running",
    );
    assert_eq!(super::managed_running_bash_pid(&forged_text), None);

    let mut managed = ActionOutcome::new(
        ActionStatus::BackgroundRunning,
        "human-readable text without a pid",
    );
    managed.bash_result = Some(BashResultEvidence {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: None,
        signal: None,
        pid: Some(49189),
        timed_out: true,
        pid_kind: Some(super::managed_bash_pid_kind().to_string()),
        error_type: None,
    });
    assert_eq!(super::managed_running_bash_pid(&managed), Some(49189));

    managed.status = ActionStatus::Timeout;
    assert_eq!(super::managed_running_bash_pid(&managed), None);

    managed.status = ActionStatus::BackgroundRunning;
    managed.bash_result.as_mut().unwrap().pid_kind = Some("external_process".to_string());
    assert_eq!(super::managed_running_bash_pid(&managed), None);

    #[cfg(unix)]
    {
        managed.bash_result.as_mut().unwrap().pid_kind = Some("runtime_child_process".to_string());
        assert_eq!(super::managed_running_bash_pid(&managed), None);
    }
    #[cfg(not(unix))]
    {
        managed.bash_result.as_mut().unwrap().pid_kind =
            Some("runtime_child_process_group".to_string());
        assert_eq!(super::managed_running_bash_pid(&managed), None);
    }
}

fn test_mcp_tool(action_name: &str, description: &str) -> mcp::McpTool {
    mcp::McpTool {
        server_id: "test_server".to_string(),
        server_name: "Test".to_string(),
        name: action_name.to_string(),
        action_name: action_name.to_string(),
        description: description.to_string(),
        input_schema: json!({ "type": "object", "properties": {} }),
    }
}

fn test_mcp_server() -> mcp::McpServerConfig {
    mcp::McpServerConfig {
        id: "filesystem".to_string(),
        name: "Filesystem MCP".to_string(),
        enabled: true,
        transport: mcp::McpTransportConfig::default(),
        request_timeout_ms: 1_000,
    }
}

fn test_filesystem_mcp_tool() -> mcp::McpTool {
    let mut tool = test_mcp_tool("mcp_filesystem_mcp__echo", "Echo");
    tool.server_id = "filesystem".to_string();
    tool.server_name = "Filesystem MCP".to_string();
    tool
}

#[test]
fn mcp_capability_update_is_injected_only_when_tool_content_changes() {
    let mut core = test_core("mcp_deferred_update");
    let original = test_mcp_tool("mcp_test__echo", "Original description");
    core.configure_mcp(
        CapabilityRegistry::builtin(),
        mcp::McpRuntime::default(),
        Vec::new(),
        vec![original.clone()],
    )
    .unwrap();
    assert!(core.pending_prompt_components.is_empty());
    assert_eq!(core.deltas.len(), 1);

    assert!(!core
        .apply_mcp_update(
            CapabilityRegistry::builtin(),
            mcp::McpRuntime::default(),
            Vec::new(),
            vec![original],
        )
        .unwrap());
    assert!(core.pending_prompt_components.is_empty());
    assert_eq!(core.deltas.len(), 1);

    assert!(core
        .apply_mcp_update(
            CapabilityRegistry::builtin(),
            mcp::McpRuntime::default(),
            Vec::new(),
            vec![
                test_mcp_tool("mcp_test__echo", "Updated description"),
                test_mcp_tool("mcp_test__search", "Search description"),
            ],
        )
        .unwrap());
    assert!(core.pending_prompt_components.is_empty());
    let prompt = core.build_next_prompt();
    assert!(prompt.contains("<RUNTIME>"));
    assert!(prompt.contains("MCP update: newly available actions: mcp_test__search."));
    assert!(prompt.contains("MCP update: updated action definitions: mcp_test__echo."));

    assert!(core
        .apply_mcp_update(
            CapabilityRegistry::builtin(),
            mcp::McpRuntime::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap());
    let prompt = core.build_next_prompt();
    assert!(prompt
        .contains("MCP update: actions no longer available: mcp_test__echo, mcp_test__search."));
}

#[test]
fn disabling_mcp_server_appends_explicit_persistent_runtime_update() {
    let mut core = test_core("mcp_disabled_update");
    core.configure_mcp(
        CapabilityRegistry::builtin(),
        mcp::McpRuntime::default(),
        vec![test_mcp_server()],
        vec![test_filesystem_mcp_tool()],
    )
    .unwrap();
    let catalog_delta_count = core.deltas.len();
    assert!(core
        .build_next_prompt()
        .contains("MCP update: MCP Filesystem MCP (filesystem) IS ENABLED by user !!!"));

    assert!(core
        .apply_mcp_update(
            CapabilityRegistry::builtin(),
            mcp::McpRuntime::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap());
    assert_eq!(core.deltas.len(), catalog_delta_count + 1);
    let prompt = core.build_next_prompt();
    assert!(prompt.contains("MCP update: MCP Filesystem MCP (filesystem) IS DISABLED by user !!!"));
}

#[test]
fn model_transparent_mcp_configuration_update_does_not_append_prompt_delta() {
    let mut core = test_core("mcp_configuration_update");
    core.configure_mcp(
        CapabilityRegistry::builtin(),
        mcp::McpRuntime::default(),
        vec![test_mcp_server()],
        Vec::new(),
    )
    .unwrap();
    let delta_count = core.deltas.len();
    let mut updated = test_mcp_server();
    updated.request_timeout_ms = 2_000;

    assert!(!core
        .apply_mcp_update(
            CapabilityRegistry::builtin(),
            mcp::McpRuntime::default(),
            vec![updated],
            Vec::new(),
        )
        .unwrap());
    assert_eq!(core.deltas.len(), delta_count);
    let prompt = core.build_next_prompt();
    assert!(!prompt.contains("CONFIGURATION IS UPDATED"));
    assert!(!prompt.contains("request_timeout_ms"));
}

#[test]
fn mcp_server_instructions_are_persistent_and_model_visible_changes_append_updates() {
    let mut core = test_core("mcp_server_instructions");
    core.configure_mcp_with_instructions(
        CapabilityRegistry::builtin(),
        mcp::McpRuntime::default(),
        vec![test_mcp_server()],
        Vec::new(),
        BTreeMap::from([(
            "filesystem".to_string(),
            "Read metadata before modifying a file.".to_string(),
        )]),
    )
    .unwrap();
    let initial_prompt = core.build_next_prompt();
    assert!(initial_prompt
        .contains("MCP update: MCP Filesystem MCP (filesystem) IS ENABLED by user !!!"));
    assert!(initial_prompt.contains("Read metadata before modifying a file."));
    assert!(initial_prompt.contains("\"server_instructions\""));
    let initial_delta_count = core.deltas.len();

    assert!(core
        .apply_mcp_update_with_instructions(
            CapabilityRegistry::builtin(),
            mcp::McpRuntime::default(),
            vec![test_mcp_server()],
            Vec::new(),
            BTreeMap::from([(
                "filesystem".to_string(),
                "Preserve file metadata after every modification.".to_string(),
            )]),
        )
        .unwrap());
    assert_eq!(core.deltas.len(), initial_delta_count + 1);
    let updated_prompt = core.build_next_prompt();
    assert!(updated_prompt
        .contains("MCP update: instructions for MCP Filesystem MCP (filesystem) ARE UPDATED."));
    assert!(updated_prompt.contains("Preserve file metadata after every modification."));

    assert!(core
        .apply_mcp_update_with_instructions(
            CapabilityRegistry::builtin(),
            mcp::McpRuntime::default(),
            vec![test_mcp_server()],
            Vec::new(),
            BTreeMap::new(),
        )
        .unwrap());
    assert!(core.build_next_prompt().contains(
        "MCP update: instructions for MCP Filesystem MCP (filesystem) ARE NO LONGER ACTIVE !!!"
    ));
}

#[test]
fn multiple_successful_compacts_emit_one_minimal_runtime_confirmation() {
    let mut core = test_core("multiple_compacts_mcp_note");
    core.set_response_protocol(ResponseProtocolKind::Json);
    core.configure_mcp(
        CapabilityRegistry::builtin(),
        mcp::McpRuntime::default(),
        Vec::new(),
        vec![test_mcp_tool("mcp_test__echo", "Echo")],
    )
    .unwrap();
    core.append_delta(vec![("user_question".to_string(), "old one".to_string())]);
    core.append_delta(vec![("user_question".to_string(), "old two".to_string())]);
    let first_id = core.deltas[0].delta_id.clone();
    let second_id = core.deltas[1].delta_id.clone();

    let step = core.apply_model_response(LlmResponse {
        tool_calls: Vec::new(),
        content: serde_json::json!({
            "free_talk": "compact both",
            "context_compact": [
                { "discard": [first_id], "summary": "first summary" },
                { "discard": [second_id], "summary": "second summary" }
            ]
        })
        .to_string(),
        model_name: "test".to_string(),
        usage: UsageStats::zero(),
        truncated: false,
    });
    let CoreStep::NeedModel { prompt, .. } = step else {
        panic!("context compact should continue with a model request")
    };
    assert_eq!(prompt.matches("context compacted successfully.").count(), 1);
    assert_eq!(
        prompt
            .matches("context compacted successfully.\nCWD: ")
            .count(),
        1
    );
    assert!(!prompt.contains("Active MCP capabilities after context compaction"));
    assert!(!prompt.contains("Action result: context_compact"));
    assert!(!prompt.contains("removed_delta_count:"));
    assert!(!prompt.contains("scratch_id:"));
    assert_eq!(
        prompt
            .matches("MCP update: the following MCP capabilities are enabled")
            .count(),
        1,
        "compacting the active catalog must persist exactly one replacement catalog: {prompt}"
    );
    assert!(prompt.contains("mcp_test__echo"));
}

#[test]
fn workspace_instance_lock_is_exclusive_per_mem_and_reopens_after_release() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let first_root = std::env::temp_dir().join(format!(
        "timem-workspace-instance-lock-first-{}-{nonce}",
        std::process::id()
    ));
    let second_root = std::env::temp_dir().join(format!(
        "timem-workspace-instance-lock-second-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&first_root).unwrap();
    fs::create_dir_all(&second_root).unwrap();

    let first = WorkspaceInstanceLock::acquire(&first_root, "timem-shell").unwrap();
    let owner = WorkspaceInstanceLock::read_owner(&first_root).unwrap();
    assert_eq!(owner.pid, std::process::id());
    assert_eq!(owner.host, "timem-shell");
    assert_eq!(WorkspaceInstanceLock::lock_path(&first_root), first.path());
    assert_eq!(
        WorkspaceInstanceLock::acquire(&first_root, "timem-web").unwrap_err(),
        "workspace_already_in_use"
    );
    let other = WorkspaceInstanceLock::acquire(&second_root, "timem-web").unwrap();
    drop(other);
    drop(first);
    let reopened = WorkspaceInstanceLock::acquire(&first_root, "timem-web").unwrap();
    drop(reopened);

    let _ = fs::remove_dir_all(first_root);
    let _ = fs::remove_dir_all(second_root);
}

#[test]
fn prompt_marks_logical_turns_independently_from_deltas() {
    let mut core = test_core("explicit_turn_boundaries");

    let first = match core.begin_turn("first question", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    let first_marker = first
        .rfind("[BEGIN TURN turn_id: action_turn_")
        .expect("first turn marker");
    let first_question = first.rfind("first question").expect("first question");
    assert!(first_marker < first_question, "{first}");

    let supplemented = core
        .append_user_supplement("same-turn supplement")
        .expect("supplement step");
    let supplemented = match supplemented {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(
        supplemented.matches("[BEGIN TURN turn_id:").count(),
        1,
        "a supplement must not open another turn: {supplemented}"
    );
    assert!(supplemented.contains("same-turn supplement"));

    core.defer_next_turn_slices(vec![(
        "llm_response".to_string(),
        "deferred previous answer".to_string(),
    )]);
    let second = match core.begin_turn("second question", None) {
        CoreStep::NeedModel { prompt, .. } => prompt,
        other => panic!("unexpected step: {other:?}"),
    };
    assert_eq!(
        second.matches("[BEGIN TURN turn_id:").count(),
        2,
        "{second}"
    );
    let deferred = second
        .rfind("deferred previous answer")
        .expect("deferred previous answer");
    let second_marker = second
        .rfind("[BEGIN TURN turn_id: action_turn_")
        .expect("second turn marker");
    let second_question = second.rfind("second question").expect("second question");
    assert!(deferred < second_marker, "{second}");
    assert!(second_marker < second_question, "{second}");
}

#[test]
fn action_audit_capacity_removes_oldest_turns_without_changing_schema() {
    let turns = (0..6)
        .map(|index| ActionAuditTurn {
            turn_id: format!("turn_{index}"),
            started_at_ms: index,
            user_question: format!("question {index} {}", "x".repeat(120)),
            interactions: vec![ActionAuditInteraction {
                round: 1,
                actions: vec![ActionAuditEntry {
                    time_ms: index,
                    round: 1,
                    action: "readfile".to_string(),
                    status: "completed".to_string(),
                    input: json!({"path": format!("file_{index}")}),
                    result_summary: Some("ok".to_string()),
                }],
            }],
        })
        .collect::<Vec<_>>();
    let doc = ActionAuditDocument { version: 1, turns };

    let text = bounded_action_audit_text(&doc, 1_400).unwrap();
    let retained: ActionAuditDocument = serde_json::from_str(&text).unwrap();

    assert_eq!(retained.version, 1);
    assert!(!retained.turns.is_empty());
    assert_eq!(retained.turns.last().unwrap().turn_id, "turn_5");
    assert_ne!(retained.turns.first().unwrap().turn_id, "turn_0");
    assert!(text.len() <= 1_400 || retained.turns.len() == 1);
    assert_eq!(retained.turns.last().unwrap().interactions[0].round, 1);
}

#[test]
fn action_audit_capacity_summarizes_one_oversized_turn_without_changing_schema() {
    let doc = ActionAuditDocument {
        version: 1,
        turns: vec![ActionAuditTurn {
            turn_id: "turn_large".to_string(),
            started_at_ms: 1,
            user_question: "q".repeat(20_000),
            interactions: vec![ActionAuditInteraction {
                round: 1,
                actions: vec![
                    ActionAuditEntry {
                        time_ms: 1,
                        round: 1,
                        action: "old_action".to_string(),
                        status: "completed".to_string(),
                        input: json!({"payload": "x".repeat(2_000_000)}),
                        result_summary: Some("old".repeat(10_000)),
                    },
                    ActionAuditEntry {
                        time_ms: 2,
                        round: 1,
                        action: "latest_action".to_string(),
                        status: "completed".to_string(),
                        input: json!({"payload": "y".repeat(2_000_000)}),
                        result_summary: Some("latest".repeat(10_000)),
                    },
                ],
            }],
        }],
    };

    let text = bounded_action_audit_text(&doc, 32 * 1024).unwrap();
    let retained: ActionAuditDocument = serde_json::from_str(&text).unwrap();

    assert!(text.len() <= 32 * 1024, "{}", text.len());
    assert_eq!(retained.version, 1);
    assert_eq!(retained.turns.len(), 1);
    assert_eq!(retained.turns[0].interactions.len(), 1);
    assert_eq!(retained.turns[0].interactions[0].actions.len(), 1);
    let latest = &retained.turns[0].interactions[0].actions[0];
    assert_eq!(latest.action, "latest_action");
    assert_eq!(latest.input["payload_omitted"], true);
    assert!(latest.input["payload_bytes"].as_u64().unwrap() > 1_000_000);
    assert!(retained.turns[0].user_question.contains("original_chars="));
    assert!(latest
        .result_summary
        .as_deref()
        .unwrap()
        .contains("original_chars="));
}

#[test]
fn legacy_multi_turn_action_audit_migrates_before_new_turn_without_losing_history() {
    let root = std::env::temp_dir().join(format!(
        "timem_action_audit_migration_{}_{}",
        std::process::id(),
        now_ms()
    ));
    let audit_dir = root.join("audit");
    fs::create_dir_all(&audit_dir).unwrap();
    let legacy = ActionAuditDocument {
        version: 1,
        turns: vec![
            ActionAuditTurn {
                turn_id: "legacy_one".to_string(),
                started_at_ms: 1,
                user_question: "first".to_string(),
                interactions: Vec::new(),
            },
            ActionAuditTurn {
                turn_id: "legacy_two".to_string(),
                started_at_ms: 2,
                user_question: "second".to_string(),
                interactions: Vec::new(),
            },
        ],
    };
    fs::write(
        audit_dir.join("action_audit.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let store = FileActionAuditStore::new(&root);
    store.begin_turn("new_turn", 3, "third");

    let turns_dir = audit_dir.join("action_audit.json.turns");
    let migrated = fs::read_dir(&turns_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<ActionAuditTurn>(&bytes).ok())
        .map(|turn| turn.turn_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        migrated,
        BTreeSet::from([
            "legacy_one".to_string(),
            "legacy_two".to_string(),
            "new_turn".to_string(),
        ])
    );
    let latest: ActionAuditDocument =
        serde_json::from_slice(&fs::read(audit_dir.join("action_audit.json")).unwrap()).unwrap();
    assert_eq!(latest.turns.len(), 1);
    assert_eq!(latest.turns[0].turn_id, "new_turn");
    let _ = fs::remove_dir_all(root);
}
