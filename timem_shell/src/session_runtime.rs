use crate::{
    append_audit, call_model_with_cancel, format_token_count, supporting_context, ProviderConfig,
    RuntimeProfiler,
};
use agent_core::{AgentCore, ApprovalRequest, CoreStep, LlmResponse, UsageStats};
use serde_json::json;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct TurnRequest<'a> {
    pub input: &'a str,
    pub session: &'a str,
    pub audit_file: &'a Path,
    pub additional_context: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub text: String,
    pub stats: UsageStats,
    pub latest_usage: Option<UsageStats>,
    pub elapsed: Duration,
    pub repair_issue: Option<String>,
}

pub trait TurnUi {
    fn is_cancel_requested(&mut self) -> bool {
        false
    }

    fn take_cancel_request(&mut self) -> bool {
        self.is_cancel_requested()
    }

    fn on_model_request(&mut self, _round: u32, _prompt: &str) {}

    fn on_model_response(&mut self, _round: u32, _usage: &UsageStats, _content: &str) {}

    fn on_model_error(&mut self, _error: &str) {}

    fn pause_for_user_decision(&mut self) {}

    fn resume_after_user_decision(&mut self) {}

    fn request_user_approval(&mut self, _request: &ApprovalRequest) -> bool {
        false
    }

    fn request_round_limit_continue(&mut self, _max_rounds: u32) -> bool {
        false
    }

    fn can_request_output_expansion(&mut self) -> bool {
        false
    }

    fn request_expand_output_tokens(&mut self, _current_tokens: u32) -> bool {
        false
    }
}

pub struct NoopTurnUi;

impl TurnUi for NoopTurnUi {}

trait ModelClient {
    fn call_model(
        &mut self,
        config: &ProviderConfig,
        prompt: &str,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String>;
}

struct CurlModelClient;

impl ModelClient for CurlModelClient {
    fn call_model(
        &mut self,
        config: &ProviderConfig,
        prompt: &str,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        call_model_with_cancel(config, prompt, audit_file, should_cancel)
    }
}

pub fn run_session_turn(
    core: &mut AgentCore,
    config: &mut ProviderConfig,
    request: TurnRequest<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
) -> TurnOutcome {
    let mut model_client = CurlModelClient;
    run_session_turn_with_model_client(core, config, request, ui, profiler, &mut model_client)
}

fn run_session_turn_with_model_client(
    core: &mut AgentCore,
    config: &mut ProviderConfig,
    request: TurnRequest<'_>,
    ui: &mut dyn TurnUi,
    mut profiler: Option<&mut RuntimeProfiler>,
    model_client: &mut dyn ModelClient,
) -> TurnOutcome {
    let turn_id = format!("turn_{}", epoch_millis());
    let _ = append_audit(
        request.audit_file,
        &json!({"type":"turn_start","session":request.session,"turn_id":turn_id,"user_input":request.input}),
    );
    let start = Instant::now();
    let mut user_wait_this_turn = Duration::ZERO;
    let mut context = supporting_context(&config.provider, &config.model, request.input);
    if let Some(extra) = request
        .additional_context
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        context.push_str("\n\n");
        context.push_str(extra);
    }
    let mut step = core.begin_turn(request.input, Some(&context));
    let mut rounds = 0u32;
    let mut model_wait_this_turn = Duration::ZERO;
    let mut latest_usage: Option<UsageStats> = None;

