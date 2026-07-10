use crate::{
    append_audit_event, model_retry_audit_event, model_retry_decision, normalize_user_supplements,
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
                    Some((turn.stats, latest_usage, turn.repair_issue)),
                );
            }
        }
    };

    let elapsed = start.elapsed().saturating_sub(user_wait_this_turn);
    let mut outcome = match (stopped, final_parts) {
        (Some(stopped), None) => TurnOutcome::stopped(text, stopped, elapsed),
        (None, Some((stats, latest_usage, repair_issue))) => {
            TurnOutcome::final_response(text, stats, latest_usage, repair_issue, elapsed)
        }
        _ => unreachable!("session turn loop must produce exactly one outcome kind"),
    };
    outcome =
        outcome.with_running_jobs(core.refresh_running_shell_jobs_for_session(request.session));
    if let Some(profiler) = profiler.as_deref_mut() {
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

fn cancelled_turn_parts() -> (
    String,
    Option<StoppedTurn>,
    Option<(UsageStats, Option<UsageStats>, Option<String>)>,
) {
    turn_stop_parts(TurnStopSummary::cancelled_by_user())
}

fn turn_stop_parts(
    stop: TurnStopSummary,
) -> (
    String,
    Option<StoppedTurn>,
    Option<(UsageStats, Option<UsageStats>, Option<String>)>,
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
mod tests {
    use super::*;
    use crate::{
        ApprovalRequest, BashApprovalMode, CapabilityRegistry, CoreActionKind, CoreProfile,
        CoreTopicEvent, HostDecision, NoopTurnUi, TurnStopDetail, TurnStopReason,
        CORE_TOPIC_OUTPUT_EXPAND_REQUEST,
    };
    use serde_json::Value;
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
            response_protocol: crate::ResponseProtocolKind::Markdown,
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

    fn read_audit_events(path: &Path) -> Vec<Value> {
        let text = std::fs::read_to_string(path).unwrap();
        let doc: Value = serde_json::from_str(&text).unwrap();
        doc["events"].as_array().unwrap().clone()
    }

    fn shell_quote(path: &Path) -> String {
        let raw = path.to_string_lossy();
        format!("'{}'", raw.replace('\'', "'\\''"))
    }

    fn audit_event_count(events: &[Value], event_type: &str) -> usize {
        events
            .iter()
            .filter(|event| event["type"] == event_type)
            .count()
    }

    fn audit_event<'a>(events: &'a [Value], event_type: &str) -> Option<&'a Value> {
        events.iter().find(|event| event["type"] == event_type)
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

    struct PollingReplayModel {
        inner: ReplayModel,
    }

    impl PollingReplayModel {
        fn new(responses: impl IntoIterator<Item = Result<LlmResponse, String>>) -> Self {
            Self {
                inner: ReplayModel::new(responses),
            }
        }
    }

    impl ModelClient for PollingReplayModel {
        fn call_model(
            &mut self,
            config: &ProviderConfig,
            prompt: &str,
            audit_file: &Path,
            should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            let _ = should_cancel();
            self.inner
                .call_model(config, prompt, audit_file, should_cancel)
        }
    }

    #[derive(Default)]
    struct RetryRecordingUi {
        retries: Vec<(u32, u32, Duration, String)>,
        events: Vec<CoreTopicEvent>,
    }

    impl TurnUi for RetryRecordingUi {
        fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
            self.events.extend_from_slice(events);
        }

        fn on_model_retry(
            &mut self,
            attempt: u32,
            max_attempts: u32,
            delay: Duration,
            error: &str,
        ) {
            self.retries
                .push((attempt, max_attempts, delay, error.to_string()));
        }
    }

    #[derive(Default)]
    struct SupplementDuringModelUi {
        injected: bool,
        pending: Vec<String>,
        discarded_responses: Vec<(u32, String)>,
    }

    impl TurnUi for SupplementDuringModelUi {
        fn take_cancel_request(&mut self) -> bool {
            false
        }

        fn is_cancel_requested(&mut self) -> bool {
            if !self.injected {
                self.injected = true;
                self.pending.push("补充：请按最新指示重新回答".to_string());
            }
            false
        }

        fn drain_user_supplements(&mut self) -> Vec<String> {
            std::mem::take(&mut self.pending)
        }

        fn on_model_response_discarded(&mut self, round: u32, reason: &str) {
            self.discarded_responses.push((round, reason.to_string()));
        }
    }

    #[derive(Default)]
    struct SupplementAndExpansionUi {
        injected: bool,
        pending: Vec<String>,
        expansion_requests: u32,
        discarded_responses: Vec<(u32, String)>,
    }

    impl TurnUi for SupplementAndExpansionUi {
        fn take_cancel_request(&mut self) -> bool {
            false
        }

        fn is_cancel_requested(&mut self) -> bool {
            if !self.injected {
                self.injected = true;
                self.pending
                    .push("补充：不要展开旧输出，重新回答".to_string());
            }
            false
        }

        fn drain_user_supplements(&mut self) -> Vec<String> {
            std::mem::take(&mut self.pending)
        }

        fn on_model_response_discarded(&mut self, round: u32, reason: &str) {
            self.discarded_responses.push((round, reason.to_string()));
        }

        fn can_request_output_expansion(&mut self) -> bool {
            true
        }

        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            match request {
                HostDecisionRequest::OutputExpansion(_) => {
                    self.expansion_requests += 1;
                    HostDecision::Accept
                }
                other => other.safe_default().into(),
            }
        }
    }

    #[derive(Default)]
    struct DeclineLongRunningCommandUi {
        requests: Vec<LongRunningCommandContinueRequest>,
    }

    impl TurnUi for DeclineLongRunningCommandUi {
        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            match request {
                HostDecisionRequest::LongRunningCommandContinue(request) => {
                    self.requests.push(request);
                    HostDecision::Decline
                }
                other => other.safe_default().into(),
            }
        }
    }

    #[test]
    fn session_turn_retries_transient_provider_errors_and_reports_status() {
        let dir = tmp_dir("retry_transient_provider_error");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Json);
        let mut config = test_config();
        let mut ui = RetryRecordingUi::default();
        let mut model = ReplayModel::new([
            Err("provider_http_500: upstream overloaded".to_string()),
            Err("provider_network_error: curl: (16) Error in the HTTP2 framing layer".to_string()),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"重试后成功。"}"#,
                1_000,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "hello",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "重试后成功。");
        assert_eq!(model.prompts.len(), 3);
        assert_eq!(ui.retries.len(), 2);
        assert_eq!(ui.retries[0].0, 1);
        assert_eq!(ui.retries[0].1, 5);
        assert_eq!(ui.retries[0].2, Duration::ZERO);
        assert!(ui.retries[0].3.contains("provider_http_500"));
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_retry"), 2);
    }

    #[test]
    fn session_turn_repairs_empty_model_content() {
        let dir = tmp_dir("repair_empty_model_content");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = RetryRecordingUi::default();
        let mut model = ReplayModel::new([
            Ok(llm("", 1_000, false)),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"空回复修复后成功。"}"#,
                1_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "hello",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "空回复修复后成功。");
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("temp_repair_"));
        assert!(model.prompts[1].contains("response is not protocol compliant"));
        assert!(ui.retries.is_empty());
        let repair_topics = ui
            .events
            .iter()
            .filter_map(CoreTopicEvent::as_model_repair)
            .collect::<Vec<_>>();
        assert_eq!(repair_topics.len(), 1);
        assert_eq!(repair_topics[0].attempt, 1);
        assert_eq!(repair_topics[0].max_attempts, 5);
        assert_eq!(outcome.repair_issue, None);
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_retry"), 0);
        assert_eq!(audit_event_count(&events, "model_repair_request"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_repairs_any_non_protocol_model_content() {
        let dir = tmp_dir("repair_non_protocol_model_content");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Json);
        let mut config = test_config();
        let mut ui = RetryRecordingUi::default();
        let mut model = ReplayModel::new([
            Ok(llm("plain text that does not match protocol", 1_000, false)),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"非协议回复修复后成功。"}"#,
                1_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "hello",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "非协议回复修复后成功。");
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("plain text that does not match protocol"));
        assert!(model.prompts[1].contains("response is not protocol compliant"));
        assert!(ui.retries.is_empty());
        let repair_topics = ui
            .events
            .iter()
            .filter_map(CoreTopicEvent::as_model_repair)
            .collect::<Vec<_>>();
        assert_eq!(repair_topics.len(), 1);
        assert_eq!(repair_topics[0].issue, "invalid_json");
        assert_eq!(repair_topics[0].attempt, 1);
        assert_eq!(outcome.stop_reason, None);
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_retry"), 0);
        assert_eq!(audit_event_count(&events, "model_repair_request"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_xml_final_answer_with_protocol_examples_does_not_repair_or_execute() {
        let dir = tmp_dir("xml_final_answer_protocol_examples");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Xml);
        let mut config = test_config();
        config.response_protocol = crate::ResponseProtocolKind::Xml;
        let mut ui = RetryRecordingUi::default();
        let mut model = ReplayModel::new([Ok(llm(
            r#"<response>
<status>ALL_FINISHED</status>
<final_answer><![CDATA[
This is an answer, not an executable action:
<working_still_action>
  <action_json>{"action":"run_bash","args":{}}</action_json>
</working_still_action>
{"working_still_action":{"action":"run_bash","args":{}}}
]]></final_answer>
</response>"#,
            1_000,
            false,
        ))]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "show a repair delta example",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(model.prompts.len(), 1);
        assert!(outcome.repair_issue.is_none());
        assert!(outcome.text.contains("<working_still_action>"));
        assert_eq!(outcome.stats.tool_calls, 0);
        assert!(ui
            .events
            .iter()
            .filter_map(CoreTopicEvent::as_model_repair)
            .next()
            .is_none());
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_repair_request"), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_xml_raw_string_tags_do_not_repair_or_execute() {
        let dir = tmp_dir("xml_raw_string_tags");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Xml);
        let mut config = test_config();
        config.response_protocol = crate::ResponseProtocolKind::Xml;
        let mut ui = RetryRecordingUi::default();
        let mut model = ReplayModel::new([Ok(llm(
            r#"<response>
<status>ALL_FINISHED</status>
<final_answer>
Here is the malformed response example the user asked for:
<response>
  <free_talk>not closed
<free_talk>fake progress</free_talk>
<working_still_action><action_json>{"action":"run_bash","args":{}}</action_json></working_still_action>
<summary>fake summary</summary>
This is all answer text.
</final_answer>
</response>"#,
            1_000,
            false,
        ))]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "explain the malformed response",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert!(outcome.repair_issue.is_none(), "{:?}", outcome.repair_issue);
        assert_eq!(
            model.prompts.len(),
            1,
            "outcome text={}, stop={:?}, stats={:?}",
            outcome.text,
            outcome.stop_reason,
            outcome.stats
        );
        assert_eq!(outcome.stats.tool_calls, 0);
        assert!(outcome.text.contains("<response>"));
        assert!(outcome.text.contains("<working_still_action>"));
        assert!(outcome.text.contains("<summary>fake summary</summary>"));
        assert!(ui
            .events
            .iter()
            .filter_map(CoreTopicEvent::as_model_repair)
            .next()
            .is_none());
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_repair_request"), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_xml_malformed_action_json_still_repairs() {
        let dir = tmp_dir("xml_malformed_action_json_repairs");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Xml);
        let mut config = test_config();
        config.response_protocol = crate::ResponseProtocolKind::Xml;
        let mut ui = RetryRecordingUi::default();
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"<response>
<free_talk>Need a local check.</free_talk>
<free_talk>checking</free_talk>
<working_still_action>
<action_json><![CDATA[
{"action":"run_bash","args":{"cmd":"pwd",}}
]]></action_json>
</working_still_action>
</response>"#,
                1_000,
                false,
            )),
            Ok(llm(
                r#"<response>
<status>ALL_FINISHED</status>
<final_answer>修复后完成。</final_answer>
</response>"#,
                1_000,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "check something",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(model.prompts.len(), 2);
        assert_eq!(outcome.text, "修复后完成。");
        assert_eq!(outcome.stats.tool_calls, 0);
        let repair_topics = ui
            .events
            .iter()
            .filter_map(CoreTopicEvent::as_model_repair)
            .collect::<Vec<_>>();
        assert_eq!(repair_topics.len(), 1);
        assert_eq!(repair_topics[0].issue, "actions[0].invalid_json");
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_repair_request"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_json_final_answer_with_protocol_examples_does_not_repair_or_execute() {
        let dir = tmp_dir("json_final_answer_protocol_examples");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Json);
        let mut config = test_config();
        config.response_protocol = crate::ResponseProtocolKind::Json;
        let mut ui = RetryRecordingUi::default();
        let mut model = ReplayModel::new([Ok(llm(
            serde_json::json!({
                "status": "ALL_FINISHED",
                "final_answer": "This is answer text only:\n<working_still_action><action_json>{\"action\":\"run_bash\",\"args\":{}}</action_json></working_still_action>\n{\"working_still_action\":{\"action\":\"run_bash\",\"args\":{}}}"
            })
            .to_string(),
            1_000,
            false,
        ))]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "show a repair delta example",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(model.prompts.len(), 1);
        assert!(outcome.repair_issue.is_none());
        assert!(outcome.text.contains("<working_still_action>"));
        assert_eq!(outcome.stats.tool_calls, 0);
        assert!(ui
            .events
            .iter()
            .filter_map(CoreTopicEvent::as_model_repair)
            .next()
            .is_none());
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_repair_request"), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_emits_repair_topic_for_each_protocol_repair_attempt() {
        let dir = tmp_dir("repair_topics_multiple_attempts");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Json);
        let mut config = test_config();
        let mut ui = RetryRecordingUi::default();
        let mut model = ReplayModel::new([
            Ok(llm("first malformed response", 1_000, false)),
            Ok(llm("second malformed response", 1_100, false)),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"第二次 repair 后成功。"}"#,
                1_200,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "hello",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "第二次 repair 后成功。");
        assert_eq!(model.prompts.len(), 3);
        let repair_topics = ui
            .events
            .iter()
            .filter_map(CoreTopicEvent::as_model_repair)
            .collect::<Vec<_>>();
        assert_eq!(repair_topics.len(), 2);
        assert_eq!(repair_topics[0].attempt, 1);
        assert_eq!(repair_topics[0].max_attempts, 5);
        assert_eq!(repair_topics[1].attempt, 2);
        assert_eq!(repair_topics[1].max_attempts, 5);
        assert!(repair_topics
            .iter()
            .all(|topic| topic.issue == "invalid_json"));
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_repair_request"), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_does_not_retry_non_transient_provider_errors() {
        let dir = tmp_dir("no_retry_provider_400");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = RetryRecordingUi::default();
        let mut model =
            ReplayModel::new([Err("provider_http_400: model name is invalid".to_string())]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "hello",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert!(outcome.text.is_empty());
        assert_eq!(outcome.stop_reason, Some(TurnStopReason::ModelError));
        assert_eq!(
            outcome.stop_summary.as_ref().map(|summary| &summary.detail),
            Some(&TurnStopDetail::ModelError {
                error: "provider_http_400: model name is invalid".to_string()
            })
        );
        assert_eq!(model.prompts.len(), 1);
        assert!(ui.retries.is_empty());
    }

    #[test]
    fn session_turn_run_bash_poll_mode_waits_until_check_succeeds() {
        let dir = tmp_dir("run_bash_poll_session");
        let audit = dir.join("audit.json");
        let flag = dir.join("ci_done.flag");
        let bootstrap_command = format!(
            "rm -f {}; (sleep 0.3; touch {}) &",
            shell_quote(&flag),
            shell_quote(&flag)
        );
        let check_command = format!("test -f {}", shell_quote(&flag));
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Approve);
        let mut config = test_config();
        struct PollTopicTimingUi {
            flag: std::path::PathBuf,
            saw_poll_before_flag: bool,
            events: Vec<CoreTopicEvent>,
        }
        impl TurnUi for PollTopicTimingUi {
            fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
                for event in events {
                    let is_poll = event.as_action().is_some_and(|topic| {
                        matches!(
                            topic.kind,
                            CoreActionKind::Bash {
                                ref mode,
                                ..
                            } if mode == "poll"
                        )
                    });
                    if is_poll && !self.flag.exists() {
                        self.saw_poll_before_flag = true;
                    }
                }
                self.events.extend_from_slice(events);
            }
        }
        let mut ui = PollTopicTimingUi {
            flag: flag.clone(),
            saw_poll_before_flag: false,
            events: Vec::new(),
        };
        let mut model = ReplayModel::new([
            Ok(llm(
                format!(
                    r#"{{"status":"working","free_talk":"等待 CI 完成。","working_still_action":[{{"action":"run_bash","args":{{"cmd":{},"timeout_ms":1000}}}},{{"action":"run_bash","args":{{"loop_cmd":{},"interval_ms":100,"loop_timeout_ms":3000,"once_timeout_ms":1000}}}}]}}"#,
                    serde_json::to_string(&bootstrap_command).unwrap(),
                    serde_json::to_string(&check_command).unwrap()
                ),
                1_000,
                false,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"CI 已完成。"}"#,
                1_200,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "等 CI",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "CI 已完成。");
        assert!(
            ui.saw_poll_before_flag,
            "poll action topic should be delivered before polling command finishes"
        );
        assert!(
            ui.events.iter().any(|event| {
                event.as_action().is_some_and(
                |topic| topic.event == "finish"
                    && topic.status == "completed"
                    && matches!(topic.kind, CoreActionKind::Bash { ref mode, .. } if mode == "poll")
            )
            }),
            "poll action should emit a finish/completed topic"
        );
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("Action result: run_bash"));
        assert!(model.prompts[1].contains("Polling state: finished"));
    }

    #[test]
    fn session_turn_long_positive_timeout_command_decline_becomes_user_supplement() {
        let _guard = crate::shell_exec::set_long_running_command_prompt_after_for_tests(
            Duration::from_millis(50),
        );
        let dir = tmp_dir("long_command_decline_supplement");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Approve);
        let mut config = test_config();
        let mut ui = DeclineLongRunningCommandUi::default();
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"{"status":"working","free_talk":"运行一个长命令。","working_still_action":[{"action":"run_bash","args":{"cmd":"sleep 2; printf should_not_finish","timeout_ms":5000}}]}"#,
                1_000,
                false,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"已按用户停止等待后的补充继续处理。"}"#,
                1_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "run a blocking command",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "已按用户停止等待后的补充继续处理。");
        assert_eq!(ui.requests.len(), 1);
        assert_eq!(ui.requests[0].command, "sleep 2; printf should_not_finish");
        assert_eq!(ui.requests[0].timeout_ms, Some(5000));
        assert!(model.prompts[1].contains("user cancels the command"));
        assert!(
            model.prompts[1].contains("You can initiate action to check current working status")
        );
        let prompt = core.render_prompt();
        assert!(prompt.contains("The command was cancelled before it completed"));
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "user_supplement"), 1);
    }

    #[test]
    fn sequential_group_with_long_timeout_command_uses_host_decision_path() {
        let _guard = crate::shell_exec::set_long_running_command_prompt_after_for_tests(
            Duration::from_millis(50),
        );
        let dir = tmp_dir("sequential_long_timeout_decline");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Approve);
        let mut config = test_config();
        let mut ui = DeclineLongRunningCommandUi::default();
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"## Free_talk
启动顺序动作组。

