use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn version() -> Option<String> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    for key in ["PRETTY_NAME", "NAME"] {
        if let Some(value) = os_release_value(&content, key) {
            return Some(value);
        }
    }
    None
}

pub(super) fn config_root(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    if let Some(path) = xdg.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join("timem");
    }
    home.filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("timem"))
        .unwrap_or_else(|| PathBuf::from("/etc/xdg/timem"))
}

pub(super) fn browser_command(url: &str) -> (OsString, Vec<OsString>) {
    (OsString::from("xdg-open"), vec![OsString::from(url)])
}

pub(super) fn terminal_command(path: &Path) -> (OsString, Vec<OsString>) {
    (
        OsString::from("x-terminal-emulator"),
        vec![
            OsString::from("--working-directory"),
            path.as_os_str().to_os_string(),
        ],
    )
}

pub(super) fn graphical_session_available() -> bool {
    ["DISPLAY", "WAYLAND_DISPLAY"]
        .into_iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
}

pub(super) fn os_release_value(content: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    content.lines().find_map(|line| {
        let value = line.strip_prefix(&prefix)?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value)
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
        crate::api::non_empty_one_line(&value)
    })
}

#[allow(dead_code)]
pub(super) fn process_identity(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesized and may contain spaces or `)`, so split
    // only after its final closing parenthesis. Linux proc(5) field 22 is the
    // process start time in clock ticks since boot; after removing pid+comm it
    // is index 19 in the remaining field list beginning with state (field 3).
    let tail = stat.rsplit_once(") ")?.1;
    let start_ticks = tail.split_whitespace().nth(19)?;
    Some(format!("linux-start-ticks:{start_ticks}"))
}

const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const TIMEM_JOB_SUBTREE: &str = "timem.jobs";
const TIMEM_SELF_SUBTREE: &str = "timem.self";
const RUNTIME_SCOPE_PREFIX: &str = "runtime-";
const SESSION_SCOPE_PREFIX: &str = "session-";
static NEXT_JOB_CGROUP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static STALE_SCOPE_CLEANUP: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Linux cgroup-v2 ownership for one managed process job.
///
/// The hierarchy is `timem.jobs/runtime-*/session-*/job-*`. Parent scopes stay
/// empty and are observation/aggregation points; user processes enter only the
/// leaf Job before any user-controlled program executes.
pub(super) struct LinuxCgroupProcessJob {
    path: PathBuf,
    procs: std::fs::File,
}

impl std::fmt::Debug for LinuxCgroupProcessJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxCgroupProcessJob")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl LinuxCgroupProcessJob {
    pub(super) fn create(session_id: Option<&str>) -> std::io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;

        let (_runtime_path, session_path) =
            ensure_aggregate_scope_paths(session_id.unwrap_or("runtime"))?;

        for _ in 0..64 {
            let id = NEXT_JOB_CGROUP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = session_path.join(format!("job-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    let procs = match std::fs::OpenOptions::new()
                        .write(true)
                        .custom_flags(libc::O_CLOEXEC)
                        .open(path.join("cgroup.procs"))
                    {
                        Ok(file) => file,
                        Err(error) => {
                            let _ = fs::remove_dir(&path);
                            return Err(error);
                        }
                    };
                    if !path.join("cgroup.kill").is_file() {
                        let _ = fs::remove_dir(&path);
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Unsupported,
                            "cgroup.kill is unavailable",
                        ));
                    }
                    return Ok(Self { path, procs });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique managed-job cgroup",
        ))
    }
}

impl crate::process_job::ProcessJobBackend for LinuxCgroupProcessJob {
    fn name(&self) -> &'static str {
        "linux_cgroup_v2"
    }

    fn configure_command(&self, command: &mut std::process::Command) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;

        let procs = self.procs.try_clone()?;
        unsafe {
            command.pre_exec(move || {
                let bytes = b"0\n";
                let written = libc::write(procs.as_raw_fd(), bytes.as_ptr().cast(), bytes.len());
                if written == bytes.len() as isize {
                    Ok(())
                } else if written < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "short write to cgroup.procs",
                    ))
                }
            });
        }
        Ok(())
    }

    fn kill_all(&self) -> std::io::Result<()> {
        fs::write(self.path.join("cgroup.kill"), b"1\n")
    }

    fn is_empty(&self) -> std::io::Result<bool> {
        cgroup_is_empty(&self.path)
    }

    fn member_pids(&self) -> std::io::Result<Vec<u32>> {
        cgroup_member_pids(&self.path)
    }

    fn observation_note(&self) -> Option<String> {
        Some(format!("cgroup: {}", self.path.display()))
    }
}

impl Drop for LinuxCgroupProcessJob {
    fn drop(&mut self) {
        // ShellJobManager first signals and joins the supervisor. Removal is
        // therefore a non-destructive finalization step and succeeds only when
        // the kernel says the leaf is empty. Parent scopes are removed only
        // after their final Job disappears.
        let _ = fs::remove_dir(&self.path);
        // Runtime and Session parents are stable aggregate observation scopes.
        // The Runtime safety-net guard removes them on normal shutdown after
        // all Job leaves have been checked and removed.
    }
}

pub(super) fn stale_process_scope_snapshots(
    session_id: &str,
) -> Vec<crate::process_job::StaleProcessScopeSnapshot> {
    let Ok(subtree) = timem_job_subtree() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&subtree) else {
        return Vec::new();
    };
    let session_name = format!("{SESSION_SCOPE_PREFIX}{}", session_scope_key(session_id));
    let mut out = Vec::new();
    for entry in entries.flatten().take(256) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some((owner_pid, owner_start_ticks)) = parse_runtime_scope_name(name) else {
            continue;
        };
        if runtime_owner_matches(owner_pid, owner_start_ticks) {
            continue;
        }
        let runtime_path = entry.path();
        let session_path = runtime_path.join(&session_name);
        if let Ok(false) = session_scope_has_live_members(&session_path) {
            cleanup_empty_session_scope(&session_path);
            let _ = fs::remove_dir(&runtime_path);
            continue;
        }
        out.push(crate::process_job::StaleProcessScopeSnapshot {
            observation_note: format!("cgroup: {}", session_path.display()),
            owner_pid,
        });
    }
    out.sort_by_key(|scope| (scope.owner_pid, scope.observation_note.clone()));
    out
}

fn current_cgroup_mounted_path() -> std::io::Result<PathBuf> {
    let relative = current_cgroup_v2_path()?;
    let mount = Path::new(CGROUP_ROOT);
    let current = mount.join(relative.strip_prefix("/").unwrap_or(&relative));
    let canonical_mount = mount.canonicalize()?;
    let canonical_current = current.canonicalize()?;
    if !canonical_current.starts_with(&canonical_mount) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "current cgroup escapes cgroup-v2 mount",
        ));
    }
    Ok(canonical_current)
}

/// The instance root is the cgroup the `timem.jobs` subtree hangs off of:
/// the main process cgroup itself before self-migration, its parent once the
/// main process lives in the managed `timem.self` sibling.
pub(super) fn instance_root_cgroup(current: &Path) -> &Path {
    if current.file_name().and_then(OsStr::to_str) == Some(TIMEM_SELF_SUBTREE) {
        current.parent().unwrap_or(current)
    } else {
        current
    }
}

