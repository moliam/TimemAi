use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::OnceLock;

#[cfg(unix)]
pub const BASH_EXECUTABLE: &str = "/bin/bash";
#[cfg(windows)]
pub const BASH_EXECUTABLE: &str = "bash.exe";

#[cfg(unix)]
pub const POSIX_SHELL_EXECUTABLE: &str = "/bin/sh";
#[cfg(windows)]
pub const POSIX_SHELL_EXECUTABLE: &str = "sh.exe";

static HOST_ENVIRONMENT: OnceLock<String> = OnceLock::new();
static POWERSHELL_HOST_ENVIRONMENT: OnceLock<String> = OnceLock::new();

/// Opens a single-writer lease file while allowing concurrent diagnostic reads.
///
/// Contention is normalized to `ErrorKind::WouldBlock`; callers own only their
/// domain-specific error text, while permission/share/locking policy stays in
/// the platform backend.
pub fn open_diagnostic_file_lease(path: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    return crate::shared::open_diagnostic_file_lease(path);
    #[cfg(windows)]
    return crate::windows::open_diagnostic_file_lease(path);
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)
    }
}

/// Applies platform-specific privacy and sharing flags before a new sensitive file is opened.
pub fn configure_private_file_options(options: &mut std::fs::OpenOptions) {
    #[cfg(unix)]
    crate::shared::configure_private_file_options(options);
    #[cfg(windows)]
    crate::windows::configure_private_file_options(options);
    #[cfg(not(any(unix, windows)))]
    let _ = options;
}

pub fn fill_secure_random(bytes: &mut [u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    return crate::shared::fill_secure_random(bytes);
    #[cfg(windows)]
    return crate::windows::fill_secure_random(bytes);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = bytes;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "secure_random_unsupported",
        ))
    }
}

pub fn user_home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        crate::windows::user_home_dir()
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }
}

pub fn local_time(secs: libc::time_t) -> Option<libc::tm> {
    #[cfg(unix)]
    return crate::shared::local_time(secs);
    #[cfg(windows)]
    return crate::windows::local_time(secs);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = secs;
        None
    }
}

pub fn local_command_execution_available() -> bool {
    cfg!(any(unix, windows))
}

pub fn bash_execution_available() -> bool {
    bash_version().is_some()
}

pub fn powershell_execution_available() -> bool {
    #[cfg(windows)]
    {
        powershell_version().is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn local_shell_tool_name() -> &'static str {
    if cfg!(windows) {
        "run_powershell"
    } else {
        "run_bash"
    }
}

pub fn command_for_local_shell(command_text: &str) -> Result<Command, String> {
    #[cfg(unix)]
    {
        let mut command = Command::new(BASH_EXECUTABLE);
        command.args(["--noprofile", "--norc", "-lc", command_text]);
        Ok(command)
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
        ]);
        command.arg(command_text);
        Ok(command)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = command_text;
        Err("local_shell_platform_unsupported".to_string())
    }
}

pub fn command_for_script(path: &Path) -> Result<Command, String> {
    #[cfg(unix)]
    {
        let mut command = Command::new(POSIX_SHELL_EXECUTABLE);
        command.arg(path);
        Ok(command)
    }
    #[cfg(windows)]
    {
        crate::windows::command_for_script(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err("command_script_platform_unsupported".to_string())
    }
}

/// Replaces the inherited environment with the minimum platform environment
/// needed to start trusted local script interpreters. This intentionally does
/// not forward arbitrary application variables or credentials.
pub fn configure_sanitized_child_environment(command: &mut Command) {
    command.env_clear().env(
        "PATH",
        std::env::var_os("PATH").unwrap_or_else(|| {
            OsString::from(if cfg!(windows) {
                r"C:\Windows\System32;C:\Windows"
            } else {
                "/usr/bin:/bin"
            })
        }),
    );
    #[cfg(unix)]
    crate::shared::configure_sanitized_child_environment(command);
    #[cfg(windows)]
    crate::windows::configure_sanitized_child_environment(command);
}

pub fn command_for_tool_language(language: &str, path: &Path) -> Result<Command, String> {
    match language.trim().to_ascii_lowercase().as_str() {
        "python" | "python3" => {
            let mut command = Command::new(if cfg!(windows) {
                "python.exe"
            } else {
                "python3"
            });
            command.arg(path);
            Ok(command)
        }
        "bash" | "shell" | "sh" => {
            if !bash_execution_available() {
                return Err("tool_language_bash_unavailable".to_string());
            }
            let mut command = Command::new(BASH_EXECUTABLE);
            command.arg(path);
            Ok(command)
        }
        "powershell" | "pwsh" => {
            #[cfg(windows)]
            {
                crate::windows::powershell_script_command(path)
            }
            #[cfg(not(windows))]
            {
                let mut command = Command::new("pwsh");
                command.args(["-NoProfile", "-NonInteractive", "-File"]);
                command.arg(path);
                Ok(command)
            }
        }
        _ => command_for_script(path),
    }
}

pub fn host_environment() -> &'static str {
    HOST_ENVIRONMENT
        .get_or_init(|| {
            format!(
                "OS: {}; Bash: {}",
                version().unwrap_or_else(|| "unknown".to_string()),
                bash_version().unwrap_or_else(|| "unknown".to_string())
            )
        })
        .as_str()
}

pub fn powershell_host_environment() -> &'static str {
    POWERSHELL_HOST_ENVIRONMENT
        .get_or_init(|| {
            format!(
                "OS: {}; PowerShell: {}",
                version().unwrap_or_else(|| "unknown".to_string()),
                powershell_version().unwrap_or_else(|| "unknown".to_string())
            )
        })
        .as_str()
}

