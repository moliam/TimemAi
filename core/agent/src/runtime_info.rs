//! RuntimeInfo: the model's observation aggregator. Modules register
//! reporters (callbacks); before each model request the registry collects
//! their important state and, only when some reporter has something to say,
//! renders a single `### RUNTIME_INFO` section that rides along with the
//! next request. Unimportant facts (e.g. a job that finished normally this
//! round and already has an action result) produce nothing.

use std::{
    fmt::Write as _,
    time::{Duration, Instant},
};

/// Snapshot of agent state handed to every reporter on collection.
#[derive(Default, Clone)]
pub struct RuntimeInfoInputs {
    /// Jobs still running (the STILL RUNNING source).
    pub running: Vec<RunningJobSnapshot>,
    /// Jobs that exited since the last request, with their exit status.
    pub updates: Vec<JobExitSnapshot>,
    /// Platform-native aggregate observation point for the current Agent's
    /// Session-owned process Jobs. AgentCore supplies this only for a one-shot
    /// startup/restart or post-compaction reminder.
    pub process_scope: Option<String>,
    /// Previous Runtime scopes for this Session that still contain live work.
    pub stale_process_scopes: Vec<StaleProcessScopeSnapshot>,
    /// Live descendants watched by the Runtime fallback chain after their
    /// original supervision chain ended.
    pub fallback_processes: Vec<FallbackProcessSnapshot>,
    /// Delta-based disk pressure notice produced by DiskPressureTracker at
    /// its latest sampled observation point, if it triggered.
    pub disk_pressure_notice: Option<String>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // `path` is diagnostic context for future reporters; usage math reads the byte fields.
pub struct FilesystemUsage {
    pub path: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Clone)]
#[allow(dead_code)] // Fields carry raw state for registered reporters; not all read today.
pub struct RunningJobSnapshot {
    pub pid: u32,
    pub tool_call_id: String,
    pub command: String,
    pub cwd: String,
    pub created_at_ms: i64,
    /// Elapsed runtime captured at the request observation point.
    pub elapsed_ms: i64,
    /// Concise platform-native observation location, if one is available.
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackProcessSnapshot {
    pub pid: u32,
    pub process_name: String,
    pub zombie: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleProcessScopeSnapshot {
    pub observation_note: String,
    pub owner_pid: u32,
}

#[derive(Clone)]
#[allow(dead_code)] // Fields carry raw state for registered reporters; not all read today.
pub struct JobExitSnapshot {
    pub pid: u32,
    pub tool_call_id: String,
    pub command: String,
    pub elapsed_ms: i64,
    pub status: String,
}

/// One registered module reporter. `name` becomes the `#### <name>` field
/// heading inside RUNTIME_INFO; the callback returns None when the module
/// has nothing important to report this request.
pub struct RuntimeInfoReporter {
    pub name: &'static str,
    pub report: fn(&RuntimeInfoInputs) -> Option<String>,
}

/// Registry of module reporters. Rendering is deterministic: reporters run
/// in registration order; the section is built only if at least one
/// reporter returned content.
#[derive(Default)]
pub struct RuntimeInfoRegistry {
    reporters: Vec<RuntimeInfoReporter>,
}

/// The default registry with the built-in module reporters. New modules
/// opt into model visibility by registering here.
pub fn default_registry() -> RuntimeInfoRegistry {
    let mut registry = RuntimeInfoRegistry::new();
    registry.register(RuntimeInfoReporter {
        name: "jobmanager",
        report: jobmanager_report,
    });
    registry.register(RuntimeInfoReporter {
        name: "sysstat",
        report: sysstat_report,
    });
    registry
}

impl RuntimeInfoRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, reporter: RuntimeInfoReporter) {
        self.reporters.push(reporter);
    }

