use std::process::{Command, ExitStatus};

pub(super) fn configure_child_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

pub(super) fn exit_signal(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

pub(super) fn process_is_alive(pid: u64) -> Option<bool> {
    let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return Some(true);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Some(false),
        Some(libc::EPERM) => Some(true),
        _ => None,
    }
}

pub(super) fn child_process_running(pid: u32) -> bool {
    let mut status = 0;
    let wait = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
    if wait == pid as libc::pid_t {
        return false;
    }
    if wait == 0 {
        return true;
    }
    if let Ok(output) = std::process::Command::new("/bin/ps")
        .args(["-o", "stat=", "-p"])
        .arg(pid.to_string())
        .output()
    {
        if !output.status.success() {
            return false;
        }
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        return !state.is_empty() && !state.contains('Z');
    }
    process_is_alive(u64::from(pid)).unwrap_or(false)
}

pub(super) fn is_runtime_child_process_group(pid: u32) -> bool {
    if pid <= 1 || pid == std::process::id() {
        return false;
    }
    let pid = pid as libc::pid_t;
    let pgid = unsafe { libc::getpgid(pid) };
    pgid == pid && pgid != unsafe { libc::getpgrp() }
}

pub(super) fn runtime_child_pid_kind() -> &'static str {
    "runtime_child_process_group"
}

pub(super) fn terminate_process(pid: u32) {
    let pid = pid as libc::pid_t;
    let pgid = unsafe { libc::getpgid(pid) };
    if pgid < 0 {
        return;
    }
    if pgid == pid && pgid != unsafe { libc::getpgrp() } {
        terminate_process_group(pgid as u32);
        return;
    }
    signal_process(pid, libc::SIGTERM);
    std::thread::sleep(std::time::Duration::from_millis(100));
    if process_is_alive(pid as u64).unwrap_or(false) {
        signal_process(pid, libc::SIGKILL);
    }
}

pub(super) fn terminate_process_group(group_leader_pid: u32) {
    let pgid = group_leader_pid as libc::pid_t;
    if pgid <= 1 || pgid == unsafe { libc::getpgrp() } {
        return;
    }
    signal_process_group(pgid, libc::SIGTERM);
    std::thread::sleep(std::time::Duration::from_millis(100));
    if process_group_running(group_leader_pid) {
        signal_process_group(pgid, libc::SIGKILL);
    }
}

pub(super) fn kill_process_group(pid: u32) {
    let pid = pid as libc::pid_t;
    if pid > 1 && pid != unsafe { libc::getpgrp() } {
        let _ = unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
}

pub(super) fn process_group_running(group_leader_pid: u32) -> bool {
    if group_leader_pid <= 1 || group_leader_pid as libc::pid_t == unsafe { libc::getpgrp() } {
        return false;
    }
    let result = unsafe { libc::kill(-(group_leader_pid as libc::pid_t), 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn signal_process(pid: libc::pid_t, signal: libc::c_int) {
    if pid > 1 && pid != unsafe { libc::getpid() } {
        let _ = unsafe { libc::kill(pid, signal) };
    }
}

fn signal_process_group(pgid: libc::pid_t, signal: libc::c_int) {
    if pgid > 1 && pgid != unsafe { libc::getpgrp() } {
        let _ = unsafe { libc::kill(-pgid, signal) };
    }
}
