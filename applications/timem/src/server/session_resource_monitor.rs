//! Session resource observation.
//!
//! Samples the managed job scope of every live Session and turns kernel
//! accounting into a small set of runtime observations. A busy Session gets a
//! runtime supplement that rides the next model request; an idle Session gets
//! a runtime turn of its own. Sampling is read-only and bounded, and every
//! alert needs either a sustained window or a monotonic kernel counter, so a
//! single noisy sample can never page the model. `memory.high` intentionally
//! produces no notice: usage may hover around the soft bound, and the kernel
//! already reclaims/throttles there. Only a real OOM kill is injected.
//!
//! OOM attribution is evidence based: the kernel records the kill on the Job
//! leaf that contained the victim and propagates the counter to the Session
//! scope, so the monitor can name the exact managed job. When a kill happens,
//! the Session with the largest current memory share also receives a
//! dominant-share notice; a shared cap that no single Session crossed is never
//! blamed on an arbitrary Session.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use agent_core::resource_caps::SessionResourceUsage;

use super::{
    primary_worker_handle, session_has_active_turn, submit_turn_with_selected_attachments_and_kind,
    AppState,
};

/// History/entry kind for a runtime observation that started its own turn.
pub(super) const RESOURCE_OBSERVATION_ENTRY_KIND: &str = "resource_observation";

const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const CPU_HIGH_WINDOW: Duration = Duration::from_secs(40);
const CPU_RESET_WINDOW: Duration = Duration::from_secs(6);
const CPU_HIGH_RATIO: f64 = 0.9;
const CPU_LOW_RATIO: f64 = 0.6;
const MAX_NAMED_OOM_JOBS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ResourceObservation {
    CpuHigh {
        cores: f64,
        quota_cores: Option<f64>,
        memory_current_bytes: Option<u64>,
    },
    OomKilled {
        killed_jobs: Vec<String>,
        killed_job_total: usize,
        peak_bytes: Option<u64>,
        high_bytes: Option<u64>,
    },
    OomDominantShare {
        top_bytes: u64,
        total_bytes: u64,
    },
}

impl ResourceObservation {
    fn kind(&self) -> &'static str {
        match self {
            Self::CpuHigh { .. } => "cpu_high",
            Self::OomKilled { .. } => "oom_killed",
            Self::OomDominantShare { .. } => "oom_dominant_share",
        }
    }

    fn log_detail(&self) -> serde_json::Value {
        match self {
            Self::CpuHigh {
                cores,
                quota_cores,
                memory_current_bytes,
            } => serde_json::json!({
                "cores": cores,
                "quota_cores": quota_cores,
                "memory_current_bytes": memory_current_bytes,
            }),
            Self::OomKilled {
                killed_jobs,
                killed_job_total,
                peak_bytes,
                high_bytes,
            } => serde_json::json!({
                "killed_jobs": killed_jobs,
                "killed_job_total": killed_job_total,
                "peak_bytes": peak_bytes,
                "high_bytes": high_bytes,
            }),
            Self::OomDominantShare {
                top_bytes,
                total_bytes,
            } => serde_json::json!({
                "top_bytes": top_bytes,
                "total_bytes": total_bytes,
            }),
        }
    }
}

#[derive(Default)]
pub(super) struct SessionObservationState {
    last_sample_at: Option<Instant>,
    last_cpu_usage_usec: Option<u64>,
    last_memory_current_bytes: Option<u64>,
    last_session_oom_kill_count: Option<u64>,
    last_job_oom_kills: HashMap<String, u64>,
    cpu_high_for: Duration,
    cpu_low_for: Duration,
    cpu_alerted: bool,
}