fn timem_job_subtree() -> std::io::Result<PathBuf> {
    let canonical_current = current_cgroup_mounted_path()?;
    ensure_plain_directory(&instance_root_cgroup(&canonical_current).join(TIMEM_JOB_SUBTREE))
}

fn ensure_plain_directory(path: &Path) -> std::io::Result<PathBuf> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Timem cgroup scope is not a plain directory",
        ));
    }
    Ok(path.to_path_buf())
}

/// Make `cpu` and `memory` available to children of a job-hierarchy level.
/// Enabling a controller in a parent is what materializes `memory.current` /
/// `memory.events` in its children; the instance root alone is not enough.
/// Best effort: a platform without delegated controllers keeps running jobs
/// (fail-open), but per-Session observation stays unavailable and the host
/// degrades to no resource notices instead of pretending.
fn enable_job_controllers(path: &Path) {
    let control = path.join("cgroup.subtree_control");
    // One write keeps the two controllers atomic from the caller's view. If
    // memory is not delegated, fall back to cpu so command execution keeps
    // its original behavior.
    if fs::write(&control, "+cpu +memory\n").is_err() {
        let _ = fs::write(&control, "+cpu\n");
    }
}

fn current_runtime_scope_name() -> String {
    let start_ticks = linux_start_ticks(std::process::id()).unwrap_or_else(|| "unknown".into());
    format!("{RUNTIME_SCOPE_PREFIX}{}-{start_ticks}", std::process::id())
}

fn parse_runtime_scope_name(name: &str) -> Option<(u32, &str)> {
    let rest = name.strip_prefix(RUNTIME_SCOPE_PREFIX)?;
    let (pid, start_ticks) = rest.split_once('-')?;
    Some((pid.parse().ok()?, start_ticks))
}

fn runtime_owner_matches(pid: u32, expected_start_ticks: &str) -> bool {
    linux_start_ticks(pid).as_deref() == Some(expected_start_ticks)
}

fn linux_start_ticks(pid: u32) -> Option<String> {
    process_identity(pid)?
        .strip_prefix("linux-start-ticks:")
        .map(str::to_string)
}

fn session_scope_key(session_id: &str) -> String {
    // Stable FNV-1a avoids exposing the Session id in the OS path and remains
    // deterministic across Runtime restarts without adding a crypto dependency.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in session_id.trim().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn cgroup_is_empty(path: &Path) -> std::io::Result<bool> {
    let events = fs::read_to_string(path.join("cgroup.events"))?;
    for line in events.lines() {
        if let Some(value) = line.strip_prefix("populated ") {
            return match value.trim() {
                "0" => Ok(true),
                "1" => Ok(false),
                _ => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid cgroup.events populated value",
                )),
            };
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "cgroup.events lacks populated state",
    ))
}

fn cgroup_member_pids(path: &Path) -> std::io::Result<Vec<u32>> {
    let mut pids = fs::read_to_string(path.join("cgroup.procs"))?
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

fn session_scope_has_live_members(session_path: &Path) -> std::io::Result<bool> {
    let jobs = match fs::read_dir(session_path) {
        Ok(jobs) => jobs,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for (index, job) in jobs.enumerate() {
        if index >= 4096 {
            // A bounded observation must not turn omitted entries into proof of
            // emptiness. Treat an overfull scope as potentially live.
            return Ok(true);
        }
        let job = job?;
        let path = job.path();
        if !path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("job-"))
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "managed Job scope is not a plain directory",
            ));
        }
        if !cgroup_is_empty(&path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn runtime_scope_has_live_members(runtime_path: &Path) -> std::io::Result<bool> {
    let sessions = match fs::read_dir(runtime_path) {
        Ok(sessions) => sessions,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for (index, session) in sessions.enumerate() {
        if index >= 1024 {
            return Ok(true);
        }
        let session = session?;
        let path = session.path();
        if !path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(SESSION_SCOPE_PREFIX))
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "managed Session scope is not a plain directory",
            ));
        }
        if session_scope_has_live_members(&path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn ensure_aggregate_scope_paths(session_id: &str) -> std::io::Result<(PathBuf, PathBuf)> {
    let subtree = timem_job_subtree()?;
    STALE_SCOPE_CLEANUP.get_or_init(|| cleanup_stale_runtime_scopes(&subtree));
    // Each level must hand the controllers to the next one: timem.jobs ->
    // runtime -> session -> job leaf. Without this, a Session scope has no
    // memory accounting even though the tree root carries the caps.
    enable_job_controllers(&subtree);
    let runtime_path = ensure_plain_directory(&subtree.join(current_runtime_scope_name()))?;
    enable_job_controllers(&runtime_path);
    let session_key = session_scope_key(session_id);
    let session_path =
        ensure_plain_directory(&runtime_path.join(format!("{SESSION_SCOPE_PREFIX}{session_key}")))?;
    enable_job_controllers(&session_path);
    Ok((runtime_path, session_path))
}

pub(super) fn process_aggregate_scope_snapshot(
    session_id: &str,
) -> std::io::Result<crate::process_job::ProcessAggregateScopeSnapshot> {
    let (runtime_path, session_path) = ensure_aggregate_scope_paths(session_id)?;
    Ok(crate::process_job::ProcessAggregateScopeSnapshot {
        runtime_observation_note: format!("cgroup: {}", runtime_path.display()),
        session_observation_note: format!("cgroup: {}", session_path.display()),
    })
}

/// Resolve the current Runtime's Session scope without creating anything.
/// The resource monitor samples continuously, so creating scopes here would
/// resurrect directories for Sessions that never ran a managed job.
fn existing_session_scope_paths(session_id: &str) -> std::io::Result<Option<(PathBuf, PathBuf)>> {
    let canonical_current = current_cgroup_mounted_path()?;
    let subtree = instance_root_cgroup(&canonical_current).join(TIMEM_JOB_SUBTREE);
    let session_path = subtree.join(current_runtime_scope_name()).join(format!(
        "{SESSION_SCOPE_PREFIX}{}",
        session_scope_key(session_id)
    ));
    match fs::symlink_metadata(&session_path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            Ok(Some((session_path, subtree)))
        }
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "managed Session scope is not a plain directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_stat_value(content: &str, key: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let (name, value) = line.split_once(' ')?;
        if name != key {
            return None;
        }
        value.trim().parse().ok()
    })
}

/// `cpu.max` is `<quota> <period>` in microseconds; `max` means unbounded.
fn parse_cpu_max_cores(content: &str) -> Option<f64> {
    let mut fields = content.split_whitespace();
    let quota = fields.next()?;
    let period = fields.next()?.parse::<f64>().ok()?;
    if quota == "max" || period <= 0.0 {
        return None;
    }
    let quota = quota.parse::<f64>().ok()?;
    (quota > 0.0).then_some(quota / period)
}

/// `memory.high`/`memory.max` are byte counts; `max` means unbounded.
fn parse_memory_limit(content: &str) -> Option<u64> {
    let content = content.trim();
    if content == "max" {
        None
    } else {
        content.parse().ok()
    }
}

fn memory_events_oom_kill(path: &Path) -> u64 {
    fs::read_to_string(path.join("memory.events"))
        .ok()
        .as_deref()
        .and_then(|content| parse_stat_value(content, "oom_kill"))
        .unwrap_or(0)
}