## Working_Still_Action
```action
[
  {
    "order": "sequential",
    "actions": [
      {"action":"run_bash","args":{"cmd":"printf quick","timeout_ms":3000}},
      {"action":"run_bash","args":{"cmd":"sleep 2; printf late","timeout_ms":5000}}
    ]
  }
]
```"#,
                1_000,
                false,
            )),
            Ok(llm(
                r#"## Status
finished

## Final_Answer
已按停止等待后的补充继续。"#,
                1_200,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "执行含长阻塞的并行动作组",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "已按停止等待后的补充继续。");
        assert!(!ui.requests.is_empty());
        assert_eq!(
            ui.requests.last().map(|request| request.command.as_str()),
            Some("sleep 2; printf late")
        );
        assert!(model.prompts[1].contains("quick"));
        assert!(model.prompts[1].contains("user cancels the command"));
        assert!(core
            .render_prompt()
            .contains("The command was cancelled before it completed"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_executes_parallel_action_group_before_next_group() {
        let dir = tmp_dir("parallel_action_groups_session");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Approve);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"## Free_talk
正在并行检查两个本地状态。

## Working_Still_Action
```action
[
  {
    "order": "parallel",
    "actions": [
      {"action":"run_bash","args":{"cmd":"sleep 1; printf group_a","timeout_ms":3000}},
      {"action":"run_bash","args":{"cmd":"sleep 1; printf group_b","timeout_ms":3000}}
    ]
  },
  {
    "order": "sequential",
    "actions": [
      {"action":"run_bash","args":{"cmd":"printf group_c","timeout_ms":3000}}
    ]
  }
]
```"#,
                1_000,
                false,
            )),
            Ok(llm(
                r#"## Status
finished

## Final_Answer
分组动作完成。"#,
                1_200,
                false,
            )),
        ]);

        let started = std::time::Instant::now();
        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "执行分组动作",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );
        let elapsed = started.elapsed();

        assert_eq!(outcome.text, "分组动作完成。");
        assert!(
            elapsed < std::time::Duration::from_millis(1800),
            "parallel group should not run two one-second commands serially; elapsed={elapsed:?}"
        );
        assert!(model.prompts[1].contains("group_a"));
        assert!(model.prompts[1].contains("group_b"));
        assert!(model.prompts[1].contains("group_c"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_parallel_group_spawns_bash_while_running_builtin_actions_in_order() {
        let dir = tmp_dir("mixed_parallel_action_group_session");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Approve);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"## Free_talk
并行执行两个 bash，同时执行一个 builtin 查询。

## Working_Still_Action
```action
[
  {
    "order": "parallel",
    "actions": [
      {"action":"run_bash","args":{"cmd":"sleep 1; printf group_a","timeout_ms":3000}},
      {"action":"memmgr","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 1","params":["%project%"],"limit":1}},
      {"action":"run_bash","args":{"cmd":"sleep 1; printf group_b","timeout_ms":3000}}
    ]
  }
]
```"#,
                1_000,
                false,
            )),
            Ok(llm(
                r#"## Status
finished

## Final_Answer
混合并行动作完成。"#,
                1_200,
                false,
            )),
        ]);

        let started = std::time::Instant::now();
        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "执行混合 parallel 动作",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );
        let elapsed = started.elapsed();

        assert_eq!(outcome.text, "混合并行动作完成。");
        assert!(
            elapsed < std::time::Duration::from_millis(1800),
            "parallel group should spawn bash before builtin work; elapsed={elapsed:?}"
        );
        let second_parts = crate::prompt_parts_from_rendered_prompt(&model.prompts[1]);
        let results_start = second_parts
            .new_delta
            .find("The following are results")
            .unwrap();
        let results = &second_parts.new_delta[results_start..];
        let first_bash = results.find("group_a").unwrap();
        let builtin = results.find("Action result: memmgr").unwrap();
        let second_bash = results.find("group_b").unwrap();
        assert!(first_bash < builtin);
        assert!(builtin < second_bash);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_parallel_group_collects_approvals_then_spawns_bash_concurrently() {
        let dir = tmp_dir("parallel_approval_group_session");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Ask);
        let mut config = test_config();
        let mut ui = ApproveAllUi {
            approval_requests: 0,
        };
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"## Free_talk
先审批两个 Bash，然后并发执行。