impl SessionObservationState {
    /// Advance the state machine by one sample. `dt` is the wall time since
    /// the previous sample of this Session. The first sample only establishes
    /// baselines for cumulative counters.
    pub(super) fn observe(
        &mut self,
        usage: &SessionResourceUsage,
        dt: Duration,
        host_cores: Option<u32>,
    ) -> Vec<ResourceObservation> {
        let mut observations = Vec::new();

        let cpu_cores = match (self.last_cpu_usage_usec, usage.cpu_usage_usec) {
            (Some(previous), Some(current)) if current >= previous && !dt.is_zero() => {
                Some((current - previous) as f64 / dt.as_secs_f64() / 1_000_000.0)
            }
            // A counter that moved backwards is a replaced scope, not load.
            _ => None,
        };
        self.last_cpu_usage_usec = usage.cpu_usage_usec;

        if let Some(cores) = cpu_cores {
            let quota = usage
                .cpu_quota_cores
                .or_else(|| host_cores.map(f64::from))
                .filter(|quota| *quota > 0.0);
            if let Some(quota) = quota {
                let high = quota * CPU_HIGH_RATIO;
                let low = quota * CPU_LOW_RATIO;
                if cores >= high {
                    self.cpu_high_for += dt;
                    self.cpu_low_for = Duration::ZERO;
                    if !self.cpu_alerted && self.cpu_high_for >= CPU_HIGH_WINDOW {
                        observations.push(ResourceObservation::CpuHigh {
                            cores,
                            quota_cores: usage.cpu_quota_cores,
                            memory_current_bytes: usage.memory_current_bytes,
                        });
                        self.cpu_alerted = true;
                    }
                } else {
                    self.cpu_high_for = Duration::ZERO;
                    if cores <= low {
                        self.cpu_low_for += dt;
                    } else {
                        self.cpu_low_for = Duration::ZERO;
                    }
                    if self.cpu_alerted && self.cpu_low_for >= CPU_RESET_WINDOW {
                        self.cpu_alerted = false;
                        self.cpu_low_for = Duration::ZERO;
                    }
                }
            }
        }

        // Only counters that moved since the previous sample belong to this
        // event. The first sample of a Session baselines every counter, so a
        // host restart can never replay historical OOM kills into a prompt.
        let first_sample = self.last_session_oom_kill_count.is_none();
        let mut killed_jobs = Vec::new();
        let mut killed_job_total = 0usize;
        for job in &usage.jobs {
            match self.last_job_oom_kills.get(&job.scope_name).copied() {
                Some(previous) if job.oom_kill_count > previous => {
                    killed_job_total += (job.oom_kill_count - previous) as usize;
                    if killed_jobs.len() < MAX_NAMED_OOM_JOBS {
                        killed_jobs.push(job.scope_name.clone());
                    }
                }
                // A fresh Job leaf starts at zero; a positive count here means
                // it was killed before the monitor's first sample of that job.
                None if job.oom_kill_count > 0 => {
                    killed_job_total += job.oom_kill_count as usize;
                    if killed_jobs.len() < MAX_NAMED_OOM_JOBS {
                        killed_jobs.push(job.scope_name.clone());
                    }
                }
                _ => {}
            }
        }
        let session_oom_delta = self
            .last_session_oom_kill_count
            .map(|previous| usage.oom_kill_count.saturating_sub(previous))
            .unwrap_or(0);
        let job_oom_delta = usage
            .jobs
            .iter()
            .map(|job| {
                job.oom_kill_count.saturating_sub(
                    self.last_job_oom_kills
                        .get(&job.scope_name)
                        .copied()
                        .unwrap_or(0),
                )
            })
            .sum::<u64>();
        let unexplained_oom = session_oom_delta > job_oom_delta;

        self.last_job_oom_kills = usage
            .jobs
            .iter()
            .map(|job| (job.scope_name.clone(), job.oom_kill_count))
            .collect();
        self.last_session_oom_kill_count = Some(usage.oom_kill_count);

        if !first_sample && (killed_job_total > 0 || unexplained_oom) {
            observations.push(ResourceObservation::OomKilled {
                killed_jobs,
                killed_job_total,
                // The post-kill charge has already dropped. Prefer the
                // sample immediately before the kill; a lifetime peak may
                // predate this event by a long time.
                peak_bytes: self
                    .last_memory_current_bytes
                    .or(usage.memory_peak_bytes)
                    .or(usage.memory_current_bytes),
                high_bytes: usage.memory_high_bytes.or(usage.memory_max_bytes),
            });
        }
        self.last_memory_current_bytes = usage.memory_current_bytes;

        observations
    }
}

