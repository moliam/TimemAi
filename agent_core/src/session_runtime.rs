use crate::{
    append_audit_event, is_model_input_too_large_error, model_input_overflow_recovery_audit_event,
    model_retry_audit_event, model_retry_decision, normalize_user_supplements,
    turn_supporting_context, ActionRuntime, AgentCore, CoreStep, CoreTopicEvent,
    HostDecisionRequest, LlmResponse, LongRunningCommandContinueRequest,
    LongRunningCommandDecision, LongRunningCommandStatus, ModelCallOutcome, ModelSystemRetryPolicy,
    OutputExpansionRequest, OutputExpansionResolution, ProviderConfig, ProviderModelClient,
    RoundLimitDecisionRequest, RoundLimitResolution, RuntimeProfiler, StoppedTurn,
    SupportingContextInput, TurnInput, TurnOutcome, TurnStopReason, TurnStopSummary, TurnUi,
    UsageStats,
};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub trait ModelClient {
    fn call_model(
        &mut self,
        config: &ProviderConfig,
        prompt: &str,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String>;
}

pub fn run_session_turn(
    core: &mut AgentCore,
    config: &mut ProviderConfig,
    request: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
) -> TurnOutcome {
    let mut model_client = ProviderModelClient;
    run_session_turn_with_model_client(core, config, request, ui, profiler, &mut model_client)
}

pub fn run_session_turn_with_model_client(
    core: &mut AgentCore,
    config: &mut ProviderConfig,
    request: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    mut profiler: Option<&mut RuntimeProfiler>,
    model_client: &mut dyn ModelClient,
) -> TurnOutcome {
    core.set_response_protocol(config.response_protocol);
    let turn_id = format!("turn_{}", epoch_millis());
    core.record_turn_start_audit(request.audit_file, request.session, &turn_id, request.input);
    let start = Instant::now();
    let mut user_wait_this_turn = Duration::ZERO;
    let context = turn_supporting_context(
        SupportingContextInput {
            provider: &config.provider,
            model: &config.model,
            runtime: request.runtime,
            run_bash_target: request.run_bash_target,
        },
        request.additional_context,
    );
    let mut step = core.begin_turn(request.input, Some(&context));
    let mut rounds = 0u32;
    let mut model_wait_this_turn = Duration::ZERO;
    let mut latest_usage: Option<UsageStats> = None;

    let (text, stopped, final_parts) = loop {
        if ui.take_cancel_request() {
            break cancelled_turn_parts();
        }
        match step {
            CoreStep::NeedModel { ref prompt, .. } => {
                let supplements = normalize_user_supplements(ui.drain_user_supplements());
                if !supplements.is_empty() {
                    if let Some(next_step) = core.append_user_supplements_with_audit(
                        supplements,
                        request.audit_file,
                        request.session,
                        &turn_id,
                    ) {
                        step = next_step;
                    }
                    continue;
                }
                rounds += 1;
                ui.on_model_request(rounds, prompt);
                match call_model_with_system_retries(
                    model_client,
                    config,
                    prompt,
                    request.audit_file,
                    ui,
                    &mut profiler,
                    request.session,
                    &turn_id,
                ) {
                    Ok(response) => {
                        model_wait_this_turn = model_wait_this_turn.saturating_add(
                            response.model_wait.saturating_add(response.retry_wait),
                        );
                        if ui.take_cancel_request() {
                            break cancelled_turn_parts();
                        }
                        let supplements = normalize_user_supplements(ui.drain_user_supplements());
                        if !supplements.is_empty() {
                            core.record_discarded_model_response_usage(&response.response.usage);
                            ui.on_model_response_discarded(
                                rounds,
                                "user_supplement_preempted_stale_response",
                            );
                            if let Some(next_step) = core.append_user_supplements_with_audit(
                                supplements,
                                request.audit_file,
                                request.session,
                                &turn_id,
                            ) {
                                step = next_step;
                            }
                            continue;
                        }
                        if response.response.truncated && ui.can_request_output_expansion() {
                            let expansion =
                                OutputExpansionRequest::new(config.max_llm_output_tokens);
                            ui.pause_for_user_decision();
                            let user_wait_start = Instant::now();
                            let should_expand = ui
                                .request_host_decision_topic(
                                    request.session,
                                    HostDecisionRequest::OutputExpansion(expansion),
                                )
                                .as_bool();
                            user_wait_this_turn =
                                user_wait_this_turn.saturating_add(user_wait_start.elapsed());
                            match core.resolve_output_expansion_with_audit(
                                config,
                                expansion,
                                should_expand,
                                response.response.usage,
                                request.audit_file,
                                request.session,
                                &turn_id,
                            ) {
                                OutputExpansionResolution::RetryWithExpandedLimit { .. } => {
                                    ui.resume_after_user_decision();
                                    continue;
                                }
                                OutputExpansionResolution::Stop(stop) => {
                                    break turn_stop_parts(stop);
                                }
                            }
                        }
                        latest_usage = Some(response.response.usage.clone());
                        ui.on_model_response(
                            rounds,
                            &response.response.usage,
                            &response.response.content,
                        );
                        let mut action_runtime = TurnActionRuntime::new(ui, request.session);
                        step = core.apply_model_response_with_repair_audit_and_runtime(
                            response.response,
                            request.audit_file,
                            request.session,
                            &turn_id,
                            &mut action_runtime,
                        );
                        user_wait_this_turn =
                            user_wait_this_turn.saturating_add(action_runtime.user_wait());
                        let command_supplements = action_runtime.take_pending_supplements();
                        if !command_supplements.is_empty() {
                            if let Some(next_step) = core.append_user_supplements_with_audit(
                                command_supplements,
                                request.audit_file,
                                request.session,
                                &turn_id,
                            ) {
                                step = next_step;
                            }
                        }
                    }
                    Err(err) => {
                        if ui.take_cancel_request() {
                            break cancelled_turn_parts();
                        }
                        if is_model_input_too_large_error(&err) {
                            if let Some(recovery) = core.recover_from_model_input_too_large(&err) {
                                let _ = append_audit_event(
                                    request.audit_file,
                                    &model_input_overflow_recovery_audit_event(
                                        request.session,
                                        &turn_id,
                                        &recovery.removed_delta_id,
                                        recovery.removed_action_output_bytes,
                                        &err,
                                    ),
                                );
                                step = recovery.step;
                                continue;
                            }
                        }
                        ui.on_model_error(&err);
                        core.record_turn_error_audit(
                            request.audit_file,
                            request.session,
                            &turn_id,
                            &err,
                        );
                        break turn_stop_parts(TurnStopSummary::model_error(err));
                    }
                }
            }
            CoreStep::NeedsUserApproval { request: approval } => {
                ui.pause_for_user_decision();
                let user_wait_start = Instant::now();
                let approved = ui
                    .request_host_decision_topic(
                        request.session,
                        HostDecisionRequest::UserApproval(approval.clone()),
                    )
                    .as_bool();
                user_wait_this_turn = user_wait_this_turn.saturating_add(user_wait_start.elapsed());
                if ui.take_cancel_request() {
                    step = core.resolve_user_approval_with_audit_and_cancel(
                        &approval,
                        false,
                        request.audit_file,
                        request.session,
                        &turn_id,
                        &mut || ui.is_cancel_requested(),
                    );
                    ui.resume_after_user_decision();
                    continue;
                }
                let mut action_runtime = TurnActionRuntime::new(ui, request.session);
                step = core.resolve_user_approval_with_audit_and_runtime(
                    &approval,
                    approved,
                    request.audit_file,
                    request.session,
                    &turn_id,
                    &mut action_runtime,
                );
                user_wait_this_turn =
                    user_wait_this_turn.saturating_add(action_runtime.user_wait());
                let command_supplements = action_runtime.take_pending_supplements();
                if !command_supplements.is_empty() {
                    if let Some(next_step) = core.append_user_supplements_with_audit(
                        command_supplements,
                        request.audit_file,
                        request.session,
                        &turn_id,
                    ) {
                        step = next_step;
                    }
                }
                ui.resume_after_user_decision();
            }
            CoreStep::RoundLimitReached { max_rounds } => {
                let decision_request = RoundLimitDecisionRequest::new(max_rounds);
                ui.pause_for_user_decision();
                let user_wait_start = Instant::now();
                let should_continue = ui
                    .request_host_decision_topic(
                        request.session,
                        HostDecisionRequest::RoundLimitContinue(decision_request),
                    )
                    .as_bool();
                user_wait_this_turn = user_wait_this_turn.saturating_add(user_wait_start.elapsed());
                match core.resolve_round_limit_with_audit(
                    decision_request,
                    should_continue,
                    latest_usage.clone(),
                    request.audit_file,
                    request.session,
                    &turn_id,
                ) {
                    RoundLimitResolution::Continue(next_step) => {
                        step = next_step;
                        ui.resume_after_user_decision();
                    }
                    RoundLimitResolution::Stop(stop) => break turn_stop_parts(stop),
                }
            }
            CoreStep::Final(turn) => {
                if let Some(stop) = turn.stop_summary {
                    break turn_stop_parts(stop);
                }
                break (
                    turn.final_answer,
                    None,
                    Some((
                        turn.stats,
                        latest_usage,
                        turn.repair_issue,
                        turn.toolgen_retrospect,
                    )),
                );
            }
        }
    };

    let elapsed = start.elapsed().saturating_sub(user_wait_this_turn);
    let mut outcome = match (stopped, final_parts) {
        (Some(stopped), None) => TurnOutcome::stopped(text, stopped, elapsed),
        (None, Some((stats, latest_usage, repair_issue, toolgen_retrospect))) => {
            TurnOutcome::final_response(text, stats, latest_usage, repair_issue, elapsed)
                .with_toolgen_retrospect(toolgen_retrospect)
        }
        _ => unreachable!("session turn loop must produce exactly one outcome kind"),
    };
    outcome =
        outcome.with_running_jobs(core.refresh_running_shell_jobs_for_session(request.session));
    if let Some(profiler) = profiler {
        profiler.record_turn(elapsed, model_wait_this_turn);
    }
    core.record_turn_final_audit(request.audit_file, request.session, &turn_id, &outcome);
    outcome
}

struct TurnActionRuntime<'a> {
    ui: &'a mut dyn TurnUi,
    session: &'a str,
    pending_supplements: Vec<String>,
    user_wait: Duration,
}

