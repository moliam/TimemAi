fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match timem::dispatch_mode(args) {
        Ok(timem::LaunchMode::Web(args)) => timem::run_web(args),
        Ok(timem::LaunchMode::Shell(args)) => timem_shell::run_shell(args),
        Err(error) => {
            eprintln!("[config_error] {error}");
            std::process::exit(2);
        }
    }
}
