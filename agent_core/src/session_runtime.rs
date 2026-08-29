use crate::turn_state::TurnProjectionState;
use crate::{
    append_audit_event, is_model_input_too_large_error, model_input_overflow_recovery_audit_event,
    model_retry_audit_event, model_retry_decision, normalize_user_supplements_with_context,
    ActionRuntime, AgentCore, CoreStep, CoreTopicEvent, HostDecisionRequest, HttpModelClient,
    LlmResponse, LongRunningCommandDecision, LongRunningCommandStatus, ModelCallOutcome,
    ModelInteractionRequest, ModelServiceConfig, ModelSystemRetryPolicy, PromptComponentRole,
    RoundLimitDecisionRequest, RoundLimitResolution, RuntimeProfiler, StoppedTurn, TurnActivity,
    TurnInput, TurnOutcome, TurnProjection, TurnProjectionOutcome, TurnStopReason, TurnStopSummary,
    TurnToken, TurnUi, UsageStats, UserSupplement,
};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct TimeReminderSchedule {
    interval: Duration,
    last_emitted_period: u64,
    tips: Vec<String>,
}

struct RoundReminderSchedule {
    interval: u32,
    last_emitted_period: u32,
    tips: Vec<String>,
}

struct TurnReminderSchedules {
    time: Vec<TimeReminderSchedule>,
    rounds: Vec<RoundReminderSchedule>,
    random_state: RandomState,
    turn_id: String,
}

const PROGRESS_UPDATE_REMINDER_ROUNDS: u32 = 6;
const PROGRESS_UPDATE_REMINDER: &str = "REMIND: It has been 6 rounds since your last progress update to the user (that is, a useful update beyond tool calls). Pay attention to the user experience and keep the user well informed.";

#[derive(Default)]
struct TurnProgressReminder {
    consecutive_tool_only_rounds: u32,
    last_emitted_period: u32,
}

impl TurnProgressReminder {
    fn observe(&mut self, has_tool_call: bool, has_free_talk: bool) {
        if has_tool_call && !has_free_talk {
            self.consecutive_tool_only_rounds = self.consecutive_tool_only_rounds.saturating_add(1);
            return;
        }
        self.consecutive_tool_only_rounds = 0;
        self.last_emitted_period = 0;
    }

    fn take_due(&mut self) -> Option<&'static str> {
        let period = self.consecutive_tool_only_rounds / PROGRESS_UPDATE_REMINDER_ROUNDS;
        if period == 0 || period <= self.last_emitted_period {
            return None;
        }
        self.last_emitted_period = period;
        Some(PROGRESS_UPDATE_REMINDER)
    }
}

impl TurnReminderSchedules {
    fn new(turn_id: &str, config: &crate::ReminderTipsConfig) -> Self {
        let mut time = Vec::new();
        let mut rounds = Vec::new();
        for schedule in &config.schedules {
            if let Some(minutes) = schedule.every_minutes {
                time.push(TimeReminderSchedule {
                    interval: Duration::from_secs(minutes.saturating_mul(60)),
                    last_emitted_period: 0,
                    tips: schedule.tips.clone(),
                });
            }
            if let Some(interval) = schedule.every_rounds.filter(|interval| *interval > 0) {
                rounds.push(RoundReminderSchedule {
                    interval,
                    last_emitted_period: 0,
                    tips: schedule.tips.clone(),
                });
            }
        }
        Self {
            time,
            rounds,
            random_state: RandomState::new(),
            turn_id: turn_id.to_string(),
        }
    }

    fn override_first_time_interval(&mut self, interval: Duration) {
        if let Some(schedule) = self.time.first_mut() {
            schedule.interval = interval;
        }
    }

    fn take_due_time(&mut self, active_elapsed: Duration) -> Vec<String> {
        let mut due = Vec::new();
        for (index, schedule) in self.time.iter_mut().enumerate() {
            let interval_nanos = schedule.interval.as_nanos();
            if interval_nanos == 0 {
                continue;
            }
            let period = active_elapsed.as_nanos() / interval_nanos;
            let period = u64::try_from(period).unwrap_or(u64::MAX);
            if period == 0 || period <= schedule.last_emitted_period {
                continue;
            }
            // Collapse missed periods after a long blocking operation instead
            // of injecting a backlog of stale reminders.
            schedule.last_emitted_period = period;
            if let Some(tip) = choose_tip(
                &self.random_state,
                &self.turn_id,
                "minutes",
                index,
                period,
                &schedule.tips,
            ) {
                due.push(tip);
            }
        }
        due
    }

