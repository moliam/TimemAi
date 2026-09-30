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
    session_path: PathBuf,
    runtime_path: PathBuf,
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

        let subtree = timem_job_subtree()?;
        STALE_SCOPE_CLEANUP.get_or_init(|| cleanup_stale_runtime_scopes(&subtree));
        let runtime_path = ensure_plain_directory(&subtree.join(current_runtime_scope_name()))?;
        let session_key = session_scope_key(session_id.unwrap_or("runtime"));
        let session_path = ensure_plain_directory(
            &runtime_path.join(format!("{SESSION_SCOPE_PREFIX}{session_key}")),
        )?;

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
                            cleanup_empty_scope_parents(&session_path, &runtime_path);
                            return Err(error);
                        }
                    };
                    if !path.join("cgroup.kill").is_file() {
                        let _ = fs::remove_dir(&path);
                        cleanup_empty_scope_parents(&session_path, &runtime_path);
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Unsupported,
                            "cgroup.kill is unavailable",
                        ));
                    }
                    return Ok(Self {
                        path,
                        session_path,
                        runtime_path,
                        procs,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        cleanup_empty_scope_parents(&session_path, &runtime_path);
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
        cleanup_empty_scope_parents(&self.session_path, &self.runtime_path);
    }
}

pub(super) fn session_process_scope_snapshot(
    session_id: &str,
) -> Option<crate::process_job::SessionProcessScopeSnapshot> {
    let subtree = timem_job_subtree().ok()?;
    let runtime_path = subtree.join(current_runtime_scope_name());
    let session_path = runtime_path.join(format!(
        "{SESSION_SCOPE_PREFIX}{}",
        session_scope_key(session_id)
    ));
    session_scope_snapshot(&session_path)
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

fn session_scope_snapshot(path: &Path) -> Option<crate::process_job::SessionProcessScopeSnapshot> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    Some(crate::process_job::SessionProcessScopeSnapshot {
        observation_note: format!("cgroup: {}", path.display()),
    })
}

fn timem_job_subtree() -> std::io::Result<PathBuf> {
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
    ensure_plain_directory(&canonical_current.join(TIMEM_JOB_SUBTREE))
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

fn cleanup_empty_scope_parents(session_path: &Path, runtime_path: &Path) {
    cleanup_empty_session_scope(session_path);
    let _ = fs::remove_dir(runtime_path);
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
