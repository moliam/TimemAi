use super::*;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

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
fn secure_random_fills_buffers_without_reusing_a_fixed_value() {
    let mut first = [0_u8; 32];
    let mut second = [0_u8; 32];
    fill_secure_random(&mut first).expect("platform secure random source");
    fill_secure_random(&mut second).expect("platform secure random source");
    assert_ne!(first, [0_u8; 32]);
    assert_ne!(first, second);
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

#[cfg(target_os = "linux")]
fn wait_until_linux_process_stops(pid: u32, timeout: std::time::Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
        let executing = stat
            .ok()
            .and_then(|stat| stat.rsplit_once(") ").map(|(_, tail)| tail.to_string()))
            .and_then(|tail| tail.split_whitespace().next().map(str::to_string))
            .is_some_and(|state| state != "Z" && state != "X");
        if !executing {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("Linux process {pid} remained executable after {timeout:?}");
}

#[cfg(target_os = "linux")]
fn wait_for_file(path: &Path, timeout: std::time::Duration) -> String {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(value) = std::fs::read_to_string(path) {
            if !value.trim().is_empty() {
                return value;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_policy_handles_empty_xdg_and_os_release_boundaries() {
    assert_eq!(
        linux::config_root(Some(OsStr::new("")), Some(OsStr::new("/home/alice"))),
        PathBuf::from("/home/alice/.config/timem")
    );
    assert_eq!(
        linux::config_root(Some(OsStr::new("")), Some(OsStr::new(""))),
        PathBuf::from("/etc/xdg/timem")
    );
    assert_eq!(
        linux::os_release_value(
            "NAME=Fallback\nPRETTY_NAME=\"Example \\\"Linux\\\" \\\\ Host\"\n",
            "PRETTY_NAME"
        ),
        Some("Example \"Linux\" \\ Host".to_string())
    );
    assert_eq!(linux::os_release_value("NAME=\"   \"", "NAME"), None);
    assert_eq!(linux::os_release_value("NOT_NAME=Linux", "NAME"), None);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_identity_comes_from_proc_start_ticks() {
    let pid = std::process::id();
    let identity = process_identity(pid).expect("current Linux process identity");
    let ticks = identity
        .strip_prefix("linux-start-ticks:")
        .expect("Linux identity prefix");
    assert!(!ticks.is_empty());
    assert!(
        ticks.bytes().all(|byte| byte.is_ascii_digit()),
        "{identity}"
    );
    assert_eq!(process_identity(pid), Some(identity));
    assert_eq!(process_identity(u32::MAX), None);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_child_running_distinguishes_running_and_reaped_children() {
    let mut running = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .expect("spawn running Linux child");
    let running_pid = running.id();
    assert!(child_process_running(running_pid));
    terminate_process(running_pid);
    let status = running.wait().expect("reap terminated Linux child");
    assert_eq!(exit_signal(&status), Some(libc::SIGTERM));
    assert!(!process_running(running_pid));

    let mut exited = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 7"])
        .spawn()
        .expect("spawn exiting Linux child");
    let exited_pid = exited.id();
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!child_process_running(exited_pid));
    assert!(!process_running(exited_pid));
    let wait_error = exited
        .wait()
        .expect_err("child_process_running should have reaped the exited child");
    assert_eq!(wait_error.raw_os_error(), Some(libc::ECHILD));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_runtime_process_group_termination_reaches_descendants() {
    let root = std::env::temp_dir().join(format!(
        "timem-linux-os-group-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create Linux process-group test directory");
    let pid_file = root.join("descendant.pid");
    let script = format!(
        "sleep 30 & child=$!; printf '%s' \"$child\" > '{}'; trap 'kill \"$child\" 2>/dev/null || true; wait \"$child\" 2>/dev/null || true; exit 0' TERM; wait \"$child\"",
        pid_file.display()
    );
    let mut command = std::process::Command::new("/bin/sh");
    command.args(["-c", &script]);
    configure_child_process_group(&mut command);
    let mut leader = command.spawn().expect("spawn Linux process group");
    let leader_pid = leader.id();
    let descendant_pid = wait_for_file(&pid_file, std::time::Duration::from_secs(2))
        .trim()
        .parse::<u32>()
        .expect("numeric descendant pid");

    assert!(is_runtime_child_process_group(leader_pid));
    assert_eq!(runtime_child_pid_kind(), "runtime_child_process_group");
    assert!(process_group_running(leader_pid));
    assert!(process_running(descendant_pid));

    terminate_process(leader_pid);
    let _ = leader.wait().expect("reap Linux process-group leader");
    wait_until_linux_process_stops(descendant_pid, std::time::Duration::from_secs(2));
    assert!(!process_group_running(leader_pid));
    assert!(!process_running(leader_pid));
    assert!(!is_runtime_child_process_group(leader_pid));

    std::fs::remove_dir_all(root).expect("remove Linux process-group test directory");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_group_safety_guards_current_runtime() {
    let current_pid = std::process::id();
    let current_group = unsafe { libc::getpgrp() } as u32;

    assert!(!is_runtime_child_process_group(current_pid));
    assert!(!process_group_running(current_group));
    terminate_process(current_pid);
    kill_process_group(current_group);
    assert_eq!(process_is_alive(u64::from(current_pid)), Some(true));
    assert_eq!(unsafe { libc::kill(libc::getpid(), 0) }, 0);
}

#[cfg(windows)]
#[test]
fn windows_policy_selects_native_script_interpreters() {
    let powershell = windows::command_for_script(Path::new(r"C:\tools\echo.ps1"))
        .expect("PowerShell script command");
    assert_eq!(powershell.get_program(), "powershell.exe");
    let powershell_args = powershell.get_args().collect::<Vec<_>>();
    assert!(powershell_args.contains(&OsStr::new("-NoProfile")));
    assert!(powershell_args.contains(&OsStr::new("-NonInteractive")));
    assert!(powershell_args.contains(&OsStr::new("-File")));
    assert_eq!(
        powershell_args.last().copied(),
        Some(OsStr::new(r"C:\tools\echo.ps1"))
    );

    let batch =
        windows::command_for_script(Path::new(r"C:\tools\echo.cmd")).expect("cmd script command");
    assert_eq!(batch.get_program(), "cmd.exe");
    assert_eq!(
        batch.get_args().collect::<Vec<_>>(),
        vec![
            OsStr::new("/d"),
            OsStr::new("/s"),
            OsStr::new("/c"),
            OsStr::new(r"C:\tools\echo.cmd"),
        ]
    );

    let executable = windows::command_for_script(Path::new(r"C:\tools\echo.exe"))
        .expect("native executable command");
    assert_eq!(executable.get_program(), OsStr::new(r"C:\tools\echo.exe"));
    assert_eq!(
        windows::command_for_script(Path::new(r"C:\tools\echo.py")).unwrap_err(),
        "unsupported_windows_command_extension:py"
    );
}

#[cfg(windows)]
#[test]
fn windows_process_identity_and_parent_are_available() {
    let pid = std::process::id();
    assert_eq!(process_is_alive(u64::from(pid)), Some(true));
    let identity = process_identity(pid).expect("current Windows process identity");
    assert!(identity.starts_with("windows-creation-time:"), "{identity}");
    assert_eq!(process_identity(pid), Some(identity));
    assert!(current_parent_pid().is_some());
}