pub fn version() -> Option<String> {
    platform_version().or_else(uname_version)
}

pub fn bash_version() -> Option<String> {
    command_first_line(
        BASH_EXECUTABLE,
        &[
            "--noprofile",
            "--norc",
            "-c",
            "printf '%s\\n' \"$BASH_VERSION\"",
        ],
    )
}

pub fn powershell_version() -> Option<String> {
    #[cfg(windows)]
    {
        command_first_line(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$PSVersionTable.PSVersion.ToString()",
            ],
        )
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn default_config_root(
    explicit: Option<&OsStr>,
    xdg: Option<&OsStr>,
    home: Option<&OsStr>,
) -> PathBuf {
    if let Some(path) = explicit.filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    platform_config_root(xdg, home)
}

pub fn browser_command(url: &str) -> Option<(OsString, Vec<OsString>)> {
    platform_browser_command(url)
}

pub fn terminal_command(path: &Path) -> Option<(OsString, Vec<OsString>)> {
    platform_terminal_command(path)
}

pub fn graphical_session_available() -> bool {
    platform_graphical_session_available()
}

/// Process-wide orphan safety-net initialization state.
///
/// The safety net is optional: unsupported platforms and initialization
/// failures must never block local command execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessSafetyNetStatus {
    Active,
    Unsupported,
    InitializationFailed,
}

/// Starts the Runtime-wide orphan safety net at most once per process.
///
/// Linux installs the process as a child subreaper and runs the bounded
/// fallback reaper. Other platforms report Unsupported. Failure never blocks
/// local command execution; callers retain process-group/platform fallbacks.
#[derive(Debug)]
pub struct ProcessSafetyNetGuard {
    status: ProcessSafetyNetStatus,
}

impl ProcessSafetyNetGuard {
    pub fn status(&self) -> ProcessSafetyNetStatus {
        self.status
    }
}

impl Drop for ProcessSafetyNetGuard {
    fn drop(&mut self) {
        cleanup_process_safety_net();
    }
}

const REAPER_FAST_CADENCE: std::time::Duration = std::time::Duration::from_millis(250);
const REAPER_MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(4);

/// Idle backoff for the orphan-reaper loop: any reaping or managed-child
/// spawn activity restores the fast 250 ms cadence; otherwise the poll
/// interval doubles up to a 4 s ceiling.
pub(crate) fn next_reaper_backoff(
    reaped: usize,
    churn: u64,
    current: std::time::Duration,
) -> std::time::Duration {
    if reaped > 0 || churn > 0 {
        REAPER_FAST_CADENCE
    } else {
        current.saturating_mul(2).min(REAPER_MAX_BACKOFF)
    }
}

fn take_spawn_churn() -> u64 {
    #[cfg(unix)]
    return crate::shared::take_spawn_churn();
    #[cfg(not(unix))]
    0
}

pub fn ensure_process_safety_net() -> ProcessSafetyNetGuard {
    static STATUS: OnceLock<ProcessSafetyNetStatus> = OnceLock::new();
    let status = *STATUS.get_or_init(|| {
        if !install_process_subreaper() {
            return if cfg!(target_os = "linux") {
                ProcessSafetyNetStatus::InitializationFailed
            } else {
                ProcessSafetyNetStatus::Unsupported
            };
        }
        match std::thread::Builder::new()
            .name("orphan-reaper".to_string())
            .spawn(|| {
                let mut reaper = FallbackProcessReaper::for_runtime();
                // Fixed 250 ms full-/proc polling costs a constant CPU share
                // on busy hosts. Reaping or managed-child spawns keep the fast
                // cadence; quiet periods double the interval up to 4 s. A
                // newly dead orphan is then at most ~4 s from reaping, which
                // stays invisible to the runtime lifecycle.
                let mut backoff = std::time::Duration::from_millis(250);
                loop {
                    let reaped = reaper.reap_adopted_zombies();
                    let churn = take_spawn_churn();
                    backoff = next_reaper_backoff(reaped, churn, backoff);
                    std::thread::sleep(backoff);
                }
            }) {
            Ok(_) => ProcessSafetyNetStatus::Active,
            Err(_) => ProcessSafetyNetStatus::InitializationFailed,
        }
    });
    ProcessSafetyNetGuard { status }
}

/// Best-effort removal of empty process-management facilities owned by this
/// Runtime. Live or unrecognised scopes are preserved, and cleanup failure
/// never changes command or shutdown behavior.
pub fn cleanup_process_safety_net() {
    #[cfg(target_os = "linux")]
    crate::linux::cleanup_current_runtime_process_scope();
}

pub fn install_process_subreaper() -> bool {
    #[cfg(unix)]
    return crate::shared::install_process_subreaper();
    #[cfg(not(unix))]
    {
        false
    }
}

/// Registers a direct child as owned by its `Child` supervisor. Keep the
/// returned guard alive until that owner has completed `wait`.
pub struct ManagedChildRegistration {
    #[cfg(unix)]
    _inner: crate::shared::ManagedChildRegistration,
}

pub fn register_managed_child(pid: u32) -> ManagedChildRegistration {
    #[cfg(not(unix))]
    let _ = pid;
    ManagedChildRegistration {
        #[cfg(unix)]
        _inner: crate::shared::register_managed_child(pid),
    }
}

/// Runs a synchronous command while reserving its child exit status for this
/// owner, so the Runtime fallback reaper cannot consume it.
pub fn command_status(command: &mut Command) -> std::io::Result<ExitStatus> {
    let mut child = command.spawn()?;
    let registration = register_managed_child(child.id());
    let status = child.wait();
    drop(registration);
    status
}