fn format_gib(bytes: u64) -> String {
    format!("{:.1}G", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

fn memory_fragment(bytes: Option<u64>, total_bytes: Option<u64>) -> String {
    match (bytes, total_bytes) {
        (Some(bytes), Some(total)) if total > 0 => format!(
            "{} ({:.0}% of total memory)",
            format_gib(bytes),
            bytes as f64 / total as f64 * 100.0
        ),
        (Some(bytes), _) => format_gib(bytes),
        (None, _) => "memory stats unavailable".to_string(),
    }
}

fn format_killed_jobs(killed_jobs: &[String], killed_job_total: usize) -> String {
    if killed_jobs.is_empty() {
        return String::new();
    }
    let shown = killed_jobs.join(", ");
    let remaining = killed_job_total.saturating_sub(killed_jobs.len());
    if remaining > 0 {
        let noun = if remaining == 1 { "job" } else { "jobs" };
        format!("{shown} and {remaining} more {noun}")
    } else {
        shown
    }
}

/// Sentence appended when the OOM victim is also the dominant memory user, so
/// the detailed notice already answers "who else is large".
pub(super) fn dominant_share_suffix(share: f64) -> String {
    format!(
        " This session now has the largest memory usage of all sessions ({share:.0}% of the total across sessions)."
    )
}

pub(super) fn render_observation(
    observation: &ResourceObservation,
    total_memory_bytes: Option<u64>,
) -> String {
    match observation {
        ResourceObservation::CpuHigh {
            memory_current_bytes,
            ..
        } => {
            let memory = memory_fragment(*memory_current_bytes, total_memory_bytes);
            if memory_current_bytes.is_none() {
                "Runtime observation: task CPU load in this session has stayed high for more than 40s."
                    .to_string()
            } else {
                format!(
                    "Runtime observation: task CPU load in this session has stayed high for more than 40s; memory usage {memory}."
                )
            }
        }
        ResourceObservation::OomKilled {
            killed_jobs,
            killed_job_total,
            peak_bytes,
            high_bytes,
        } => {
            let memory = memory_fragment(*peak_bytes, total_memory_bytes);
            let jobs = format_killed_jobs(killed_jobs, *killed_job_total);
            let (subject, verb) = if jobs.is_empty() {
                ("Tasks in this session".to_string(), "were")
            } else if *killed_job_total <= 1 {
                (format!("{jobs} in this session"), "was")
            } else {
                (format!("{jobs} in this session"), "were")
            };
            // Only claim the quota was exceeded when the peak actually proves
            // it. A global OOM kill can hit a scope below its soft bound.
            match (peak_bytes, high_bytes) {
                (Some(peak), Some(high)) if peak >= high => format!(
                    "Runtime observation: {subject} {verb} OOM-killed; total task memory usage {memory}, above the quota threshold."
                ),
                _ => format!(
                    "Runtime observation: {subject} {verb} OOM-killed; total task memory usage {memory}."
                ),
            }
        }
        ResourceObservation::OomDominantShare {
            top_bytes,
            total_bytes,
        } => {
            let share = if *total_bytes > 0 {
                *top_bytes as f64 / *total_bytes as f64 * 100.0
            } else {
                0.0
            };
            format!(
                "Runtime observation: an OOM kill just happened; this session now has the largest memory usage of all sessions ({}, {share:.0}% of the total across sessions).",
                format_gib(*top_bytes)
            )
        }
    }
}

/// Deliver one observation. A live turn receives it as a runtime supplement so
/// the model sees it on the next request; an idle Session starts a runtime
/// turn. A supplement rejected while the turn closes falls back to the idle
/// path only when the Session is genuinely idle again.
fn deliver_observation(state: &AppState, session_id: &str, text: String) -> Result<(), String> {
    if session_has_active_turn(state, session_id)? {
        let worker = primary_worker_handle(state, session_id)?;
        let accepted = worker.try_add_system_supplement_with_context_and_command_id_after(
            text.clone(),
            None,
            None,
            || Ok(()),
        )?;
        if accepted {
            return Ok(());
        }
        if session_has_active_turn(state, session_id)? {
            return Err("active_turn_rejected_system_supplement".to_string());
        }
    }
    submit_turn_with_selected_attachments_and_kind(
        state,
        session_id,
        text,
        None,
        None,
        Vec::new(),
        RESOURCE_OBSERVATION_ENTRY_KIND,
    )?;
    Ok(())
}

fn publish_observation(
    state: &AppState,
    session_id: &str,
    observation: &ResourceObservation,
    total_memory_bytes: Option<u64>,
) {
    let text = render_observation(observation, total_memory_bytes);
    state.runtime_log.record(
        "session_resource_observation",
        serde_json::json!({
            "session_id": session_id,
            "kind": observation.kind(),
            "detail": observation.log_detail(),
            "text": text,
        }),
    );
    if let Err(error) = deliver_observation(state, session_id, text) {
        state.runtime_log.record(
            "session_resource_observation_delivery_failed",
            serde_json::json!({
                "session_id": session_id,
                "error": error,
            }),
        );
    }
}

/// Session with the largest current memory charge and the total charge that
/// share is measured against. Ties keep the first Session in iteration order.
pub(super) fn dominant_memory_session(
    samples: &[(String, Option<u64>)],
) -> Option<(String, u64, u64)> {
    let mut total = 0u64;
    let mut top: Option<(&str, u64)> = None;
    for (session_id, bytes) in samples {
        let Some(bytes) = bytes else {
            continue;
        };
        total = total.saturating_add(*bytes);
        if top.is_none_or(|(_, current)| *bytes > current) {
            top = Some((session_id.as_str(), *bytes));
        }
    }
    let (session_id, bytes) = top?;
    (total > 0).then(|| (session_id.to_string(), bytes, total))
}

pub(super) fn spawn_session_resource_monitor(state: AppState) {
    tokio::spawn(async move {
        let mut states: HashMap<String, SessionObservationState> = HashMap::new();
        loop {
            sample_sessions(&state, &mut states).await;
            tokio::time::sleep(SAMPLE_INTERVAL).await;
        }
    });
}

struct SessionSample {
    session_id: String,
    usage: SessionResourceUsage,
    observations: Vec<ResourceObservation>,
}

/// Async wrapper: cgroup accounting is a burst of synchronous file reads.
/// Move the whole sweep to the blocking pool so a slow filesystem or many
/// Sessions cannot stall the Tokio timers that drive the rest of the host.
async fn sample_sessions(state: &AppState, states: &mut HashMap<String, SessionObservationState>) {
    if state
        .session_restore_in_progress
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let session_ids: Vec<String> = {
        let Ok(sessions) = state.sessions.lock() else {
            return;
        };
        sessions
            .values()
            .filter(|session| !session.workers.is_empty())
            .map(|session| session.session_id.clone())
            .collect()
    };
    let alive: HashSet<&str> = session_ids.iter().map(String::as_str).collect();
    states.retain(|session_id, _| alive.contains(session_id.as_str()));
    let ids = session_ids.clone();
    let Ok((host_cores, total_memory_bytes, usage_by_session)) =
        tokio::task::spawn_blocking(move || {
            let host_cores = agent_core::resource_caps::system_cpu_cores();
            let total_memory_bytes = agent_core::resource_caps::system_mem_total_bytes();
            let usages = ids
                .into_iter()
                .map(|session_id| {
                    let usage = agent_core::resource_caps::session_resource_usage(&session_id);
                    (session_id, usage)
                })
                .collect::<Vec<_>>();
            (host_cores, total_memory_bytes, usages)
        })
        .await
    else {
        return;
    };
    let mut samples = Vec::with_capacity(usage_by_session.len());
    for (session_id, usage) in usage_by_session {
        let usage = match usage {
            Ok(Some(usage)) => usage,
            // No scope yet, or the platform cannot account for it: stay quiet
            // instead of paging the model on every sample.
            Ok(None) | Err(_) => {
                states.remove(&session_id);
                continue;
            }
        };
        let now = Instant::now();
        let observation_state = states.entry(session_id.clone()).or_default();
        let dt = observation_state
            .last_sample_at
            .map(|last| now.saturating_duration_since(last))
            .unwrap_or_default();
        observation_state.last_sample_at = Some(now);
        let observations = observation_state.observe(&usage, dt, host_cores);
        samples.push(SessionSample {
            session_id,
            usage,
            observations,
        });
    }

    let oom_happened = samples.iter().any(|sample| {
        sample
            .observations
            .iter()
            .any(|observation| matches!(observation, ResourceObservation::OomKilled { .. }))
    });
    let dominant = oom_happened.then(|| {
        let memory: Vec<(String, Option<u64>)> = samples
            .iter()
            .map(|sample| (sample.session_id.clone(), sample.usage.memory_current_bytes))
            .collect();
        dominant_memory_session(&memory)
    });
    let dominant = dominant.flatten();

    for sample in &samples {
        for observation in &sample.observations {
            let mut text = render_observation(observation, total_memory_bytes);
            if matches!(observation, ResourceObservation::OomKilled { .. }) {
                if let Some((top_session, top_bytes, total)) = dominant.as_ref() {
                    if top_session == &sample.session_id {
                        let share = *top_bytes as f64 / *total as f64 * 100.0;
                        text.push_str(&dominant_share_suffix(share));
                    }
                }
            }
            state.runtime_log.record(
                "session_resource_observation",
                serde_json::json!({
                    "session_id": sample.session_id,
                    "kind": observation.kind(),
                    "detail": observation.log_detail(),
                    "text": text,
                }),
            );
            if let Err(error) = deliver_observation(state, &sample.session_id, text) {
                state.runtime_log.record(
                    "session_resource_observation_delivery_failed",
                    serde_json::json!({
                        "session_id": sample.session_id,
                        "error": error,
                    }),
                );
            }
        }
    }

    if let Some((top_session, top_bytes, total)) = dominant {
        let top_is_victim = samples.iter().any(|sample| {
            sample.session_id == top_session
                && sample
                    .observations
                    .iter()
                    .any(|observation| matches!(observation, ResourceObservation::OomKilled { .. }))
        });
        if !top_is_victim {
            let observation = ResourceObservation::OomDominantShare {
                top_bytes,
                total_bytes: total,
            };
            publish_observation(state, &top_session, &observation, total_memory_bytes);
        }
    }
}