/// OOM kills recorded for the Session scope. The kernel propagates an
/// `oom_kill` event to the cgroup of the killed task and its ancestors below
/// the limit, so the Session scope already covers every Job leaf. Summing the
/// leaves too would double count (verified on the live kernel).
fn session_oom_kill_count(session_path: &Path) -> u64 {
    memory_events_oom_kill(session_path)
}

/// Managed Job leaves under one Session scope. The kernel records an
/// `oom_kill` event on the exact Job leaf that contained the victim, so this
/// is the attribution source for "which job was killed".
fn job_resource_usages(session_path: &Path) -> Vec<crate::JobResourceUsage> {
    let Ok(entries) = fs::read_dir(session_path) else {
        return Vec::new();
    };
    let mut jobs = Vec::new();
    for (index, entry) in entries.flatten().enumerate() {
        if index >= 4096 {
            break;
        }
        let path = entry.path();
        let Some(scope_name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if !scope_name.starts_with("job-") {
            continue;
        }
        if !fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            continue;
        }
        jobs.push(crate::JobResourceUsage {
            scope_name: scope_name.to_string(),
            oom_kill_count: memory_events_oom_kill(&path),
            memory_current_bytes: fs::read_to_string(path.join("memory.current"))
                .ok()
                .and_then(|content| content.trim().parse::<u64>().ok()),
        });
    }
    jobs.sort_by(|left, right| left.scope_name.cmp(&right.scope_name));
    jobs
}

fn session_resource_usage_at(
    session_path: &Path,
    job_subtree: &Path,
) -> crate::SessionResourceUsage {
    let read = |path: PathBuf| fs::read_to_string(path).ok();
    let cpu_usage_usec = read(session_path.join("cpu.stat"))
        .as_deref()
        .and_then(|content| parse_stat_value(content, "usage_usec"));
    let cpu_quota_cores = read(job_subtree.join("cpu.max"))
        .as_deref()
        .and_then(parse_cpu_max_cores);
    let memory_current_bytes = read(session_path.join("memory.current"))
        .as_deref()
        .and_then(|content| content.trim().parse::<u64>().ok());
    // memory.peak exists only on newer kernels. Do not substitute the current
    // charge: by the time an OOM kill is observed it has already dropped.
    let memory_peak_bytes = read(session_path.join("memory.peak"))
        .as_deref()
        .and_then(|content| content.trim().parse::<u64>().ok());
    let swap_current_bytes = read(session_path.join("swap.current"))
        .as_deref()
        .and_then(|content| content.trim().parse::<u64>().ok());
    let memory_high_bytes = read(job_subtree.join("memory.high"))
        .as_deref()
        .and_then(parse_memory_limit);
    let memory_max_bytes = read(job_subtree.join("memory.max"))
        .as_deref()
        .and_then(parse_memory_limit);
    crate::SessionResourceUsage {
        cpu_usage_usec,
        cpu_quota_cores,
        memory_current_bytes,
        memory_peak_bytes,
        swap_current_bytes,
        memory_high_bytes,
        memory_max_bytes,
        oom_kill_count: session_oom_kill_count(session_path),
        jobs: job_resource_usages(session_path),
    }
}

pub(super) fn session_resource_usage(
    session_id: &str,
) -> std::io::Result<Option<crate::SessionResourceUsage>> {
    let Some((session_path, job_subtree)) = existing_session_scope_paths(session_id)? else {
        return Ok(None);
    };
    Ok(Some(session_resource_usage_at(&session_path, &job_subtree)))
}

fn is_plain_named_directory(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.starts_with(prefix))
        && fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

/// Remove only empty managed Job leaves from one exact Session scope. This
/// never traverses sibling Sessions and never signals or adopts live members.
fn cleanup_empty_session_scope(session_path: &Path) {
    if !is_plain_named_directory(session_path, SESSION_SCOPE_PREFIX) {
        return;
    }
    let Ok(jobs) = fs::read_dir(session_path) else {
        return;
    };
    for job in jobs.flatten().take(4096) {
        let job_path = job.path();
        if !is_plain_named_directory(&job_path, "job-") {
            continue;
        }
        if cgroup_is_empty(&job_path).unwrap_or(false) {
            let _ = fs::remove_dir(&job_path);
        }
    }
    let _ = fs::remove_dir(session_path);
}

/// Best-effort cleanup for a stale Runtime that has been proven to have no
/// live managed Job members. Every Session is still checked independently;
/// populated or unrecognised entries make parent removal fail closed.
fn cleanup_empty_runtime_scope(runtime_path: &Path) {
    if !is_plain_named_directory(runtime_path, RUNTIME_SCOPE_PREFIX) {
        return;
    }
    let Ok(sessions) = fs::read_dir(runtime_path) else {
        return;
    };
    for session in sessions.flatten().take(1024) {
        let session_path = session.path();
        if is_plain_named_directory(&session_path, SESSION_SCOPE_PREFIX) {
            cleanup_empty_session_scope(&session_path);
        }
    }
    let _ = fs::remove_dir(runtime_path);
}

pub(super) fn cleanup_current_runtime_process_scope() {
    let Ok(subtree) = existing_timem_job_subtree() else {
        return;
    };
    cleanup_empty_runtime_scope(&subtree.join(current_runtime_scope_name()));
    let _ = fs::remove_dir(&subtree);
}

/// Best-effort removal of the instance's managed cgroup facility after a
/// graceful shutdown, so a Ctrl+C/`SIGTERM` exit leaves no managed residue:
/// the `timem.jobs` tree, the controllers this runtime enabled on the
/// instance root, and the `timem.self` sibling. Only directories that ended
/// up empty are removed: populated groups (live orphan jobs keep their
/// fence) and layers the user does not own are kept, and the next startup
/// re-derives everything idempotently. The instance root itself and sibling
/// scopes are never touched.
pub fn cleanup_instance_cgroup_subtrees() {
    let current = match current_cgroup_mounted_path() {
        Ok(path) => path,
        Err(_) => return,
    };
    let instance_root = instance_root_cgroup(&current).to_path_buf();
    remove_empty_cgroup_tree(&instance_root.join(TIMEM_JOB_SUBTREE));
    // Return the root's subtree_control to its pre-runtime shape. A shared
    // root keeps whatever was enabled there: this runtime never enabled
    // controllers on a shared root, and foreign settings are not ours to
    // undo. Disabling fails harmlessly while the job subtree above still
    // exists, so a live orphan keeps its kernel fence.
    if !root_shared_with_other_processes(&instance_root) {
        let control = instance_root.join("cgroup.subtree_control");
        for controller in ["memory", "cpu"] {
            if subtree_contains_controller(&instance_root, controller)
                && path_writable_by_current_user(&control)
            {
                let _ = fs::write(&control, format!("-{controller}"));
            }
        }
    }
    // A group still holding this process cannot be removed: migrate back to
    // the instance root first, which the shutdown is about to exit anyway.
    if current.file_name().and_then(OsStr::to_str) == Some(TIMEM_SELF_SUBTREE) {
        let _ = fs::write(
            instance_root.join("cgroup.procs"),
            std::process::id().to_string(),
        );
    }
    remove_empty_cgroup_tree(&instance_root.join(TIMEM_SELF_SUBTREE));
}

fn remove_empty_cgroup_tree(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            remove_empty_cgroup_tree(&entry.path());
        }
    }
    let _ = fs::remove_dir(path);
}