/// Runs a synchronous command with captured output while reserving its child
/// exit status for this owner.
pub fn command_output(command: &mut Command) -> std::io::Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command.spawn()?;
    let registration = register_managed_child(child.id());
    let output = child.wait_with_output();
    drop(registration);
    output
}

/// Periodic, targeted reaper for dead descendants adopted by a Linux
/// subreaper. It never waits on a registered managed child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrphanProcessEvent {
    pub pid: u32,
    pub process_name: String,
    pub state: &'static str,
}

/// One bounded internal adoption or terminal transition from the Runtime
/// fallback process supervisor. These transitions support diagnostics and
/// tests; model-facing RuntimeInfo uses only current fallback snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackProcessSnapshot {
    pub pid: u32,
    pub process_name: String,
    pub zombie: bool,
}

/// Read-only diagnostic entry point; this does not establish task ownership.
pub fn process_observation_note(pid: u32) -> String {
    #[cfg(target_os = "linux")]
    {
        format!("proc: /proc/{pid}; cgroup membership: /proc/{pid}/cgroup (observation only, not task ownership)")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        String::new()
    }
}

/// Current descendants being watched by the Runtime fallback chain because
/// no registered direct-child owner remains.
pub fn fallback_process_snapshots() -> Vec<FallbackProcessSnapshot> {
    #[cfg(unix)]
    return crate::shared::fallback_process_snapshots()
        .into_iter()
        .map(|snapshot| FallbackProcessSnapshot {
            pid: snapshot.pid,
            process_name: snapshot.process_name,
            zombie: snapshot.zombie,
        })
        .collect();
    #[cfg(not(unix))]
    Vec::new()
}

/// Current Runtime and Session aggregate process-observation scopes.
/// Linux creates stable empty cgroup-v2 parent directories above per-Job
/// leaves. Other platforms return `Ok(None)` until they provide an equivalent
/// native read-only observation scope.
pub fn process_aggregate_scope_snapshot(
    session_id: &str,
) -> std::io::Result<Option<crate::ProcessAggregateScopeSnapshot>> {
    #[cfg(target_os = "linux")]
    return crate::linux::process_aggregate_scope_snapshot(session_id).map(Some);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = session_id;
        Ok(None)
    }
}

/// Previous Runtime scopes for this exact Session that still contain live
/// members. Empty stale directories are cleaned internally and not returned.
pub fn stale_process_scope_snapshots(session_id: &str) -> Vec<crate::StaleProcessScopeSnapshot> {
    #[cfg(target_os = "linux")]
    return crate::linux::stale_process_scope_snapshots(session_id);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = session_id;
        Vec::new()
    }
}

pub fn take_orphan_process_events() -> Vec<OrphanProcessEvent> {
    #[cfg(unix)]
    return crate::shared::take_orphan_process_events()
        .into_iter()
        .map(|event| OrphanProcessEvent {
            pid: event.pid,
            process_name: event.process_name,
            state: event.state,
        })
        .collect();
    #[cfg(not(unix))]
    Vec::new()
}

pub struct FallbackProcessReaper {
    #[cfg(unix)]
    inner: crate::shared::FallbackProcessReaper,
}

impl FallbackProcessReaper {
    pub fn for_runtime() -> Self {
        Self {
            #[cfg(unix)]
            inner: crate::shared::FallbackProcessReaper::new(),
        }
    }

    pub fn reap_adopted_zombies(&mut self) -> usize {
        #[cfg(unix)]
        return self.inner.reap_once();
        #[cfg(not(unix))]
        0
    }
}

/// Remove the runtime's managed resource-limit scopes (`timem.self` and
/// `timem.jobs`) after a graceful shutdown. Best effort: populated groups
/// (live orphan jobs keep their fence) and layers the user does not own are
/// kept; the next startup re-derives everything idempotently. Platforms
/// without a kernel quota facility no-op.
pub fn cleanup_instance_resource_limits() {
    #[cfg(target_os = "linux")]
    crate::linux::cleanup_instance_cgroup_subtrees();
}

