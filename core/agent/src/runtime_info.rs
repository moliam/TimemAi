//! RuntimeInfo: the model's observation aggregator. Modules register
//! reporters (callbacks); before each model request the registry collects
//! their important state and, only when some reporter has something to say,
//! renders a single `### RUNTIME_INFO` section that rides along with the
//! next request. Unimportant facts (e.g. a job that finished normally this
//! round and already has an action result) produce nothing.

use std::fmt::Write as _;

/// Snapshot of agent state handed to every reporter on collection.
#[derive(Default, Clone)]
pub struct RuntimeInfoInputs {
    /// Jobs still running (the STILL RUNNING source).
    pub running: Vec<RunningJobSnapshot>,
    /// Jobs that exited since the last request, with their exit status.
    pub updates: Vec<JobExitSnapshot>,
    /// Live orphan pids reparented to this runtime after escaping managed
    /// process groups (e.g. via `setsid`).
    pub escaped_pids: Vec<u32>,
    /// Filesystems the current work may write to (session working dir plus
    /// the cwd of every running job), already deduplicated per device.
    /// Kept for tests and future per-filesystem reporters; the sysstat
    /// report reads the tracker notice instead of re-walking this list.
    #[allow(dead_code)]
    pub filesystems: Vec<FilesystemUsage>,
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
            "still running jobs:\n\n| pid | created by tool_call id | command |\n|---:|---|---|",
        );
        for job in &inputs.running {
            let call_id = if job.tool_call_id.trim().is_empty() {
                "unknown_tool_call"
            } else {
                &job.tool_call_id
            };
            let command = job.command.chars().take(500).collect::<String>();
            let command = command.replace('|', "\\|").replace('\n', " ");
            let _ = writeln!(table, "\n| {} | `{}` | `{}` |", job.pid, call_id, command);
        }
        parts.push(table);
    }

    if !inputs.escaped_pids.is_empty() {
        let pids = inputs
            .escaped_pids
            .iter()
            .map(|pid| pid.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!(
            "ORPHAN_PROCESS: these programs are still running on this machine but no longer belong to any tracked task: [{pids}]. Check what they are (e.g. `ps -fp <pid>`), and stop them with `kill <pid>` if they are leftovers from earlier work."
        ));
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

    for update in &inputs.updates {
        if looks_killed(&update.status) {
            parts.push(format!(
                "JOB_KILLED: pid={} cmd=`{}` exited with `{}` — not a normal exit. If the command itself does not print an error, suspect OOM kill (check dmesg/`journalctl -k`) or an external terminator.",
                update.pid,
                update.command.chars().take(200).collect::<String>(),
                update.status
            ));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
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
            }],
            escaped_pids: vec![77],
            filesystems: vec![FilesystemUsage {
                path: "/".into(),
                total_bytes: 1000,
                free_bytes: 5,
            }],
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
        assert!(out.contains("| 42 | `call_1` | `sleep 100` |"), "{out}");
        assert!(out.contains("ORPHAN_PROCESS"), "{out}");
        assert!(out.contains("[77]"), "{out}");
        assert!(out.contains("DISK_PRESSURE"), "{out}");
        assert!(out.contains("observation points"), "{out}");
        assert!(out.contains("JOB_KILLED"), "{out}");
        assert!(out.contains("SIGKILL"), "{out}");
    }

    #[test]
    fn healthy_disk_and_normal_exits_are_not_reported() {
        let inputs = RuntimeInfoInputs {
            filesystems: vec![FilesystemUsage {
                path: "/".into(),
                total_bytes: 1000,
                free_bytes: 500,
            }],
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

/// Observation-point-throttled disk pressure tracker.
///
/// An observation point is one completed tool run or one model API request.
/// Every 10 points the total free space of all sampled disks is compared
/// with a baseline:
/// - `new > base`: space was reclaimed; refresh `base = new` (no notice).
/// - `new <= base` and free space dropped by more than the threshold: emit
///   a notice and refresh `base = new`; otherwise keep the baseline so slow
///   consumption still accumulates toward the threshold.
#[derive(Debug)]
pub struct DiskPressureTracker {
    observations: u32,
    baseline: Option<u64>,
}

const DISK_SAMPLE_INTERVAL: u32 = 10;
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
            observations: 0,
            baseline: None,
        }
    }

    /// Current baseline value (for tests).
    #[cfg(test)]
    pub fn baseline(&self) -> Option<u64> {
        self.baseline
    }

    /// True when the latest observe call closed a sampling window (for tests).
    #[cfg(test)]
    pub fn window_complete(&self) -> bool {
        self.observations.is_multiple_of(DISK_SAMPLE_INTERVAL)
    }

    /// Record one observation point (tool run or API request) with the
    /// sampled (total_free, total_capacity) of the working disks. Returns a
    /// notice string when the sampled interval triggered disk pressure.
    pub fn observe(&mut self, sample: Option<(u64, u64)>) -> Option<String> {
        self.observations = self.observations.saturating_add(1);
        if !self.observations.is_multiple_of(DISK_SAMPLE_INTERVAL) {
            return None;
        }
        let (new, capacity) = sample?;
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
            return Some(format!(
                "DISK_PRESSURE: total free space across the working disks dropped by {} within the last {} observation points (now {}). Free space is being consumed fast; clean up or move data if writes are expected to continue.",
                human_bytes(dropped),
                DISK_SAMPLE_INTERVAL,
                human_bytes(new)
            ));
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

    fn advance_to_sample(t: &mut DiskPressureTracker, free: u64) -> Option<String> {
        loop {
            let notice = t.observe(Some((free, CAP)));
            if t.window_complete() {
                return notice;
            }
        }
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
        loop {
            t.observe(Some((small_cap, small_cap)));
            if t.window_complete() {
                break;
            }
        }
        // Just above the scaled threshold must trigger.
        let drop = scaled + 1;
        let notice = loop {
            let notice = t.observe(Some((small_cap - drop, small_cap)));
            if t.window_complete() {
                break notice;
            }
        };
        assert!(
            notice.is_some(),
            "drop just above the 8% scaled threshold must trigger"
        );
        // Just below the scaled threshold must not.
        let mut t2 = tracker();
        loop {
            t2.observe(Some((small_cap, small_cap)));
            if t2.window_complete() {
                break;
            }
        }
        let notice = loop {
            let notice = t2.observe(Some((small_cap - scaled + 1, small_cap)));
            if t2.window_complete() {
                break notice;
            }
        };
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