    fn take_due_rounds(&mut self, completed_rounds: u32) -> Vec<String> {
        let mut due = Vec::new();
        for (index, schedule) in self.rounds.iter_mut().enumerate() {
            let period = completed_rounds / schedule.interval;
            if period == 0 || period <= schedule.last_emitted_period {
                continue;
            }
            schedule.last_emitted_period = period;
            if let Some(tip) = choose_tip(
                &self.random_state,
                &self.turn_id,
                "rounds",
                index,
                u64::from(period),
                &schedule.tips,
            ) {
                due.push(tip);
            }
        }
        due
    }
}

fn choose_tip(
    random_state: &RandomState,
    turn_id: &str,
    schedule_kind: &str,
    schedule_index: usize,
    period: u64,
    tips: &[String],
) -> Option<String> {
    if tips.is_empty() {
        return None;
    }
    let mut hasher = random_state.build_hasher();
    turn_id.hash(&mut hasher);
    schedule_kind.hash(&mut hasher);
    schedule_index.hash(&mut hasher);
    period.hash(&mut hasher);
    let tip = tips[(hasher.finish() as usize) % tips.len()].trim();
    (!tip.eq_ignore_ascii_case("NONE")).then(|| tip.to_string())
}

pub trait ModelClient {
    fn call_model(
        &mut self,
        config: &ModelServiceConfig,
        prompt: &str,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String>;

    fn call_model_interaction(
        &mut self,
        config: &ModelServiceConfig,
        request: &ModelInteractionRequest,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.call_model(config, &request.rendered_prompt, audit_file, should_cancel)
    }
}

pub fn run_session_turn(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    request: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
) -> TurnOutcome {
    let mut model_client = HttpModelClient;
    run_session_turn_with_model_client(core, config, request, ui, profiler, &mut model_client)
}

pub fn run_session_turn_with_model_client(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    request: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
    model_client: &mut dyn ModelClient,
) -> TurnOutcome {
    run_session_turn_with_model_client_and_reminder_override(
        core,
        config,
        request,
        ui,
        profiler,
        model_client,
        None,
    )
}

#[cfg(test)]
fn run_session_turn_with_model_client_and_focus_interval(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    request: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
    model_client: &mut dyn ModelClient,
    focus_reminder_interval: Duration,
) -> TurnOutcome {
    run_session_turn_with_model_client_and_reminder_override(
        core,
        config,
        request,
        ui,
        profiler,
        model_client,
        Some(focus_reminder_interval),
    )
}

fn run_session_turn_with_model_client_and_reminder_override(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    request: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    mut profiler: Option<&mut RuntimeProfiler>,
    model_client: &mut dyn ModelClient,
    focus_reminder_interval: Option<Duration>,
) -> TurnOutcome {
    core.set_response_protocol(config.response_protocol);
    let turn_token = TurnToken::allocate(request.session, epoch_millis());
    let turn_id = turn_token.turn_id.clone();
    let (mut turn_projection, started_projection) = TurnProjectionState::start(turn_token);
    ui.on_turn_projection(&started_projection);
    let mut reminders = TurnReminderSchedules::new(&turn_id, core.reminder_tips_config());
    let mut progress_reminder = TurnProgressReminder::default();
    if let Some(interval) = focus_reminder_interval {
        reminders.override_first_time_interval(interval);
    }
    core.record_turn_start_audit(request.audit_file, request.session, &turn_id, request.input);
    let profile =
        crate::negotiate_interaction(model_client, config, request.audit_file, &mut || {
            ui.is_cancel_requested()
        });
    core.set_interaction_profile(&profile);
    ui.on_interaction_profile(&profile);
    let start = Instant::now();
    let mut user_wait_this_turn = Duration::ZERO;
    let additional_context = request
        .additional_context
        .map(str::trim)
        .filter(|context| !context.is_empty());
    let mut step = core.begin_turn(request.input, additional_context);
    let mut rounds = 0u32;
    let mut model_wait_this_turn = Duration::ZERO;
    let mut latest_usage: Option<UsageStats> = None;

    let (text, stopped, final_parts) = loop {
        if take_cancel_request(ui, &mut turn_projection) {
            break cancelled_turn_parts();
        }
        match step {
            CoreStep::NeedModel { ref prompt, .. } => {
                if ui.apply_pending_runtime_updates(core, config) {
                    core.set_response_protocol(config.response_protocol);
                    let profile = crate::negotiate_interaction(
                        model_client,
                        config,
                        request.audit_file,
                        &mut || ui.is_cancel_requested(),
                    );
                    core.set_interaction_profile(&profile);
                    ui.on_interaction_profile(&profile);
                    step = CoreStep::NeedModel {
                        prompt: core.build_next_prompt(),
                        rounds_remaining: core.remaining_rounds(),
                    };
                    continue;
                }
                let supplements = normalize_user_supplements_with_context(
                    ui.drain_user_supplements_with_context(),
                );
                if !supplements.is_empty() {
                    if let Some(next_step) = core.append_user_supplements_with_context_and_audit(
                        supplements,
                        request.audit_file,
                        request.session,
                        &turn_id,
                    ) {
                        step = next_step;
                    }
                    continue;
                }
                // Reminders guide an already-running model turn. Never inject one
                // before the first model request, even when runtime preparation
                // has already crossed a time boundary.
                if rounds > 0 {
                    if let Some(reminder) = progress_reminder.take_due() {
                        core.submit_prompt_component(
                            PromptComponentRole::system(),
                            "turn_progress_reminder",
                            reminder,
                            "turn_runtime",
                        );
                        step = CoreStep::NeedModel {
                            prompt: core.build_next_prompt(),
                            rounds_remaining: core.remaining_rounds(),
                        };
                        continue;
                    }
                    let active_elapsed = start.elapsed().saturating_sub(user_wait_this_turn);
                    let time_reminders = reminders.take_due_time(active_elapsed);
                    if !time_reminders.is_empty() {
                        for reminder in time_reminders {
                            core.submit_prompt_component(
                                PromptComponentRole::system(),
                                "turn_time_reminder",
                                reminder,
                                "turn_runtime",
                            );
                        }
                        step = CoreStep::NeedModel {
                            prompt: core.build_next_prompt(),
                            rounds_remaining: core.remaining_rounds(),
                        };
                        continue;
                    }
                    let round_reminders = reminders.take_due_rounds(rounds);
                    if !round_reminders.is_empty() {
                        for reminder in round_reminders {
                            core.submit_prompt_component(
                                PromptComponentRole::system(),
                                "turn_round_reminder",
                                reminder,
                                "turn_runtime",
                            );
                        }
                        step = CoreStep::NeedModel {
                            prompt: core.build_next_prompt(),
                            rounds_remaining: core.remaining_rounds(),
                        };
                        continue;
                    }
                }
                rounds += 1;
                let mut action_runtime = TurnActionRuntime::new(ui);
                let prompt =
                    core.build_model_request_prompt_with_runtime(prompt, &mut action_runtime);
                let interaction_request = core.model_interaction_request(prompt);
                let api_payload =
                    crate::prepare_model_interaction_http_request(config, &interaction_request)
                        .model_request
                        .body;
                publish_turn_projection(
                    ui,
                    turn_projection.set_activity(TurnActivity::WaitingModel { round: rounds }),
                );
                ui.on_model_api_request(rounds, &interaction_request, &api_payload);
                match call_model_with_system_retries(
                    model_client,
                    config,
                    &interaction_request,
                    request.audit_file,
                    ui,
                    &mut profiler,
                    request.session,
                    &turn_id,
                ) {
                    Ok(response) => {
                        publish_turn_projection(
                            ui,
                            turn_projection.set_activity(TurnActivity::Running),
                        );
                        model_wait_this_turn = model_wait_this_turn.saturating_add(
                            response.model_wait.saturating_add(response.retry_wait),
                        );
                        if take_cancel_request(ui, &mut turn_projection) {
                            break cancelled_turn_parts();
                        }
                        latest_usage = Some(response.response.usage.clone());
                        if !core.should_suppress_model_response(&response.response) {
                            ui.on_model_interaction_response(rounds, &response.response);
                        }
                        let continue_supplements_after_final_answer =
                            ui.continue_supplements_after_final_answer();
                        let mut action_runtime = TurnActionRuntime::new(ui);
                        step = core.apply_model_response_with_repair_audit_and_runtime(
                            response.response,
                            request.audit_file,
                            request.session,
                            &turn_id,
                            &mut action_runtime,
                        );
                        if let Some((has_tool_call, has_free_talk)) =
                            action_runtime.take_model_response_progress()
                        {
                            progress_reminder.observe(has_tool_call, has_free_talk);
                        }
                        user_wait_this_turn =
                            user_wait_this_turn.saturating_add(action_runtime.user_wait());
                        if !is_terminal_stop(&step)
                            && (!matches!(step, CoreStep::Final(_))
                                || continue_supplements_after_final_answer)
                        {
                            let mut all_supplements = action_runtime
                                .take_pending_supplements()
                                .into_iter()
                                .map(UserSupplement::from)
                                .collect::<Vec<_>>();
                            all_supplements.extend(normalize_user_supplements_with_context(
                                ui.drain_user_supplements_with_context(),
                            ));
                            if !all_supplements.is_empty() {
                                if let Some(next_step) = core
                                    .append_user_supplements_with_context_and_audit(
                                        all_supplements,
                                        request.audit_file,
                                        request.session,
                                        &turn_id,
                                    )
                                {
                                    step = next_step;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        publish_turn_projection(
                            ui,
                            turn_projection.set_activity(TurnActivity::Running),
                        );
                        if take_cancel_request(ui, &mut turn_projection) {
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
                publish_turn_projection(
                    ui,
                    turn_projection.set_activity(TurnActivity::WaitingUser),
                );
                ui.pause_for_user_decision();
                let user_wait_start = Instant::now();
                let approved = ui
                    .request_host_decision_topic(
                        request.session,
                        HostDecisionRequest::UserApproval(approval.clone()),
                    )
                    .as_bool();
                user_wait_this_turn = user_wait_this_turn.saturating_add(user_wait_start.elapsed());
                publish_turn_projection(ui, turn_projection.set_activity(TurnActivity::Running));
                if take_cancel_request(ui, &mut turn_projection) {
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
                let mut action_runtime = TurnActionRuntime::new(ui);
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
                if !is_terminal_stop(&step) {
                    let mut all_supplements = action_runtime
                        .take_pending_supplements()
                        .into_iter()
                        .map(UserSupplement::from)
                        .collect::<Vec<_>>();
                    all_supplements.extend(normalize_user_supplements_with_context(
                        ui.drain_user_supplements_with_context(),
                    ));
                    if !all_supplements.is_empty() {
                        if let Some(next_step) = core
                            .append_user_supplements_with_context_and_audit(
                                all_supplements,
                                request.audit_file,
                                request.session,
                                &turn_id,
                            )
                        {
                            step = next_step;
                        }
                    }
                }
                ui.resume_after_user_decision();
            }
            CoreStep::RoundLimitReached { max_rounds } => {
                let decision_request = RoundLimitDecisionRequest::new(max_rounds);
                publish_turn_projection(
                    ui,
                    turn_projection.set_activity(TurnActivity::WaitingUser),
                );
                ui.pause_for_user_decision();
                let user_wait_start = Instant::now();
                let should_continue = ui
                    .request_host_decision_topic(
                        request.session,
                        HostDecisionRequest::RoundLimitContinue(decision_request),
                    )
                    .as_bool();
                user_wait_this_turn = user_wait_this_turn.saturating_add(user_wait_start.elapsed());
                publish_turn_projection(ui, turn_projection.set_activity(TurnActivity::Running));
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
                if ui.continue_supplements_after_final_answer() {
                    let supplements = normalize_user_supplements_with_context(
                        ui.drain_user_supplements_with_context(),
                    );
                    if !supplements.is_empty() {
                        if let Some(next_step) = core
                            .append_user_supplements_with_context_and_audit(
                                supplements,
                                request.audit_file,
                                request.session,
                                &turn_id,
                            )
                        {
                            step = next_step;
                            continue;
                        }
                    }
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
    if outcome.stop_reason == Some(TurnStopReason::CancelledByUser) {
        // Stop performs an immediate best-effort resource sweep. Repeat it at
        // the authoritative turn boundary to catch a background job whose
        // registration raced the first sweep.
        core.cancel_background_resources_for_session(request.session);
        core.mark_user_interrupted_work();
    }
    let mut action_runtime = TurnActionRuntime::new(ui);
    outcome = outcome.with_running_jobs(core.refresh_running_shell_jobs_for_session_with_runtime(
        request.session,
        Some(&mut action_runtime),
    ));
    if let Some(profiler) = profiler {
        profiler.record_turn(elapsed, model_wait_this_turn);
    }
    core.record_turn_final_audit(request.audit_file, request.session, &turn_id, &outcome);
    publish_turn_projection(ui, turn_projection.close_input());
    let projection_outcome = projection_outcome_from_turn_outcome(&outcome);
    publish_turn_projection(ui, turn_projection.finish(projection_outcome));
    outcome
}

fn publish_turn_projection(ui: &mut dyn TurnUi, projection: Option<TurnProjection>) {
    if let Some(projection) = projection {
        ui.on_turn_projection(&projection);
    }
}

fn take_cancel_request(ui: &mut dyn TurnUi, turn_projection: &mut TurnProjectionState) -> bool {
    if !ui.take_cancel_request() {
        return false;
    }
    publish_turn_projection(ui, turn_projection.request_stop());
    true
}

fn projection_outcome_from_turn_outcome(outcome: &TurnOutcome) -> TurnProjectionOutcome {
    match outcome.stop_reason {
        None => TurnProjectionOutcome::Completed,
        Some(TurnStopReason::CancelledByUser) => TurnProjectionOutcome::Cancelled,
        Some(TurnStopReason::ModelError) => TurnProjectionOutcome::Failed {
            code: "model_error".to_string(),
        },
        Some(TurnStopReason::ProtocolRepairFailed) => TurnProjectionOutcome::Failed {
            code: "protocol_repair_failed".to_string(),
        },
        Some(TurnStopReason::OutputLimitStoppedByUser) => TurnProjectionOutcome::Interrupted {
            code: "output_limit_stopped_by_user".to_string(),
        },
        Some(TurnStopReason::RoundLimitReached) => TurnProjectionOutcome::Interrupted {
            code: "round_limit_reached".to_string(),
        },
    }
}

fn is_terminal_stop(step: &CoreStep) -> bool {
    matches!(step, CoreStep::Final(turn) if turn.stop_summary.is_some())
}

struct TurnActionRuntime<'a> {
    ui: &'a mut dyn TurnUi,
    pending_supplements: Vec<String>,
    user_wait: Duration,
    model_response_progress: Option<(bool, bool)>,
}

impl<'a> TurnActionRuntime<'a> {
    fn new(ui: &'a mut dyn TurnUi) -> Self {
        Self {
            ui,
            pending_supplements: Vec::new(),
            user_wait: Duration::ZERO,
            model_response_progress: None,
        }
    }

    fn take_pending_supplements(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_supplements)
    }

    fn user_wait(&self) -> Duration {
        self.user_wait
    }

    fn take_model_response_progress(&mut self) -> Option<(bool, bool)> {
        self.model_response_progress.take()
    }
}

impl ActionRuntime for TurnActionRuntime<'_> {
    fn should_cancel(&mut self) -> bool {
        self.ui.is_cancel_requested()
    }

    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        self.ui.on_core_topic_events(events);
    }

    fn on_model_response_parsed(
        &mut self,
        tool_count: usize,
        has_free_talk: bool,
        has_tool_call: bool,
    ) {
        self.model_response_progress = Some((has_tool_call, has_free_talk));
        self.ui.on_model_response_parsed(tool_count);
    }

    fn on_long_running_command(
        &mut self,
        _status: &LongRunningCommandStatus,
    ) -> LongRunningCommandDecision {
        LongRunningCommandDecision::Continue
    }
}

#[allow(clippy::too_many_arguments)]
fn call_model_with_system_retries(
    model_client: &mut dyn ModelClient,
    config: &ModelServiceConfig,
    request: &ModelInteractionRequest,
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
        let result = model_client.call_model_interaction(config, request, audit_file, &mut || {
            ui.is_cancel_requested()
        });
        let model_wait = model_wait_start.elapsed();
        total_model_wait = total_model_wait.saturating_add(model_wait);
        match result {
            Ok(response) => {
                ui.on_model_request_completed(model_wait);
                if let Some(profiler) = profiler.as_deref_mut() {
                    profiler.record_model_wait(&response.model_name, &response.usage, model_wait);
                }
                return Ok(ModelCallOutcome {
                    response,
                    model_wait: total_model_wait,
                    retry_wait: total_retry_wait,
                });
            }
            Err(err) => {
                if let Some(profiler) = profiler.as_deref_mut() {
                    profiler.record_model_wait(&config.model, &UsageStats::zero(), model_wait);
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
    Err("model_network_error: retry loop exhausted".to_string())
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
