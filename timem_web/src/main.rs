mod server;

#[tokio::main]
async fn main() {
    if let Err(error) = server::run_from_env().await {
        eprintln!("[timem_web_error] {error}");
        std::process::exit(2);
    }
}