fn existing_timem_job_subtree() -> std::io::Result<PathBuf> {
    let relative = current_cgroup_v2_path()?;
    let mount = Path::new(CGROUP_ROOT);
    let current = mount.join(relative.strip_prefix("/").unwrap_or(&relative));
    let canonical_mount = mount.canonicalize()?;
    let canonical_current = current.canonicalize()?;
    if !canonical_current.starts_with(&canonical_mount) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "current cgroup escapes cgroup-v2 mount",
        ));
    }
    let subtree = instance_root_cgroup(&canonical_current).join(TIMEM_JOB_SUBTREE);
    let metadata = fs::symlink_metadata(&subtree)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Timem cgroup subtree is not a plain directory",
        ));
    }
    Ok(subtree)
}

fn cleanup_stale_runtime_scopes(subtree: &Path) {
    let Ok(entries) = fs::read_dir(subtree) else {
        return;
    };
    for entry in entries.flatten().take(256) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some((owner_pid, owner_start_ticks)) = parse_runtime_scope_name(name) else {
            continue;
        };
        if runtime_owner_matches(owner_pid, owner_start_ticks) {
            continue;
        }
        if matches!(runtime_scope_has_live_members(&entry.path()), Ok(false)) {
            cleanup_empty_runtime_scope(&entry.path());
        }
    }
}

fn current_cgroup_v2_path() -> std::io::Result<PathBuf> {
    let content = fs::read_to_string("/proc/self/cgroup")?;
    let value = content
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "unified cgroup-v2 membership is unavailable",
            )
        })?;
    let path = Path::new(value);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid cgroup-v2 membership path",
        ));
    }
    Ok(path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Instance resource caps (cgroup v2)

use crate::api::{
    InstanceResourceCaps, ResourceCapEscalationAction, ResourceCapReadiness, ResourceCapStatus,
    ResourceCapsApplication, ResourceCapsReadiness, SystemResourceUsage,
};

/// Platform-native unbounded values: writing these clears a bound.
pub(super) const CPU_MAX_UNBOUNDED: &str = "max 100000";
pub(super) const MEMORY_LIMIT_UNBOUNDED: &str = "max";
/// Throttle starts at 85% of the hard memory cap, leaving headroom for
/// reclaim before the kernel has to OOM-kill an unreclaimable allocation.
pub(super) const MEMORY_HIGH_TO_MAX_PERCENT: u64 = 85;

const ESCALATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Online logical CPUs describe the machine's total CPU capacity more
/// directly than the affinity/cgroup-aware parallelism hint.
pub(super) fn system_cpu_cores() -> Option<u32> {
    let online = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if online > 0 {
        Some(online as u32)
    } else {
        std::thread::available_parallelism()
            .ok()
            .map(|value| value.get() as u32)
    }
}

/// Parse the `MemTotal:` line (`MemTotal: 32878012 kB`) from /proc/meminfo.
pub(super) fn parse_meminfo_total_bytes(content: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let value = line.strip_prefix("MemTotal:")?;
        let kb = value
            .trim()
            .strip_suffix(" kB")?
            .trim()
            .parse::<u64>()
            .ok()?;
        Some(kb.saturating_mul(1024))
    })
}

pub(super) fn system_mem_total_bytes() -> Option<u64> {
    parse_meminfo_total_bytes(&fs::read_to_string("/proc/meminfo").ok()?)
}

/// Parse `MemAvailable:` from /proc/meminfo.
pub(super) fn parse_meminfo_available_bytes(content: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let value = line.strip_prefix("MemAvailable:")?;
        let kb = value
            .trim()
            .strip_suffix(" kB")?
            .trim()
            .parse::<u64>()
            .ok()?;
        Some(kb.saturating_mul(1024))
    })
}

/// Parse the aggregate `cpu` line from /proc/stat into (busy, total) ticks.
/// Fields are user nice system idle iowait irq softirq steal; guest time is
/// already included in user/nice, so only the first eight fields are summed.
pub(super) fn parse_proc_stat_cpu(content: &str) -> Option<(u64, u64)> {
    let line = content.lines().find(|line| line.starts_with("cpu "))?;
    let mut ticks = [0u64; 8];
    let mut values = line.split_whitespace().skip(1);
    for tick in &mut ticks {
        *tick = values.next()?.parse::<u64>().ok()?;
    }
    let total = ticks.iter().copied().sum::<u64>();
    let idle = ticks[3].saturating_add(ticks[4]);
    Some((total.saturating_sub(idle), total))
}

pub(super) fn system_resource_usage() -> std::io::Result<Option<SystemResourceUsage>> {
    let stat = match fs::read_to_string("/proc/stat") {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    let Some((cpu_busy_ticks, cpu_total_ticks)) = parse_proc_stat_cpu(&stat) else {
        return Ok(None);
    };
    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    Ok(Some(SystemResourceUsage {
        cpu_busy_ticks: Some(cpu_busy_ticks),
        cpu_total_ticks: Some(cpu_total_ticks),
        memory_total_bytes: parse_meminfo_total_bytes(&meminfo),
        memory_available_bytes: parse_meminfo_available_bytes(&meminfo),
    }))
}

/// `cpu.max` bounding the job tree to `percent` of `cores` cores, expressed
/// with the default 100 ms period. The kernel minimum quota is 1000 us.
pub(super) fn format_cpu_max(percent: f64, cores: u32) -> String {
    const PERIOD_US: u64 = 100_000;
    let quota = ((percent / 100.0) * f64::from(cores) * PERIOD_US as f64)
        .round()
        .clamp(1000.0, u64::MAX as f64) as u64;
    format!("{quota} {PERIOD_US}")
}

/// Hard memory cap in bytes: `percent` of the system total.
pub(super) fn format_memory_max(percent: f64, total_bytes: u64) -> u64 {
    ((percent / 100.0) * total_bytes as f64).round() as u64
}

/// Soft throttle point in bytes: 85% of the hard cap.
pub(super) fn format_memory_high(max_bytes: u64) -> u64 {
    ((u128::from(max_bytes) * u128::from(MEMORY_HIGH_TO_MAX_PERCENT)) / 100) as u64
}

fn cgroup_write_value(path: &Path, value: &str) -> Result<(), String> {
    fs::write(path, format!("{value}\n")).map_err(|error| format!("{}: {error}", path.display()))
}

fn cgroup_read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|content| content.trim().to_string())
}

fn subtree_contains_controller(cgroup: &Path, controller: &str) -> bool {
    fs::read_to_string(cgroup.join("cgroup.subtree_control"))
        .ok()
        .map(|content| {
            content
                .split_whitespace()
                .any(|token| token.trim_start_matches('+') == controller)
        })
        .unwrap_or(false)
}

