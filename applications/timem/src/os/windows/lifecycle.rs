use super::ShutdownTrigger;

pub(crate) struct ShutdownSignalMonitor;

impl ShutdownSignalMonitor {
    pub(crate) fn capture() -> Self {
        Self
    }

    pub(crate) async fn detect(self, launch_parent_pid: Option<u32>) -> ShutdownTrigger {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => ShutdownTrigger::CtrlC,
            _ = wait_for_launch_parent_exit(launch_parent_pid) => ShutdownTrigger::ParentProcessExited,
        }
    }
}

#[cfg(test)]
pub(crate) fn shutdown_signal_names() -> &'static [&'static str] {
    &["Ctrl+C", "parent process exit"]
}

pub(crate) fn current_launch_parent_pid() -> Option<u32> {
    agent_core::os::current_parent_pid()
}

async fn wait_for_launch_parent_exit(initial_parent_pid: Option<u32>) {
    let Some(initial_parent_pid) = initial_parent_pid.filter(|pid| *pid > 1) else {
        std::future::pending::<()>().await;
        return;
    };
    let initial_identity = agent_core::os::process_identity(initial_parent_pid);
    let mut check = tokio::time::interval(std::time::Duration::from_millis(250));
    check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        check.tick().await;
        let alive = agent_core::os::process_is_alive(u64::from(initial_parent_pid));
        let identity_matches = match initial_identity.as_deref() {
            Some(expected) => {
                agent_core::os::process_identity(initial_parent_pid).as_deref() == Some(expected)
            }
            None => true,
        };
        if alive == Some(false) || !identity_matches {
            eprintln!(
                "Timem Web launcher process {initial_parent_pid} exited; shutting down the entire Agent runtime."
            );
            return;
        }
    }
}
