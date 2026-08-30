mod debug_session;
mod lifecycle_diagnostics;
mod os;
mod runtime_log;
mod semantic_delivery;
mod server;
mod session_groups;
mod web_instance;
mod worker_roles;

use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    Web(Vec<String>),
    Shell(Vec<String>),
}

pub fn dispatch_mode(args: Vec<String>) -> Result<LaunchMode, String> {
    let shell_count = args.iter().filter(|arg| arg.as_str() == "--shell").count();
    if shell_count > 1 {
        return Err("duplicate_option:--shell".to_string());
    }
    if shell_count == 0 {
        return Ok(LaunchMode::Web(args));
    }
    Ok(LaunchMode::Shell(
        args.into_iter().filter(|arg| arg != "--shell").collect(),
    ))
}

const WORKSPACE_HANDOFF_TIMEOUT: Duration = Duration::from_secs(3);
const WORKSPACE_HANDOFF_POLL_INTERVAL: Duration = Duration::from_millis(50);

fn acquire_workspace_instance_lock(
    memory_root: &std::path::Path,
) -> Result<agent_core::WorkspaceInstanceLock, String> {
    let deadline = Instant::now() + WORKSPACE_HANDOFF_TIMEOUT;
    loop {
        match agent_core::WorkspaceInstanceLock::acquire(memory_root, "timem-web") {
            Ok(lock) => return Ok(lock),
            Err(error) if error == "workspace_already_in_use" && Instant::now() < deadline => {
                std::thread::sleep(
                    WORKSPACE_HANDOFF_POLL_INTERVAL
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn run_web(args: Vec<String>) {
    let memory_root = match lifecycle_diagnostics::memory_root_from_args(&args) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("\nTimem Web could not start.\n\n{error}\n");
            std::process::exit(2);
        }
    };
    let help_requested = args.iter().any(|arg| arg == "--help" || arg == "-h");
    let workspace_lock = if help_requested {
        None
    } else {
        if let Err(error) = agent_core::create_memory_dir(&memory_root) {
            eprintln!("[timem_web_diagnostics_unavailable] {error}");
            eprintln!("\nTimem Web could not start.\n\n{error}\n");
            std::process::exit(2);
        }
        match acquire_workspace_instance_lock(&memory_root) {
            Ok(lock) => Some(lock),
            Err(error) => {
                let message = server::friendly_workspace_instance_error(
                    error,
                    memory_root.to_string_lossy().as_ref(),
                );
                eprintln!("\nTimem Web could not start.\n\n{message}\n");
                std::process::exit(2);
            }
        }
    };
    let _workspace_lock = workspace_lock;
    let diagnostics = lifecycle_diagnostics::LifecycleDiagnostics::install_for(&memory_root, &args)
        .unwrap_or_else(|error| {
            eprintln!("[timem_web_diagnostics_unavailable] {error}");
            lifecycle_diagnostics::LifecycleDiagnostics::disabled()
        });
    if let Some(root) = diagnostics.root() {
        println!("Timem Web lifecycle diagnostics: {}", root.display());
    }

    // Debug-build-only process fault injection for deterministic diagnostics tests.
    // Release binaries do not compile this branch.
    #[cfg(debug_assertions)]
    if std::env::var("TIMEM_WEB_TEST_FAULT").as_deref() == Ok("panic_after_diagnostics_install") {
        diagnostics.event("test_fault_injected", serde_json::json!({"kind": "panic"}));
        panic!("injected panic with Bearer test-private-token");
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let message = format!("tokio_runtime_create_failed:{error}");
            diagnostics.finish("runtime_initialization_error", false, Some(&message));
            eprintln!("\nTimem Web could not start.\n\n{message}\n");
            std::process::exit(2);
        }
    };

    match runtime.block_on(server::run(&args, &diagnostics)) {
        Ok(reason) => diagnostics.finish(reason.label(), true, None),
        Err(error) => {
            diagnostics.finish("startup_or_runtime_error", false, Some(&error));
            eprintln!("\nTimem Web could not start.\n\n{error}\n");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod launch_mode_tests {
    use super::*;

    #[test]
    fn defaults_to_web_and_reserves_only_the_top_level_shell_flag() {
        assert_eq!(
            dispatch_mode(vec!["--public".to_string()]).unwrap(),
            LaunchMode::Web(vec!["--public".to_string()])
        );
        assert_eq!(
            dispatch_mode(vec![
                "--shell".to_string(),
                "--once-json".to_string(),
                "hello".to_string(),
            ])
            .unwrap(),
            LaunchMode::Shell(vec!["--once-json".to_string(), "hello".to_string()])
        );
    }

    #[test]
    fn duplicate_shell_selector_is_rejected() {
        assert_eq!(
            dispatch_mode(vec!["--shell".to_string(), "--shell".to_string()]),
            Err("duplicate_option:--shell".to_string())
        );
    }
}