fn path_writable_by_current_user(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        unsafe { libc::access(cpath.as_ptr(), libc::W_OK | libc::R_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// A root that also holds other processes cannot be vacated by migrating the
/// main process away. Enabling a controller there still succeeds — the kernel
/// converts the populated root to a threaded subtree, which carries cpu — but
/// memory stays permanently unusable, and a populated root with a populated
/// child even rejects cpu with EBUSY. Shared roots therefore keep the
/// legacy layout (no migration) and disclose the memory limit as unavailable.
pub(super) fn pids_indicate_shared_root(pids: &[u32], own_pid: u32) -> bool {
    pids.iter().any(|pid| *pid != own_pid)
}

#[cfg(not(test))]
fn root_shared_with_other_processes(root: &Path) -> bool {
    match cgroup_member_pids(root) {
        Ok(pids) => pids_indicate_shared_root(&pids, std::process::id()),
        // An unreadable root is an unexpected layout: keep the legacy path.
        Err(_) => true,
    }
}

// Test builds keep probes side-effect free; the suite's own root always
// looks shared otherwise (the test runner's parent stays populated).
#[cfg(test)]
fn root_shared_with_other_processes(_root: &Path) -> bool {
    false
}

/// The limit files a dimension writes under the `timem.jobs` subtree.
/// Memory has both a soft throttle point and a hard cap; both must be
/// writable before the dimension can be applied.
fn subtree_cap_files(instance_root: &Path, controller: &str) -> Vec<PathBuf> {
    let subtree = instance_root.join(TIMEM_JOB_SUBTREE);
    if controller == "cpu" {
        vec![subtree.join("cpu.max")]
    } else {
        vec![subtree.join("memory.high"), subtree.join("memory.max")]
    }
}

/// Best-effort unprivileged self-migration: move the main process into the
/// managed `timem.self` sibling so the instance root stays unpopulated.
/// Populated roots get force-converted to threaded subtrees by the kernel the
/// first time a controller is enabled, and threaded subtrees carry only cpu —
/// memory would stay permanently unusable. Any failure keeps the current
/// layout unchanged; the escalation path can still repair it with root.
#[cfg(not(test))]
fn try_self_migrate_without_privilege() -> bool {
    let Ok(current) = current_cgroup_mounted_path() else {
        return false;
    };
    if current.file_name().and_then(OsStr::to_str) == Some(TIMEM_SELF_SUBTREE) {
        return true;
    }
    // A shared root cannot be vacated: migrating would leave it populated
    // anyway (other processes stay) while a populated child makes even cpu
    // enablement fail with EBUSY. Keep the legacy layout there.
    if root_shared_with_other_processes(&current) {
        return false;
    }
    let self_dir = match ensure_plain_directory(&current.join(TIMEM_SELF_SUBTREE)) {
        Ok(dir) => dir,
        Err(_) => return false,
    };
    // Never share a group we did not create: unexpected members mean an
    // unexpected layout, not an invitation to append.
    match cgroup_member_pids(&self_dir) {
        Ok(pids) if pids.iter().all(|pid| *pid == std::process::id()) => {}
        _ => return false,
    }
    if fs::write(
        self_dir.join("cgroup.procs"),
        format!("{}\n", std::process::id()),
    )
    .is_err()
    {
        return false;
    }
    // Verify the kernel actually moved us; a silent no-op must not pass as a
    // successful migration.
    matches!(current_cgroup_mounted_path(), Ok(now) if now == self_dir)
}

// Test builds keep probes side-effect free: the suite must never reparent the
// test binary or leave managed groups behind in the developer's cgroup tree.
// The real path is exercised by the true end-to-end run against a live host.
#[cfg(test)]
fn try_self_migrate_without_privilege() -> bool {
    false
}

/// Minimal root-authorized actions that make `controller` usable for the
/// `timem.jobs` subtree hanging off `instance_root`: migrate the main process
/// into the managed `timem.self` sibling while it still lives directly in the
/// root, enable the controller in every missing `cgroup.subtree_control` along
/// the ancestor chain (shallowest first), then give the current user ownership
/// of the instance root when it is root-owned. The root is the deepest entry;
/// writing its `subtree_control` is only legal while it holds no processes,
/// which the migration action arranges before the enablement runs.
fn escalation_actions_for_dimension(
    instance_root: &Path,
    process_cgroup: &Path,
    controller: &str,
) -> Result<Vec<ResourceCapEscalationAction>, String> {
    let root = Path::new(CGROUP_ROOT);
    let root_available = fs::read_to_string(root.join("cgroup.controllers"))
        .ok()
        .map(|content| {
            content
                .split_whitespace()
                .any(|token| token.trim_start_matches('+') == controller)
        })
        .unwrap_or(false);
    if !root_available {
        return Err(format!(
            "{controller} controller is not available at the cgroup root"
        ));
    }
    let chain = controller_ancestor_chain(instance_root);
    let mut actions = Vec::new();
    let mut enable: Vec<ResourceCapEscalationAction> = Vec::new();
    for layer in &chain {
        if !subtree_contains_controller(layer, controller) {
            enable.push(ResourceCapEscalationAction::EnableController {
                cgroup: layer.display().to_string(),
                controller: controller.to_string(),
            });
        }
    }
    // Migration only buys something when a controller still has to be
    // enabled: with the chain already complete the populated root is
    // harmless. The apply entry point re-sorts it ahead of everything else
    // even when plans from several dimensions are merged.
    if !enable.is_empty()
        && process_cgroup == instance_root
        && !root_shared_with_other_processes(instance_root)
    {
        actions.push(ResourceCapEscalationAction::MigrateSelf {
            cgroup: instance_root.join(TIMEM_SELF_SUBTREE).display().to_string(),
            pid: std::process::id(),
        });
    }
    actions.extend(enable);
    // Enabling a controller materializes `cpu.max`/`memory.high`/`memory.max`
    // under the subtree as root-owned files, so the cap write would fail with
    // EACCES even though the directory itself is writable. Take ownership
    // whenever the root is unwritable, a cap file is still missing (it will
    // appear root-owned right after the enablement above), or it exists
    // unwritable.
    let cap_writable = subtree_cap_files(instance_root, controller)
        .iter()
        .all(|cap_file| match fs::symlink_metadata(cap_file) {
            Ok(metadata) if metadata.is_file() => path_writable_by_current_user(cap_file),
            // The controller is enabled but the file does not exist yet: it
            // will be created on demand inside the user-owned `timem.jobs`
            // and inherit that ownership, so a writable root is sufficient.
            _ => {
                subtree_contains_controller(instance_root, controller)
                    && path_writable_by_current_user(instance_root)
            }
        });
    if !path_writable_by_current_user(instance_root) || !cap_writable {
        actions.push(ResourceCapEscalationAction::MakeWritable {
            cgroup: instance_root.display().to_string(),
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
        });
    }
    Ok(actions)
}

/// Ancestor chain from the instance root up to (excluding) the cgroup mount
/// root, shallowest first: deeper layers inherit controller availability.
fn controller_ancestor_chain(instance_root: &Path) -> Vec<PathBuf> {
    let root = Path::new(CGROUP_ROOT);
    let mut chain: Vec<PathBuf> = vec![instance_root.to_path_buf()];
    let mut cursor = instance_root.to_path_buf();
    while let Some(parent) = cursor.parent() {
        if parent == root || !parent.starts_with(root) {
            break;
        }
        chain.push(parent.to_path_buf());
        cursor = parent.to_path_buf();
    }
    chain.reverse();
    chain
}

/// The unprivileged twin of the escalation plan's enablement step. When the
/// instance root is owned by the current user (delegated subtree), missing
/// controllers can be enabled directly: writing `+controller` to each
/// writable `cgroup.subtree_control` along the ancestor chain, shallowest
/// first. The walk stops at the first layer the user does not own or that
/// rejects the write; those layers remain in the root-authorized plan, which
/// re-derives everything from the live tree. Real writes are skipped under
/// `cfg(test)` so unit tests never touch the developer's cgroup tree.
/// Best-effort unprivileged self-repair shared by the probe and the direct
/// apply path: migrate the main process into the managed `timem.self`
/// sibling when that is what makes controller enablement possible, then
/// enable the missing controllers along the user-writable ancestor chain.
/// Layers the user does not own stay in the root-authorized escalation
/// plan. No-ops under `cfg(test)` through the callees' test shims, so unit
/// tests never touch the developer's cgroup tree.
fn unprivileged_self_repair(instance_root: &Path, process_cgroup: &Path) {
    let missing = ["cpu", "memory"]
        .iter()
        .any(|controller| !subtree_contains_controller(instance_root, controller));
    let shared = root_shared_with_other_processes(instance_root);
    if !shared && missing && process_cgroup == instance_root {
        try_self_migrate_without_privilege();
    }
    // The `populated` flag in `cgroup.events` lags behind a migration; the
    // authoritative vacated check is an empty `cgroup.procs`.
    if !shared
        && missing
        && path_writable_by_current_user(instance_root)
        && cgroup_member_pids(instance_root)
            .map(|pids| pids.is_empty())
            .unwrap_or(false)
    {
        enable_missing_controllers_without_privilege(instance_root, &["cpu", "memory"]);
    }
}

#[cfg(not(test))]
fn enable_missing_controllers_without_privilege(instance_root: &Path, controllers: &[&str]) {
    let chain = controller_ancestor_chain(instance_root);
    for controller in controllers {
        for layer in &chain {
            if subtree_contains_controller(layer, controller) {
                continue;
            }
            let control = layer.join("cgroup.subtree_control");
            if !path_writable_by_current_user(&control) {
                break;
            }
            if fs::write(&control, format!("+{controller}")).is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
fn enable_missing_controllers_without_privilege(_instance_root: &Path, _controllers: &[&str]) {}

#[cfg(test)]
mod cgroup_cleanup_tests {
    use super::*;

    #[test]
    fn remove_empty_cgroup_tree_removes_nested_empty_dirs_only() {
        let base = std::env::temp_dir().join(format!("timem-cg-clean-{}", std::process::id()));
        let nested = base.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(base.join("a").join("keeps-parent.txt"), b"x").unwrap();
        remove_empty_cgroup_tree(&base);
        assert!(!nested.exists(), "deepest empty dir should be removed");
        assert!(
            base.join("a").exists(),
            "a dir holding a file must stay (best-effort semantics)"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn remove_empty_cgroup_tree_ignores_missing_path() {
        remove_empty_cgroup_tree(&std::env::temp_dir().join("timem-cg-clean-missing"));
    }
}

pub(super) fn probe_instance_resource_caps() -> ResourceCapsReadiness {
    let initial = match current_cgroup_mounted_path() {
        Ok(path) => path,
        Err(error) => {
            let reason = format!("cgroup v2 layout unavailable: {error}");
            return ResourceCapsReadiness {
                scope: String::new(),
                cpu: ResourceCapReadiness::Unavailable {
                    reason: reason.clone(),
                },
                mem: ResourceCapReadiness::Unavailable { reason },
            };
        }
    };
    let mut instance_root = instance_root_cgroup(&initial).to_path_buf();
    // Migrate only when it matters: a root whose chain already carries every
    // controller gains nothing from moving the main process out, and a root
    // shared with other processes cannot be vacated at all.
    unprivileged_self_repair(&instance_root, &initial);
    let process_cgroup = match current_cgroup_mounted_path() {
        Ok(path) => path,
        Err(error) => {
            let reason = format!("cgroup v2 layout unavailable: {error}");
            return ResourceCapsReadiness {
                scope: String::new(),
                cpu: ResourceCapReadiness::Unavailable {
                    reason: reason.clone(),
                },
                mem: ResourceCapReadiness::Unavailable { reason },
            };
        }
    };
    instance_root = instance_root_cgroup(&process_cgroup).to_path_buf();
    let shared = root_shared_with_other_processes(&instance_root);
    let dimension = |controller: &str| match escalation_actions_for_dimension(
        &instance_root,
        &process_cgroup,
        controller,
    ) {
        Ok(actions) if actions.is_empty() => ResourceCapReadiness::Ready,
        Ok(actions) => ResourceCapReadiness::NeedsEscalation { actions },
        Err(reason) => ResourceCapReadiness::Unavailable { reason },
    };
    let mem = if shared {
        // Root vacated is the only shape in which the kernel ever enables
        // memory; with sibling processes staying behind it always ends in a
        // threaded subtree (cpu only). Disclose instead of planning a repair
        // that cannot succeed.
        ResourceCapReadiness::Unavailable {
            reason: crate::api::SHARED_INSTANCE_ROOT_REASON.to_string(),
        }
    } else {
        dimension("memory")
    };
    ResourceCapsReadiness {
        scope: instance_root.display().to_string(),
        cpu: dimension("cpu"),
        mem,
    }
}

fn validate_escalation_cgroup(path: &str) -> Result<(), String> {
    let well_formed = path.starts_with("/sys/fs/cgroup/")
        && path.len() <= 512
        && path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '.' | '-' | '@'))
        && !path.contains("..");
    if well_formed {
        Ok(())
    } else {
        Err("escalation plan contains an unsafe cgroup path".to_string())
    }
}

/// Render the fixed `sh` script for a plan. Only the three whitelisted action
/// kinds are ever rendered; paths are strictly validated so no user input
/// can break out of the quoting.
pub(super) fn render_escalation_script(
    actions: &[ResourceCapEscalationAction],
) -> Result<String, String> {
    let mut lines = Vec::new();
    for action in actions {
        match action {
            ResourceCapEscalationAction::MigrateSelf { cgroup, pid } => {
                validate_escalation_cgroup(cgroup)?;
                lines.push(format!(
                    "mkdir -p '{cgroup}' && echo '{pid}' > '{cgroup}/cgroup.procs' || exit 77"
                ));
            }
            ResourceCapEscalationAction::EnableController { cgroup, controller } => {
                validate_escalation_cgroup(cgroup)?;
                if controller.is_empty()
                    || !controller
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch == '_')
                {
                    return Err("escalation plan contains an unsafe controller name".to_string());
                }
                lines.push(format!(
                    "grep -qw '{controller}' '{cgroup}/cgroup.subtree_control' 2>/dev/null || echo '+{controller}' > '{cgroup}/cgroup.subtree_control' || exit 77"
                ));
            }
            ResourceCapEscalationAction::MakeWritable { cgroup, uid, gid } => {
                validate_escalation_cgroup(cgroup)?;
                lines.push(format!("chown -R '{uid}:{gid}' '{cgroup}' || exit 77"));
            }
        }
    }
    if lines.is_empty() {
        return Err("escalation plan is empty".to_string());
    }
    Ok(lines.join("\n"))
}

/// Execute the plan through `sudo -S`. The password is written to sudo's
/// stdin only; stdout is never echoed back. Error attribution is fixed-text:
/// sudo's own exit 1 (authorization rejected before the script ran) maps to a
/// fixed message, while a failing whitelisted action makes the script exit 77
/// and only a bounded kernel `sh` stderr tail is appended — text produced by
/// the kernel/sudo about paths and errno, never containing the password.
pub(super) fn apply_resource_cap_escalation_plan(
    actions: &[ResourceCapEscalationAction],
    password: &[u8],
) -> Result<(), String> {
    if actions.is_empty() {
        return Ok(());
    }
    // The caller may merge per-dimension plans in arbitrary order, so the
    // script is re-layered here: migration first (enabling a controller under
    // a populated root force-converts the cgroup to a threaded subtree that
    // carries only cpu), then enablement, and ownership repair last — the
    // chown must run after the controller files it has to cover materialize.
    // Stable sort keeps the order inside each layer.
    let mut ordered: Vec<ResourceCapEscalationAction> = actions.to_vec();
    ordered.sort_by_key(|action| match action {
        ResourceCapEscalationAction::MigrateSelf { .. } => 0u8,
        ResourceCapEscalationAction::EnableController { .. } => 1u8,
        ResourceCapEscalationAction::MakeWritable { .. } => 2u8,
    });
    let script = render_escalation_script(&ordered)?;
    let mut child = std::process::Command::new("sudo")
        .args(["-S", "-p", "", "--", "sh", "-c", &script])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| "sudo is unavailable".to_string())?;
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().ok_or("escalation stdin unavailable")?;
        stdin
            .write_all(password)
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|_| "escalation stdin write failed")?;
        // Dropping the handle closes sudo's stdin right after the password.
    }
    let deadline = std::time::Instant::now() + ESCALATION_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("escalation command timed out".to_string());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => return Err("escalation command wait failed".to_string()),
        }
    };
    // Drain stderr after the process exits: the output is a few kernel/sudo
    // lines (bounded by sudo itself), so draining post-exit cannot deadlock,
    // and the tail discloses which action the kernel rejected.
    let mut stderr_tail = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        use std::io::Read;
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        let start = lines.len().saturating_sub(2);
        stderr_tail = lines[start..].join("; ");
        let count = stderr_tail.chars().count();
        if count > 400 {
            stderr_tail = stderr_tail.chars().skip(count - 400).collect();
        }
    }
    match status.code() {
        Some(0) => Ok(()),
        // sudo exits 1 for its own failures (wrong password, sudoers
        // rejection) before the script ever runs.
        Some(1) => Err("root authorization rejected".to_string()),
        // The rendered script exits 77 the moment a whitelisted action fails,
        // e.g. the kernel rejecting a controller write because the current
        // cgroup layout cannot support it (a threaded subtree only carries
        // the cpu controller, never memory).
        Some(77) if stderr_tail.is_empty() => Err("escalation script failed".to_string()),
        Some(77) => Err(format!("escalation script failed: {stderr_tail}")),
        _ => Err("escalation command failed".to_string()),
    }
}