    let (text, stats, latest_usage, repair_issue) = loop {
        if ui.take_cancel_request() {
            break cancelled_turn_result();
        }
        match step {
            CoreStep::NeedModel { ref prompt, .. } => {
                rounds += 1;
                ui.on_model_request(rounds, prompt);
                let model_wait_start = Instant::now();
                match model_client.call_model(config, prompt, request.audit_file, &mut || {
                    ui.is_cancel_requested()
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
                        if ui.take_cancel_request() {
                            break cancelled_turn_result();
                        }
                        if response.truncated && ui.can_request_output_expansion() {
                            ui.pause_for_user_decision();
                            let user_wait_start = Instant::now();
                            let should_expand =
                                ui.request_expand_output_tokens(config.max_llm_output_tokens);
                            user_wait_this_turn =
                                user_wait_this_turn.saturating_add(user_wait_start.elapsed());
                            if should_expand {
                                config.max_llm_output_tokens =
                                    config.max_llm_output_tokens.saturating_add(10_000);
                                let _ = append_audit(
                                    request.audit_file,
                                    &json!({
                                        "type":"max_llm_output_increased",
                                        "session":request.session,
                                        "turn_id":turn_id,
                                        "max_llm_output_tokens":config.max_llm_output_tokens
                                    }),
                                );
                                ui.resume_after_user_decision();
                                continue;
                            }
                            break (
                                format!(
                                    "模型输出达到当前上限 {}，已按你的选择停止本轮。可用 /config 调大 TIMEM_MAX_LLM_OUTPUT 后重试。",
                                    format_token_count(config.max_llm_output_tokens)
                                ),
                                response.usage.clone(),
                                Some(response.usage),
                                Some("truncated_output_stopped_by_user".to_string()),
                            );
                        }
                        latest_usage = Some(response.usage.clone());
                        ui.on_model_response(rounds, &response.usage, &response.content);
                        step = core.apply_model_response(response);
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
                        if ui.take_cancel_request() {
                            break cancelled_turn_result();
                        }
                        ui.on_model_error(&err);
                        let _ = append_audit(
                            request.audit_file,
                            &json!({"type":"turn_error","session":request.session,"turn_id":turn_id,"error":err}),
                        );
                        break (
                            format!("模型调用失败：{err}"),
                            UsageStats::zero(),
                            None,
                            None,
                        );
                    }
                }
            }
            CoreStep::NeedsUserApproval { request: approval } => {
                ui.pause_for_user_decision();
                let user_wait_start = Instant::now();
                let approved = ui.request_user_approval(&approval);
                user_wait_this_turn = user_wait_this_turn.saturating_add(user_wait_start.elapsed());
                if ui.take_cancel_request() {
                    step = core.resolve_user_approval(&approval.approval_id, false);
                    continue;
                }
                let _ = append_audit(
                    request.audit_file,
                    &json!({
                        "type":"user_approval",
                        "session":request.session,
                        "turn_id":turn_id,
                        "approval_id":approval.approval_id,
                        "action":approval.action,
                        "command":approval.command,
                        "risk":approval.risk,
                        "reason":approval.reason,
                        "approved":approved
                    }),
                );
                step = core.resolve_user_approval(&approval.approval_id, approved);
                ui.resume_after_user_decision();
            }
            CoreStep::RoundLimitReached { max_rounds } => {
                ui.pause_for_user_decision();
                let user_wait_start = Instant::now();
                let should_continue = ui.request_round_limit_continue(max_rounds);
                user_wait_this_turn = user_wait_this_turn.saturating_add(user_wait_start.elapsed());
                let _ = append_audit(
                    request.audit_file,
                    &json!({
                        "type":"round_limit",
                        "session":request.session,
                        "turn_id":turn_id,
                        "max_rounds":max_rounds,
                        "continued":should_continue
                    }),
                );
                if should_continue {
                    step = core.continue_after_round_limit();
                    ui.resume_after_user_decision();
                } else {
                    break (
                        format!(
                            "已达到本轮最大交互次数 {max_rounds}，已停止。你可以继续输入来开启新一轮。"
                        ),
                        core.current_stats().clone(),
                        latest_usage,
                        Some("round_limit_reached".to_string()),
                    );
                }
            }
            CoreStep::Final(turn) => {
                break (
                    turn.response_to_user,
                    turn.stats,
                    latest_usage,
                    turn.repair_issue,
                );
            }
        }
    };

    let elapsed = start.elapsed().saturating_sub(user_wait_this_turn);
    if let Some(profiler) = profiler.as_deref_mut() {
        profiler.record_turn(elapsed, model_wait_this_turn);
    }
    let _ = append_audit(
        request.audit_file,
        &json!({
            "type":"turn_final",
            "session":request.session,
            "turn_id":turn_id,
            "assistant_output":text,
            "stats":stats,
            "latest_usage":latest_usage,
            "repair_issue":repair_issue,
            "elapsed_ms":elapsed.as_millis()
        }),
    );
    TurnOutcome {
        text,
        stats,
        latest_usage,
        elapsed,
        repair_issue,
    }
}

pub fn cancelled_turn_result() -> (String, UsageStats, Option<UsageStats>, Option<String>) {
    (
        "已取消本轮。".to_string(),
        UsageStats::zero(),
        None,
        Some("cancelled_by_user".to_string()),
    )
}

pub fn estimate_prompt_context_tokens(prompt: &str) -> u32 {
    prompt.chars().count().div_ceil(4).min(u32::MAX as usize) as u32
}

fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{BashApprovalMode, CoreProfile};
    use std::collections::VecDeque;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "timem_session_runtime_{}_{}_{}",
            name,
            std::process::id(),
            epoch_millis()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_profile() -> CoreProfile {
        CoreProfile {
            name: "test".to_string(),
            provider: "test".to_string(),
            model: "test-model".to_string(),
        }
    }

    fn test_config() -> ProviderConfig {
        ProviderConfig {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            api_key: "dummy".to_string(),
            base_url: "http://127.0.0.1:9/v1".to_string(),
            api_protocol: crate::ApiProtocol::OpenAiCompatible,
            timeout_secs: 1,
            max_llm_input_tokens: 100_000,
            max_llm_output_tokens: 10_000,
        }
    }

    fn usage(prompt_tokens: u32, completion_tokens: u32) -> UsageStats {
        UsageStats {
            llm_calls: 1,
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            ..UsageStats::zero()
        }
    }

    fn llm(content: impl Into<String>, prompt_tokens: u32, truncated: bool) -> LlmResponse {
        LlmResponse {
            content: content.into(),
            model_name: "test-model".to_string(),
            usage: usage(prompt_tokens, 10),
            truncated,
        }
    }

    fn prompt_field_values(prompt: &str, field: &str) -> Vec<String> {
        let prefix = format!("{field}: ");
        prompt
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix))
            .map(ToString::to_string)
            .collect()
    }

    struct ReplayModel {
        responses: VecDeque<Result<LlmResponse, String>>,
        prompts: Vec<String>,
    }

    impl ReplayModel {
        fn new(responses: impl IntoIterator<Item = Result<LlmResponse, String>>) -> Self {
            Self {
                responses: responses.into_iter().collect(),
                prompts: Vec::new(),
            }
        }
    }

    impl ModelClient for ReplayModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &Path,
            _should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            self.prompts.push(prompt.to_string());
            self.responses
                .pop_front()
                .unwrap_or_else(|| Err("unexpected_extra_model_call".to_string()))
        }
    }

    struct ShrinkReplayModel {
        prompts: Vec<String>,
    }

    impl ModelClient for ShrinkReplayModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &Path,
            _should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            self.prompts.push(prompt.to_string());
            if self.prompts.len() == 1 {
                assert!(prompt.contains("mode=force_shrink_required"));
                let mut delta_ids = prompt_field_values(prompt, "delta_id");
                delta_ids.sort();
                delta_ids.dedup();
                assert!(!delta_ids.is_empty());
                let content = format!(
                    r#"{{"response_to_user":"","next_actions":[{{"action":"prompt_shrink","intent":"Remove visible dynamic context after checkpointing.","input":{{"delta_ids":{}}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["shrink result"]}}}}"#,
                    serde_json::to_string(&delta_ids).unwrap()
                );
                return Ok(llm(content, 13_253, false));
            }
            assert_eq!(self.prompts.len(), 2);
            assert!(prompt.contains("Action result: prompt_shrink"));
            assert!(!prompt.contains("mode=force_shrink_required"));
            Ok(llm(
                r#"{"response_to_user":"压缩已完成，可以继续对话。","acceptance_check":{"is_satisfied":true}}"#,
                1_200,
                false,
            ))
        }
    }

    struct CancelImmediately;

    impl TurnUi for CancelImmediately {
        fn take_cancel_request(&mut self) -> bool {
            true
        }
    }

    #[test]
    fn session_turn_can_cancel_before_provider_call_without_network() {
        let dir = tmp_dir("cancel");
        let audit = dir.join("audit.jsonl");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Ask);
        let mut config = test_config();
        let mut ui = CancelImmediately;

        let outcome = run_session_turn(
            &mut core,
            &mut config,
            TurnRequest {
                input: "hello",
                session: "test_session",
                audit_file: &audit,
                additional_context: None,
            },
            &mut ui,
            None,
        );

        assert_eq!(outcome.text, "已取消本轮。");
        assert_eq!(outcome.repair_issue.as_deref(), Some("cancelled_by_user"));
        let audit_text = std::fs::read_to_string(&audit).unwrap();
        assert!(audit_text.contains("\"turn_start\""));
        assert!(audit_text.contains("\"turn_final\""));
        assert!(!audit_text.contains("\"llm_request\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_forced_shrink_runs_to_final_without_repeated_shrink() {
        let dir = tmp_dir("forced_shrink_e2e");
        let audit = dir.join("audit.jsonl");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_max_llm_input_tokens(10_000);
        let mut config = test_config();
        config.max_llm_input_tokens = 10_000;

        let _ = core.begin_turn(&"old dynamic context ".repeat(1_500), None);
        let seed_step = core.apply_model_response(llm(
            r#"{"response_to_user":"seeded","acceptance_check":{"is_satisfied":true}}"#,
            13_253,
            false,
        ));
        assert!(matches!(seed_step, CoreStep::Final(_)));

        let mut ui = NoopTurnUi;
        let mut model = ShrinkReplayModel {
            prompts: Vec::new(),
        };
        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnRequest {
                input: "继续",
                session: "test_session",
                audit_file: &audit,
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "压缩已完成，可以继续对话。");
        assert_eq!(model.prompts.len(), 2);
        assert_eq!(
            model
                .prompts
                .iter()
                .filter(|prompt| prompt.contains("mode=force_shrink_required"))
                .count(),
            1
        );
        let audit_text = std::fs::read_to_string(&audit).unwrap();
        assert!(audit_text.contains("\"turn_start\""));
        assert!(audit_text.contains("\"turn_final\""));
        assert!(audit_text.contains("压缩已完成，可以继续对话。"));
        let _ = std::fs::remove_dir_all(dir);
    }

    struct ExpandOutputUi {
        expansion_requests: u32,
    }

    impl TurnUi for ExpandOutputUi {
        fn can_request_output_expansion(&mut self) -> bool {
            true
        }

        fn request_expand_output_tokens(&mut self, _current_tokens: u32) -> bool {
            self.expansion_requests += 1;
            true
        }
    }

    #[test]
    fn session_turn_truncated_output_expands_limit_and_retries_same_turn() {
        let dir = tmp_dir("truncated_expansion_e2e");
        let audit = dir.join("audit.jsonl");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        config.max_llm_output_tokens = 10_000;
        let mut ui = ExpandOutputUi {
            expansion_requests: 0,
        };
        let mut model = ReplayModel::new([
            Ok(llm(r#"{"response_to_user":"partial""#, 5_000, true)),
            Ok(llm(
                r#"{"response_to_user":"扩容后完成。","acceptance_check":{"is_satisfied":true}}"#,
                5_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnRequest {
                input: "生成长报告",
                session: "test_session",
                audit_file: &audit,
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "扩容后完成。");
        assert_eq!(ui.expansion_requests, 1);
        assert_eq!(model.prompts.len(), 2);
        assert_eq!(config.max_llm_output_tokens, 20_000);
        let audit_text = std::fs::read_to_string(&audit).unwrap();
        assert!(audit_text.contains("\"max_llm_output_increased\""));
        assert!(audit_text.contains("\"turn_final\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    struct ContinueRoundLimitUi {
        continue_requests: u32,
    }

    impl TurnUi for ContinueRoundLimitUi {
        fn request_round_limit_continue(&mut self, _max_rounds: u32) -> bool {
            self.continue_requests += 1;
            true
        }
    }

    #[test]
    fn session_turn_round_limit_continue_recharges_and_finishes_same_task() {
        let dir = tmp_dir("round_limit_continue_e2e");
        let audit = dir.join("audit.jsonl");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_max_rounds(1);
        let mut config = test_config();
        let mut ui = ContinueRoundLimitUi {
            continue_requests: 0,
        };
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"{"response_to_user":"","next_actions":[{"action":"query_memory","intent":"Look up evidence before answering.","input":{"query":"round limit e2e","limit":5}}],"acceptance_check":{"is_satisfied":false,"missing_info":["memory evidence"]}}"#,
                4_000,
                false,
            )),
            Ok(llm(
                r#"{"response_to_user":"续跑后完成。","acceptance_check":{"is_satisfied":true}}"#,
                4_200,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnRequest {
                input: "需要多轮完成",
                session: "test_session",
                audit_file: &audit,
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "续跑后完成。");
        assert_eq!(ui.continue_requests, 1);
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("Runtime round budget continued by user."));
        let audit_text = std::fs::read_to_string(&audit).unwrap();
        assert!(audit_text.contains("\"round_limit\""));
        assert!(audit_text.contains("\"continued\":true"));
        assert!(audit_text.contains("\"turn_final\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    struct ApproveAllUi {
        approval_requests: u32,
    }

    impl TurnUi for ApproveAllUi {
        fn request_user_approval(&mut self, _request: &ApprovalRequest) -> bool {
            self.approval_requests += 1;
            true
        }
    }

    #[test]
    fn session_turn_bash_approval_executes_action_then_finishes_with_audit() {
        let dir = tmp_dir("bash_approval_e2e");
        let audit = dir.join("audit.jsonl");
        let output_file = dir.join("approved.txt");
        let command = format!("printf approved > {}", output_file.display());
        let first_response = format!(
            r#"{{"response_to_user":"","next_actions":[{{"action":"run_bash","intent":"Write approved test output.","input":{{"command":{},"timeout_ms":5000}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["bash result"]}}}}"#,
            serde_json::to_string(&command).unwrap()
        );

        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Ask);
        let mut config = test_config();
        let mut ui = ApproveAllUi {
            approval_requests: 0,
        };
        let mut model = ReplayModel::new([
            Ok(llm(first_response, 3_000, false)),
            Ok(llm(
                r#"{"response_to_user":"命令已执行并确认。","acceptance_check":{"is_satisfied":true}}"#,
                3_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnRequest {
                input: "执行一个需要审批的本地写入",
                session: "test_session",
                audit_file: &audit,
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "命令已执行并确认。");
        assert_eq!(ui.approval_requests, 1);
        assert_eq!(std::fs::read_to_string(&output_file).unwrap(), "approved");
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("Action result: run_bash"));
        assert!(model.prompts[1].contains("status: 0"));
        let audit_text = std::fs::read_to_string(&audit).unwrap();
        assert!(audit_text.contains("\"user_approval\""));
        assert!(audit_text.contains("\"approved\":true"));
        assert!(audit_text.contains("\"turn_final\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    struct ScratchOffloadReplayModel {
        prompts: Vec<String>,
    }

    impl ModelClient for ScratchOffloadReplayModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &Path,
            _should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            self.prompts.push(prompt.to_string());
            if self.prompts.len() == 1 {
                let mut delta_ids = prompt_field_values(prompt, "delta_id");
                delta_ids.sort();
                delta_ids.dedup();
                assert!(!delta_ids.is_empty());
                let content = format!(
                    r#"{{"response_to_user":"","next_actions":[{{"action":"scratch_write","intent":"Offload visible prompt context for later retrieval.","input":{{"type":"context_offload","label":"session e2e offload","delta_ids":{}}}}}],"acceptance_check":{{"is_satisfied":false,"missing_info":["scratch id"]}}}}"#,
                    serde_json::to_string(&delta_ids).unwrap()
                );
                return Ok(llm(content, 4_000, false));
            }
            assert_eq!(self.prompts.len(), 2);
            assert!(prompt.contains("Action result: scratch_write"));
            assert!(prompt.contains("id: scratch_"));
            assert!(prompt.contains("label: session e2e offload"));
            Ok(llm(
                r#"{"response_to_user":"scratch 已记录，可以继续。","acceptance_check":{"is_satisfied":true}}"#,
                4_100,
                false,
            ))
        }
    }

    #[test]
    fn session_turn_scratch_context_offload_records_id_and_continues() {
        let dir = tmp_dir("scratch_offload_e2e");
        let audit = dir.join("audit.jsonl");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ScratchOffloadReplayModel {
            prompts: Vec::new(),
        };

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnRequest {
                input: "把当前上下文转存到 scratch 后继续",
                session: "test_session",
                audit_file: &audit,
                additional_context: Some("extra context that should be offloaded"),
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "scratch 已记录，可以继续。");
        assert_eq!(model.prompts.len(), 2);
        let scratch_text = std::fs::read_to_string(dir.join("scratch_notes.jsonl")).unwrap();
        assert!(scratch_text.contains(r#""scratch_type":"context_offload""#));
        assert!(scratch_text.contains(r#""label":"session e2e offload""#));
        assert!(scratch_text.contains("extra context that should be offloaded"));
        let audit_text = std::fs::read_to_string(&audit).unwrap();
        assert!(audit_text.contains("\"turn_final\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn noop_turn_ui_defaults_to_noninteractive_denials() {
        let mut ui = NoopTurnUi;
        let request = ApprovalRequest {
            approval_id: "approval_1".to_string(),
            action: "run_bash".to_string(),
            intent: "test".to_string(),
            command: "echo hi".to_string(),
            read_back_command: String::new(),
            risk: "test".to_string(),
            reason: "test".to_string(),
        };

        assert!(!ui.is_cancel_requested());
        assert!(!ui.take_cancel_request());
        assert!(!ui.request_user_approval(&request));
        assert!(!ui.request_round_limit_continue(20));
        assert!(!ui.can_request_output_expansion());
        assert!(!ui.request_expand_output_tokens(10_000));
    }
}
