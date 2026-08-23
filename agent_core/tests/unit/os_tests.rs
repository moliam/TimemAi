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