pub(super) fn apply_dimension_bound(
    subtree: &Path,
    file: &str,
    enabled: bool,
    unbounded: &str,
    render_limit: impl FnOnce() -> String,
) -> ResourceCapStatus {
    let path = subtree.join(file);
    if enabled {
        let limit = render_limit();
        match cgroup_write_value(&path, &limit) {
            Ok(()) => ResourceCapStatus::Applied { limit },
            Err(reason) => ResourceCapStatus::Unavailable { reason },
        }
    } else {
        match cgroup_write_value(&path, unbounded) {
            Ok(()) => ResourceCapStatus::Disabled,
            // Disabling means "unbounded": if the kernel already sits at the
            // unbounded value the intent is reached even though the write
            // (e.g. an unmanaged read-only cgroup) failed.
            Err(_) if cgroup_read_trimmed(&path).as_deref() == Some(unbounded) => {
                ResourceCapStatus::Disabled
            }
            Err(reason) => ResourceCapStatus::Unavailable {
                reason: format!("clearing the existing bound failed: {reason}"),
            },
        }
    }
}

/// Write the memory dimension as a hard cap plus a soft throttle point:
/// `memory.max` is the configured share of the system total, and
/// `memory.high` is 85% of it. The kernel reclaims/throttles at high and only
/// OOM-kills an allocation that cannot be reclaimed below max. A failure in
/// either file marks the dimension unavailable instead of leaving a
/// half-applied bound.
fn apply_memory_bounds(
    subtree: &Path,
    enabled: bool,
    percent: f64,
    total_bytes: u64,
) -> ResourceCapStatus {
    let max_bytes = format_memory_max(percent, total_bytes);
    let high_bytes = format_memory_high(max_bytes);
    let high_path = subtree.join("memory.high");
    let max_path = subtree.join("memory.max");
    // A dimension is two kernel files. If one write fails, restore both to
    // their previous values: reporting `Unavailable` while leaving one bound
    // applied would make the projection disagree with what the kernel
    // enforces. Best effort — a restore failure is still reported as
    // `Unavailable` below, never silently as success.
    let previous_high = cgroup_read_trimmed(&high_path);
    let previous_max = cgroup_read_trimmed(&max_path);
    let high = apply_dimension_bound(
        subtree,
        "memory.high",
        enabled,
        MEMORY_LIMIT_UNBOUNDED,
        || high_bytes.to_string(),
    );
    let max = apply_dimension_bound(
        subtree,
        "memory.max",
        enabled,
        MEMORY_LIMIT_UNBOUNDED,
        || max_bytes.to_string(),
    );
    match (high, max) {
        (ResourceCapStatus::Applied { .. }, ResourceCapStatus::Applied { .. }) => {
            ResourceCapStatus::Applied {
                limit: format!("high={high_bytes}; max={max_bytes}"),
            }
        }
        (ResourceCapStatus::Unavailable { reason }, _)
        | (_, ResourceCapStatus::Unavailable { reason }) => {
            if let Some(previous) = previous_high.as_deref() {
                let _ = cgroup_write_value(&high_path, previous);
            }
            if let Some(previous) = previous_max.as_deref() {
                let _ = cgroup_write_value(&max_path, previous);
            }
            ResourceCapStatus::Unavailable { reason }
        }
        _ => ResourceCapStatus::Disabled,
    }
}

