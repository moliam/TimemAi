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

/// Lists live (non-zombie) members of the process group led by
/// `group_leader_pid`. Used after a tracked job exits to surface leftover
/// descendants (orphans) to the model instead of leaving them invisible.
/// Filesystem usage snapshot for the filesystem containing `path`.
/// Returns (total_bytes, free_bytes). Unix uses statvfs; used = total - free.
#[cfg(unix)]
/// Local real filesystem mount points from /proc/mounts (ext4, xfs, btrfs,
/// vfat, ntfs, f2fs, zfs). Pseudo filesystems (proc, sysfs, tmpfs, devpts,
/// overlays, snap loops) are excluded. Used so disk sampling covers writes
/// to any data disk, not only the working directory.
#[cfg(target_os = "linux")]
pub(super) fn local_filesystem_mount_points() -> Vec<std::path::PathBuf> {
    let supported = [
        "ext4", "ext3", "ext2", "xfs", "btrfs", "vfat", "exfat", "ntfs", "ntfs3", "f2fs", "zfs",
        "apfs", "hfsplus",
    ];
    let mut out = Vec::new();
    let Ok(content) = std::fs::read_to_string("/proc/mounts") else {
        return out;
    };
    for line in content.lines() {
        let mut fields = line.split_whitespace();
        let (Some(_dev), Some(mount), Some(fstype)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if supported.contains(&fstype) {
            out.push(std::path::PathBuf::from(mount));
        }
    }
    out
}

/// Local real filesystem mount points on macOS: APFS/HFS+ volumes from
/// TODO(platform/macos): 待实现验证——此实现基于 getmntinfo 的静态核对，
/// 尚未在真实 macOS 上编译与运行测试；到 macOS 平台开发时需补单元测试
/// （/ 与 /Volumes/* 出现，devfs/autofs 排除）并按实际行为修正。
/// getmntinfo (covers /, /System/Volumes/* and mounted /Volumes/* disks);
/// pseudo mount types (devfs, autofs, nullfs, ...) are excluded.
#[cfg(target_os = "macos")]
pub(super) fn local_filesystem_mount_points() -> Vec<std::path::PathBuf> {
    let supported = [
        "apfs", "hfs", "hfsplus", "msdos", "exfat", "ntfs", "udf", "nfs",
    ];
    let mut out = Vec::new();
    let mut mounts: *mut libc::statfs = std::ptr::null_mut();
    let count = unsafe { libc::getmntinfo(&mut mounts, libc::MNT_NOWAIT) };
    if count <= 0 {
        return out;
    }
    for index in 0..count as usize {
        let entry = unsafe { &*mounts.add(index) };
        let fstype = unsafe { std::ffi::CStr::from_ptr(entry.fstype.as_ptr().cast()) }
            .to_string_lossy()
            .to_string();
        if supported.contains(&fstype.as_str()) {
            let mount = unsafe { std::ffi::CStr::from_ptr(entry.f_mntonname.as_ptr().cast()) };
            if let Ok(mount) = mount.to_str() {
                out.push(std::path::PathBuf::from(mount));
            }
        }
    }
    out
}

/// Local real filesystem mount points on other Unix systems: the root
/// filesystem is always reported (best-effort baseline).
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(super) fn local_filesystem_mount_points() -> Vec<std::path::PathBuf> {
    vec![std::path::PathBuf::from("/")]
}

/// Local real filesystem mount points (Windows reports fixed drives).
#[cfg(windows)]
pub(super) fn local_filesystem_mount_points() -> Vec<std::path::PathBuf> {
    crate::windows::local_filesystem_mount_points()
}

/// Stable device identifier of the filesystem containing `path`, used to
/// deduplicate multiple paths on the same disk. Unix uses stat's st_dev.
#[cfg(unix)]
pub(super) fn filesystem_device_id(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|meta| meta.dev())
}

/// Stable device identifier of the filesystem containing `path`.
#[cfg(windows)]
pub(super) fn filesystem_device_id(path: &std::path::Path) -> Option<u64> {
    crate::windows::filesystem_device_id(path)
}

pub(super) fn filesystem_usage_bytes(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let c_path = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    let total = (stat.f_blocks as u64).saturating_mul(stat.f_frsize as u64);
    let free = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
    Some((total, free))
}

/// Filesystem usage snapshot for the filesystem containing `path`.
/// Returns (total_bytes, free_bytes).
#[cfg(windows)]
pub(super) fn filesystem_usage_bytes(path: &std::path::Path) -> Option<(u64, u64)> {
    crate::windows::filesystem_usage_bytes(path)
}

pub(super) fn list_live_process_group_members(group_leader_pid: u32) -> Vec<u32> {
    let mut members = Vec::new();
    if group_leader_pid <= 1 || group_leader_pid as libc::pid_t == unsafe { libc::getpgrp() } {
        return members;
    }
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return members;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
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
            members.push(pid);
        }
    }
    members.sort_unstable();
    members
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
