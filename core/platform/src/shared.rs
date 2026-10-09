pub(super) fn local_time(secs: libc::time_t) -> Option<libc::tm> {
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let ptr = unsafe { libc::localtime_r(&secs, tm.as_mut_ptr()) };
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { tm.assume_init() })
    }
}

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::process::{Command, ExitStatus};
use std::sync::{Mutex, OnceLock};

const MAX_MANAGED_CHILD_PIDS: usize = 4096;
const ORPHAN_STABLE_OBSERVATIONS: u8 = 4;
const MAX_ORPHAN_PROCESS_EVENTS: usize = 256;

#[derive(Default)]
struct ManagedChildRegistry {
    pids: HashMap<u32, usize>,
    overflowed_registrations: usize,
}

static MANAGED_CHILDREN: OnceLock<Mutex<ManagedChildRegistry>> = OnceLock::new();
static ORPHAN_PROCESS_EVENTS: OnceLock<Mutex<VecDeque<OrphanProcessEvent>>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OrphanProcessEvent {
    pub pid: u32,
    pub process_name: String,
    pub state: &'static str,
}

fn orphan_process_events() -> &'static Mutex<VecDeque<OrphanProcessEvent>> {
    ORPHAN_PROCESS_EVENTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn publish_orphan_process_event(event: OrphanProcessEvent) {
    let Ok(mut events) = orphan_process_events().lock() else {
        return;
    };
    if events.len() >= MAX_ORPHAN_PROCESS_EVENTS {
        events.pop_front();
    }
    events.push_back(event);
}

pub(super) fn take_orphan_process_events() -> Vec<OrphanProcessEvent> {
    orphan_process_events()
        .lock()
        .map(|mut events| events.drain(..).collect())
        .unwrap_or_default()
}

fn managed_children() -> &'static Mutex<ManagedChildRegistry> {
    MANAGED_CHILDREN.get_or_init(|| Mutex::new(ManagedChildRegistry::default()))
}

/// RAII ownership marker for a direct child whose `Child` owner exclusively
/// owns its exit status. The global orphan reaper must never wait on it.
pub(super) struct ManagedChildRegistration {
    pid: u32,
    tracked: bool,
}

impl Drop for ManagedChildRegistration {
    fn drop(&mut self) {
        let Ok(mut registry) = managed_children().lock() else {
            return;
        };
        if self.tracked {
            if let Some(refs) = registry.pids.get_mut(&self.pid) {
                *refs = refs.saturating_sub(1);
                if *refs == 0 {
                    registry.pids.remove(&self.pid);
                }
            }
        } else {
            registry.overflowed_registrations = registry.overflowed_registrations.saturating_sub(1);
        }
    }
}

pub(super) fn register_managed_child(pid: u32) -> ManagedChildRegistration {
    let Ok(mut registry) = managed_children().lock() else {
        // A poisoned registry must fail closed: keep the reaper disabled via
        // an untracked guard rather than risk consuming a managed exit status.
        return ManagedChildRegistration {
            pid,
            tracked: false,
        };
    };
    if let Some(refs) = registry.pids.get_mut(&pid) {
        *refs = refs.saturating_add(1);
        return ManagedChildRegistration { pid, tracked: true };
    }
    if registry.pids.len() >= MAX_MANAGED_CHILD_PIDS {
        registry.overflowed_registrations = registry.overflowed_registrations.saturating_add(1);
        return ManagedChildRegistration {
            pid,
            tracked: false,
        };
    }
    registry.pids.insert(pid, 1);
    ManagedChildRegistration { pid, tracked: true }
}