impl<'a> TurnActionRuntime<'a> {
    fn new(ui: &'a mut dyn TurnUi, session: &'a str) -> Self {
        Self {
            ui,
            session,
            pending_supplements: Vec::new(),
            user_wait: Duration::ZERO,
        }
    }

    fn take_pending_supplements(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_supplements)
    }

    fn user_wait(&self) -> Duration {
        self.user_wait
    }
}

impl ActionRuntime for TurnActionRuntime<'_> {
    fn should_cancel(&mut self) -> bool {
        self.ui.is_cancel_requested()
    }

    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        self.ui.on_core_topic_events(events);
    }

    fn on_long_running_command(
        &mut self,
        status: &LongRunningCommandStatus,
    ) -> LongRunningCommandDecision {
        self.ui.pause_for_user_decision();
        let user_wait_start = Instant::now();
        let decision = self
            .ui
            .request_host_decision_topic(
                self.session,
                HostDecisionRequest::LongRunningCommandContinue(
                    LongRunningCommandContinueRequest::new(
                        status.action.clone(),
                        status.command.clone(),
                        status.elapsed,
                        status.timeout_ms,
                    ),
                ),
            )
            .as_bool();
        self.user_wait = self.user_wait.saturating_add(user_wait_start.elapsed());
        self.ui.resume_after_user_decision();
        if decision {
            LongRunningCommandDecision::Continue
        } else {
            self.pending_supplements.push(format!(
                "user cancels the command: {} (already running {} secs). You can initiate action to check current working status. If you feel it is still necessary, initiate action again with an explanation in free_talk.",
                status.command,
                status.elapsed.as_secs()
            ));
            LongRunningCommandDecision::Cancel
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn call_model_with_system_retries(
    model_client: &mut dyn ModelClient,
    config: &ProviderConfig,
    prompt: &str,
    audit_file: &Path,
    ui: &mut dyn TurnUi,
    profiler: &mut Option<&mut RuntimeProfiler>,
    session: &str,
    turn_id: &str,
) -> Result<ModelCallOutcome<LlmResponse>, String> {
    let retry_policy = model_system_retry_policy();
    let mut total_model_wait = Duration::ZERO;
    let mut total_retry_wait = Duration::ZERO;
    for attempt in 0..=retry_policy.max_attempts {
        let model_wait_start = Instant::now();
        let result =
            model_client.call_model(config, prompt, audit_file, &mut || ui.is_cancel_requested());
        let model_wait = model_wait_start.elapsed();
        total_model_wait = total_model_wait.saturating_add(model_wait);
        match result {
            Ok(response) => {
                if let Some(profiler) = profiler.as_deref_mut() {
                    profiler.record_model_wait(
                        &config.provider,
                        &response.model_name,
                        &response.usage,
                        model_wait,
                    );
                }
                return Ok(ModelCallOutcome {
                    response,
                    model_wait: total_model_wait,
                    retry_wait: total_retry_wait,
                });
            }
            Err(err) => {
                if let Some(profiler) = profiler.as_deref_mut() {
                    profiler.record_model_wait(
                        &config.provider,
                        &config.model,
                        &UsageStats::zero(),
                        model_wait,
                    );
                }
                let Some(decision) =
                    model_retry_decision(&err, attempt, retry_policy, ui.is_cancel_requested())
                else {
                    return Err(err);
                };
                ui.on_model_retry(
                    decision.retry_attempt,
                    decision.max_attempts,
                    decision.delay,
                    &err,
                );
                let _ = append_audit_event(
                    audit_file,
                    &model_retry_audit_event(
                        session,
                        turn_id,
                        decision.retry_attempt,
                        decision.max_attempts,
                        decision.delay,
                        &err,
                    ),
                );
                let waited = wait_retry_delay(ui, decision.delay);
                total_retry_wait = total_retry_wait.saturating_add(waited);
                if ui.is_cancel_requested() {
                    return Err("cancelled_by_user".to_string());
                }
            }
        }
    }
    Err("provider_network_error: retry loop exhausted".to_string())
}

#[cfg(not(test))]
fn model_system_retry_policy() -> ModelSystemRetryPolicy {
    ModelSystemRetryPolicy::default()
}

#[cfg(test)]
fn model_system_retry_policy() -> ModelSystemRetryPolicy {
    ModelSystemRetryPolicy {
        delay: Duration::ZERO,
        ..ModelSystemRetryPolicy::default()
    }
}

fn wait_retry_delay(ui: &mut dyn TurnUi, delay: Duration) -> Duration {
    let start = Instant::now();
    while start.elapsed() < delay {
        if ui.is_cancel_requested() {
            break;
        }
        let remaining = delay.saturating_sub(start.elapsed());
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
    start.elapsed().min(delay)
}

pub fn cancelled_turn_result() -> (
    String,
    UsageStats,
    Option<UsageStats>,
    Option<String>,
    Option<TurnStopReason>,
) {
    let (text, stopped, _) = cancelled_turn_parts();
    let stopped = stopped.expect("cancelled turn must stop");
    (
        text,
        stopped.stats,
        stopped.latest_usage,
        stopped.repair_issue,
        Some(stopped.stop_reason),
    )
}

#[allow(clippy::type_complexity)]
fn cancelled_turn_parts() -> (
    String,
    Option<StoppedTurn>,
    Option<(UsageStats, Option<UsageStats>, Option<String>, String)>,
) {
    turn_stop_parts(TurnStopSummary::cancelled_by_user())
}

#[allow(clippy::type_complexity)]
fn turn_stop_parts(
    stop: TurnStopSummary,
) -> (
    String,
    Option<StoppedTurn>,
    Option<(UsageStats, Option<UsageStats>, Option<String>, String)>,
) {
    (String::new(), Some(stop.into_stopped_turn()), None)
}

fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
#[path = "../tests/unit/session_runtime_tests.rs"]
mod tests;
