use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::OnceLock;

pub const BASH_EXECUTABLE: &str = "/bin/bash";
pub const POSIX_SHELL_EXECUTABLE: &str = "/bin/sh";

static HOST_ENVIRONMENT: OnceLock<String> = OnceLock::new();

pub fn local_command_execution_available() -> bool {
    Path::new(BASH_EXECUTABLE).is_file()
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

pub fn configure_child_process_group(command: &mut Command) {
    #[cfg(unix)]
    crate::shared::configure_child_process_group(command);
    #[cfg(not(unix))]
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
    #[cfg(not(unix))]
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
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

pub fn child_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    return crate::shared::child_process_running(pid);
    #[cfg(not(unix))]
    {
        process_running(pid)
    }
}

pub fn is_runtime_child_process_group(pid: u32) -> bool {
    #[cfg(unix)]
    return crate::shared::is_runtime_child_process_group(pid);
    #[cfg(not(unix))]
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
    #[cfg(not(unix))]
    let _ = pid;
}

pub fn terminate_process_group(group_leader_pid: u32) {
    #[cfg(unix)]
    crate::shared::terminate_process_group(group_leader_pid);
    #[cfg(not(unix))]
    let _ = group_leader_pid;
}

pub fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    crate::shared::kill_process_group(pid);
    #[cfg(not(unix))]
    let _ = pid;
}

pub fn process_group_running(group_leader_pid: u32) -> bool {
    #[cfg(unix)]
    return crate::shared::process_group_running(group_leader_pid);
    #[cfg(not(unix))]
    {
        process_running(group_leader_pid)
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
    let output = Command::new(program).args(args).output().ok()?;
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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_graphical_session_available() -> bool {
    false
}
