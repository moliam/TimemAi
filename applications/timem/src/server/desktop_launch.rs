//! Product desktop launch adapters.
//!
//! These helpers translate browser and terminal launch requests into direct
//! platform process arguments. They do not use a shell and do not own Web,
//! Session, or Core lifecycle state.

use std::{ffi::OsString, path::Path, process::Command};

pub(super) fn browser_command(url: &str) -> Result<(OsString, Vec<OsString>), String> {
    agent_core::os::browser_command(url).ok_or_else(|| "browser_open_unsupported".to_string())
}

pub(super) fn open_browser(url: &str) -> Result<(), String> {
    let (program, args) = browser_command(url)?;
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|error| error.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub(super) fn should_auto_open_browser() -> bool {
    let is_ssh = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .into_iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()));

    browser_auto_open_allowed_for(is_ssh, agent_core::os::graphical_session_available())
}

pub(super) fn browser_auto_open_allowed_for(is_ssh: bool, has_graphical_session: bool) -> bool {
    !is_ssh && has_graphical_session
}

pub(super) fn open_directory_in_terminal(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err("tool_directory_not_found".to_string());
    }
    let (program, args) = agent_core::os::terminal_command(path)
        .ok_or_else(|| "terminal_open_unsupported".to_string())?;
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|error| format!("terminal_open_failed:{error}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
