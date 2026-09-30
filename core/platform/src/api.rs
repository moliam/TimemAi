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

/// Install this process as a child subreaper (Linux). Returns whether the
/// platform supports and applied the flag. Called once at runtime startup so
/// orphaned descendants are reparented to the runtime instead of init.
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

/// Drains bounded, not-yet-reported adoption and terminal events from the
/// runtime fallback process supervisor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackProcessSnapshot {
    pub pid: u32,
    pub process_name: String,
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
        })
        .collect();
    #[cfg(not(unix))]
    Vec::new()
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
    crate::shared::local_filesystem_mount_points()
}

/// Stable device identifier of the filesystem containing `path`, used to
/// deduplicate paths on the same disk.
pub fn filesystem_device_id(path: &std::path::Path) -> Option<u64> {
    crate::shared::filesystem_device_id(path)
}

/// Filesystem usage for the filesystem containing `path`:
/// (total_bytes, free_bytes). None when the stat call fails.
pub fn filesystem_usage_bytes(path: &std::path::Path) -> Option<(u64, u64)> {
    crate::shared::filesystem_usage_bytes(path)
}

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
