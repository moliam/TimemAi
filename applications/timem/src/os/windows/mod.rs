mod lifecycle;

pub(crate) use lifecycle::{current_launch_parent_pid, ShutdownSignalMonitor};

#[cfg(test)]
pub(crate) use lifecycle::shutdown_signal_names;
