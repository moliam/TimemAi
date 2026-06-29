use crate::{
    append_audit, call_model_with_cancel, format_token_count, supporting_context, ProviderConfig,
    RuntimeProfiler,
};
use agent_core::{AgentCore, ApprovalRequest, CoreStep, UsageStats};
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

pub fn run_session_turn(
    core: &mut AgentCore,
    config: &mut ProviderConfig,
    request: TurnRequest<'_>,
    ui: &mut dyn TurnUi,
    mut profiler: Option<&mut RuntimeProfiler>,
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
                match call_model_with_cancel(config, prompt, request.audit_file, || {
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

    struct CancelImmediately;

    impl TurnUi for CancelImmediately {
        fn take_cancel_request(&mut self) -> bool {
            true
        }
    }

    #[test]
    fn session_turn_can_cancel_before_provider_call_without_network() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "timem_session_runtime_cancel_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let audit = dir.join("audit.jsonl");
        let mut core = AgentCore::new(
            r#"{"role":"test static prompt"}"#,
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir,
        );
        core.set_bash_approval_mode(BashApprovalMode::Ask);
        let mut config = ProviderConfig {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            api_key: "dummy".to_string(),
            base_url: "http://127.0.0.1:9/v1".to_string(),
            api_protocol: crate::ApiProtocol::OpenAiCompatible,
            timeout_secs: 1,
            max_llm_input_tokens: 100_000,
            max_llm_output_tokens: 10_000,
        };
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
