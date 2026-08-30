#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownTrigger {
    CtrlC,
    #[cfg(unix)]
    Sigterm,
    #[cfg(unix)]
    Sighup,
    ParentProcessExited,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LaunchParent {
    pid: Option<u32>,
}

impl LaunchParent {
    pub(crate) fn capture() -> Self {
        Self {
            pid: current_launch_parent_pid(),
        }
    }

    pub(crate) fn pid_u32(self) -> Option<u32> {
        self.pid
    }
}

pub(crate) struct ShutdownSignalMonitor {
    launch_parent: LaunchParent,
    #[cfg(unix)]
    platform: unix::ShutdownSignalMonitor,
    #[cfg(windows)]
    platform: windows::ShutdownSignalMonitor,
}

impl ShutdownSignalMonitor {
    pub(crate) fn capture(launch_parent: LaunchParent) -> Self {
        Self {
            launch_parent,
            #[cfg(unix)]
            platform: unix::ShutdownSignalMonitor::capture(),
            #[cfg(windows)]
            platform: windows::ShutdownSignalMonitor::capture(),
        }
    }

    pub(crate) async fn detect(self) -> ShutdownTrigger {
        #[cfg(unix)]
        {
            self.platform.detect(self.launch_parent.pid).await
        }
        #[cfg(windows)]
        {
            self.platform.detect(self.launch_parent.pid).await
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = self.launch_parent;
            let _ = tokio::signal::ctrl_c().await;
            ShutdownTrigger::CtrlC
        }
    }
}

#[cfg(test)]
pub(crate) fn shutdown_signal_names() -> &'static [&'static str] {
    #[cfg(unix)]
    {
        unix::shutdown_signal_names()
    }
    #[cfg(windows)]
    {
        windows::shutdown_signal_names()
    }
    #[cfg(not(any(unix, windows)))]
    {
        &["Ctrl+C"]
    }
}

pub(crate) fn current_launch_parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        unix::current_launch_parent_pid()
    }
    #[cfg(windows)]
    {
        windows::current_launch_parent_pid()
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(unix)]
pub(crate) fn launch_parent_has_exited(initial_parent_pid: u32, current_parent_pid: u32) -> bool {
    initial_parent_pid > 1 && current_parent_pid != initial_parent_pid
}
