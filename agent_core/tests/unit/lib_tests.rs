use super::*;

fn test_core(name: &str) -> AgentCore {
    let dir = std::env::temp_dir().join(format!(
        "timem_prompt_component_test_{}_{}",
        name,
        super::unique_id("tmp")
    ));
    AgentCore::new(
        "static prompt\n{{RESPONSE_PROTOCOL_SECTION}}\n{{TOOL_CATALOG}}\n{{SKILL_HEADERS}}",
        CoreProfile {
            name: "test".to_string(),
            provider: "test".to_string(),
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
    let system_first = prompt
        .find("## SYSTEM\n\nThe following are results of Ai4 newly initiated actions:")
        .unwrap();
    let action_result = prompt.find("Action result: run_bash").unwrap();
    let user = prompt.find("## USER\n\nnew input").unwrap();
    let system_second = prompt.find("## SYSTEM\n\nfound something new").unwrap();
    let assistant = prompt.find("## Ai4\n\nassistant note").unwrap();

    assert!(system_first < user);
    assert!(system_first < action_result);
    assert!(action_result < user);
    assert!(user < system_second);
    assert!(system_second < assistant);
    assert!(prompt.matches("## SYSTEM").count() >= 2);
    let dynamic_prompt = prompt.split("[BEGIN DELTA]").nth(1).unwrap_or("");
    assert!(!dynamic_prompt.contains("created_at_ms"));
    assert!(!dynamic_prompt.contains("sequence"));
    assert!(!dynamic_prompt.contains("batch_id"));
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
    let mut at_limit = test_core("action_output_exact_95");
    at_limit.set_max_llm_input_tokens(10_000);
    let current_tokens = estimate_prompt_tokens(&at_limit.render_prompt());
    let available_tokens = 9_500u32
        .saturating_sub(current_tokens)
        .saturating_sub(PROMPT_DELTA_RENDER_OVERHEAD_TOKENS);
    assert!(available_tokens > 10);
    let exact_output = "x".repeat(available_tokens as usize * 4);
    assert!(!at_limit.append_delta_with_action_output_budget(vec![(
        "result_of_llm_action".to_string(),
        exact_output,
    )]));

    let mut over_limit = test_core("action_output_over_95");
    over_limit.set_max_llm_input_tokens(10_000);
    let current_tokens = estimate_prompt_tokens(&over_limit.render_prompt());
    let available_tokens = 9_500u32
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
fn provider_overflow_recovery_removes_only_latest_action_results() {
    let mut core = test_core("provider_overflow_recovery");
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
        .recover_from_model_input_too_large("provider_http_400: context_length_exceeded")
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
        .recover_from_model_input_too_large("provider_http_413")
        .is_none());
}

#[test]
fn provider_overflow_does_not_delete_older_action_history() {
    let mut core = test_core("provider_overflow_keeps_old_history");
    core.append_delta(vec![(
        "result_of_llm_action".to_string(),
        "Action result: run_bash\nOLDER_RESULT".to_string(),
    )]);
    core.append_delta(vec![(
        "user_question".to_string(),
        "A newer user message that is not an action result".to_string(),
    )]);

    assert!(core
        .recover_from_model_input_too_large("provider_http_413")
        .is_none());
    let prompt = core.render_prompt();
    assert!(prompt.contains("OLDER_RESULT"));
    assert!(prompt.contains("A newer user message"));
}

#[test]
fn action_result_pid_extracts_timeout_pid_for_action_topic_metadata() {
    let result = "Action result: run_bash\npid=49189, timeout, but is still running\nTimeout means Timem stopped waiting; the process was not killed and there is no final exit code yet.\nCommand: sleep 18";
    assert_eq!(super::action_result_pid(result), Some(49189));
}
