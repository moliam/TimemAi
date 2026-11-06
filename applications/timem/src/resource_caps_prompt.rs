//! Startup escalation prompt for instance resource caps.
//!
//! When a cap dimension needs root (the kernel resource-control hierarchy
//! does not enable the controller yet), a truly interactive terminal may offer
//! a one-shot `sudo` repair.
//! Interactivity is judged by `isatty(stdin)`: background services and test
//! harnesses run with a non-tty stdin and never block on a prompt. The
//! password is streamed to sudo's stdin and zeroized immediately after use;
//! it is never logged, echoed, or persisted.

use std::io::{BufRead, BufReader};
use std::sync::mpsc;

const PROMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn stdin_is_interactive() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::isatty(0) != 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn set_stdin_echo(enabled: bool) {
    let _ = std::process::Command::new("stty")
        .arg(if enabled { "echo" } else { "-echo" })
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Read one line from stdin with a hard timeout; `None` on timeout or read
/// failure. A timed-out reader thread stays blocked on stdin, so the caller
/// must not read stdin again afterwards.
fn read_stdin_line_with_timeout() -> Option<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(std::io::stdin());
        let mut line = String::new();
        let outcome = reader
            .read_line(&mut line)
            .map(|read| (read > 0).then_some(line));
        let _ = sender.send(outcome.ok().flatten());
    });
    receiver.recv_timeout(PROMPT_TIMEOUT).ok().flatten()
}

/// Zeroizes the contained password on drop so every early-return path wipes it.
struct ZeroizingPassword(Vec<u8>);

impl ZeroizingPassword {
    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for ZeroizingPassword {
    fn drop(&mut self) {
        for byte in self.0.iter_mut() {
            *byte = 0;
        }
    }
}

/// Probe the cap dimensions; when root could repair one and stdin is a real
/// terminal, offer the one-shot sudo repair. Unattended starts and declines
/// keep the normal degraded startup path.
pub fn prompt_escalation_and_apply_if_needed() {
    let readiness = agent_core::resource_caps::probe_instance_resource_caps();
    if !readiness.needs_escalation() {
        return;
    }
    let mut dimensions = Vec::new();
    let mut actions = Vec::new();
    for (name, status) in [("cpu", &readiness.cpu), ("memory", &readiness.mem)] {
        if let agent_core::resource_caps::ResourceCapReadiness::NeedsEscalation { actions: plan } =
            status
        {
            dimensions.push(name);
            for action in plan {
                if !actions.contains(action) {
                    actions.push(action.clone());
                }
            }
        }
    }
    let summary = dimensions.join(" + ");
    if !stdin_is_interactive() {
        eprintln!(
            "[resource_caps] {summary} caps need root authorization; start timem once from an interactive terminal to grant it, or apply it later in the WebUI system settings."
        );
        return;
    }
    eprintln!(
        "[resource_caps] {summary} caps protection need root: the kernel resource-control chain does not enable the controller yet."
    );
    eprint!("Authorize a one-shot sudo config now? (Recommended y) [y/N] ");
    use std::io::Write;
    let _ = std::io::stderr().flush();
    let answer = read_stdin_line_with_timeout().unwrap_or_default();
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        eprintln!(
            "[resource_caps] declined or timed out; continuing without kernel {summary} caps (apply later in the WebUI system settings)."
        );
        return;
    }
    eprint!("root password: ");
    let _ = std::io::stderr().flush();
    set_stdin_echo(false);
    let mut raw = read_stdin_line_with_timeout().unwrap_or_default();
    set_stdin_echo(true);
    eprintln!();
    while raw.ends_with('\n') || raw.ends_with('\r') {
        raw.pop();
    }
    let password = ZeroizingPassword(raw.into_bytes());
    if password.as_bytes().is_empty() {
        eprintln!("[resource_caps] empty password; continuing without kernel {summary} caps.");
        return;
    }
    match agent_core::resource_caps::apply_resource_cap_escalation_plan(
        &actions,
        password.as_bytes(),
    ) {
        Ok(()) => eprintln!("[resource_caps] root authorized; {summary} caps are now enforceable."),
        Err(error) => eprintln!(
            "[resource_caps] escalation failed ({error}); continuing without kernel {summary} caps."
        ),
    }
}
