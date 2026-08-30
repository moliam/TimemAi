fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match timem_web::dispatch_mode(args) {
        Ok(timem_web::LaunchMode::Web(args)) => timem_web::run_web(args),
        Ok(timem_web::LaunchMode::Shell(args)) => timem_shell::run_shell(args),
        Err(error) => {
            eprintln!("[config_error] {error}");
            std::process::exit(2);
        }
    }
}
