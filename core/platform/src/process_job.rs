use std::process::Command;

/// Platform-neutral control surface for one Runtime-owned process Job.
///
/// Callers never depend on cgroups, Job Objects, or another platform-specific
/// primitive. A backend must establish ownership before user-controlled code
/// executes, target destructive operations only at this Job, and provide a
/// kernel-derived empty/member view.
pub struct ManagedProcessJob {
    backend: Box<dyn ProcessJobBackend>,
}

impl std::fmt::Debug for ManagedProcessJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedProcessJob")
            .field("backend", &self.backend.name())
            .finish_non_exhaustive()
    }
}

/// Internal backend contract. Platform modules implement this; Core consumers
/// use only `ManagedProcessJob` so a platform mechanism can change without
/// leaking into Agent lifecycle code.
pub(crate) trait ProcessJobBackend: std::fmt::Debug + Send + Sync {
    fn name(&self) -> &'static str;
    fn configure_command(&self, command: &mut Command) -> std::io::Result<()>;
    fn kill_all(&self) -> std::io::Result<()>;
    fn is_empty(&self) -> std::io::Result<bool>;
    fn member_pids(&self) -> std::io::Result<Vec<u32>>;
    fn observation_note(&self) -> Option<String>;
}

impl ManagedProcessJob {
    /// Creates the native exact-ownership backend for the current platform.
    /// Unsupported or undelegated environments return an explicit error;
    /// individual callers decide whether their contract permits degradation.
    pub fn create() -> std::io::Result<Self> {
        create_platform_backend().map(|backend| Self { backend })
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    pub fn configure_command(&self, command: &mut Command) -> std::io::Result<()> {
        self.backend.configure_command(command)
    }

    pub fn kill_all(&self) -> std::io::Result<()> {
        self.backend.kill_all()
    }

    pub fn is_empty(&self) -> std::io::Result<bool> {
        self.backend.is_empty()
    }

    pub fn member_pids(&self) -> std::io::Result<Vec<u32>> {
        self.backend.member_pids()
    }

    /// Platform-native location that is useful without knowledge of Runtime
    /// internals. The string is intentionally concise and directly usable by
    /// standard operating-system tools.
    pub fn observation_note(&self) -> Option<String> {
        self.backend.observation_note()
    }
}

/// Platform-provided operator guidance for enabling the native exact Job
/// backend. Callers surface this text without knowing the backend mechanism.
pub fn managed_process_job_permission_hint() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        Some("delegate a writable cgroup v2 subtree to Timem (for a systemd service, set Delegate=yes)")
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn create_platform_backend() -> std::io::Result<Box<dyn ProcessJobBackend>> {
    crate::linux::LinuxCgroupProcessJob::create()
        .map(|backend| Box::new(backend) as Box<dyn ProcessJobBackend>)
}

#[cfg(not(target_os = "linux"))]
fn create_platform_backend() -> std::io::Result<Box<dyn ProcessJobBackend>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "exact per-job process management is unavailable on this platform",
    ))
}