/// Live orphaned descendants reparented to this runtime that escaped into a
/// different session (for example via `setsid`). This is an observation API,
/// not proof that a particular job owns the process.
pub fn reparented_detached_child_pids() -> Vec<u32> {
    #[cfg(unix)]
    return crate::shared::reparented_detached_child_pids();
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

/// Reap a reparented orphan child that has been terminated. Without this the
/// child remains a zombie, and kill(pid, 0) keeps reporting it as alive.
/// Non-blocking reap attempt of a dead child (biological or subreaper-adopted).
/// Returns true when reaped or not our child; false while it is terminating.
pub fn try_reap_child_process(pid: u32) -> bool {
    #[cfg(unix)]
    return crate::shared::try_reap_child_process(pid);
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

pub fn reap_child_process(pid: u32) {
    #[cfg(unix)]
    crate::shared::reap_child_process(pid);
    #[cfg(not(unix))]
    let _ = pid;
}

/// Contain a freshly spawned child in the runtime's OS-level containment
/// (Windows job object; no-op returning true on Unix, where the subreaper
/// safety net applies instead).
pub fn contain_child_process(pid: u32) -> bool {
    #[cfg(windows)]
    return crate::windows::contain_process_in_runtime_job(pid);
    #[cfg(not(windows))]
    {
        let _ = pid;
        true
    }
}

pub fn configure_child_process_group(command: &mut Command) {
    #[cfg(unix)]
    crate::shared::configure_child_process_group(command);
    #[cfg(windows)]
    crate::windows::configure_child_process_group(command);
    #[cfg(not(any(unix, windows)))]
    let _ = command;
}

pub fn exit_signal(status: &ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    return crate::shared::exit_signal(status);
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

pub fn process_is_alive(pid: u64) -> Option<bool> {
    #[cfg(unix)]
    return crate::shared::process_is_alive(pid);
    #[cfg(windows)]
    return crate::windows::process_is_alive(pid);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

pub fn process_running(pid: u32) -> bool {
    process_is_alive(u64::from(pid)).unwrap_or(false)
}

/// Conservative liveness check for ownership/lock decisions. Unsupported
/// platforms return true so callers never steal resources from a process that
/// may still be alive.
pub fn process_may_be_alive(pid: u32) -> bool {
    process_is_alive(u64::from(pid)).unwrap_or(true)
}

/// Returns true only when the platform can positively establish that the
/// process does not exist.
pub fn process_is_definitely_dead(pid: u32) -> bool {
    matches!(process_is_alive(u64::from(pid)), Some(false))
}

/// Returns whether a filesystem entry is owned by the effective user. Unknown
/// platforms fail closed because this is used before deleting stale artifacts.
pub fn path_owned_by_current_user(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path)
            .map(|metadata| metadata.uid() == unsafe { libc::geteuid() })
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Returns a kernel-derived identity that changes when an operating-system PID
/// is reused. Callers must treat `None` as "identity unavailable", not as a
/// positive match.
pub fn process_identity(pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    return crate::macos::process_identity(pid);
    #[cfg(target_os = "linux")]
    return crate::linux::process_identity(pid);
    #[cfg(windows)]
    return crate::windows::process_identity(pid);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = pid;
        None
    }
}

pub fn child_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    return crate::shared::child_process_running(pid);
    #[cfg(windows)]
    return crate::windows::child_process_running(pid);
    #[cfg(not(any(unix, windows)))]
    {
        process_running(pid)
    }
}

pub fn is_runtime_child_process_group(pid: u32) -> bool {
    #[cfg(unix)]
    return crate::shared::is_runtime_child_process_group(pid);
    #[cfg(windows)]
    return crate::windows::is_runtime_child_process_group(pid);
    #[cfg(not(any(unix, windows)))]
    {
        pid > 1 && pid != std::process::id()
    }
}

pub fn runtime_child_pid_kind() -> &'static str {
    #[cfg(unix)]
    return crate::shared::runtime_child_pid_kind();
    #[cfg(not(unix))]
    {
        "runtime_child_process"
    }
}

pub fn terminate_process(pid: u32) {
    #[cfg(unix)]
    crate::shared::terminate_process(pid);
    #[cfg(windows)]
    crate::windows::terminate_process(pid);
    #[cfg(not(any(unix, windows)))]
    let _ = pid;
}

pub fn terminate_process_group(group_leader_pid: u32) {
    #[cfg(unix)]
    crate::shared::terminate_process_group(group_leader_pid);
    #[cfg(windows)]
    crate::windows::terminate_process(group_leader_pid);
    #[cfg(not(any(unix, windows)))]
    let _ = group_leader_pid;
}

pub fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    crate::shared::kill_process_group(pid);
    #[cfg(windows)]
    crate::windows::terminate_process(pid);
    #[cfg(not(any(unix, windows)))]
    let _ = pid;
}

pub fn process_group_running(group_leader_pid: u32) -> bool {
    #[cfg(unix)]
    return crate::shared::process_group_running(group_leader_pid);
    #[cfg(windows)]
    return crate::windows::process_tree_running(group_leader_pid);
    #[cfg(not(any(unix, windows)))]
    {
        process_running(group_leader_pid)
    }
}

/// Local real filesystem mount points (data disks), excluding pseudo
/// filesystems. Disk sampling uses this so writes to any data disk are
/// covered, not only the working directory.
pub fn local_filesystem_mount_points() -> Vec<std::path::PathBuf> {
    #[cfg(unix)]
    return crate::shared::local_filesystem_mount_points();
    #[cfg(windows)]
    return crate::windows::local_filesystem_mount_points();
    #[cfg(not(any(unix, windows)))]
    {
        Vec::new()
    }
}