/// Bound the managed `timem.jobs` subtree. Child scopes (`runtime-*`,
/// `session-*`, leaves) copy controller settings from their parent when
/// created, so one write at the subtree root bounds every current and future
/// managed job of this instance. Timem itself lives outside the subtree and
/// stays unbounded; the job tree is the heavy consumer.
pub(super) fn apply_instance_resource_caps(caps: &InstanceResourceCaps) -> ResourceCapsApplication {
    // The direct write path has no escalation plan to lean on, so run the
    // unprivileged self-repair (migrate + enable) in place first; otherwise
    // the cap writes below hit a not-yet-existing controller file (EACCES).
    if let Ok(initial) = current_cgroup_mounted_path() {
        let instance_root = instance_root_cgroup(&initial).to_path_buf();
        unprivileged_self_repair(&instance_root, &initial);
    }
    let unavailable = |reason: String| ResourceCapsApplication {
        caps: *caps,
        cpu: ResourceCapStatus::Unavailable {
            reason: reason.clone(),
        },
        mem: ResourceCapStatus::Unavailable { reason },
        scope: String::new(),
    };
    let Some(cores) = system_cpu_cores() else {
        return unavailable("cpu core count unavailable".to_string());
    };
    let Some(total_bytes) = system_mem_total_bytes() else {
        return unavailable("system memory total unavailable".to_string());
    };
    let subtree = match timem_job_subtree() {
        Ok(path) => path,
        Err(error) => return unavailable(format!("timem.jobs subtree unavailable: {error}")),
    };
    enable_job_controllers(&subtree);
    let cpu = apply_dimension_bound(
        &subtree,
        "cpu.max",
        caps.cpu_enabled,
        CPU_MAX_UNBOUNDED,
        || format_cpu_max(caps.cpu_percent, cores),
    );
    let mem = apply_memory_bounds(&subtree, caps.mem_enabled, caps.mem_percent, total_bytes);
    ResourceCapsApplication {
        caps: *caps,
        cpu,
        mem,
        scope: subtree.display().to_string(),
    }
}

