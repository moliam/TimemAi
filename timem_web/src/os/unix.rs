use super::{launch_parent_has_exited, ShutdownTrigger};

pub(crate) struct ShutdownSignalMonitor {
    interrupt: Option<tokio::signal::unix::Signal>,
    terminate: Option<tokio::signal::unix::Signal>,
    hangup: Option<tokio::signal::unix::Signal>,
}

impl ShutdownSignalMonitor {
    pub(crate) fn capture() -> Self {
        use tokio::signal::unix::{signal, SignalKind};
        Self {
            interrupt: signal(SignalKind::interrupt()).ok(),
            terminate: signal(SignalKind::terminate()).ok(),
            hangup: signal(SignalKind::hangup()).ok(),
        }
    }

    pub(crate) async fn detect(mut self, launch_parent_pid: Option<u32>) -> ShutdownTrigger {
        tokio::select! {
            _ = recv_optional_signal(&mut self.interrupt) => ShutdownTrigger::CtrlC,
            _ = recv_optional_signal(&mut self.terminate) => ShutdownTrigger::Sigterm,
            _ = recv_optional_signal(&mut self.hangup) => ShutdownTrigger::Sighup,
            _ = wait_for_launch_parent_exit(launch_parent_pid) => ShutdownTrigger::ParentProcessExited,
        }
    }
}

#[cfg(test)]
pub(crate) fn shutdown_signal_names() -> &'static [&'static str] {
    &["Ctrl+C", "SIGTERM", "SIGHUP", "parent shell exit"]
}

pub(crate) fn current_launch_parent_pid() -> Option<u32> {
    u32::try_from(unsafe { libc::getppid() })
        .ok()
        .filter(|pid| *pid > 1)
}

async fn wait_for_launch_parent_exit(initial_parent_pid: Option<u32>) {
    let Some(initial_parent_pid) = initial_parent_pid.filter(|pid| *pid > 1) else {
        std::future::pending::<()>().await;
        return;
    };

    let mut check = tokio::time::interval(std::time::Duration::from_millis(250));
    check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        check.tick().await;
        let current_parent_pid = current_launch_parent_pid().unwrap_or(1);
        if launch_parent_has_exited(initial_parent_pid, current_parent_pid) {
            eprintln!(
                "Timem Web launcher process {initial_parent_pid} exited; shutting down the entire Agent runtime."
            );
            return;
        }
    }
}

async fn recv_optional_signal(stream: &mut Option<tokio::signal::unix::Signal>) {
    if let Some(stream) = stream.as_mut() {
        let _ = stream.recv().await;
    } else {
        std::future::pending::<()>().await;
    }
}
