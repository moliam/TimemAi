mod debug_session;
mod event_journal;
mod lifecycle_diagnostics;
mod server;
mod session_groups;
mod worker_roles;

fn main() {
    let diagnostics = lifecycle_diagnostics::LifecycleDiagnostics::install_from_env()
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

    match runtime.block_on(server::run_from_env(&diagnostics)) {
        Ok(reason) => diagnostics.finish(reason.label(), true, None),
        Err(error) => {
            diagnostics.finish("startup_or_runtime_error", false, Some(&error));
            eprintln!("\nTimem Web could not start.\n\n{error}\n");
            std::process::exit(2);
        }
    }
}
