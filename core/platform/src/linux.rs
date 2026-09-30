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
static NEXT_JOB_CGROUP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static STALE_JOB_CGROUP_CLEANUP: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Linux cgroup-v2 ownership for one managed process job.
///
/// The opened `cgroup.procs` descriptor is inherited only into `pre_exec`,
/// where the child moves itself before any user-controlled program executes.
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
    pub(super) fn create() -> std::io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;

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
        let subtree = canonical_current.join(TIMEM_JOB_SUBTREE);
        match fs::create_dir(&subtree) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(&subtree)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Timem cgroup subtree is not a directory",
            ));
        }
        STALE_JOB_CGROUP_CLEANUP.get_or_init(|| cleanup_empty_job_cgroups(&subtree));

        for _ in 0..64 {
            let id = NEXT_JOB_CGROUP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = subtree.join(format!("job-{}-{id}", std::process::id()));
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
        let events = fs::read_to_string(self.path.join("cgroup.events"))?;
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

    fn member_pids(&self) -> std::io::Result<Vec<u32>> {
        let mut pids = fs::read_to_string(self.path.join("cgroup.procs"))?
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .collect::<Vec<_>>();
        pids.sort_unstable();
        pids.dedup();
        Ok(pids)
    }

    fn observation_note(&self) -> Option<String> {
        Some(format!("cgroup: {}", self.path.display()))
    }
}

impl Drop for LinuxCgroupProcessJob {
    fn drop(&mut self) {
        // Removal succeeds only when the kernel says the cgroup is empty.
        // Never kill implicitly here: callers choose persistence vs cleanup.
        let _ = fs::remove_dir(&self.path);
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

fn cleanup_empty_job_cgroups(subtree: &Path) {
    const MAX_STALE_SCAN: usize = 256;
    let Ok(entries) = fs::read_dir(subtree) else {
        return;
    };
    for entry in entries.flatten().take(MAX_STALE_SCAN) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("job-") {
            continue;
        }
        let Some(rest) = name.strip_prefix("job-") else {
            continue;
        };
        let Some((owner_pid, _job_id)) = rest.split_once('-') else {
            continue;
        };
        let Ok(owner_pid) = owner_pid.parse::<u32>() else {
            continue;
        };
        // An empty cgroup can be the legitimate pre-spawn window of another
        // Timem instance. Reclaim it only when the encoded creator PID is
        // positively known to be gone; PID reuse deliberately fails closed.
        if crate::shared::process_is_alive(u64::from(owner_pid)) != Some(false) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            let _ = fs::remove_dir(path);
        }
    }
}