fn managed_child_reaping_is_safe(pid: u32) -> bool {
    managed_children()
        .lock()
        .map(|registry| registry.overflowed_registrations == 0 && !registry.pids.contains_key(&pid))
        .unwrap_or(false)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DirectChildIdentity {
    pid: u32,
    start_ticks: String,
    process_name: String,
    zombie: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FallbackProcessSnapshot {
    pub pid: u32,
    pub process_name: String,
    pub zombie: bool,
}

#[derive(Clone, Debug)]
struct ActiveFallbackProcess {
    process_name: String,
    zombie: bool,
}

static ACTIVE_FALLBACK_PROCESSES: OnceLock<Mutex<HashMap<(u32, String), ActiveFallbackProcess>>> =
    OnceLock::new();

fn active_fallback_processes() -> &'static Mutex<HashMap<(u32, String), ActiveFallbackProcess>> {
    ACTIVE_FALLBACK_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn fallback_process_snapshots() -> Vec<FallbackProcessSnapshot> {
    active_fallback_processes()
        .lock()
        .map(|active| {
            let mut snapshots = active
                .iter()
                .map(|((pid, _), process)| FallbackProcessSnapshot {
                    pid: *pid,
                    process_name: process.process_name.clone(),
                    zombie: process.zombie,
                })
                .collect::<Vec<_>>();
            snapshots.sort_by_key(|snapshot| snapshot.pid);
            snapshots
        })
        .unwrap_or_default()
}

/// Runtime-wide fallback supervisor for descendants whose original parent
/// exited. Registered direct children remain exclusively owned by their
/// `Child` supervisor. Unregistered children must be observed repeatedly
/// before adoption, closing the spawn-to-registration race.
pub struct FallbackProcessReaper {
    observations: HashMap<(u32, String), u8>,
}

impl FallbackProcessReaper {
    pub(super) fn new() -> Self {
        Self {
            observations: HashMap::new(),
        }
    }

    pub(super) fn reap_once(&mut self) -> usize {
        let children = direct_child_identities();
        let current = children
            .iter()
            .map(|child| (child.pid, child.start_ticks.clone()))
            .collect::<HashSet<_>>();
        self.observations
            .retain(|identity, _| current.contains(identity));
        if let Ok(mut active) = active_fallback_processes().lock() {
            active.retain(|identity, _| current.contains(identity));
        }

        let mut reaped = 0;
        for child in children {
            let identity = (child.pid, child.start_ticks.clone());
            if !managed_child_reaping_is_safe(child.pid) {
                self.observations.remove(&identity);
                if let Ok(mut active) = active_fallback_processes().lock() {
                    active.remove(&identity);
                }
                continue;
            }
            if self.observations.len() >= MAX_MANAGED_CHILD_PIDS
                && !self.observations.contains_key(&identity)
            {
                continue;
            }
            let count = self.observations.entry(identity.clone()).or_default();
            *count = count.saturating_add(1);
            if *count < ORPHAN_STABLE_OBSERVATIONS {
                continue;
            }

            let newly_adopted = active_fallback_processes()
                .lock()
                .map(|mut active| {
                    if active.len() >= MAX_MANAGED_CHILD_PIDS && !active.contains_key(&identity) {
                        return false;
                    }
                    active
                        .insert(
                            identity.clone(),
                            ActiveFallbackProcess {
                                process_name: child.process_name.clone(),
                                zombie: child.zombie,
                            },
                        )
                        .is_none()
                })
                .unwrap_or(false);
            if newly_adopted {
                publish_orphan_process_event(OrphanProcessEvent {
                    pid: child.pid,
                    process_name: child.process_name.clone(),
                    state: "adopted",
                });
            }
            if !child.zombie {
                continue;
            }

            let rc = unsafe {
                libc::waitpid(
                    child.pid as libc::pid_t,
                    std::ptr::null_mut(),
                    libc::WNOHANG,
                )
            };
            if rc == child.pid as libc::pid_t {
                publish_orphan_process_event(OrphanProcessEvent {
                    pid: child.pid,
                    process_name: child.process_name,
                    state: "reaped",
                });
                self.observations.remove(&identity);
                if let Ok(mut active) = active_fallback_processes().lock() {
                    active.remove(&identity);
                }
                reaped += 1;
            } else if rc < 0 {
                self.observations.remove(&identity);
                if let Ok(mut active) = active_fallback_processes().lock() {
                    active.remove(&identity);
                }
            }
        }
        reaped
    }
}

fn direct_child_identities() -> Vec<DirectChildIdentity> {
    #[cfg(target_os = "linux")]
    {
        let self_pid = std::process::id();
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return out;
        };
        for entry in dir.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(pid) = name.parse::<u32>() else {
                continue;
            };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some((head, rest)) = stat.rsplit_once(") ") else {
                continue;
            };
            let process_name = head
                .split_once(" (")
                .map(|(_, name)| name)
                .unwrap_or("unknown")
                .to_string();
            let fields = rest.split_whitespace().collect::<Vec<_>>();
            let (Some(state), Some(ppid), Some(start_ticks)) =
                (fields.first(), fields.get(1), fields.get(19))
            else {
                continue;
            };
            if ppid
                .parse::<u32>()
                .ok()
                .is_none_or(|ppid| !parent_belongs_to_this_process(ppid, self_pid))
            {
                continue;
            }
            out.push(DirectChildIdentity {
                pid,
                start_ticks: (*start_ticks).to_string(),
                process_name,
                zombie: *state == "Z",
            });
        }
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

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
#[cfg(target_os = "linux")]
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
    if let Ok(output) = crate::api::command_output(
        std::process::Command::new("/bin/ps")
            .args(["-o", "stat=", "-p"])
            .arg(pid.to_string()),
    ) {
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

/// libproc returns a PID count, although its buffer size argument is bytes.
/// A full buffer may be truncated; retry with bounded growth, never certify
/// an incomplete snapshot as an empty group.
#[cfg(target_os = "macos")]
fn macos_pid_snapshot(mut list: impl FnMut(&mut [libc::pid_t]) -> i32) -> Option<Vec<libc::pid_t>> {
    const MAX_PIDS: usize = 1_048_576;
    let estimate = list(&mut []);
    if estimate <= 0 {
        return None;
    }
    let mut capacity = (estimate as usize).checked_add(32)?;
    for _ in 0..3 {
        if capacity > MAX_PIDS {
            return None;
        }
        let mut buffer = vec![0; capacity];
        let written = list(&mut buffer);
        if written <= 0 {
            return None;
        }
        if (written as usize) < capacity {
            buffer.truncate(written as usize);
            return Some(buffer);
        }
        capacity = capacity.checked_mul(2)?;
    }
    None
}

/// macOS live (non-zombie) group members. Unknown query failures must not
/// turn a live group into an apparently completed job.
#[cfg(target_os = "macos")]
fn macos_live_process_group_member_pids(group_leader_pid: u32) -> Option<Vec<u32>> {
    // Query only this group: unrelated protected system processes may deny
    // PROC_PIDTBSDINFO even when every member of our group is queryable.
    let buffer = macos_pid_snapshot(|buffer| unsafe {
        libc::proc_listpgrppids(
            group_leader_pid as libc::pid_t,
            if buffer.is_empty() {
                std::ptr::null_mut()
            } else {
                buffer.as_mut_ptr().cast()
            },
            std::mem::size_of_val(buffer) as libc::c_int,
        )
    })?;
    let info_size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let mut members = Vec::new();
    for pid in buffer {
        if pid <= 1 {
            continue;
        }
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                info_size,
            )
        };
        if read != info_size {
            if read <= 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                // Exited between enumeration and query.
                continue;
            }
            return None;
        }
        let info = unsafe { info.assume_init() };
        if info.pbi_pgid == group_leader_pid && info.pbi_status != libc::SZOMB {
            members.push(pid as u32);
        }
    }
    members.sort_unstable();
    Some(members)
}