/// Stable device identifier of the filesystem containing `path`, used to
/// deduplicate paths on the same disk.
pub fn filesystem_device_id(path: &std::path::Path) -> Option<u64> {
    #[cfg(unix)]
    return crate::shared::filesystem_device_id(path);
    #[cfg(windows)]
    return crate::windows::filesystem_device_id(path);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Filesystem usage for the filesystem containing `path`:
/// (total_bytes, free_bytes). None when the stat call fails.
pub fn filesystem_usage_bytes(path: &std::path::Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    return crate::shared::filesystem_usage_bytes(path);
    #[cfg(windows)]
    return crate::windows::filesystem_usage_bytes(path);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// One successfully sampled filesystem, deduplicated by platform device identity.
/// `path` is a diagnostic alias, not the identity used for comparisons.
#[derive(Clone, Debug)]
pub struct FilesystemUsage {
    pub device_id: u64,
    pub path: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// Sample caller-provided working paths and local data mounts once per device.
/// Unavailable paths are omitted (never interpreted as zero free bytes). A later
/// alias may still supply a sample when an earlier usage query failed. Consumers
/// must compare device membership/capacity before comparing aggregate byte totals.
/// No polling, baseline, pressure threshold, or notification policy lives here.
pub fn filesystem_usage_snapshot(working_paths: &[PathBuf]) -> Vec<FilesystemUsage> {
    collect_filesystem_usage(
        working_paths
            .iter()
            .cloned()
            .chain(local_filesystem_mount_points()),
        filesystem_device_id,
        filesystem_usage_bytes,
    )
}

fn collect_filesystem_usage(
    paths: impl IntoIterator<Item = PathBuf>,
    mut device_id: impl FnMut(&Path) -> Option<u64>,
    mut usage_bytes: impl FnMut(&Path) -> Option<(u64, u64)>,
) -> Vec<FilesystemUsage> {
    let mut seen = std::collections::HashSet::new();
    let mut sampled = Vec::new();
    for path in paths {
        let Some(device) = device_id(&path) else {
            continue;
        };
        if seen.contains(&device) {
            continue;
        }
        let Some((total, free)) = usage_bytes(&path) else {
            continue;
        };
        seen.insert(device);
        sampled.push(FilesystemUsage {
            device_id: device,
            path: path.display().to_string(),
            total_bytes: total,
            free_bytes: free,
        });
    }
    sampled
}

#[cfg(test)]
#[path = "../tests/unit/filesystem_snapshot_tests.rs"]
mod filesystem_snapshot_tests;

pub fn list_live_process_group_members(group_leader_pid: u32) -> Vec<u32> {
    #[cfg(unix)]
    return crate::shared::list_live_process_group_members(group_leader_pid);
    #[cfg(windows)]
    {
        // Windows job containment reports the whole tree at shutdown; live
        // member listing is a Unix /proc facility, so report none here.
        let _ = group_leader_pid;
        Vec::new()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = group_leader_pid;
        Vec::new()
    }
}

pub fn current_parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        u32::try_from(unsafe { libc::getppid() })
            .ok()
            .filter(|pid| *pid > 1)
    }
    #[cfg(windows)]
    {
        crate::windows::current_parent_pid().filter(|pid| *pid > 1)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn uname_version() -> Option<String> {
    let system = command_first_line("/usr/bin/uname", &["-s"])
        .or_else(|| command_first_line("uname", &["-s"]))?;
    let release = command_first_line("/usr/bin/uname", &["-r"])
        .or_else(|| command_first_line("uname", &["-r"]));
    Some(match release {
        Some(release) => format!("{system} {release}"),
        None => system,
    })
}

pub(crate) fn command_first_line(program: &str, args: &[&str]) -> Option<String> {
    let output = command_output(Command::new(program).args(args)).ok()?;
    if !output.status.success() {
        return None;
    }
    non_empty_one_line(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn non_empty_one_line(value: &str) -> Option<String> {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!value.is_empty()).then_some(value)
}

#[cfg(target_os = "macos")]
fn platform_version() -> Option<String> {
    crate::macos::version()
}

#[cfg(target_os = "linux")]
fn platform_version() -> Option<String> {
    crate::linux::version()
}

#[cfg(windows)]
fn platform_version() -> Option<String> {
    crate::windows::version()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_version() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn platform_config_root(_xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    crate::macos::config_root(home)
}

#[cfg(target_os = "linux")]
fn platform_config_root(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    crate::linux::config_root(xdg, home)
}

#[cfg(windows)]
fn platform_config_root(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    crate::windows::config_root(xdg, home)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_config_root(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    if let Some(path) = xdg.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join("timem");
    }
    home.filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("timem"))
        .unwrap_or_else(|| PathBuf::from("timem"))
}

#[cfg(target_os = "macos")]
fn platform_browser_command(url: &str) -> Option<(OsString, Vec<OsString>)> {
    Some(crate::macos::browser_command(url))
}

#[cfg(target_os = "linux")]
fn platform_browser_command(url: &str) -> Option<(OsString, Vec<OsString>)> {
    Some(crate::linux::browser_command(url))
}

#[cfg(windows)]
fn platform_browser_command(url: &str) -> Option<(OsString, Vec<OsString>)> {
    Some(crate::windows::browser_command(url))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_browser_command(_url: &str) -> Option<(OsString, Vec<OsString>)> {
    None
}

#[cfg(target_os = "macos")]
fn platform_terminal_command(path: &Path) -> Option<(OsString, Vec<OsString>)> {
    Some(crate::macos::terminal_command(path))
}

#[cfg(target_os = "linux")]
fn platform_terminal_command(path: &Path) -> Option<(OsString, Vec<OsString>)> {
    Some(crate::linux::terminal_command(path))
}

#[cfg(windows)]
fn platform_terminal_command(path: &Path) -> Option<(OsString, Vec<OsString>)> {
    Some(crate::windows::terminal_command(path))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_terminal_command(_path: &Path) -> Option<(OsString, Vec<OsString>)> {
    None
}

#[cfg(target_os = "macos")]
fn platform_graphical_session_available() -> bool {
    crate::macos::graphical_session_available()
}

#[cfg(target_os = "linux")]
fn platform_graphical_session_available() -> bool {
    crate::linux::graphical_session_available()
}

#[cfg(windows)]
fn platform_graphical_session_available() -> bool {
    crate::windows::graphical_session_available()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_graphical_session_available() -> bool {
    false
}

// ---------------------------------------------------------------------------
// Instance resource caps
//
// Instance-scoped upper bounds on how much of the machine's total CPU
// capacity and total memory the current Timem instance's managed job tree may
// consume. Caps are percentages of system totals; the kernel enforces them so
// a runaway job tree cannot starve the whole machine. Scheduling, reporting,
// and policy stay with Agent; this facade only records facts and applies
// kernel bounds.

/// Caps for the current instance as percentages (0 < p <= 100) of the
/// machine's total CPU capacity and total memory. Each dimension carries an
/// independent enable flag: a disabled dimension is written back to the
/// kernel as unbounded ("max"), so the switch reflects the actual bound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceResourceCaps {
    pub cpu_enabled: bool,
    pub cpu_percent: f64,
    pub mem_enabled: bool,
    pub mem_percent: f64,
}

impl InstanceResourceCaps {
    /// Validate caller-provided percentages. Values must be finite and inside
    /// `(0, 100]` regardless of the enable flag (a stored preset stays valid
    /// for when the dimension is re-enabled); 100 keeps the kernel tracking
    /// without throttling below full capacity.
    pub fn validated(
        cpu_enabled: bool,
        cpu_percent: f64,
        mem_enabled: bool,
        mem_percent: f64,
    ) -> Result<Self, String> {
        fn check(value: f64, name: &str) -> Result<(), String> {
            if !value.is_finite() || value <= 0.0 || value > 100.0 {
                return Err(format!("{name} cap {value} is outside (0, 100]"));
            }
            Ok(())
        }
        check(cpu_percent, "cpu")?;
        check(mem_percent, "mem")?;
        Ok(Self {
            cpu_enabled,
            cpu_percent,
            mem_enabled,
            mem_percent,
        })
    }
}

/// Kernel-side application result of one cap dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceCapStatus {
    /// The kernel now bounds the instance job tree; `limit` is the
    /// platform-native value written to the kernel.
    Applied { limit: String },
    /// The user disabled this dimension; the kernel bound was cleared back
    /// to the platform-native unbounded value ("max").
    Disabled,
    /// This platform has no kernel-level instance quota facility yet.
    Unsupported,
    /// The facility exists but the bound could not be established now.
    Unavailable { reason: String },
}

/// Outcome snapshot for both cap dimensions, the requested caps, and the
/// platform-native scope (diagnostic location) the bounds were applied to.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceCapsApplication {
    pub caps: InstanceResourceCaps,
    pub cpu: ResourceCapStatus,
    pub mem: ResourceCapStatus,
    pub scope: String,
}

impl ResourceCapsApplication {
    /// True only when both dimensions reached their configured intent right
    /// now: bounded (`Applied`) or intentionally unbounded (`Disabled`).
    pub fn fully_applied(&self) -> bool {
        matches!(
            self.cpu,
            ResourceCapStatus::Applied { .. } | ResourceCapStatus::Disabled
        ) && matches!(
            self.mem,
            ResourceCapStatus::Applied { .. } | ResourceCapStatus::Disabled
        )
    }
}

static INSTANCE_RESOURCE_CAPS: std::sync::RwLock<Option<std::sync::Arc<ResourceCapsApplication>>> =
    std::sync::RwLock::new(None);

/// Apply instance resource caps exactly once per process. The first call
/// wins; later calls return the recorded outcome regardless of the caps
/// passed, so host startup is the only natural writer. Invalid caps never
/// take effect as wrong quotas: they are recorded as `Unavailable` outcomes
/// so the failure stays visible to consumers. Runtime changes go through
/// [`update_instance_resource_caps`].
pub fn apply_instance_resource_caps(
    caps: InstanceResourceCaps,
) -> std::sync::Arc<ResourceCapsApplication> {
    if let Some(existing) = instance_resource_caps_application() {
        return existing;
    }
    let mut guard = INSTANCE_RESOURCE_CAPS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.clone() {
        return existing;
    }
    let application = std::sync::Arc::new(platform_apply_instance_resource_caps(caps));
    *guard = Some(application.clone());
    application
}

/// The recorded outcome, or `None` before the first application attempt.
pub fn instance_resource_caps_application() -> Option<std::sync::Arc<ResourceCapsApplication>> {
    INSTANCE_RESOURCE_CAPS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Update the caps at runtime (WebUI system-settings path). Validates the
/// request, rewrites the kernel bound, and records the new outcome. On
/// validation or kernel failure nothing changes and the error is returned,
/// so a stored setting can never drift away from what the kernel enforces.
pub fn update_instance_resource_caps(
    caps: InstanceResourceCaps,
) -> Result<ResourceCapsApplication, String> {
    let application = platform_update_instance_resource_caps(caps)?;
    let mut guard = INSTANCE_RESOURCE_CAPS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(std::sync::Arc::new(application.clone()));
    Ok(application)
}

/// Logical CPUs online, when the platform can report them.
pub fn system_cpu_cores() -> Option<u32> {
    platform_system_cpu_cores()
}

/// Total system memory in bytes, when the platform can report it.
pub fn system_mem_total_bytes() -> Option<u64> {
    platform_system_mem_total_bytes()
}

/// Point-in-time machine-wide resource counters.
///
/// CPU values are cumulative kernel ticks, so consumers derive utilization
/// from two consecutive samples; memory fields are direct snapshots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemResourceUsage {
    /// Cumulative busy CPU ticks across all CPUs (kernel USER_HZ units).
    pub cpu_busy_ticks: Option<u64>,
    /// Cumulative total CPU ticks across all CPUs.
    pub cpu_total_ticks: Option<u64>,
    /// Total physical memory in bytes.
    pub memory_total_bytes: Option<u64>,
    /// Available memory in bytes, when the platform reports it.
    pub memory_available_bytes: Option<u64>,
}

/// Read machine-wide CPU/memory counters. Returns `Ok(None)` when the
/// platform has no equivalent accounting facility.
pub fn system_resource_usage() -> std::io::Result<Option<SystemResourceUsage>> {
    platform_system_resource_usage()
}

/// Point-in-time resource accounting for one Session's managed job scope.
///
/// Values aggregate every managed Job leaf below the Session scope. Limits are
/// the effective values the instance enforces for its managed job tree; a
/// child scope can show `max` while an ancestor bound still applies, so the
/// limit fields are read where the instance writes them. Optional fields stay
/// `None` when the platform lacks that dimension instead of being faked.
/// One managed Job leaf under a Session scope.
#[derive(Clone, Debug, PartialEq)]
pub struct JobResourceUsage {
    /// Opaque managed scope name (`job-*`). The same name appears in the
    /// process observation note the model receives for that job.
    pub scope_name: String,
    /// OOM kills recorded by this Job leaf (cumulative counter).
    pub oom_kill_count: u64,
    /// Memory currently charged to this Job leaf, in bytes.
    pub memory_current_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionResourceUsage {
    /// CPU time consumed by the Session's job tree, in microseconds.
    pub cpu_usage_usec: Option<u64>,
    /// Effective CPU bandwidth quota in cores, when a quota is enforced.
    pub cpu_quota_cores: Option<f64>,
    /// Memory currently charged to the Session's job tree, in bytes.
    pub memory_current_bytes: Option<u64>,
    /// Peak memory charged to the Session's job tree, in bytes, when the
    /// platform tracks a peak.
    pub memory_peak_bytes: Option<u64>,
    /// Swap currently charged to the Session's job tree, in bytes.
    pub swap_current_bytes: Option<u64>,
    /// Effective soft memory bound for the managed tree, in bytes.
    pub memory_high_bytes: Option<u64>,
    /// Effective hard memory bound for the managed tree, in bytes.
    pub memory_max_bytes: Option<u64>,
    /// OOM kills observed in the Session's job tree (cumulative counter).
    pub oom_kill_count: u64,
    /// Managed Job leaves under the Session scope, sorted by scope name and
    /// bounded by the scope entry cap.
    pub jobs: Vec<JobResourceUsage>,
}

/// Read-only snapshot of one Session's managed job scope. Returns `Ok(None)`
/// when the platform has no accounting scope for that Session (no jobs have
/// been started yet, or the platform provides no equivalent facility). The
/// call never creates scopes and never signals processes.
pub fn session_resource_usage(session_id: &str) -> std::io::Result<Option<SessionResourceUsage>> {
    #[cfg(target_os = "linux")]
    return crate::linux::session_resource_usage(session_id);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = session_id;
        Ok(None)
    }
}

#[cfg(target_os = "linux")]
fn platform_apply_instance_resource_caps(caps: InstanceResourceCaps) -> ResourceCapsApplication {
    match InstanceResourceCaps::validated(
        caps.cpu_enabled,
        caps.cpu_percent,
        caps.mem_enabled,
        caps.mem_percent,
    ) {
        Ok(caps) => crate::linux::apply_instance_resource_caps(&caps),
        Err(reason) => ResourceCapsApplication {
            caps,
            cpu: ResourceCapStatus::Unavailable {
                reason: reason.clone(),
            },
            mem: ResourceCapStatus::Unavailable { reason },
            scope: String::new(),
        },
    }
}

#[cfg(not(target_os = "linux"))]
fn platform_apply_instance_resource_caps(caps: InstanceResourceCaps) -> ResourceCapsApplication {
    // Kernel-level instance quota facilities today: Linux cgroup v2
    // (`cpu.max` + `memory.max` / `memory.high`). Future platform plans: Windows Job Object
    // CPU rate control + JobMemoryLimit on the runtime-wide job; macOS has no
    // kernel-level quota facility (soft priorities only). Report the gap
    // instead of pretending the caps are enforced.
    ResourceCapsApplication {
        caps,
        cpu: ResourceCapStatus::Unsupported,
        mem: ResourceCapStatus::Unsupported,
        scope: "no kernel-level instance quota facility on this platform yet".to_string(),
    }
}

#[cfg(target_os = "linux")]
fn platform_update_instance_resource_caps(
    caps: InstanceResourceCaps,
) -> Result<ResourceCapsApplication, String> {
    let validated = InstanceResourceCaps::validated(
        caps.cpu_enabled,
        caps.cpu_percent,
        caps.mem_enabled,
        caps.mem_percent,
    )?;
    Ok(crate::linux::apply_instance_resource_caps(&validated))
}

#[cfg(not(target_os = "linux"))]
fn platform_update_instance_resource_caps(
    _caps: InstanceResourceCaps,
) -> Result<ResourceCapsApplication, String> {
    Err("instance resource caps are not supported on this platform yet".to_string())
}

#[cfg(target_os = "linux")]
fn platform_system_cpu_cores() -> Option<u32> {
    crate::linux::system_cpu_cores()
}

#[cfg(not(target_os = "linux"))]
fn platform_system_cpu_cores() -> Option<u32> {
    None
}

#[cfg(target_os = "linux")]
fn platform_system_mem_total_bytes() -> Option<u64> {
    crate::linux::system_mem_total_bytes()
}

#[cfg(not(target_os = "linux"))]
fn platform_system_mem_total_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn platform_system_resource_usage() -> std::io::Result<Option<SystemResourceUsage>> {
    crate::linux::system_resource_usage()
}

#[cfg(not(target_os = "linux"))]
fn platform_system_resource_usage() -> std::io::Result<Option<SystemResourceUsage>> {
    Ok(None)
}

/// Fixed, machine-consumable reason for the shared-instance-root memory
/// limitation. The login session cgroup already holds unrelated processes, so
/// enabling a controller converts it to a threaded subtree that carries only
/// cpu; memory caps cannot be enforced until Timem runs in its own scope.
pub const SHARED_INSTANCE_ROOT_REASON: &str = "instance root is shared with other processes: the kernel converts a populated root to a threaded subtree that carries only cpu, so memory caps cannot be enforced here";

/// Environment marker set on the re-exec into a private user scope. It keeps a
/// process that still cannot reach a private root from nesting another scope.
pub const INDEPENDENT_SCOPE_ENV: &str = "TIMEM_INDEPENDENT_SCOPE";

/// One cap dimension's escalation-free availability right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceCapReadiness {
    /// The controller is reachable and the process cgroup is writable
    /// without root; caps can apply as-is.
    Ready,
    /// Root can make this dimension usable right now; `actions` is the
    /// minimal plan for [`apply_resource_cap_escalation_plan`].
    NeedsEscalation {
        actions: Vec<ResourceCapEscalationAction>,
    },
    /// Root cannot repair this either (unexpected kernel layout).
    Unavailable { reason: String },
    /// This platform has no kernel-level instance quota facility.
    Unsupported,
}

/// Per-dimension readiness probe for the current platform resource-control scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceCapsReadiness {
    /// The platform-native scope the managed job tree hangs off of.
    pub scope: String,
    pub cpu: ResourceCapReadiness,
    pub mem: ResourceCapReadiness,
}

impl ResourceCapsReadiness {
    /// True when any dimension could become usable through root.
    pub fn needs_escalation(&self) -> bool {
        matches!(self.cpu, ResourceCapReadiness::NeedsEscalation { .. })
            || matches!(self.mem, ResourceCapReadiness::NeedsEscalation { .. })
    }
}

/// One root-authorized step toward usable caps. Plans contain only cgroup-v2
/// self migration, subtree enablement, and ownership repair; nothing else is
/// ever executed. This vocabulary is Linux-specific: platforms without a
/// cgroup-v2 facility report `Unsupported` and never produce a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceCapEscalationAction {
    /// Move the main process into the managed `timem.self` sibling so the
    /// instance root stays unpopulated. Enabling controllers under a
    /// populated root makes the kernel force-convert the cgroup to a
    /// threaded subtree, which carries only cpu — memory would stay
    /// permanently unusable.
    MigrateSelf { cgroup: String, pid: u32 },
    /// Enable a controller in a cgroup's `cgroup.subtree_control`.
    EnableController { cgroup: String, controller: String },
    /// Give the current user ownership of a cgroup directory so the
    /// `timem.jobs` subtree can be created without root.
    MakeWritable { cgroup: String, uid: u32, gid: u32 },
}

/// Probe whether each cap dimension is usable right now without root.
pub fn probe_instance_resource_caps() -> ResourceCapsReadiness {
    platform_probe_instance_resource_caps()
}

/// True when the current instance root cannot structurally carry memory
/// controllers: it is shared with unrelated processes or already sits in a
/// threaded/invalid cgroup. Platforms without a cgroup-v2 instance scope
/// report false because there is nothing to relocate into.
pub fn instance_scope_needs_isolation() -> bool {
    platform_instance_scope_needs_isolation()
}

/// Replace this process with a re-launch inside a transient, delegated
/// `systemd-run --user --scope` unit. Returns only on failure; callers keep
/// running in the current scope in that case.
pub fn relaunch_in_private_user_scope(program: &Path, args: &[OsString]) -> std::io::Result<()> {
    platform_relaunch_in_private_user_scope(program, args)
}

/// Execute a root-authorized escalation plan via `sudo -S`. The password is
/// streamed to sudo's stdin and is never logged, stored, or embedded in
/// errors; clearing the caller's buffer stays the caller's duty.
pub fn apply_resource_cap_escalation_plan(
    actions: &[ResourceCapEscalationAction],
    password: &[u8],
) -> Result<(), String> {
    platform_apply_resource_cap_escalation_plan(actions, password)
}

#[cfg(target_os = "linux")]
fn platform_probe_instance_resource_caps() -> ResourceCapsReadiness {
    crate::linux::probe_instance_resource_caps()
}

#[cfg(target_os = "linux")]
fn platform_instance_scope_needs_isolation() -> bool {
    crate::linux::instance_scope_needs_isolation()
}

#[cfg(target_os = "linux")]
fn platform_relaunch_in_private_user_scope(
    program: &Path,
    args: &[OsString],
) -> std::io::Result<()> {
    crate::linux::relaunch_in_private_user_scope(program, args)
}

#[cfg(not(target_os = "linux"))]
fn platform_instance_scope_needs_isolation() -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
fn platform_relaunch_in_private_user_scope(
    _program: &Path,
    _args: &[OsString],
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "independent user scopes are only supported on Linux",
    ))
}

#[cfg(target_os = "linux")]
fn platform_apply_resource_cap_escalation_plan(
    actions: &[ResourceCapEscalationAction],
    password: &[u8],
) -> Result<(), String> {
    crate::linux::apply_resource_cap_escalation_plan(actions, password)
}

#[cfg(not(target_os = "linux"))]
fn platform_probe_instance_resource_caps() -> ResourceCapsReadiness {
    ResourceCapsReadiness {
        scope: String::new(),
        cpu: ResourceCapReadiness::Unsupported,
        mem: ResourceCapReadiness::Unsupported,
    }
}

#[cfg(not(target_os = "linux"))]
fn platform_apply_resource_cap_escalation_plan(
    _actions: &[ResourceCapEscalationAction],
    _password: &[u8],
) -> Result<(), String> {
    Err("root escalation is not supported on this platform yet".to_string())
}