## Working_Still_Action
```action
[
  {
    "order": "parallel",
    "actions": [
      {"action":"run_bash","args":{"cmd":"sleep 1; printf approved_a","timeout_ms":3000}},
      {"action":"memmgr","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 1","params":["%project%"],"limit":1}},
      {"action":"run_bash","args":{"cmd":"sleep 1; printf approved_b","timeout_ms":3000}}
    ]
  }
]
```"#,
                1_000,
                false,
            )),
            Ok(llm(
                r#"## Status
finished

## Final_Answer
审批后的并行动作完成。"#,
                1_200,
                false,
            )),
        ]);

        let started = std::time::Instant::now();
        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "执行需要审批的并行动作",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );
        let elapsed = started.elapsed();

        assert_eq!(outcome.text, "审批后的并行动作完成。");
        assert_eq!(ui.approval_requests, 2);
        assert!(
            elapsed < std::time::Duration::from_millis(1800),
            "approved parallel bash actions should run concurrently after approval; elapsed={elapsed:?}"
        );
        let second_parts = crate::prompt_parts_from_rendered_prompt(&model.prompts[1]);
        let first_bash = second_parts
            .new_delta
            .find("Command: sleep 1; printf approved_a")
            .unwrap();
        let builtin = second_parts
            .new_delta
            .find("Action result: memmgr")
            .unwrap();
        let second_bash = second_parts
            .new_delta
            .find("Command: sleep 1; printf approved_b")
            .unwrap();
        assert!(first_bash < builtin);
        assert!(builtin < second_bash);
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "user_approval"), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_user_supplement_during_model_wait_continues_after_stale_final() {
        let dir = tmp_dir("user_supplement_during_wait");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = SupplementDuringModelUi::default();
        let mut model = PollingReplayModel::new([
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"旧答案。"}"#,
                1_000,
                false,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"已按补充重新回答。"}"#,
                1_200,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "先回答",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "已按补充重新回答。");
        assert_eq!(outcome.stats.llm_calls, 2);
        assert_eq!(outcome.stats.prompt_tokens, 2_200);
        assert_eq!(
            outcome
                .latest_usage
                .as_ref()
                .map(|usage| usage.prompt_tokens),
            Some(1_200)
        );
        assert_eq!(model.inner.prompts.len(), 2);
        assert!(!model.inner.prompts[0].contains("user_supplement"));
        assert!(model.inner.prompts[1].contains("## USER"));
        assert!(model.inner.prompts[1].contains("补充：请按最新指示重新回答"));
        assert_eq!(
            ui.discarded_responses,
            vec![(1, "user_supplement_preempted_stale_response".to_string())]
        );
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "user_supplement"), 1);
    }

    #[test]
    fn session_turn_user_supplement_preempts_stale_truncated_output_expansion() {
        let dir = tmp_dir("user_supplement_preempts_truncated_expand");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = SupplementAndExpansionUi::default();
        let mut model = PollingReplayModel::new([
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"旧输出被截断"#,
                10_000,
                true,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"已按补充重新回答。"}"#,
                1_200,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "先回答",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "已按补充重新回答。");
        assert_eq!(outcome.stats.llm_calls, 2);
        assert_eq!(outcome.stats.prompt_tokens, 11_200);
        assert_eq!(
            outcome
                .latest_usage
                .as_ref()
                .map(|usage| usage.prompt_tokens),
            Some(1_200)
        );
        assert_eq!(ui.expansion_requests, 0);
        assert_eq!(model.inner.prompts.len(), 2);
        assert!(!model.inner.prompts[0].contains("user_supplement"));
        assert!(model.inner.prompts[1].contains("## USER"));
        assert!(model.inner.prompts[1].contains("不要展开旧输出"));
        assert_eq!(
            ui.discarded_responses,
            vec![(1, "user_supplement_preempted_stale_response".to_string())]
        );
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "user_supplement"), 1);
        assert_eq!(audit_event_count(&events, "max_llm_output_increased"), 0);
    }

    #[test]
    fn session_turn_preserves_incremental_prompt_cache_plan_across_rounds() {
        let dir = tmp_dir("session_cache_plan");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"{"status":"working","free_talk":"查询 scratch 后继续。","working_still_action":{"action":"memmgr","args":{"type":"scratch","op":"search","search_text":"","limit":3}}}"#,
                5_000,
                false,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"没有找到相关 scratch。"}"#,
                5_800,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "帮我看看最近 scratch 里有什么",
                session: "cache_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "没有找到相关 scratch。");
        assert_eq!(model.prompts.len(), 2);

        let first_blocks = crate::plan_prompt_cache(&model.prompts[0]);
        assert_eq!(first_blocks.len(), 3);
        assert_eq!(first_blocks[0].cache, crate::CacheControl::Ephemeral);
        assert_eq!(first_blocks[1].cache, crate::CacheControl::Ephemeral);
        assert_eq!(first_blocks[2].cache, crate::CacheControl::None);
        assert_eq!(
            first_blocks[2].text,
            crate::prompt_render::formatted_response_trailer("XML")
        );

        let second_parts = crate::prompt_parts_from_rendered_prompt(&model.prompts[1]);
        assert!(second_parts.static_prompt.contains("test static prompt"));
        assert!(second_parts.old_deltas.contains("帮我看看最近 scratch"));
        assert!(second_parts.new_delta.contains("Action result: memmgr"));
        assert!(second_parts.new_delta.contains("查询 scratch 后继续。"));
        let second_blocks = crate::plan_incremental_cache(second_parts);
        assert_eq!(second_blocks.len(), 3);
        assert_eq!(second_blocks[0].cache, crate::CacheControl::Ephemeral);
        assert!(second_blocks[1..]
            .iter()
            .all(|block| block.cache == crate::CacheControl::Ephemeral));
    }

    #[test]
    fn session_turn_preserves_cache_plan_with_json_response_protocol() {
        let dir = tmp_dir("session_cache_plan_json_protocol");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(
            include_str!("../../resources/system_prompt/system_prompt.md"),
            test_profile(),
            &dir,
        );
        core.set_response_protocol(crate::ResponseProtocolKind::Markdown);
        core.set_response_protocol(crate::ResponseProtocolKind::Json);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"{"status":"working","free_talk":"查询 scratch 后继续。","working_still_action":[{"action":"memmgr","args":{"type":"scratch","op":"search","search_text":"","limit":3}}]}"#,
                5_000,
                false,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"没有找到相关 scratch。"}"#,
                5_800,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "帮我看看最近 scratch 里有什么",
                session: "cache_json_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "没有找到相关 scratch。");
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[0].contains("Always use exactly one top-level JSON object."));
        assert!(model.prompts[1].contains("Always use exactly one top-level JSON object."));

        let second_parts = crate::prompt_parts_from_rendered_prompt(&model.prompts[1]);
        assert!(second_parts
            .static_prompt
            .contains("Always use exactly one top-level JSON object."));
        assert!(second_parts.old_deltas.contains("帮我看看最近 scratch"));
        assert!(second_parts.new_delta.contains("Action result: memmgr"));
        let second_blocks = crate::plan_incremental_cache(second_parts);
        assert_eq!(second_blocks.len(), 3);
        assert_eq!(second_blocks[0].cache, crate::CacheControl::Ephemeral);
        assert!(second_blocks[1..]
            .iter()
            .all(|block| block.cache == crate::CacheControl::Ephemeral));
    }

    #[test]
    fn session_turn_preserves_cache_plan_with_markdown_response_protocol() {
        let dir = tmp_dir("session_cache_plan_markdown_protocol");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(
            include_str!("../../resources/system_prompt/system_prompt.md"),
            test_profile(),
            &dir,
        );
        core.set_response_protocol(crate::ResponseProtocolKind::Markdown);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                r##"## Free_talk
查询 scratch 后继续。

## Working_Still_Action
```action
{
  "action": "memmgr",
  "args": {
    "type": "scratch",
    "op": "search",
    "search_text": "",
    "limit": 3
  }
}
```"##,
                5_000,
                false,
            )),
            Ok(llm(
                r#"## Status
finished

## Final_Answer
没有找到相关 scratch。"#,
                5_800,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "帮我看看最近 scratch 里有什么",
                session: "cache_markdown_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "没有找到相关 scratch。");
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[0].contains("The top-level response is Markdown, not JSON."));
        assert!(model.prompts[1].contains("The top-level response is Markdown, not JSON."));

        let second_parts = crate::prompt_parts_from_rendered_prompt(&model.prompts[1]);
        assert!(second_parts
            .static_prompt
            .contains("The top-level response is Markdown, not JSON."));
        assert!(second_parts.old_deltas.contains("帮我看看最近 scratch"));
        assert!(second_parts.new_delta.contains("Action result: memmgr"));
        let second_blocks = crate::plan_incremental_cache(second_parts);
        assert_eq!(second_blocks.len(), 3);
        assert_eq!(second_blocks[0].cache, crate::CacheControl::Ephemeral);
        assert!(second_blocks[1..]
            .iter()
            .all(|block| block.cache == crate::CacheControl::Ephemeral));
    }

    #[test]
    fn session_turn_preserves_cache_plan_with_xml_response_protocol() {
        let dir = tmp_dir("session_cache_plan_xml_protocol");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(
            include_str!("../../resources/system_prompt/system_prompt.md"),
            test_profile(),
            &dir,
        );
        core.set_response_protocol(crate::ResponseProtocolKind::Xml);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"<response>
<free_talk>查询 scratch 后继续。</free_talk>
<working_still_action>
<action_json><![CDATA[
{
  "action": "memmgr",
  "args": {
    "type": "scratch",
    "op": "search",
    "search_text": "",
    "limit": 3
  }
}
]]></action_json>
</working_still_action>
</response>"#,
                5_000,
                false,
            )),
            Ok(llm(
                r#"<response>
<status>ALL_FINISHED</status>
<final_answer>没有找到相关 scratch。</final_answer>
</response>"#,
                5_800,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "帮我看看最近 scratch 里有什么",
                session: "cache_xml_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "没有找到相关 scratch。");
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[0].contains("# System Response Protocol"));
        assert!(model.prompts[1].contains("# System Response Protocol"));

        let second_parts = crate::prompt_parts_from_rendered_prompt(&model.prompts[1]);
        assert!(second_parts
            .static_prompt
            .contains("# System Response Protocol"));
        assert!(second_parts.old_deltas.contains("帮我看看最近 scratch"));
        assert!(second_parts.new_delta.contains("Action result: memmgr"));
        let second_blocks = crate::plan_incremental_cache(second_parts);
        assert_eq!(second_blocks.len(), 3);
        assert_eq!(second_blocks[0].cache, crate::CacheControl::Ephemeral);
        assert!(second_blocks[1..]
            .iter()
            .all(|block| block.cache == crate::CacheControl::Ephemeral));
    }

    #[test]
    fn session_turn_replays_previous_assistant_components_before_next_user_input() {
        let dir = tmp_dir("session_prompt_component_replay");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Json);
        core.set_assistant_speaker_name("Ai4");
        let mut config = test_config();
        config.response_protocol = crate::ResponseProtocolKind::Json;
        let mut ui = NoopTurnUi;

        let mut first_model = ReplayModel::new([Ok(llm(
            r#"{"status":"ALL_FINISHED","free_talk":"previous free talk","final_answer":"previous answer"}"#,
            4_000,
            false,
        ))]);
        let first = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "first user input",
                session: "component_replay_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut first_model,
        );
        assert_eq!(first.text, "previous answer");

        let mut second_model = ReplayModel::new([Ok(llm(
            r#"{"status":"ALL_FINISHED","final_answer":"second answer"}"#,
            4_200,
            false,
        ))]);
        let second = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "second user input",
                session: "component_replay_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: Some("runtime note after user"),
            },
            &mut ui,
            None,
            &mut second_model,
        );
        assert_eq!(second.text, "second answer");

        let prompt = &second_model.prompts[0];
        let free_talk = prompt.find("previous free talk").unwrap();
        let previous_answer = prompt.find("previous answer").unwrap();
        let user = prompt.find("second user input").unwrap();
        let runtime_note = prompt.find("runtime note after user").unwrap();
        assert!(free_talk < user);
        assert!(previous_answer < user);
        assert!(user < runtime_note);
        assert!(prompt.contains("## Ai4"));
        assert!(!prompt.contains("created_at_ms"));
        assert!(!prompt.contains("batch_id"));
    }

    #[test]
    fn session_turn_defaults_to_raw_assistant_output_replay() {
        let dir = tmp_dir("session_raw_assistant_replay");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Xml);
        core.set_assistant_speaker_name("Ai4");
        let mut config = test_config();
        config.response_protocol = crate::ResponseProtocolKind::Xml;
        let mut ui = NoopTurnUi;

        let raw_first_response = r#"<response>