#[cfg(test)]
mod process_scope_tests {
    use super::*;

    #[test]
    fn session_scope_keys_are_stable_distinct_and_opaque() {
        let raw_a = "session/customer-visible-A";
        let raw_b = "session/customer-visible-B";
        let key_a = session_scope_key(raw_a);
        assert_eq!(key_a, session_scope_key(raw_a));
        assert_ne!(key_a, session_scope_key(raw_b));
        assert_eq!(key_a.len(), 16);
        assert!(key_a.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert!(!key_a.contains(raw_a));
    }

    #[test]
    fn runtime_scope_identity_requires_pid_and_start_ticks() {
        let name = current_runtime_scope_name();
        let (pid, start_ticks) = parse_runtime_scope_name(&name).expect("runtime scope name");
        assert_eq!(pid, std::process::id());
        assert!(runtime_owner_matches(pid, start_ticks));
        assert!(!runtime_owner_matches(
            pid,
            "definitely-not-the-current-start-ticks"
        ));
    }

    fn temporary_scope(label: &str) -> PathBuf {
        static NEXT_TEST_SCOPE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT_TEST_SCOPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "timem-platform-{label}-{}-{id}",
            std::process::id()
        ))
    }

    #[test]
    fn session_scope_live_check_uses_populated_state() {
        let root = temporary_scope("populated");
        let session = root.join("session-test");
        let job = session.join("job-test");
        fs::create_dir_all(&job).unwrap();

        fs::write(job.join("cgroup.events"), "populated 1\n").unwrap();
        assert!(session_scope_has_live_members(&session).unwrap());

        fs::write(job.join("cgroup.events"), "populated 0\n").unwrap();
        assert!(!session_scope_has_live_members(&session).unwrap());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_scope_live_check_fails_closed_on_unreadable_state() {
        let root = temporary_scope("unreadable");
        let session = root.join("session-test");
        let job = session.join("job-test");
        fs::create_dir_all(&job).unwrap();

        assert!(session_scope_has_live_members(&session).is_err());
        fs::write(job.join("cgroup.events"), "populated unknown\n").unwrap();
        assert!(session_scope_has_live_members(&session).is_err());

        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod session_resource_tests {
    use super::*;

    fn temporary_scope(label: &str) -> PathBuf {
        static NEXT_TEST_SCOPE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT_TEST_SCOPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "timem-platform-usage-{label}-{}-{id}",
            std::process::id()
        ))
    }

    #[test]
    fn parses_cpu_max_and_memory_limits() {
        assert_eq!(parse_cpu_max_cores("280000 100000"), Some(2.8));
        assert_eq!(parse_cpu_max_cores("50000 100000"), Some(0.5));
        assert_eq!(parse_cpu_max_cores("max 100000"), None);
        assert_eq!(parse_cpu_max_cores("broken"), None);
        assert_eq!(parse_memory_limit("1665314816\n"), Some(1665314816));
        assert_eq!(parse_memory_limit("max\n"), None);
    }

    #[test]
    fn session_usage_aggregates_cpu_memory_and_counts_oom_once() {
        let root = temporary_scope("snapshot");
        let session = root.join("session-test");
        let job = session.join("job-1");
        fs::create_dir_all(&job).unwrap();
        fs::write(
            session.join("cpu.stat"),
            "usage_usec 123456\nuser_usec 100\n",
        )
        .unwrap();
        fs::write(session.join("memory.current"), "1048576\n").unwrap();
        fs::write(session.join("swap.current"), "4096\n").unwrap();
        fs::write(job.join("memory.events"), "oom_kill 2\nhigh 0\n").unwrap();
        fs::write(session.join("memory.events"), "oom_kill 2\nhigh 3\n").unwrap();
        fs::write(root.join("cpu.max"), "50000 100000\n").unwrap();
        fs::write(root.join("memory.high"), "52428800\n").unwrap();
        fs::write(root.join("memory.max"), "max\n").unwrap();

        let usage = session_resource_usage_at(&session, &root);
        assert_eq!(usage.cpu_usage_usec, Some(123456));
        assert_eq!(usage.cpu_quota_cores, Some(0.5));
        assert_eq!(usage.memory_current_bytes, Some(1048576));
        // No memory.peak file here: the snapshot must not fake a peak.
        assert_eq!(usage.memory_peak_bytes, None);
        assert_eq!(usage.swap_current_bytes, Some(4096));
        assert_eq!(usage.memory_high_bytes, Some(52428800));
        assert_eq!(usage.memory_max_bytes, None);
        assert_eq!(usage.oom_kill_count, 2);
        assert_eq!(usage.jobs.len(), 1);
        assert_eq!(usage.jobs[0].scope_name, "job-1");
        assert_eq!(usage.jobs[0].oom_kill_count, 2);
        assert_eq!(usage.jobs[0].memory_current_bytes, None);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn job_hierarchy_enables_both_controllers() {
        let root = temporary_scope("controllers");
        fs::create_dir_all(&root).unwrap();
        enable_job_controllers(&root);
        assert_eq!(
            fs::read_to_string(root.join("cgroup.subtree_control")).unwrap(),
            "+cpu +memory\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memory_bounds_roll_back_both_files_when_enable_partially_fails() {
        let root = temporary_scope("memory-rollback-enable");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("memory.high"), "111\n").unwrap();
        // A directory at memory.max makes the hard-limit write fail after the
        // soft limit has already been written.
        fs::create_dir(root.join("memory.max")).unwrap();

        let status = apply_memory_bounds(&root, true, 50.0, 1_000_000);
        assert!(matches!(status, ResourceCapStatus::Unavailable { .. }));
        assert_eq!(
            fs::read_to_string(root.join("memory.high")).unwrap(),
            "111\n",
            "the soft limit must be restored when the hard limit fails"
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn memory_bounds_roll_back_when_disable_partially_fails() {
        let root = temporary_scope("memory-rollback-disable");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("memory.high"), "111\n").unwrap();
        fs::create_dir(root.join("memory.max")).unwrap();

        let status = apply_memory_bounds(&root, false, 50.0, 1_000_000);
        assert!(matches!(status, ResourceCapStatus::Unavailable { .. }));
        assert_eq!(
            fs::read_to_string(root.join("memory.high")).unwrap(),
            "111\n",
            "a failed disable must not leave the soft limit cleared"
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn session_usage_reports_missing_dimensions_without_faking_them() {
        let root = temporary_scope("partial");
        let session = root.join("session-test");
        fs::create_dir_all(&session).unwrap();
        fs::write(session.join("cpu.stat"), "usage_usec 7\n").unwrap();

        let usage = session_resource_usage_at(&session, &root);
        assert_eq!(usage.cpu_usage_usec, Some(7));
        assert_eq!(usage.memory_current_bytes, None);
        assert_eq!(usage.memory_peak_bytes, None);
        assert_eq!(usage.cpu_quota_cores, None);
        assert_eq!(usage.oom_kill_count, 0);

        fs::remove_dir_all(root).unwrap();
    }
}
