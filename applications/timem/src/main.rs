fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match timem::dispatch_mode(args) {
        Ok(timem::LaunchMode::Web(args)) => timem::run_web(args),
        Ok(timem::LaunchMode::Shell(args)) => timem_shell::run_shell(args),
        Ok(timem::LaunchMode::Attach(args)) => {
            if let Err(error) = timem_shell::validate_cli_value_args(&args) {
                eprintln!("[config_error] {error}");
                std::process::exit(2);
            }
            let options = timem_shell::parse_cli_args(&args);
            timem_shell::attach::run_attach(options.space.as_deref(), options.ui_mode.as_deref())
        }
        Err(error) => {
            eprintln!("[config_error] {error}");
            std::process::exit(2);
        }
    }
}