<free_talk>raw planning note</free_talk>
<status>ALL_FINISHED</status>
<final_answer>visible answer</final_answer>
</response>"#;
        let mut first_model = ReplayModel::new([Ok(llm(raw_first_response, 4_000, false))]);
        let first = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "first user input",
                session: "raw_replay_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut first_model,
        );
        assert_eq!(first.text, "visible answer");

        let mut second_model = ReplayModel::new([Ok(llm(
            r#"<response><status>ALL_FINISHED</status><final_answer>second answer</final_answer></response>"#,
            4_200,
            false,
        ))]);
        let second = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "second user input",
                session: "raw_replay_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut second_model,
        );
        assert_eq!(second.text, "second answer");

        let prompt = &second_model.prompts[0];
        let assistant = prompt.find("## Ai4").unwrap();
        let raw = prompt.find(raw_first_response).unwrap();
        let user = prompt.find("second user input").unwrap();
        assert!(assistant < raw);
        assert!(raw < user);
        assert!(!prompt.contains("Final Answer:\nvisible answer"));
        assert!(!prompt.contains("All previous pending open tasks are completed."));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_uses_host_supplied_runtime_context() {
        let dir = tmp_dir("host_runtime_context");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Json);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([Ok(llm(
            r#"{"status":"ALL_FINISHED","final_answer":"host context ok"}"#,
            1_000,
            false,
        ))]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "hello",
                session: "host_context_session",
                audit_file: &audit,
                runtime: "timem_ios_host",
                run_bash_target: "not_available",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "host context ok");
        assert_eq!(model.prompts.len(), 1);
        assert!(model.prompts[0].contains("runtime: timem_ios_host"));
        assert!(model.prompts[0].contains("run_bash_target: not_available"));
        assert!(!model.prompts[0].contains("runtime: timem_native_shell"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_records_cached_tokens_in_profiler_and_latest_usage() {
        let dir = tmp_dir("session_profiler_cache");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut profiler = RuntimeProfiler::default();
        let mut first_usage = usage(7_000, 120);
        first_usage.cached_tokens = 4_000;
        let mut second_usage = usage(8_500, 240);
        second_usage.cached_tokens = 6_500;
        let mut model = ReplayModel::new([
            Ok(LlmResponse {
                content: r#"{"status":"working","free_talk":"先查询 scratch。","working_still_action":[{"action":"memmgr","args":{"type":"scratch","op":"search","search_text":"","limit":3}}]}"#.to_string(),
                model_name: "test-model".to_string(),
                usage: first_usage.clone(),
                truncated: false,
            }),
            Ok(LlmResponse {
                content: r#"{"status":"ALL_FINISHED","final_answer":"完成。"}"#.to_string(),
                model_name: "test-model".to_string(),
                usage: second_usage.clone(),
                truncated: false,
            }),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "跑一轮带 cache usage 的任务",
                session: "profiler_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            Some(&mut profiler),
            &mut model,
        );

        assert_eq!(outcome.text, "完成。");
        assert_eq!(outcome.latest_usage, Some(second_usage));
        let profile = profiler.models().get("test:test-model").unwrap();
        assert_eq!(profile.llm_calls, 2);
        assert_eq!(profile.input_tokens, 15_500);
        assert_eq!(profile.output_tokens, 360);
        assert_eq!(profile.cached_tokens, 10_500);
        let report =
            crate::runtime_profile_report(&profiler, &dir, &audit, &dir.join("action_audit.json"));
        let model_report = report
            .models
            .iter()
            .find(|model| model.model == "test:test-model")
            .unwrap();
        assert_eq!(model_report.cache_hit_percent_tenths(), Some(677));
        assert_eq!(model_report.cached_tokens, 10_500);
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
                    r#"{{"free_talk":"","working_still_action":[{{"action":"memmgr","args":{{"type":"context","op":"discard","delta_ids":{}}}}}]}}"#,
                    serde_json::to_string(&delta_ids).unwrap()
                );
                return Ok(llm(content, 13_253, false));
            }
            assert_eq!(self.prompts.len(), 2);
            assert!(prompt.contains("Action result: memmgr"));
            assert!(prompt.contains("type: context"));
            assert!(prompt.contains("op: discard"));
            assert!(!prompt.contains("mode=force_shrink_required"));
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"压缩已完成，可以继续对话。"}"#,
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
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Ask);
        let mut config = test_config();
        let mut ui = CancelImmediately;

        let outcome = run_session_turn(
            &mut core,
            &mut config,
            TurnInput {
                input: "hello",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
        );

        assert!(outcome.text.is_empty());
        assert_eq!(outcome.repair_issue.as_deref(), Some("cancelled_by_user"));
        assert_eq!(outcome.stop_reason, Some(TurnStopReason::CancelledByUser));
        assert_eq!(
            outcome.stop_summary.as_ref().map(|summary| &summary.detail),
            Some(&TurnStopDetail::None)
        );
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "turn_start"), 1);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        assert_eq!(audit_event_count(&events, "llm_request"), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_shows_plain_text_after_protocol_repair_failure() {
        let dir = tmp_dir("plain_text_repair_fallback");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm("{not valid json}", 5_000, false)),
            Ok(llm(
                "提交成功！\n\n**commit `a91a7b8`** — `refactor: simplify app_context_policy`",
                5_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "代码提交下",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(
            outcome.text,
            "提交成功！\n\n**commit `a91a7b8`** — `refactor: simplify app_context_policy`"
        );
        assert_eq!(
            outcome.repair_issue.as_deref(),
            Some("invalid_json_plain_text_fallback")
        );
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("response is not protocol compliant"));
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let repair = audit_event(&events, "model_repair_request").unwrap();
        assert_eq!(repair["issue"], "invalid_json");
        assert_eq!(repair["repair_calls"], 1);
        assert_eq!(repair["repair_calls_delta"], 1);
        assert_eq!(repair["truncated"], false);
        let final_event = audit_event(&events, "turn_final").unwrap();
        assert!(final_event["assistant_output"]
            .as_str()
            .unwrap()
            .contains("提交成功"));
        assert!(!serde_json::to_string(&events)
            .unwrap()
            .contains("模型的回复不符合本地协议"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_protocol_repair_failure_is_structured() {
        let dir = tmp_dir("protocol_repair_failure_stop");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_response_protocol(crate::ResponseProtocolKind::Json);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new((0..6).map(|idx| {
            Ok(llm(
                &format!("{{not valid json repair attempt {idx}"),
                5_000 + idx as u32,
                false,
            ))
        }));

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "代码提交下",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "");
        assert_eq!(outcome.repair_issue.as_deref(), Some("invalid_json"));
        assert_eq!(
            outcome.stop_reason,
            Some(TurnStopReason::ProtocolRepairFailed)
        );
        assert_eq!(
            outcome.stop_summary.as_ref().map(|summary| &summary.detail),
            Some(&TurnStopDetail::ProtocolRepairFailure {
                first_issue: "invalid_json".to_string(),
                final_issue: "invalid_json".to_string(),
                truncated: false,
            })
        );
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "model_repair_request"), 5);
        let final_event = audit_event(&events, "turn_final").unwrap();
        assert_eq!(final_event["assistant_output"], "");
        assert_eq!(final_event["repair_issue"], "invalid_json");
        assert_eq!(
            final_event["stop_summary"]["detail"]["kind"],
            "protocol_repair_failure"
        );
        assert_eq!(
            final_event["stop_summary"]["detail"]["final_issue"],
            "invalid_json"
        );
        assert_eq!(
            final_event["stop_summary"]["stop_reason"],
            "protocol_repair_failed"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_markdown_repair_keeps_markdown_protocol_instruction() {
        let dir = tmp_dir("markdown_repair_protocol_instruction");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(
            include_str!("../../resources/system_prompt/system_prompt.md"),
            test_profile(),
            &dir,
        );
        core.set_response_protocol(crate::ResponseProtocolKind::Markdown);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                "## Free_talk\nI know the answer but forgot to provide an answer section.",
                5_000,
                false,
            )),
            Ok(llm(
                "## Status\nfinished\n\n## Final_Answer\n当前协议要求回复 Markdown sections。",
                5_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "现在给你的 prompt 让你回复 JSON 还是 Markdown？",
                session: "markdown_repair_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "当前协议要求回复 Markdown sections。");
        assert_eq!(model.prompts.len(), 2);
        let repair_prompt = &model.prompts[1];
        assert!(repair_prompt.contains("## SYSTEM"));
        assert!(repair_prompt.contains("response is not protocol compliant"));
        assert!(repair_prompt.contains("Markdown response protocol"));
        assert!(repair_prompt.contains("## Free_talk"));
        assert!(repair_prompt.contains("## Working_Still_Action"));
        assert!(!repair_prompt.contains("Return exactly one valid JSON object"));
        assert!(!repair_prompt.contains("Do not use markdown fences"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_forced_shrink_runs_to_final_without_repeated_shrink() {
        let dir = tmp_dir("forced_shrink_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_max_llm_input_tokens(10_000);
        let mut config = test_config();
        config.max_llm_input_tokens = 10_000;

        let _ = core.begin_turn(&"old dynamic context ".repeat(1_500), None);
        let seed_step = core.apply_model_response(llm(
            r#"{"status":"ALL_FINISHED","final_answer":"seeded"}"#,
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
            TurnInput {
                input: "继续",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
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
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "turn_start"), 1);
        let final_event = audit_event(&events, "turn_final").unwrap();
        assert!(final_event["assistant_output"]
            .as_str()
            .unwrap()
            .contains("压缩已完成，可以继续对话。"));
        let _ = std::fs::remove_dir_all(dir);
    }

    struct ExpandOutputUi {
        expansion_requests: u32,
        last_request: Option<OutputExpansionRequest>,
        request_topics: u32,
        last_topic_name: Option<String>,
        last_topic_blocking: bool,
    }

    impl TurnUi for ExpandOutputUi {
        fn can_request_output_expansion(&mut self) -> bool {
            true
        }

        fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
            for event in events {
                if event.is_blocking_request() {
                    self.request_topics += 1;
                    self.last_topic_name = Some(event.topic.name.clone());
                    self.last_topic_blocking = true;
                }
            }
        }

        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            match request {
                HostDecisionRequest::OutputExpansion(request) => {
                    self.expansion_requests += 1;
                    self.last_request = Some(request);
                    HostDecision::Accept
                }
                other => other.safe_default().into(),
            }
        }
    }

    #[test]
    fn session_turn_truncated_output_expands_limit_and_retries_same_turn() {
        let dir = tmp_dir("truncated_expansion_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        config.max_llm_output_tokens = 8192;
        let mut ui = ExpandOutputUi {
            expansion_requests: 0,
            last_request: None,
            request_topics: 0,
            last_topic_name: None,
            last_topic_blocking: false,
        };
        let mut model = ReplayModel::new([
            Ok(llm(r#"{"free_talk":"partial""#, 5_000, true)),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"扩容后完成。"}"#,
                5_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "生成长报告",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "扩容后完成。");
        assert_eq!(outcome.stop_reason, None);
        assert_eq!(ui.expansion_requests, 1);
        assert_eq!(ui.request_topics, 1);
        assert_eq!(
            ui.last_topic_name.as_deref(),
            Some(CORE_TOPIC_OUTPUT_EXPAND_REQUEST)
        );
        assert!(ui.last_topic_blocking);
        assert_eq!(
            ui.last_request,
            Some(OutputExpansionRequest {
                current_tokens: 8192,
                increment_tokens: 10_000,
                retry_same_turn: true,
            })
        );
        assert_eq!(model.prompts.len(), 2);
        assert_eq!(config.max_llm_output_tokens, 18_192);
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "max_llm_output_increased"), 1);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_noop_ui_uses_default_output_expansion() {
        let dir = tmp_dir("truncated_noop_expansion_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        config.max_llm_output_tokens = 8192;
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(r#"{"free_talk":"partial""#, 5_000, true)),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"默认扩容后完成。"}"#,
                5_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "生成长报告",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "默认扩容后完成。");
        assert_eq!(outcome.stop_reason, None);
        assert_eq!(model.prompts.len(), 2);
        assert_eq!(config.max_llm_output_tokens, 18_192);
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "max_llm_output_increased"), 1);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    struct DeclineExpandOutputUi {
        expansion_requests: u32,
    }

    impl TurnUi for DeclineExpandOutputUi {
        fn can_request_output_expansion(&mut self) -> bool {
            true
        }

        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            match request {
                HostDecisionRequest::OutputExpansion(_) => {
                    self.expansion_requests += 1;
                    HostDecision::Decline
                }
                other => other.safe_default().into(),
            }
        }
    }

    #[test]
    fn session_turn_truncated_output_stop_sets_structured_stop_reason() {
        let dir = tmp_dir("truncated_stop_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        config.max_llm_output_tokens = 8192;
        let mut ui = DeclineExpandOutputUi {
            expansion_requests: 0,
        };
        let mut model = ReplayModel::new([Ok(llm(
            r#"{"status":"ALL_FINISHED","final_answer":"partial"#,
            5_000,
            true,
        ))]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "生成长报告但不扩容",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert!(outcome.text.is_empty());
        assert_eq!(
            outcome.repair_issue.as_deref(),
            Some("truncated_output_stopped_by_user")
        );
        assert_eq!(
            outcome.stop_reason,
            Some(TurnStopReason::OutputLimitStoppedByUser)
        );
        assert_eq!(
            outcome.stop_summary.as_ref().map(|summary| &summary.detail),
            Some(&TurnStopDetail::OutputLimit {
                current_tokens: 8192
            })
        );
        assert_eq!(ui.expansion_requests, 1);
        assert_eq!(model.prompts.len(), 1);
        assert_eq!(config.max_llm_output_tokens, 8192);
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "max_llm_output_increased"), 0);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    struct ContinueRoundLimitUi {
        continue_requests: u32,
        last_request: Option<RoundLimitDecisionRequest>,
    }

    impl TurnUi for ContinueRoundLimitUi {
        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            match request {
                HostDecisionRequest::RoundLimitContinue(request) => {
                    self.continue_requests += 1;
                    self.last_request = Some(request);
                    HostDecision::Accept
                }
                other => other.safe_default().into(),
            }
        }
    }

    #[test]
    fn session_turn_round_limit_continue_recharges_and_finishes_same_task() {
        let dir = tmp_dir("round_limit_continue_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_max_rounds(1);
        let mut config = test_config();
        let mut ui = ContinueRoundLimitUi {
            continue_requests: 0,
            last_request: None,
        };
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"{"free_talk":"","working_still_action":[{"action":"memmgr","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%round limit e2e%"],"limit":5}}]}"#,
                4_000,
                false,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"续跑后完成。"}"#,
                4_200,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "需要多轮完成",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "续跑后完成。");
        assert_eq!(ui.continue_requests, 1);
        assert_eq!(
            ui.last_request,
            Some(RoundLimitDecisionRequest {
                max_rounds: 1,
                recharge_rounds: 1,
                keep_task_context: true,
            })
        );
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("Runtime round budget continued by user."));
        let events = read_audit_events(&audit);
        let round_limit = audit_event(&events, "round_limit").unwrap();
        assert_eq!(round_limit["continued"], true);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_noop_ui_uses_default_round_limit_continue() {
        let dir = tmp_dir("round_limit_noop_continue_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_max_rounds(1);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"{"status":"working","free_talk":"","working_still_action":[{"action":"memmgr","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%round limit noop%"],"limit":5}}]}"#,
                4_000,
                false,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"默认续跑后完成。"}"#,
                4_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "需要默认续跑",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "默认续跑后完成。");
        assert_eq!(outcome.stop_reason, None);
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("Runtime round budget continued by user."));
        let events = read_audit_events(&audit);
        let round_limit = audit_event(&events, "round_limit").unwrap();
        assert_eq!(round_limit["continued"], true);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    struct DeclineRoundLimitUi {
        continue_requests: u32,
    }

    impl TurnUi for DeclineRoundLimitUi {
        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            match request {
                HostDecisionRequest::RoundLimitContinue(_) => {
                    self.continue_requests += 1;
                    HostDecision::Decline
                }
                other => other.safe_default().into(),
            }
        }
    }

    #[test]
    fn session_turn_round_limit_stop_sets_structured_stop_reason() {
        let dir = tmp_dir("round_limit_stop_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_max_rounds(1);
        let mut config = test_config();
        let mut ui = DeclineRoundLimitUi {
            continue_requests: 0,
        };
        let mut model = ReplayModel::new([Ok(llm(
            r#"{"status":"working","free_talk":"先查证据。","working_still_action":[{"action":"memmgr","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%round limit stop%"],"limit":5}}]}"#,
            4_000,
            false,
        ))]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "需要多轮但不要继续",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert!(outcome.text.is_empty());
        assert_eq!(ui.continue_requests, 1);
        assert_eq!(outcome.repair_issue.as_deref(), Some("round_limit_reached"));
        assert_eq!(outcome.stop_reason, Some(TurnStopReason::RoundLimitReached));
        assert_eq!(
            outcome.stop_summary.as_ref().map(|summary| &summary.detail),
            Some(&TurnStopDetail::RoundLimit { max_rounds: 1 })
        );
        let events = read_audit_events(&audit);
        let round_limit = audit_event(&events, "round_limit").unwrap();
        assert_eq!(round_limit["continued"], false);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    struct ApproveAllUi {
        approval_requests: u32,
    }

    impl TurnUi for ApproveAllUi {
        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            match request {
                HostDecisionRequest::UserApproval(_) => {
                    self.approval_requests += 1;
                    HostDecision::Accept
                }
                other => other.safe_default().into(),
            }
        }
    }

    #[test]
    fn session_turn_bash_approval_executes_action_then_finishes_with_audit() {
        let dir = tmp_dir("bash_approval_e2e");
        let audit = dir.join("audit.json");
        let output_file = dir.join("approved.txt");
        let command = format!("printf approved > {}", output_file.display());
        let first_response = format!(
            r#"{{"free_talk":"","working_still_action":[{{"action":"run_bash","args":{{"cmd":{},"timeout_ms":5000}}}}]}}"#,
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
                r#"{"status":"ALL_FINISHED","final_answer":"命令已执行并确认。"}"#,
                3_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "执行一个需要审批的本地写入",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
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
        assert!(model.prompts[1].contains("Exit code: 0"));
        let events = read_audit_events(&audit);
        let approval = audit_event(&events, "user_approval").unwrap();
        assert_eq!(approval["approved"], true);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    struct CancelApprovalUi {
        approval_requests: u32,
        pause_count: u32,
        resume_count: u32,
        cancel_requested: bool,
    }

    impl TurnUi for CancelApprovalUi {
        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            match request {
                HostDecisionRequest::UserApproval(_) => {
                    self.approval_requests += 1;
                    self.cancel_requested = true;
                    HostDecision::Decline
                }
                other => other.safe_default().into(),
            }
        }

        fn take_cancel_request(&mut self) -> bool {
            let cancel = self.cancel_requested;
            self.cancel_requested = false;
            cancel
        }

        fn pause_for_user_decision(&mut self) {
            self.pause_count += 1;
        }

        fn resume_after_user_decision(&mut self) {
            self.resume_count += 1;
        }
    }

    #[test]
    fn session_turn_cancelled_user_approval_resumes_ui_before_continuing() {
        let dir = tmp_dir("bash_approval_cancel_resumes_ui");
        let audit = dir.join("audit.json");
        let output_file = dir.join("cancelled.txt");
        let command = format!("printf cancelled > {}", output_file.display());
        let first_response = format!(
            r#"{{"status":"working","free_talk":"需要审批。","working_still_action":[{{"action":"run_bash","args":{{"cmd":{},"timeout_ms":5000}}}}]}}"#,
            serde_json::to_string(&command).unwrap()
        );

        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Ask);
        let mut config = test_config();
        let mut ui = CancelApprovalUi {
            approval_requests: 0,
            pause_count: 0,
            resume_count: 0,
            cancel_requested: false,
        };
        let mut model = ReplayModel::new([
            Ok(llm(first_response, 3_000, false)),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"用户取消审批，已停止执行。"}"#,
                3_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "执行一个需要审批但会被取消的本地写入",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "用户取消审批，已停止执行。");
        assert_eq!(ui.approval_requests, 1);
        assert_eq!(ui.pause_count, 1);
        assert_eq!(ui.resume_count, 1);
        assert!(!output_file.exists());
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("status: denied_by_user"));
        let events = read_audit_events(&audit);
        let approval = audit_event(&events, "user_approval").unwrap();
        assert_eq!(approval["approved"], false);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_noop_ui_uses_default_user_approval() {
        let dir = tmp_dir("bash_approval_noop_e2e");
        let audit = dir.join("audit.json");
        let output_file = dir.join("approved_by_default.txt");
        let command = format!("printf default-approved > {}", output_file.display());
        let first_response = format!(
            r#"{{"status":"working","free_talk":"","working_still_action":[{{"action":"run_bash","args":{{"cmd":{},"timeout_ms":5000}}}}]}}"#,
            serde_json::to_string(&command).unwrap()
        );

        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Ask);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(first_response, 3_000, false)),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"默认审批后完成。"}"#,
                3_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "执行一个需要默认审批的本地写入",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "默认审批后完成。");
        assert_eq!(
            std::fs::read_to_string(&output_file).unwrap(),
            "default-approved"
        );
        assert_eq!(model.prompts.len(), 2);
        let events = read_audit_events(&audit);
        let approval = audit_event(&events, "user_approval").unwrap();
        assert_eq!(approval["approved"], true);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_turn_finished_with_actions_repairs_then_accepts_plain_final() {
        let dir = tmp_dir("finished_actions_session_repair");
        let audit = dir.join("audit.json");

        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_capability_registry(CapabilityRegistry::builtin());
        core.set_bash_approval_mode(BashApprovalMode::Approve);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"文件已生成并验证。","working_still_action":[{"action":"run_bash","args":{"cmd":"true","timeout_ms":5000}}]}"#,
                3_000,
                false,
            )),
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"文件已生成并验证。"}"#,
                3_100,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "生成并验证文件",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "文件已生成并验证。");
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("status_finished_must_not_include_next_actions"));
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let action_audit_text =
            std::fs::read_to_string(dir.join("audit").join("action_audit.json")).unwrap();
        assert!(!action_audit_text.contains(r#""action":"run_bash""#));
        assert!(!action_audit_text.contains(r#""status":"completed""#));
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
                    r#"{{"free_talk":"","working_still_action":[{{"action":"memmgr","args":{{"type":"scratch","op":"write","kind":"context_offload","label":"session e2e offload","delta_ids":{}}}}}]}}"#,
                    serde_json::to_string(&delta_ids).unwrap()
                );
                return Ok(llm(content, 4_000, false));
            }
            assert_eq!(self.prompts.len(), 2);
            assert!(prompt.contains("Action result: memmgr"));
            assert!(prompt.contains("type: scratch"));
            assert!(prompt.contains("op: write"));
            assert!(prompt.contains("id: scratch_"));
            assert!(prompt.contains("label: session e2e offload"));
            Ok(llm(
                r#"{"status":"ALL_FINISHED","final_answer":"scratch 已记录，可以继续。"}"#,
                4_100,
                false,
            ))
        }
    }

    #[test]
    fn session_turn_scratch_context_offload_records_id_and_continues() {
        let dir = tmp_dir("scratch_offload_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        let mut config = test_config();
        let mut ui = NoopTurnUi;
        let mut model = ScratchOffloadReplayModel {
            prompts: Vec::new(),
        };

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "把当前上下文转存到 scratch 后继续",
                session: "test_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
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
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[derive(Default)]
    struct RecordingTopicUi {
        events: Vec<CoreTopicEvent>,
    }

    impl TurnUi for RecordingTopicUi {
        fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
            self.events.extend_from_slice(events);
        }
    }

    #[test]
    fn session_turn_markdown_protocol_executes_actions_and_emits_topic_events() {
        let dir = tmp_dir("markdown_protocol_observation_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_bash_approval_mode(BashApprovalMode::Approve);
        let mut config = test_config();
        let mut ui = RecordingTopicUi::default();
        let mut model = ReplayModel::new([
            Ok(llm(
                r#"## Free_talk
正在检查本地 shell。

## Working_Still_Action
```action
{
  "action": "run_bash",
  "args": {
    "cmd": "printf markdown-ok",
    "timeout_ms": 5000
  }
}
```"#,
                2_000,
                false,
            )),
            Ok(llm(
                r#"## Status
finished

## Final_Answer
Markdown 协议动作已执行。"#,
                2_200,
                false,
            )),
        ]);

        let outcome = run_session_turn_with_model_client(
            &mut core,
            &mut config,
            TurnInput {
                input: "用 markdown 协议执行一次 shell",
                session: "markdown_session",
                audit_file: &audit,
                runtime: "timem_native_shell",
                run_bash_target: "user_local_machine",
                additional_context: None,
            },
            &mut ui,
            None,
            &mut model,
        );

        assert_eq!(outcome.text, "Markdown 协议动作已执行。");
        assert_eq!(model.prompts.len(), 2);
        assert!(model.prompts[1].contains("Action result: run_bash"));
        assert!(model.prompts[1].contains("markdown-ok"));
        assert!(ui.events.iter().any(|event| {
            event
                .as_model_response()
                .map(|topic| topic.free_talk.contains("正在检查本地 shell。"))
                .unwrap_or(false)
        }));
        assert!(ui.events.iter().any(|event| {
            event.as_action().map_or(false, |topic| {
                topic.action == "run_bash"
                    && topic.active
                    && topic.kind
                        == CoreActionKind::Bash {
                            command: "printf markdown-ok".to_string(),
                            mode: "normal".to_string(),
                            interval_ms: None,
                            timeout_ms: Some(5000),
                            loop_timeout_ms: None,
                            once_timeout_ms: None,
                        }
            })
        }));
        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "turn_final"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    struct StoryReplayModel {
        calls: usize,
        prompts: Vec<String>,
    }

    impl StoryReplayModel {
        fn new() -> Self {
            Self {
                calls: 0,
                prompts: Vec::new(),
            }
        }
    }

    impl ModelClient for StoryReplayModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &Path,
            _should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            self.calls += 1;
            self.prompts.push(prompt.to_string());
            match self.calls {
                1 => Ok(llm(
                    r#"{"status":"ALL_FINISHED","final_answer":"你好，我在。"}"#,
                    2_000,
                    false,
                )),
                2 => Ok(llm("{这不是合法 JSON，但应该走协议修复}", 2_100, false)),
                3 => {
                    assert!(prompt.contains("response is not protocol compliant"));
                    Ok(llm("畸形回复已恢复为用户可读文本。", 2_200, false))
                }
                4 => Ok(llm(
                    r#"{"free_talk":"","working_still_action":[{"action":"memmgr","args":{"type":"durable","op":"upsert","id":"project_code","content":"测试项目代号是 OMEGA-7"}}]}"#,
                    2_300,
                    false,
                )),
                5 => {
                    assert!(prompt.contains("Action result: memmgr"));
                    assert!(prompt.contains("type: durable"));
                    assert!(prompt.contains("op: insert"));
                    assert!(prompt.contains("project_code"));
                    Ok(llm(
                        r#"{"status":"ALL_FINISHED","final_answer":"已记录测试项目代号。"}"#,
                        2_400,
                        false,
                    ))
                }
                6 => Ok(llm(
                    r#"{"free_talk":"","working_still_action":[{"action":"memmgr","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%测试项目代号%"],"limit":5}}]}"#,
                    2_500,
                    false,
                )),
                7 => {
                    assert!(prompt.contains("Action result: memmgr"));
                    assert!(prompt.contains("type: durable"));
                    assert!(prompt.contains("op: sql"));
                    assert!(prompt.contains("测试项目代号是 OMEGA-7"));
                    Ok(llm(
                        r#"{"status":"ALL_FINISHED","final_answer":"测试项目代号是 OMEGA-7。"}"#,
                        7_600,
                        false,
                    ))
                }
                8 => {
                    assert!(prompt.contains("mode=force_shrink_required"));
                    let mut delta_ids = prompt_field_values(prompt, "delta_id");
                    delta_ids.sort();
                    delta_ids.dedup();
                    assert!(
                        !delta_ids.is_empty(),
                        "forced discard prompt should expose delta ids"
                    );
                    let content = format!(
                        r#"{{"free_talk":"","working_still_action":[{{"action":"memmgr","args":{{"type":"scratch","op":"write","kind":"context_offload","label":"story replay context offload","delta_ids":{}}}}}]}}"#,
                        serde_json::to_string(&delta_ids).unwrap()
                    );
                    Ok(llm(content, 7_650, false))
                }
                9 => {
                    assert!(prompt.contains("mode=force_shrink_required"));
                    assert!(prompt.contains("Action result: memmgr"));
                    assert!(prompt.contains("type: scratch"));
                    assert!(prompt.contains("op: write"));
                    assert!(prompt.contains("id: scratch_"));
                    let mut delta_ids = prompt_field_values(prompt, "delta_id");
                    delta_ids.sort();
                    delta_ids.dedup();
                    assert!(
                        !delta_ids.is_empty(),
                        "post-scratch forced discard prompt should expose delta ids"
                    );
                    let content = format!(
                        r#"{{"free_talk":"","working_still_action":[{{"action":"memmgr","args":{{"type":"context","op":"discard","delta_ids":{}}}}}]}}"#,
                        serde_json::to_string(&delta_ids).unwrap()
                    );
                    Ok(llm(content, 7_700, false))
                }
                10 => {
                    assert!(prompt.contains("Action result: memmgr"));
                    assert!(prompt.contains("type: context"));
                    assert!(prompt.contains("op: discard"));
                    assert!(!prompt.contains("mode=force_shrink_required"));
                    Ok(llm(
                        r#"{"status":"ALL_FINISHED","final_answer":"上下文已转存并压缩，可以继续。"}"#,
                        2_000,
                        false,
                    ))
                }
                _ => Err(format!("unexpected_extra_model_call_{}", self.calls)),
            }
        }
    }

    #[test]
    fn session_replay_story_covers_repair_memory_scratch_shrink_and_observation_rendering() {
        let dir = tmp_dir("story_replay_e2e");
        let audit = dir.join("audit.json");
        let mut core = AgentCore::new(r#"{"role":"test static prompt"}"#, test_profile(), &dir);
        core.set_max_llm_input_tokens(8_000);
        let mut config = test_config();
        config.max_llm_input_tokens = 8_000;
        let mut ui = RecordingTopicUi::default();
        let mut model = StoryReplayModel::new();

        let inputs = [
            "你好",
            "请用畸形回复测试协议恢复",
            "记住测试项目代号是 OMEGA-7",
            "测试项目代号是什么？",
            "继续长上下文任务",
        ];
        let long_work_context = "长工作上下文片段。".repeat(2_500);
        let additional_contexts = [None, None, None, Some(long_work_context.as_str()), None];
        let expected_outputs = [
            "你好，我在。",
            "畸形回复已恢复为用户可读文本。",
            "已记录测试项目代号。",
            "测试项目代号是 OMEGA-7。",
            "上下文已转存并压缩，可以继续。",
        ];

        let mut outputs = Vec::new();
        for (input, additional_context) in inputs.into_iter().zip(additional_contexts) {
            let outcome = run_session_turn_with_model_client(
                &mut core,
                &mut config,
                TurnInput {
                    input,
                    session: "story_session",
                    audit_file: &audit,
                    runtime: "timem_native_shell",
                    run_bash_target: "user_local_machine",
                    additional_context,
                },
                &mut ui,
                None,
                &mut model,
            );
            outputs.push(outcome.text);
        }

        assert_eq!(outputs, expected_outputs);
        assert_eq!(model.calls, 10);
        assert!(
            model
                .prompts
                .iter()
                .any(|prompt| prompt.contains("response is not protocol compliant")),
            "story should exercise malformed model response repair"
        );
        assert!(
            model
                .prompts
                .iter()
                .filter(|prompt| prompt.contains("mode=force_shrink_required"))
                .count()
                >= 2,
            "story should force shrink through scratch offload then context discard"
        );

        let memory_text = std::fs::read_to_string(dir.join("memory.jsonl")).unwrap();
        assert!(memory_text.contains("测试项目代号是 OMEGA-7"));
        let scratch_text = std::fs::read_to_string(dir.join("scratch_notes.jsonl")).unwrap();
        assert!(scratch_text.contains(r#""scratch_type":"context_offload""#));
        assert!(scratch_text.contains(r#""label":"story replay context offload""#));

        let action_topics: Vec<_> = ui
            .events
            .iter()
            .filter_map(CoreTopicEvent::as_action)
            .collect();
        assert!(action_topics.iter().any(|topic| {
            topic.kind
                == CoreActionKind::Memory {
                    surface: "durable".to_string(),
                    operation: "upsert".to_string(),
                }
        }));
        assert!(action_topics.iter().any(|topic| {
            topic.kind
                == CoreActionKind::Memory {
                    surface: "durable".to_string(),
                    operation: "sql".to_string(),
                }
        }));
        assert!(action_topics.iter().any(|topic| {
            topic.kind
                == CoreActionKind::Memory {
                    surface: "scratch".to_string(),
                    operation: "write".to_string(),
                }
        }));
        assert!(action_topics.iter().any(|topic| {
            topic.kind
                == CoreActionKind::Memory {
                    surface: "context".to_string(),
                    operation: "discard".to_string(),
                }
        }));

        let events = read_audit_events(&audit);
        assert_eq!(audit_event_count(&events, "turn_start"), inputs.len());
        assert_eq!(audit_event_count(&events, "turn_final"), inputs.len());
        let audit_json = serde_json::to_string(&events).unwrap();
        assert!(audit_json.contains("畸形回复已恢复为用户可读文本。"));
        assert!(audit_json.contains("上下文已转存并压缩，可以继续。"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn noop_turn_ui_uses_core_default_host_decisions() {
        let mut ui = NoopTurnUi;
        let request = ApprovalRequest {
            approval_id: "approval_1".to_string(),
            action: "run_bash".to_string(),
            command: "echo hi".to_string(),
            risk: "test".to_string(),
            reason: "test".to_string(),
        };

        assert!(!ui.is_cancel_requested());
        assert!(!ui.take_cancel_request());
        assert!(ui.request_user_approval(&request));
        assert!(ui.request_round_limit_continue(RoundLimitDecisionRequest::new(20)));
        assert!(ui.can_request_output_expansion());
        assert!(ui.request_expand_output_tokens(OutputExpansionRequest::new(10_000)));
    }
}
