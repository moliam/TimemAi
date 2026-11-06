//! Startup-time relocation into a private systemd user scope.
//!
//! A bare terminal, tmux or SSH launch inherits the login session's cgroup,
//! which already contains unrelated processes. Enabling a memory controller
//! there force-converts the root into a CPU-only threaded subtree, so memory
//! caps can never be enforced. Re-execing through `systemd-run --user
//! --scope` hands Timem a delegated root that contains only itself, where the
//! existing self-migration and controller-enablement path works without root.

use std::ffi::OsString;

/// Explicit opt-out for operators who manage the scope themselves.
pub(crate) const NO_INDEPENDENT_SCOPE_ENV: &str = "TIMEM_NO_INDEPENDENT_SCOPE";

const HELP_OR_VERSION_ARGS: [&str; 4] = ["-h", "--help", "-V", "--version"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeIsolationAction {
    KeepRunning,
    Relaunch,
}

/// Pure policy: the relaunch marker and the explicit opt-out always win, and
/// a scope that already supports the desired layout is left untouched.
pub(crate) fn scope_isolation_action(
    already_in_independent_scope: bool,
    opted_out: bool,
    needs_isolation: bool,
) -> ScopeIsolationAction {
    use ScopeIsolationAction::{KeepRunning, Relaunch};
    if already_in_independent_scope || opted_out {
        KeepRunning
    } else if needs_isolation {
        Relaunch
    } else {
        KeepRunning
    }
}

/// Best-effort startup relocation. A failed relocation keeps the current
/// process alive: resource caps simply stay in their existing shape, and the
/// warning states why.
pub(crate) fn relaunch_if_needed(args: &[String]) {
    let already_in_independent_scope = env_flag(agent_core::os::INDEPENDENT_SCOPE_ENV);
    let opted_out = env_flag(NO_INDEPENDENT_SCOPE_ENV);
    let needs_isolation = agent_core::os::instance_scope_needs_isolation();
    match scope_isolation_action(already_in_independent_scope, opted_out, needs_isolation) {
        ScopeIsolationAction::KeepRunning => {
            if already_in_independent_scope && needs_isolation {
                eprintln!(
                    "[timem_scope_isolation] relaunched process still lacks a private cgroup with a memory controller; continuing in the current scope"
                );
            }
        }
        ScopeIsolationAction::Relaunch => {
            if args
                .iter()
                .any(|arg| HELP_OR_VERSION_ARGS.contains(&arg.as_str()))
            {
                return;
            }
            let program = match std::env::current_exe() {
                Ok(program) => program,
                Err(error) => {
                    eprintln!(
                        "[timem_scope_isolation] cannot resolve the current executable: {error}; continuing in the current scope"
                    );
                    return;
                }
            };
            let forwarded: Vec<OsString> = args.iter().map(OsString::from).collect();
            if let Err(error) = agent_core::os::relaunch_in_private_user_scope(&program, &forwarded)
            {
                eprintln!(
                    "[timem_scope_isolation] could not enter an independent systemd user scope: {error}; continuing in the current scope (memory caps stay unavailable until Timem runs in its own scope)"
                );
            }
        }
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_shared_scope_without_marker_or_opt_out_relaunches() {
        assert_eq!(
            scope_isolation_action(false, false, true),
            ScopeIsolationAction::Relaunch
        );
        assert_eq!(
            scope_isolation_action(false, false, false),
            ScopeIsolationAction::KeepRunning
        );
        assert_eq!(
            scope_isolation_action(true, false, true),
            ScopeIsolationAction::KeepRunning,
            "the marker must prevent a relaunch loop"
        );
        assert_eq!(
            scope_isolation_action(false, true, true),
            ScopeIsolationAction::KeepRunning,
            "the explicit opt-out must win"
        );
    }
}