    /// Collect from all reporters and render the `### RUNTIME_INFO`
    /// section, or None when no module has important state.
    pub fn render(&self, inputs: &RuntimeInfoInputs) -> Option<String> {
        let mut fields = Vec::new();
        for reporter in &self.reporters {
            if let Some(text) = (reporter.report)(inputs) {
                let text = text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                fields.push(format!("#### {}\n{}", reporter.name, text));
            }
        }
        if fields.is_empty() {
            return None;
        }
        let mut out = String::from("### RUNTIME_INFO\nruntime state needing model awareness:\n");
        for field in fields {
            let _ = writeln!(out, "\n{}", field);
        }
        Some(out)
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// jobmanager reporter: STILL RUNNING table, job exits with orphan hints,
/// and escaped processes. Returns None when all jobs finished cleanly and
/// nothing escaped (normal results already reach the model via action
/// results).
pub fn jobmanager_report(inputs: &RuntimeInfoInputs) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(scope) = inputs.process_scope.as_deref() {
        parts.push(format!(
            "current agent/session aggregate observation path: {scope}"
        ));
    }

    if !inputs.running.is_empty() {
        let mut table = String::from(
            "still running jobs:\n\n| pid | elapsed | created by tool_call id | command | notes |\n|---:|---:|---|---|---|",
        );
        for job in &inputs.running {
            let call_id = if job.tool_call_id.trim().is_empty() {
                "unknown_tool_call"
            } else {
                &job.tool_call_id
            };
            let command = job.command.chars().take(500).collect::<String>();
            let command = command.replace('|', "\\|").replace('\n', " ");
            let elapsed = crate::format_time_elapsed_hms(job.elapsed_ms.max(0) as u64);
            let notes = job.notes.replace('|', "\\|").replace('\n', " ");
            let _ = writeln!(
                table,
                "\n| {} | `{}` | `{}` | `{}` | {} |",
                job.pid, elapsed, call_id, command, notes
            );
        }
        if inputs
            .running
            .iter()
            .any(|job| job.elapsed_ms > 3 * 60 * 1000)
        {
            table.push_str("\n\nneed to check whether long running job is making progress");
        }
        parts.push(table);
    }

    if !inputs.fallback_processes.is_empty() {
        let mut table = String::from(
            "unowned child processes:\n\n| pid | process | state | model decision |\n|---:|---|---|---|",
        );
        for process in &inputs.fallback_processes {
            let name = process.process_name.replace('|', "\\|").replace('\n', " ");
            let state = if process.zombie { "zombie" } else { "active" };
            let decision = if process.zombie {
                "inspect parent/reaper health; do not signal an already-dead process"
            } else {
                "inspect purpose/progress, then keep observing or terminate"
            };
            let _ = writeln!(
                table,
                "\n| {} | `{}` | `{}` | {} |",
                process.pid, name, state, decision
            );
        }
        table.push_str(
            "\n\nThese child processes are still running, but no task ownership record is available. Inspect their purpose and progress, then keep observing or terminate them. Do not infer ownership from timing alone.",
        );
        parts.push(table);
    }

    if !inputs.stale_process_scopes.is_empty() {
        let mut table = String::from(
            "stale process scopes from previous Runtime owners still contain live work:

| previous owner pid | observation point | model decision |
|---:|---|---|",
        );
        for scope in &inputs.stale_process_scopes {
            let note = scope
                .observation_note
                .replace('|', "\\|")
                .replace('\n', " ");
            let _ = writeln!(
                table,
                "\n| {} | `{}` | inspect members, then preserve or terminate explicitly |",
                scope.owner_pid, note
            );
        }
        table.push_str(
            "\n\nThe previous Runtime owner identity no longer matches. These processes were not silently adopted or killed.",
        );
        parts.push(table);
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// sysstat reporter: filesystem pressure and suspicious job kills. Reports
/// only anomalies: disk nearly full (>= 90% used) or a job exit that looks
/// like SIGKILL (possible OOM kill) — cases the model cannot diagnose from
/// the tool output alone because the job's scene is already gone.
pub fn sysstat_report(inputs: &RuntimeInfoInputs) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(notice) = &inputs.disk_pressure_notice {
        parts.push(notice.clone());
    }

    if let Some(killed) = killed_jobs_notice(&inputs.updates) {
        parts.push(killed);
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Facts about kill-looking job exits across the provided snapshots, or
/// None when every exit was normal. Shared by the sysstat reporter and the
/// persistent exit-update path so kills are never missed regardless of
/// which channel the exit lands on.
pub fn killed_jobs_notice(updates: &[JobExitSnapshot]) -> Option<String> {
    let parts: Vec<String> = updates
        .iter()
        .filter(|update| looks_killed(&update.status))
        .map(|update| {
            format!(
                "JOB_KILLED: pid={} cmd=`{}` exited with `{}` — not a normal exit. If the command itself does not print an error, suspect OOM kill (check dmesg/`journalctl -k`) or an external terminator.",
                update.pid,
                update.command.chars().take(200).collect::<String>(),
                update.status
            )
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn looks_killed(status: &str) -> bool {
    // Exit statuses here are formatted strings like "signal: 9 (SIGKILL)".
    status.contains("signal")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_nothing_when_all_modules_silent() {
        let mut registry = RuntimeInfoRegistry::new();
        registry.register(RuntimeInfoReporter {
            name: "jobmanager",
            report: jobmanager_report,
        });
        registry.register(RuntimeInfoReporter {
            name: "sysstat",
            report: sysstat_report,
        });
        let inputs = RuntimeInfoInputs {
            updates: vec![JobExitSnapshot {
                pid: 1,
                tool_call_id: "c1".into(),
                command: "echo ok".into(),
                elapsed_ms: 10,
                status: "exit code: 0".into(),
            }],
            ..Default::default()
        };
        assert!(registry.render(&inputs).is_none());
    }

    #[test]
    fn renders_job_and_sysstat_fields_when_important() {
        let mut registry = RuntimeInfoRegistry::new();
        registry.register(RuntimeInfoReporter {
            name: "jobmanager",
            report: jobmanager_report,
        });
        registry.register(RuntimeInfoReporter {
            name: "sysstat",
            report: sysstat_report,
        });
        let inputs = RuntimeInfoInputs {
            running: vec![RunningJobSnapshot {
                pid: 42,
                tool_call_id: "call_1".into(),
                command: "sleep 100".into(),
                cwd: "/tmp".into(),
                created_at_ms: 0,
                elapsed_ms: 70_000,
                notes: "cgroup: /sys/fs/cgroup/example/job-42".into(),
            }],
            process_scope: Some("cgroup: /sys/fs/cgroup/timem.jobs/runtime-1-2/session-abcd".into()),
            stale_process_scopes: vec![StaleProcessScopeSnapshot {
                observation_note: "cgroup: /sys/fs/cgroup/timem.jobs/runtime-7-8/session-abcd".into(),
                owner_pid: 7,
            }],
            fallback_processes: vec![
                FallbackProcessSnapshot {
                    pid: 77,
                    process_name: "worker-helper".into(),
                    zombie: false,
                },
                FallbackProcessSnapshot {
                    pid: 78,
                    process_name: "dead-helper".into(),
                    zombie: true,
                },
            ],
            disk_pressure_notice: Some(
                "DISK_PRESSURE: total free space across the working disks dropped within the last 15 observation points".into(),
            ),
            updates: vec![JobExitSnapshot {
                pid: 9,
                tool_call_id: "call_9".into(),
                command: "big-build".into(),
                elapsed_ms: 1000,
                status: "signal: 9 (SIGKILL)".into(),
            }],
        };
        let out = registry.render(&inputs).expect("expected RUNTIME_INFO");
        assert!(out.starts_with("### RUNTIME_INFO"), "{out}");
        assert!(out.contains("#### jobmanager"), "{out}");
        assert!(out.contains("#### sysstat"), "{out}");
        assert!(
            out.contains("| 42 | `1m10s` | `call_1` | `sleep 100` |"),
            "{out}"
        );
        assert!(out.contains("unowned child processes"), "{out}");
        assert!(out.contains("`active`"), "{out}");
        assert!(out.contains("worker-helper"), "{out}");
        assert!(out.contains("`zombie`"), "{out}");
        assert!(out.contains("dead-helper"), "{out}");
        assert!(
            out.contains("current agent/session aggregate observation path"),
            "{out}"
        );
        assert!(
            out.contains("stale process scopes from previous Runtime owners"),
            "{out}"
        );
        assert!(!out.contains("UNOWNED_CHILD_UPDATE"), "{out}");
        assert!(!out.contains("reaped"), "{out}");
        assert!(out.contains("DISK_PRESSURE"), "{out}");
        assert!(out.contains("observation points"), "{out}");
        assert!(out.contains("JOB_KILLED"), "{out}");
        assert!(out.contains("SIGKILL"), "{out}");
    }

    #[test]
    fn aggregate_observation_path_is_sufficient_without_running_jobs() {
        let out = jobmanager_report(&RuntimeInfoInputs {
            process_scope: Some(
                "cgroup: /sys/fs/cgroup/timem.jobs/runtime-1-2/session-abcd".into(),
            ),
            ..Default::default()
        })
        .expect("one-shot aggregate path report");
        assert_eq!(
            out,
            "current agent/session aggregate observation path: cgroup: /sys/fs/cgroup/timem.jobs/runtime-1-2/session-abcd"
        );
        assert!(!out.contains("memory.current"));
        assert!(!out.contains("pids.current"));
        assert!(!out.contains("cpu.stat"));
    }

    #[test]
    fn long_running_progress_reminder_requires_more_than_three_minutes() {
        let report_for = |elapsed_ms| {
            jobmanager_report(&RuntimeInfoInputs {
                running: vec![RunningJobSnapshot {
                    pid: 7,
                    tool_call_id: "call_7".into(),
                    command: "work".into(),
                    cwd: "/tmp".into(),
                    created_at_ms: 0,
                    elapsed_ms,
                    notes: String::new(),
                }],
                ..Default::default()
            })
            .expect("running job report")
        };
        let reminder = "need to check whether long running job is making progress";

        assert!(!report_for(3 * 60 * 1000 - 1).contains(reminder));
        assert!(!report_for(3 * 60 * 1000).contains(reminder));
        assert!(report_for(3 * 60 * 1000 + 1).contains(reminder));
    }

    #[test]
    fn running_job_elapsed_time_saturates_at_zero() {
        let inputs = RuntimeInfoInputs {
            running: vec![RunningJobSnapshot {
                pid: 7,
                tool_call_id: "call_7".into(),
                command: "work".into(),
                cwd: "/tmp".into(),
                created_at_ms: 100,
                elapsed_ms: -1,
                notes: String::new(),
            }],
            ..Default::default()
        };
        let out = jobmanager_report(&inputs).expect("running job report");
        assert!(out.contains("| 7 | `0.0s` | `call_7` | `work` |"), "{out}");
    }

    #[test]
    fn healthy_disk_and_normal_exits_are_not_reported() {
        let inputs = RuntimeInfoInputs {
            disk_pressure_notice: None,
            updates: vec![JobExitSnapshot {
                pid: 5,
                tool_call_id: "c".into(),
                command: "true".into(),
                elapsed_ms: 1,
                status: "exit code: 0".into(),
            }],
            ..Default::default()
        };
        assert!(sysstat_report(&inputs).is_none());
    }

    #[test]
    fn empty_reporter_output_is_skipped() {
        let mut registry = RuntimeInfoRegistry::new();
        registry.register(RuntimeInfoReporter {
            name: "silent",
            report: |_| Some("   ".into()),
        });
        let inputs = RuntimeInfoInputs::default();
        assert!(registry.render(&inputs).is_none());
    }
}

/// Triggered disk pressure drop with before/after totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskPressureEvent {
    pub dropped: u64,
    pub base: u64,
    pub new: u64,
}

impl DiskPressureEvent {
    /// Render the model-facing notice, including the per-disk breakdown of
    /// the current sample so the model can see which mount dropped.
    pub fn render(&self, filesystems: &[FilesystemUsage]) -> String {
        let mut out = format!(
            "DISK_PRESSURE: total free space across the working disks dropped by {} ({} -> {}). Free space is being consumed fast; clean up or move data if writes are expected to continue.",
            human_bytes(self.dropped),
            human_bytes(self.base),
            human_bytes(self.new)
        );
        if !filesystems.is_empty() {
            out.push_str("\ncurrent disk info:\n\n| mount | total | free |\n|---|---:|---:|");
            for fs in filesystems {
                let _ = writeln!(
                    out,
                    "\n| `{}` | {} | {} |",
                    fs.path,
                    human_bytes(fs.total_bytes),
                    human_bytes(fs.free_bytes)
                );
            }
        }
        out
    }
}

/// Observation-point-throttled disk pressure tracker.
///
/// An observation point is one completed tool run or one model API request.
/// A sample is taken when either 10 observation points accumulated or
/// 3 minutes elapsed since the previous sample. A successful sample resets
/// both gates and compares total free space with the baseline:
/// - `new > base`: space was reclaimed; refresh `base = new` (no notice).
/// - `new <= base` and free space dropped by more than the threshold: emit
///   a notice and refresh `base = new`; otherwise keep the baseline so slow
///   consumption still accumulates toward the threshold.
#[derive(Debug)]
pub struct DiskPressureTracker {
    observations_since_sample: u32,
    last_sample_at: Instant,
    baseline: Option<u64>,
}

const DISK_SAMPLE_INTERVAL: u32 = 10;
const DISK_SAMPLE_MAX_AGE: Duration = Duration::from_secs(3 * 60);
/// Drop threshold: the smaller of 200MB and 8% of the disk capacity, so
/// large disks stay sensitive (200MB cap) and tiny disks scale down.
const DISK_PRESSURE_DELTA_CAP_BYTES: u64 = 200 * 1024 * 1024;

impl Default for DiskPressureTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskPressureTracker {
    pub fn new() -> Self {
        Self {
            observations_since_sample: 0,
            last_sample_at: Instant::now(),
            baseline: None,
        }
    }

    /// Seed the baseline at runtime startup from the real disk sample so
    /// the first sampling window after a restart is not a blind window.
    /// Does not consume an observation point and never emits a notice.
    pub fn seed_baseline(&mut self, sample: Option<(u64, u64)>) {
        self.seed_baseline_at(sample, Instant::now());
    }

    fn seed_baseline_at(&mut self, sample: Option<(u64, u64)>, now: Instant) {
        if let Some((free, _capacity)) = sample {
            if self.baseline.is_none() {
                self.baseline = Some(free);
                self.observations_since_sample = 0;
                self.last_sample_at = now;
            }
        }
    }

    /// Current baseline value (for tests).
    #[cfg(test)]
    pub fn baseline(&self) -> Option<u64> {
        self.baseline
    }

    /// Observation count since the most recent successful sample (tests).
    #[cfg(test)]
    fn pending_observations(&self) -> u32 {
        self.observations_since_sample
    }

    /// Record one observation point (tool run or API request). A new sample
    /// is evaluated when either 10 observations accumulated or 3 minutes
    /// elapsed since the previous successful sample. A successful sample
    /// resets both gates and starts the next window.
    #[cfg(test)]
    pub fn observe(&mut self, sample: Option<(u64, u64)>) -> Option<DiskPressureEvent> {
        self.observe_with(|| sample)
    }

    /// Record one observation and obtain a filesystem sample only when the
    /// count or time gate is due. This keeps ordinary tool completions and
    /// model requests free of mount enumeration and stat calls.
    pub fn observe_with<F>(&mut self, sample: F) -> Option<DiskPressureEvent>
    where
        F: FnOnce() -> Option<(u64, u64)>,
    {
        self.observe_with_at(sample, Instant::now())
    }

    #[cfg(test)]
    fn observe_at(
        &mut self,
        sample: Option<(u64, u64)>,
        now: Instant,
    ) -> Option<DiskPressureEvent> {
        self.observe_with_at(|| sample, now)
    }

    fn observe_with_at<F>(&mut self, sample: F, now: Instant) -> Option<DiskPressureEvent>
    where
        F: FnOnce() -> Option<(u64, u64)>,
    {
        self.observations_since_sample = self.observations_since_sample.saturating_add(1);
        let count_due = self.observations_since_sample >= DISK_SAMPLE_INTERVAL;
        let time_due = now.saturating_duration_since(self.last_sample_at) >= DISK_SAMPLE_MAX_AGE;
        if !count_due && !time_due {
            return None;
        }
        let (new, capacity) = sample()?;
        self.observations_since_sample = 0;
        self.last_sample_at = now;
        let Some(base) = self.baseline else {
            // First sample only establishes the initial baseline.
            self.baseline = Some(new);
            return None;
        };
        if new > base {
            self.baseline = Some(new);
            return None;
        }
        let dropped = base.saturating_sub(new);
        // Threshold scales with the disk: at most 200MB, at most 8% of
        // the total capacity, whichever is smaller.
        let threshold = (DISK_PRESSURE_DELTA_CAP_BYTES).min(capacity / 100 * 8);
        if dropped > threshold {
            self.baseline = Some(new);
            return Some(DiskPressureEvent { dropped, base, new });
        }
        // Below the threshold: keep the baseline so continued consumption
        // still accumulates toward the next window's comparison.
        None
    }
}

#[cfg(test)]
mod disk_pressure_tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;
    const CAP: u64 = 10 * GB;
    // threshold = min(200MB, 8% * 10GB) = 200MB; the implementation uses
    // integer math capacity / 100 * 8, so mirror it exactly.
    const THRESHOLD: u64 = 200 * 1024 * 1024;

    fn tracker() -> DiskPressureTracker {
        DiskPressureTracker::new()
    }

    fn advance_to_sample(t: &mut DiskPressureTracker, free: u64) -> Option<DiskPressureEvent> {
        let mut event = None;
        for _ in 0..DISK_SAMPLE_INTERVAL {
            event = t.observe(Some((free, CAP)));
        }
        assert_eq!(t.pending_observations(), 0, "sample must reset count gate");
        event
    }

    #[test]
    fn sample_callback_is_lazy_until_a_gate_is_due() {
        let start = Instant::now();
        let mut t = tracker();
        t.seed_baseline_at(Some((5 * GB, CAP)), start);
        let mut sample_calls = 0;
        for step in 1..DISK_SAMPLE_INTERVAL {
            assert!(t
                .observe_with_at(
                    || {
                        sample_calls += 1;
                        Some((5 * GB, CAP))
                    },
                    start + Duration::from_secs(step as u64),
                )
                .is_none());
        }
        assert_eq!(
            sample_calls, 0,
            "sampling must stay lazy before a gate is due"
        );
        assert!(t
            .observe_with_at(
                || {
                    sample_calls += 1;
                    Some((5 * GB, CAP))
                },
                start + Duration::from_secs(DISK_SAMPLE_INTERVAL as u64),
            )
            .is_none());
        assert_eq!(sample_calls, 1, "the due observation samples exactly once");
    }

    #[test]
    fn count_gate_samples_on_tenth_observation_not_before() {
        let start = Instant::now();
        let mut t = tracker();
        t.seed_baseline_at(Some((5 * GB, CAP)), start);
        for step in 1..DISK_SAMPLE_INTERVAL {
            assert!(
                t.observe_at(
                    Some((5 * GB - THRESHOLD - 1, CAP)),
                    start + Duration::from_secs(step as u64)
                )
                .is_none(),
                "observation {step} must not sample early"
            );
            assert_eq!(t.pending_observations(), step);
        }
        assert!(
            t.observe_at(
                Some((5 * GB - THRESHOLD - 1, CAP)),
                start + Duration::from_secs(DISK_SAMPLE_INTERVAL as u64),
            )
            .is_some(),
            "tenth observation must sample"
        );
        assert_eq!(t.pending_observations(), 0);
    }

    #[test]
    fn time_gate_samples_after_three_minutes_with_one_observation() {
        let start = Instant::now();
        let mut t = tracker();
        t.seed_baseline_at(Some((5 * GB, CAP)), start);
        assert!(
            t.observe_at(
                Some((5 * GB - THRESHOLD - 1, CAP)),
                start + DISK_SAMPLE_MAX_AGE - Duration::from_millis(1),
            )
            .is_none(),
            "time gate must not fire early"
        );
        assert_eq!(t.pending_observations(), 1);
        assert!(
            t.observe_at(
                Some((5 * GB - THRESHOLD - 1, CAP)),
                start + DISK_SAMPLE_MAX_AGE,
            )
            .is_some(),
            "first observation at three minutes must sample"
        );
        assert_eq!(t.pending_observations(), 0);
    }

    #[test]
    fn successful_sample_resets_both_count_and_time_gates() {
        let start = Instant::now();
        let mut t = tracker();
        t.seed_baseline_at(Some((5 * GB, CAP)), start);
        // Time gate closes the first window with only one observation.
        assert!(t
            .observe_at(Some((5 * GB, CAP)), start + DISK_SAMPLE_MAX_AGE)
            .is_none());
        assert_eq!(t.pending_observations(), 0);
        // Nine observations and just under three minutes from the new sample
        // must not close the next window.
        for step in 1..DISK_SAMPLE_INTERVAL {
            assert!(t
                .observe_at(
                    Some((5 * GB - THRESHOLD - 1, CAP)),
                    start + DISK_SAMPLE_MAX_AGE + Duration::from_secs(step as u64),
                )
                .is_none());
        }
        assert_eq!(t.pending_observations(), DISK_SAMPLE_INTERVAL - 1);
        // The tenth observation closes it via the count gate.
        assert!(t
            .observe_at(
                Some((5 * GB - THRESHOLD - 1, CAP)),
                start + DISK_SAMPLE_MAX_AGE + Duration::from_secs(10),
            )
            .is_some());
        assert_eq!(t.pending_observations(), 0);
    }

    #[test]
    fn unavailable_sample_does_not_reset_due_window() {
        let start = Instant::now();
        let mut t = tracker();
        t.seed_baseline_at(Some((5 * GB, CAP)), start);
        for step in 1..DISK_SAMPLE_INTERVAL {
            assert!(t
                .observe_at(None, start + Duration::from_secs(step as u64))
                .is_none());
        }
        assert!(
            t.observe_at(None, start + Duration::from_secs(10))
                .is_none(),
            "missing sample cannot close the due window"
        );
        assert_eq!(t.pending_observations(), DISK_SAMPLE_INTERVAL);
        assert!(
            t.observe_at(
                Some((5 * GB - THRESHOLD - 1, CAP)),
                start + Duration::from_secs(11),
            )
            .is_some(),
            "next available sample must service the overdue window"
        );
        assert_eq!(t.pending_observations(), 0);
    }

    #[test]
    fn event_render_includes_before_after_and_disk_table() {
        let event = DiskPressureEvent {
            dropped: 255_400_000,
            base: 91_000_000_000,
            new: 90_700_000_000,
        };
        let filesystems = vec![FilesystemUsage {
            path: "/".into(),
            total_bytes: 982_862_268 * 1024,
            free_bytes: 84_600_000_000,
        }];
        let text = event.render(&filesystems);
        assert!(text.contains("dropped by 243.6 MB"), "{text}");
        assert!(text.contains("(84.8 GB -> 84.5 GB)"), "{text}");
        assert!(text.contains("current disk info:"), "{text}");
        assert!(text.contains("| `/` | 937.3 GB | 78.8 GB |"), "{text}");
    }

    #[test]
    fn seeded_baseline_makes_first_window_compare_immediately() {
        let mut t = tracker();
        t.seed_baseline(Some((5 * GB, CAP)));
        assert_eq!(t.baseline(), Some(5 * GB));
        // First window right after startup can now trigger: no blind window.
        let notice = advance_to_sample(&mut t, 5 * GB - THRESHOLD - 1);
        assert!(
            notice.is_some(),
            "startup-seeded baseline must compare in the first window"
        );
    }

    #[test]
    fn seed_is_ignored_when_baseline_already_exists() {
        let mut t = tracker();
        assert!(advance_to_sample(&mut t, 5 * GB).is_none());
        t.seed_baseline(Some((GB, CAP)));
        assert_eq!(
            t.baseline(),
            Some(5 * GB),
            "seed must not overwrite an existing baseline"
        );
    }

    #[test]
    fn first_sample_only_sets_baseline() {
        let mut t = tracker();
        assert!(advance_to_sample(&mut t, 5 * GB).is_none());
        assert_eq!(t.baseline(), Some(5 * GB));
    }

    #[test]
    fn drop_below_capped_threshold_keeps_baseline() {
        let mut t = tracker();
        assert!(advance_to_sample(&mut t, 5 * GB).is_none());
        // 150MB drop: below the 200MB capped threshold, no notice, and the
        // baseline stays so consumption accumulates.
        assert!(advance_to_sample(&mut t, 5 * GB - 150 * 1024 * 1024).is_none());
        assert_eq!(t.baseline(), Some(5 * GB));
    }

    #[test]
    fn drop_just_above_capped_threshold_triggers() {
        let mut t = tracker();
        assert!(advance_to_sample(&mut t, 5 * GB).is_none());
        // A 250MB drop on a large disk must trigger: the threshold is
        // capped at 200MB, not scaled to 8% of the capacity.
        let notice = advance_to_sample(&mut t, 5 * GB - 250 * 1024 * 1024);
        assert!(
            notice.is_some(),
            "250MB drop must trigger the 200MB-capped threshold"
        );
    }

    #[test]
    fn large_drop_triggers_once_and_rebases() {
        let mut t = tracker();
        assert!(advance_to_sample(&mut t, 5 * GB).is_none());
        let notice = advance_to_sample(&mut t, 5 * GB - THRESHOLD - 1);
        assert!(notice.is_some(), "drop just above threshold must trigger");
        assert_eq!(t.baseline(), Some(5 * GB - THRESHOLD - 1));
        // Same level again: no notice.
        assert!(advance_to_sample(&mut t, 5 * GB - THRESHOLD - 1).is_none());
    }

    #[test]
    fn small_disks_scale_below_the_150mb_cap() {
        // A 1GB disk scales to only 80MB (8%), which is now below the cap.
        let small_cap = GB;
        let scaled = small_cap / 100 * 8;
        assert!(scaled < 200 * 1024 * 1024);
        let mut t = tracker();
        for _ in 0..DISK_SAMPLE_INTERVAL {
            assert!(t.observe(Some((small_cap, small_cap))).is_none());
        }
        assert_eq!(t.pending_observations(), 0);
        // Just above the scaled threshold must trigger.
        let drop = scaled + 1;
        let mut notice = None;
        for _ in 0..DISK_SAMPLE_INTERVAL {
            notice = t.observe(Some((small_cap - drop, small_cap)));
        }
        assert_eq!(t.pending_observations(), 0);
        assert!(
            notice.is_some(),
            "drop just above the 8% scaled threshold must trigger"
        );
        // Just below the scaled threshold must not.
        let mut t2 = tracker();
        for _ in 0..DISK_SAMPLE_INTERVAL {
            assert!(t2.observe(Some((small_cap, small_cap))).is_none());
        }
        assert_eq!(t2.pending_observations(), 0);
        let mut notice = None;
        for _ in 0..DISK_SAMPLE_INTERVAL {
            notice = t2.observe(Some((small_cap - scaled + 1, small_cap)));
        }
        assert_eq!(t2.pending_observations(), 0);
        assert!(
            notice.is_none(),
            "drop below the 8% scaled threshold must not trigger"
        );
    }

    #[test]
    fn reclaiming_space_rebases_without_notice() {
        let mut t = tracker();
        assert!(advance_to_sample(&mut t, 5 * GB).is_none());
        assert!(advance_to_sample(&mut t, 6 * GB).is_none());
        assert_eq!(t.baseline(), Some(6 * GB));
    }
}
