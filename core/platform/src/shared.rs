pub(super) fn local_time(secs: libc::time_t) -> Option<libc::tm> {
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let ptr = unsafe { libc::localtime_r(&secs, tm.as_mut_ptr()) };
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { tm.assume_init() })
    }
}

use std::io::Read;
use std::process::{Command, ExitStatus};

pub(super) fn configure_private_file_options(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

pub(super) fn open_diagnostic_file_lease(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        return Err(if error.kind() == std::io::ErrorKind::WouldBlock {
            error
        } else {
            std::io::Error::new(error.kind(), format!("file_lease_lock_failed:{error}"))
        });
    }
    Ok(file)
}

pub(super) fn fill_secure_random(bytes: &mut [u8]) -> std::io::Result<()> {
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

pub(super) fn configure_sanitized_child_environment(command: &mut Command) {
    command.env("TMPDIR", std::env::temp_dir());
}

/// Install this process as a child subreaper so orphaned descendants are
/// reparented to the runtime instead of init (Linux; best effort elsewhere).
pub(super) fn install_process_subreaper() -> bool {
    #[cfg(target_os = "linux")]
    {
        let rc = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1usize, 0, 0, 0) };
        rc == 0
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Direct children of this process that live in a different session. After
/// `install_process_subreaper`, an orphan that escaped via `setsid` is
/// reparented here while keeping its own session, which distinguishes it
/// from ordinary managed children that share the runtime's session.
pub(super) fn reparented_detached_child_pids() -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        let self_pid = std::process::id();
        let self_sid = unsafe { libc::getsid(0) };
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return out;
        };
        for entry in dir.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Ok(pid) = name.parse::<u32>() else {
                continue;
            };
            if pid == self_pid {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            // Layout: pid (comm) state ppid pgrp session ...; `comm` may
            // contain spaces, so parse after the last ')'.
            let Some(rest) = stat.rsplit(')').next() else {
                continue;
            };
            let mut fields = rest.split_whitespace();
            // A zombie keeps its ppid/session but is already dead; it must not
            // be reported (the kernel reclaims it when the runtime exits).
            if fields.next() != Some("Z") {
                let (Some(ppid), Some(_pgrp), Some(session)) =
                    (fields.next(), fields.next(), fields.next())
                else {
                    continue;
                };
                if let (Ok(ppid), Ok(session)) = (ppid.parse::<u32>(), session.parse::<i32>()) {
                    if session != self_sid && parent_belongs_to_this_process(ppid, self_pid) {
                        out.push(pid);
                    }
                }
            }
        }
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// A multithreaded subreaper process gets escaped orphans reparented under
/// an arbitrary live thread TID of the process, not the leader PID, so a
/// parent pid matches when it is either the leader or one of our threads.
fn parent_belongs_to_this_process(ppid: u32, self_pid: u32) -> bool {
    if ppid == self_pid {
        return true;
    }
    std::fs::read_to_string(format!("/proc/{ppid}/status"))
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Tgid:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|tgid| tgid.parse::<u32>().ok())
        })
        == Some(self_pid)
}

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

/// One non-blocking reap attempt of any dead child (biological or
/// subreaper-adopted). Returns true when the child was reaped or is not our
/// child at all; false when it is still terminating. A subreaper runtime must
/// reap adopted orphans or they stay zombies forever.
pub(super) fn try_reap_child_process(pid: u32) -> bool {
    if pid <= 1 {
        return true;
    }
    let rc = unsafe { libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), libc::WNOHANG) };
    rc != 0
}

pub(super) fn reap_child_process(pid: u32) {
    let pid = pid as libc::pid_t;
    if pid <= 1 {
        return;
    }
    // Reap if it is our child. rc == pid means reaped; rc < 0 (ECHILD)
    // means it is gone already; rc == 0 means termination is still in
    // flight, so retry briefly.
    for _ in 0..50 {
        let rc = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        if rc != 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
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
    // kill(-pgid, 0) reports success for a group whose only member is a
    // zombie leader. Under the subreaper flag such zombies are reparented
    // here and never reaped by init, so they must not keep the group alive.
    let result = unsafe { libc::kill(-(group_leader_pid as libc::pid_t), 0) };
    if !(result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)) {
        return false;
    }
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{name}/stat")) else {
            continue;
        };
        let Some(rest) = stat.rsplit(')').next() else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let state = fields.next().unwrap_or("");
        let Some(pgrp) = fields.nth(1) else { continue };
        if pgrp.parse::<u32>() == Ok(group_leader_pid) && state != "Z" {
            return true;
        }
    }
    false
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
