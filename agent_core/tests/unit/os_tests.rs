use super::*;
use std::ffi::OsStr;

#[test]
fn host_environment_contains_os_and_bash_without_template_syntax() {
    let environment = host_environment();
    assert!(environment.starts_with("OS: "), "{environment}");
    assert!(environment.contains("; Bash: "), "{environment}");
    assert!(!environment.contains("{{"), "{environment}");
}

#[test]
fn explicit_config_root_has_priority() {
    assert_eq!(
        default_config_root(
            Some(OsStr::new("/custom/timem")),
            Some(OsStr::new("/xdg")),
            Some(OsStr::new("/home/user")),
        ),
        PathBuf::from("/custom/timem")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_policy_uses_application_support_and_native_open() {
    assert_eq!(
        macos::config_root(Some(OsStr::new("/Users/alice"))),
        PathBuf::from("/Users/alice/Library/Application Support/TimemAi")
    );

    let (program, args) = browser_command("http://127.0.0.1").expect("browser command");
    assert_eq!(program, "open");
    assert_eq!(args, vec![OsString::from("http://127.0.0.1")]);

    let (program, args) = terminal_command(Path::new("/tmp")).expect("terminal command");
    assert_eq!(program, "open");
    assert_eq!(args[0], "-a");
    assert_eq!(args[1], "Terminal");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_policy_uses_xdg_paths_and_parses_os_release() {
    assert_eq!(
        linux::config_root(
            Some(OsStr::new("/home/alice/.xdg")),
            Some(OsStr::new("/home/alice")),
        ),
        PathBuf::from("/home/alice/.xdg/timem")
    );
    assert_eq!(
        linux::config_root(None, Some(OsStr::new("/home/alice"))),
        PathBuf::from("/home/alice/.config/timem")
    );
    assert_eq!(
        linux::config_root(None, None),
        PathBuf::from("/etc/xdg/timem")
    );

    let (program, args) = linux::browser_command("http://127.0.0.1");
    assert_eq!(program, "xdg-open");
    assert_eq!(args, vec![OsString::from("http://127.0.0.1")]);

    let (program, args) = linux::terminal_command(Path::new("/tmp"));
    assert_eq!(program, "x-terminal-emulator");
    assert_eq!(
        args,
        vec![
            OsString::from("--working-directory"),
            OsString::from("/tmp"),
        ]
    );

    // These environment-backed probes may legitimately return either result on
    // a non-Linux test host; invoking them still verifies that both policy
    // interfaces remain compilable without duplicating platform logic.
    let _ = linux::version();
    let _ = linux::graphical_session_available();

    assert_eq!(
        linux::os_release_value("PRETTY_NAME=\"Example Linux 1\"", "PRETTY_NAME"),
        Some("Example Linux 1".to_string())
    );
}

#[test]
fn process_liveness_helpers_are_conservative_and_consistent() {
    let current_pid = std::process::id();
    assert_eq!(process_is_alive(u64::from(current_pid)), Some(true));
    assert!(process_may_be_alive(current_pid));
    assert!(!process_is_definitely_dead(current_pid));

    // PID zero is never a live user process on supported Unix hosts. On an
    // unsupported platform the optional primitive may be unknown, while the
    // ownership helper must still remain conservative.
    if let Some(alive) = process_is_alive(0) {
        assert!(!alive);
        assert!(process_is_definitely_dead(0));
    } else {
        assert!(process_may_be_alive(0));
        assert!(!process_is_definitely_dead(0));
    }
}

#[test]
fn current_process_file_is_owned_by_the_effective_user_when_supported() {
    let path = std::env::temp_dir().join(format!(
        "timem-owner-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos()
    ));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create current-user-owned test file");
    #[cfg(unix)]
    assert!(path_owned_by_current_user(&path));
    #[cfg(not(unix))]
    assert!(!path_owned_by_current_user(&path));
    drop(file);
    std::fs::remove_file(path).expect("remove ownership test file");
}
