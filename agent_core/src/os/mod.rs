use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::OnceLock;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

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
    #[cfg(target_os = "macos")]
    macos::configure_child_process_group(command);
    #[cfg(target_os = "linux")]
    linux::configure_child_process_group(command);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = command;
}

pub fn exit_signal(status: &ExitStatus) -> Option<i32> {
    #[cfg(target_os = "macos")]
    return macos::exit_signal(status);
    #[cfg(target_os = "linux")]
    return linux::exit_signal(status);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = status;
        None
    }
}

pub fn process_is_alive(pid: u64) -> Option<bool> {
    #[cfg(target_os = "macos")]
    return macos::process_is_alive(pid);
    #[cfg(target_os = "linux")]
    return linux::process_is_alive(pid);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

pub fn process_running(pid: u32) -> bool {
    process_is_alive(u64::from(pid)).unwrap_or(false)
}

/// Returns a kernel-derived identity that changes when an operating-system PID
/// is reused. Callers must treat `None` as "identity unavailable", not as a
/// positive match.
pub fn process_identity(pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    return macos::process_identity(pid);
    #[cfg(target_os = "linux")]
    return linux::process_identity(pid);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

pub fn child_process_running(pid: u32) -> bool {
    #[cfg(target_os = "macos")]
    return macos::child_process_running(pid);
    #[cfg(target_os = "linux")]
    return linux::child_process_running(pid);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        process_running(pid)
    }
}

pub fn is_runtime_child_process_group(pid: u32) -> bool {
    #[cfg(target_os = "macos")]
    return macos::is_runtime_child_process_group(pid);
    #[cfg(target_os = "linux")]
    return linux::is_runtime_child_process_group(pid);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        pid > 1 && pid != std::process::id()
    }
}

pub fn runtime_child_pid_kind() -> &'static str {
    #[cfg(target_os = "macos")]
    return macos::runtime_child_pid_kind();
    #[cfg(target_os = "linux")]
    return linux::runtime_child_pid_kind();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "runtime_child_process"
    }
}

pub fn terminate_process(pid: u32) {
    #[cfg(target_os = "macos")]
    macos::terminate_process(pid);
    #[cfg(target_os = "linux")]
    linux::terminate_process(pid);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = pid;
}

pub fn kill_process_group(pid: u32) {
    #[cfg(target_os = "macos")]
    macos::kill_process_group(pid);
    #[cfg(target_os = "linux")]
    linux::kill_process_group(pid);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = pid;
}

pub fn process_group_running(group_leader_pid: u32) -> bool {
    #[cfg(target_os = "macos")]
    return macos::process_group_running(group_leader_pid);
    #[cfg(target_os = "linux")]
    return linux::process_group_running(group_leader_pid);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
    macos::version()
}

#[cfg(target_os = "linux")]
fn platform_version() -> Option<String> {
    linux::version()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_version() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn platform_config_root(_xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    macos::config_root(home)
}

#[cfg(target_os = "linux")]
fn platform_config_root(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    linux::config_root(xdg, home)
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
    Some(macos::browser_command(url))
}

#[cfg(target_os = "linux")]
fn platform_browser_command(url: &str) -> Option<(OsString, Vec<OsString>)> {
    Some(linux::browser_command(url))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_browser_command(_url: &str) -> Option<(OsString, Vec<OsString>)> {
    None
}

#[cfg(target_os = "macos")]
fn platform_terminal_command(path: &Path) -> Option<(OsString, Vec<OsString>)> {
    Some(macos::terminal_command(path))
}

#[cfg(target_os = "linux")]
fn platform_terminal_command(path: &Path) -> Option<(OsString, Vec<OsString>)> {
    Some(linux::terminal_command(path))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_terminal_command(_path: &Path) -> Option<(OsString, Vec<OsString>)> {
    None
}

#[cfg(target_os = "macos")]
fn platform_graphical_session_available() -> bool {
    macos::graphical_session_available()
}

#[cfg(target_os = "linux")]
fn platform_graphical_session_available() -> bool {
    linux::graphical_session_available()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_graphical_session_available() -> bool {
    false
}

#[cfg(test)]
#[path = "../../tests/unit/os_tests.rs"]
mod tests;
