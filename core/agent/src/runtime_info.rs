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
    /// Previous Runtime scopes for this Session that still contain live work.
    pub stale_process_scopes: Vec<StaleProcessScopeSnapshot>,
    /// Live descendants watched by the Runtime fallback chain after their
    /// original supervision chain ended.
    pub fallback_processes: Vec<FallbackProcessSnapshot>,
    /// Delta-based disk pressure notice produced by DiskPressureTracker at
    /// its latest sampled observation point, if it triggered.
    pub disk_pressure_notice: Option<String>,
}

pub use crate::os::FilesystemUsage;

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
    pub notes: String,
    pub pid: u32,
    pub process_name: String,
    pub zombie: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleProcessScopeSnapshot {
    pub notes: String,
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
            "unowned child processes:\n\n| pid | process | state | model decision | notes |\n|---:|---|---|---|---|",
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
                "\n| {} | `{}` | `{}` | {} | {} |",
                process.pid,
                name,
                state,
                decision,
                process.notes.replace('|', "\\|").replace('\n', " ")
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

| previous owner pid | model decision | notes |
|---:|---|---|",
        );
        for scope in &inputs.stale_process_scopes {
            let _ = writeln!(
                table,
                "\n| {} | inspect residual processes, then preserve or terminate explicitly | {} |",
                scope.owner_pid,
                scope.notes.replace('|', "\\|").replace('\n', " ")
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
                notes: String::new(),
            }],
            stale_process_scopes: vec![StaleProcessScopeSnapshot { owner_pid: 7, notes: "cgroup: /sys/fs/cgroup/test-stale".into() }],
            fallback_processes: vec![
                FallbackProcessSnapshot {
                    notes: "cgroup membership: /proc/77/cgroup".into(),
                    pid: 77,
                    process_name: "worker-helper".into(),
                    zombie: false,
                },
                FallbackProcessSnapshot {
                    notes: "cgroup membership: /proc/78/cgroup".into(),
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

/// Comparable aggregate: device membership and capacities accompany the totals.
/// Paths and mount enumeration order are diagnostic, not filesystem identity.
#[derive(Debug, Clone)]
pub struct DiskSample {
    free: u64,
    capacity: u64,
    devices: Vec<(u64, u64)>,
}

impl DiskSample {
    pub fn from_filesystems(filesystems: &[FilesystemUsage]) -> Option<Self> {
        if filesystems.is_empty() {
            return None;
        }
        let mut devices: Vec<_> = filesystems
            .iter()
            .map(|fs| (fs.device_id, fs.total_bytes))
            .collect();
        devices.sort_unstable();
        Some(Self {
            free: filesystems
                .iter()
                .fold(0_u64, |sum, fs| sum.saturating_add(fs.free_bytes)),
            capacity: filesystems
                .iter()
                .fold(0_u64, |sum, fs| sum.saturating_add(fs.total_bytes)),
            devices,
        })
    }
}

/// Observation-point-throttled disk pressure tracker.
///
/// An observation point is one completed tool run or one model API request.
/// A sample is taken when either 10 observation points accumulated or
/// 3 minutes elapsed since the previous sample. A successful sample resets
/// both gates. Changed device membership/capacity rebases without an event;
/// comparable samples compare total free space with the baseline:
/// - `new > base`: space was reclaimed; refresh `base = new` (no notice).
/// - `new <= base` and free space dropped by more than the threshold: emit
///   a notice and refresh `base = new`; otherwise keep the baseline so slow
///   consumption still accumulates toward the threshold.
#[derive(Debug)]
pub struct DiskPressureTracker {
    observations_since_sample: u32,
    last_sample_at: Instant,
    baseline: Option<DiskSample>,
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
    pub fn seed_baseline(&mut self, sample: Option<DiskSample>) {
        self.seed_baseline_at(sample, Instant::now());
    }

    fn seed_baseline_at(&mut self, sample: Option<DiskSample>, now: Instant) {
        if let Some(sample) = sample {
            if self.baseline.is_none() {
                self.baseline = Some(sample);
                self.observations_since_sample = 0;
                self.last_sample_at = now;
            }
        }
    }

    /// Override byte totals while retaining the real startup sampling scope.
    #[cfg(test)]
    pub fn sample_with_totals_for_test(&self, free: u64, capacity: u64) -> DiskSample {
        DiskSample {
            free,
            capacity,
            devices: self
                .baseline
                .as_ref()
                .map(|s| s.devices.clone())
                .unwrap_or_default(),
        }
    }

    /// Current baseline value (for tests).
    #[cfg(test)]
    pub fn baseline(&self) -> Option<u64> {
        self.baseline.as_ref().map(|sample| sample.free)
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
    pub fn observe(&mut self, sample: Option<DiskSample>) -> Option<DiskPressureEvent> {
        self.observe_with(|| sample)
    }

    /// Record one observation and obtain a filesystem sample only when the
    /// count or time gate is due. This keeps ordinary tool completions and
    /// model requests free of mount enumeration and stat calls.
    pub fn observe_with<F>(&mut self, sample: F) -> Option<DiskPressureEvent>
    where
        F: FnOnce() -> Option<DiskSample>,
    {
        self.observe_with_at(sample, Instant::now())
    }

    #[cfg(test)]
    fn observe_at(
        &mut self,
        sample: Option<DiskSample>,
        now: Instant,
    ) -> Option<DiskPressureEvent> {
        self.observe_with_at(|| sample, now)
    }

    fn observe_with_at<F>(&mut self, sample: F, now: Instant) -> Option<DiskPressureEvent>
    where
        F: FnOnce() -> Option<DiskSample>,
    {
        self.observations_since_sample = self.observations_since_sample.saturating_add(1);
        let count_due = self.observations_since_sample >= DISK_SAMPLE_INTERVAL;
        let time_due = now.saturating_duration_since(self.last_sample_at) >= DISK_SAMPLE_MAX_AGE;
        if !count_due && !time_due {
            return None;
        }
        let sample = sample()?;
        let new = sample.free;
        let capacity = sample.capacity;
        self.observations_since_sample = 0;
        self.last_sample_at = now;
        let Some(baseline) = &self.baseline else {
            // First sample only establishes the initial baseline.
            self.baseline = Some(sample);
            return None;
        };
        if baseline.devices != sample.devices {
            // Mount/unmount, unavailable member, replacement or resize: totals
            // no longer describe the same disks. Rebase without a pressure event.
            self.baseline = Some(sample);
            return None;
        }
        let base = baseline.free;
        if new > base {
            self.baseline = Some(sample);
            return None;
        }
        let dropped = base.saturating_sub(new);
        // Threshold scales with the disk: at most 200MB, at most 8% of
        // the total capacity, whichever is smaller.
        let threshold = (DISK_PRESSURE_DELTA_CAP_BYTES).min(capacity / 100 * 8);
        if dropped > threshold {
            self.baseline = Some(sample);
            return Some(DiskPressureEvent { dropped, base, new });
        }
        // Below the threshold: keep the baseline so continued consumption
        // still accumulates toward the next window's comparison.
        None
    }
}

#[cfg(test)]
#[path = "../tests/unit/runtime_info_disk_pressure_tests.rs"]
mod disk_pressure_tests;
