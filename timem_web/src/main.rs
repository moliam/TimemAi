mod debug_session;
mod event_journal;
mod server;
mod worker_roles;

#[tokio::main]
async fn main() {
    if let Err(error) = server::run_from_env().await {
        eprintln!("\nTimem Web could not start.\n\n{error}\n");
        std::process::exit(2);
    }
}