#[cfg(all(test, target_os = "macos"))]
#[path = "../tests/unit/macos_pid_snapshot_tests.rs"]
mod macos_pid_snapshot_tests;

#[cfg(target_os = "macos")]
pub(super) fn list_live_process_group_members(group_leader_pid: u32) -> Vec<u32> {
    if group_leader_pid <= 1 || group_leader_pid as libc::pid_t == unsafe { libc::getpgrp() } {
        return Vec::new();
    }
    macos_live_process_group_member_pids(group_leader_pid).unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub(super) fn process_group_running(group_leader_pid: u32) -> bool {
    if group_leader_pid <= 1 || group_leader_pid as libc::pid_t == unsafe { libc::getpgrp() } {
        return false;
    }
    // kill(-pgid, 0) works on Darwin; a group whose only member is a zombie
    // leader still reports success, so confirm via live-member enumeration.
    let result = unsafe { libc::kill(-(group_leader_pid as libc::pid_t), 0) };
    if !(result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)) {
        return false;
    }
    match macos_live_process_group_member_pids(group_leader_pid) {
        Some(members) => !members.is_empty(),
        // Enumeration failed but kill() saw a live group: fail closed and
        // report running so the terminate path escalates to SIGKILL.
        None => true,
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
/// caller-owned getfsstat snapshots (covers / and mounted /Volumes/* disks);
/// pseudo mount types (devfs, autofs, nullfs, ...) are excluded by the
/// supported-type list.
#[cfg(target_os = "macos")]
pub(super) fn local_filesystem_mount_points() -> Vec<std::path::PathBuf> {
    // "/" covers the internal APFS container free space (system and Data
    // volumes share one container); "/Volumes/*" are real attached disks.
    // Other "/System/Volumes/*" entries (VM swap, Preboot, Update, xarts,
    // Hardware, ...) are system infrastructure whose size fluctuations would
    // masquerade as user-data disk pressure, so they are excluded by path
    // after the type filter.
    let supported = [
        "apfs", "hfs", "hfsplus", "msdos", "exfat", "ntfs", "udf", "nfs",
    ];
    let mut out = Vec::new();
    // getmntinfo returns process-global mutable storage. Concurrent Session
    // initialization can invalidate it while another thread reads a mount.
    // getfsstat writes into this call's owned buffer instead.
    for entry in macos_mount_snapshot() {
        let Some(fstype) = macos_mount_field(&entry.f_fstypename) else {
            continue;
        };
        if !supported.contains(&fstype) {
            continue;
        }
        let Some(mount) = macos_mount_field(&entry.f_mntonname) else {
            continue;
        };
        if mount == "/" {
            out.push(std::path::PathBuf::from(mount));
            continue;
        }
        if let Some(rest) = mount.strip_prefix("/Volumes/") {
            if !rest.is_empty() && !rest.contains('/') {
                out.push(std::path::PathBuf::from(mount));
            }
        }
    }
    out
}

// A full buffer may mean mounts appeared between sizing and collection.
// Retry with bounded headroom; never expose a partial topology as a snapshot.
#[cfg(target_os = "macos")]
fn macos_mount_snapshot() -> Vec<libc::statfs> {
    const MAX_MOUNTS: usize = 16_384;
    let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if count <= 0 {
        return Vec::new();
    }
    let mut capacity = (count as usize).saturating_add(16);
    for _ in 0..3 {
        if capacity > MAX_MOUNTS {
            return Vec::new();
        }
        let mut mounts = Vec::with_capacity(capacity);
        mounts.resize_with(capacity, || unsafe { std::mem::zeroed::<libc::statfs>() });
        let bytes = (capacity * std::mem::size_of::<libc::statfs>()) as libc::c_int;
        let read = unsafe { libc::getfsstat(mounts.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
        if read < 0 {
            return Vec::new();
        }
        if (read as usize) < capacity {
            mounts.truncate(read as usize);
            return mounts;
        }
        capacity = capacity.saturating_mul(2);
    }
    Vec::new()
}

#[cfg(target_os = "macos")]
fn macos_mount_field<const N: usize>(field: &[libc::c_char; N]) -> Option<&str> {
    // Bound the scan by the kernel structure's array, even for malformed data.
    let bytes = unsafe { std::slice::from_raw_parts(field.as_ptr().cast::<u8>(), N) };
    std::ffi::CStr::from_bytes_until_nul(bytes)
        .ok()?
        .to_str()
        .ok()
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

// statvfs field widths differ across Unix targets.
#[allow(clippy::unnecessary_cast)]
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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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
