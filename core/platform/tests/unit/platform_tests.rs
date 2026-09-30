use super::*;
use std::ffi::OsStr;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::ffi::OsString;
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

#[cfg(target_os = "linux")]
#[test]
fn subreaper_safety_net_adopts_and_sweeps_detached_orphans() {
    use crate::install_process_subreaper;
    use crate::reparented_detached_child_pids;
    assert!(
        install_process_subreaper(),
        "Linux runtime must support PR_SET_CHILD_SUBREAPER"
    );
    // Spawn a `setsid` child that outlives its shell and escapes the managed
    // process group; the subreaper flag must reparent it to this test process
    // in its own session, and the sweep list must find it.
    let marker =
        std::env::temp_dir().join(format!("timem_subreaper_test_{}.pid", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let script = format!(
        "setsid sh -c 'echo $$ > {}; while [ -e {0} ]; do sleep 0.2; done' &\nsleep 0.1\n",
        marker.display()
    );
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&script)
        .status()
        .expect("spawn escapee script");
    assert!(status.success(), "escapee script must run");
    let orphan_pid: u32 = std::fs::read_to_string(&marker)
        .expect("escapee marker file")
        .trim()
        .parse()
        .expect("escapee pid");
    let _ = std::fs::remove_file(&marker);
    assert_ne!(orphan_pid, std::process::id());
    // Give the kernel a moment; then the orphan must appear in the sweep list
    // (ppid == this process, own session after setsid escape).
    let mut found = false;
    for _ in 0..50 {
        if reparented_detached_child_pids().contains(&orphan_pid) {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(found, "setsid escapee must be reparented and detected");
    // Terminate the orphan via the same primitive the sweep uses and confirm
    // it leaves the sweep list.
    crate::terminate_process(orphan_pid);
    let mut gone = false;
    for _ in 0..50 {
        if !reparented_detached_child_pids().contains(&orphan_pid) {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(gone, "swept orphan must disappear from the sweep list");
}

#[cfg(target_os = "linux")]
#[test]
fn subreaper_reaps_self_exited_adopted_orphans() {
    use crate::install_process_subreaper;
    use crate::reparented_detached_child_pids;
    use crate::try_reap_child_process;
    assert!(install_process_subreaper());
    // Spawn a setsid orphan that exits on its own. The subreaper adopts it,
    // but without an explicit reap it stays a zombie forever and
    // kill(pid, 0) keeps reporting it alive — the exact regression behind
    // the lifecycle smoke "Host survived" failure.
    let marker = std::env::temp_dir().join(format!(
        "timem-orphan-reap-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let status = std::process::Command::new("bash")
        .arg("-c")
        .arg(format!(
            "setsid bash -c 'echo $$ > {:?}; exit 0' & wait",
            marker
        ))
        .status()
        .expect("spawn orphan shell");
    assert!(status.success());
    let orphan_pid: u32 = std::fs::read_to_string(&marker)
        .expect("orphan marker file")
        .trim()
        .parse()
        .expect("orphan pid");
    let _ = std::fs::remove_file(&marker);
    // Wait until it is adopted (appears in the sweep list); once it has
    // exited, try_reap must remove the zombie.
    let mut reaped = false;
    for _ in 0..100 {
        if (reparented_detached_child_pids().contains(&orphan_pid)
            || !crate::process_running(orphan_pid))
            && try_reap_child_process(orphan_pid)
            && !crate::process_running(orphan_pid)
        {
            reaped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(reaped, "self-exited adopted orphan must be reaped");
}

#[cfg(unix)]
#[test]
fn list_live_process_group_members_reports_spawned_child() {
    use crate::configure_child_process_group;
    use std::process::Command;
    let marker = std::env::temp_dir().join("timem_pgid_member_test");
    let _ = std::fs::remove_file(&marker);
    let mut child = Command::new("bash");
    configure_child_process_group(&mut child);
    let mut child = child
        .arg("-c")
        .arg(format!("echo $$ > {:?}; sleep 5", marker))
        .spawn()
        .expect("spawn group leader");
    let pgid = child.id();
    let mut member_text = String::new();
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&marker) {
            member_text = text;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let member: u32 = member_text
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("member pid from marker: {member_text:?}"));
    // the leader itself is a live member of its own group
    let members = crate::list_live_process_group_members(pgid);
    assert!(
        members.contains(&pgid),
        "leader must be listed: {members:?}"
    );
    // the spawned sleep shares the group on Linux
    if crate::process_running(member) {
        assert!(
            members.contains(&member),
            "spawned child must be listed: {members:?}"
        );
    }
    crate::kill_process_group(pgid);
    let _ = child.wait();
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn list_live_process_group_members_empty_after_group_exit() {
    use crate::configure_child_process_group;
    use std::process::Command;
    let mut child = Command::new("bash");
    configure_child_process_group(&mut child);
    let mut _child = child
        .arg("-c")
        .arg("exit 0")
        .spawn()
        .expect("spawn short-lived leader");
    let pgid = _child.id();
    let _ = _child.wait();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(crate::list_live_process_group_members(pgid).is_empty());
}

#[test]
#[cfg(target_os = "linux")]
fn local_filesystem_mount_points_reports_root_and_skips_pseudo() {
    let mounts = crate::local_filesystem_mount_points();
    assert!(
        mounts.contains(&std::path::PathBuf::from("/")),
        "{mounts:?}"
    );
    // Pseudo filesystems must never appear in the sample set.
    for mount in &mounts {
        let text = mount.display().to_string();
        assert!(!text.starts_with("/proc"), "{text}");
        assert!(!text.starts_with("/sys"), "{text}");
        assert!(!text.starts_with("/dev"), "{text}");
        assert!(!text.starts_with("/run"), "{text}");
    }
}

#[test]
fn managed_command_status_preserves_owner_exit_status() {
    let status = crate::command_status(
        std::process::Command::new(crate::POSIX_SHELL_EXECUTABLE).args(["-c", "exit 27"]),
    )
    .expect("run registered synchronous command");
    assert_eq!(status.code(), Some(27));
}

#[test]
fn managed_command_output_preserves_captured_streams_and_status() {
    let output = crate::command_output(
        std::process::Command::new(crate::POSIX_SHELL_EXECUTABLE)
            .args(["-c", "printf stdout-text; printf stderr-text >&2; exit 9"]),
    )
    .expect("run registered captured command");
    assert_eq!(output.status.code(), Some(9));
    assert_eq!(output.stdout, b"stdout-text");
    assert_eq!(output.stderr, b"stderr-text");
}

#[cfg(target_os = "linux")]
fn fallback_events_for_pid(pid: u32) -> Vec<crate::OrphanProcessEvent> {
    crate::take_orphan_process_events()
        .into_iter()
        .filter(|event| event.pid == pid)
        .collect()
}

#[cfg(target_os = "linux")]
fn fallback_contains(pid: u32) -> bool {
    crate::fallback_process_snapshots()
        .iter()
        .any(|snapshot| snapshot.pid == pid)
}

#[cfg(target_os = "linux")]
#[test]
fn linux_fallback_reaper_does_not_consume_registered_child_exit_status() {
    assert!(crate::install_process_subreaper());
    let _ = crate::take_orphan_process_events();
    let mut child = std::process::Command::new("sh")
        .args(["-c", "exit 23"])
        .spawn()
        .expect("spawn managed child");
    let child_pid = child.id();
    let _registration = crate::register_managed_child(child_pid);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while crate::process_may_be_alive(child_pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut reaper = crate::FallbackProcessReaper::for_runtime();
    for _ in 0..8 {
        assert_eq!(reaper.reap_adopted_zombies(), 0);
    }

    assert!(!fallback_contains(child_pid));
    assert!(fallback_events_for_pid(child_pid).is_empty());
    let status = child.wait().expect("managed owner retains wait status");
    assert_eq!(status.code(), Some(23));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_fallback_registration_race_requires_stable_unowned_observations() {
    assert!(crate::install_process_subreaper());
    let _ = crate::take_orphan_process_events();
    let mut child = std::process::Command::new("sh")
        .args(["-c", "sleep 5"])
        .spawn()
        .expect("spawn child before ownership registration");
    let child_pid = child.id();
    let mut reaper = crate::FallbackProcessReaper::for_runtime();

    // Three scans model the spawn-to-registration window. Adoption requires
    // four stable unowned observations, so this known PID must remain absent.
    for _ in 0..3 {
        assert_eq!(reaper.reap_adopted_zombies(), 0);
    }
    assert!(!fallback_contains(child_pid));
    assert!(fallback_events_for_pid(child_pid).is_empty());

    let registration = crate::register_managed_child(child_pid);
    for _ in 0..8 {
        assert_eq!(reaper.reap_adopted_zombies(), 0);
    }
    assert!(!fallback_contains(child_pid));
    assert!(fallback_events_for_pid(child_pid).is_empty());

    unsafe {
        libc::kill(child_pid as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.wait();
    drop(registration);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_fallback_lifecycle_is_adopted_active_then_exactly_reaped() {
    assert!(crate::install_process_subreaper());
    let _ = crate::take_orphan_process_events();
    let marker = std::env::temp_dir().join(format!(
        "timem-same-session-orphan-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let script = format!("sh -c 'echo $$ > {:?}; sleep 5' & exit 0", marker);
    let mut parent = std::process::Command::new("sh")
        .args(["-c", &script])
        .spawn()
        .expect("spawn intermediate parent");
    let registration = crate::register_managed_child(parent.id());
    assert!(parent.wait().expect("wait intermediate parent").success());
    drop(registration);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let orphan_pid: u32 = std::fs::read_to_string(&marker)
        .expect("descendant pid marker")
        .trim()
        .parse()
        .expect("descendant pid");
    let _ = std::fs::remove_file(&marker);

    // This descendant did not call setsid, so session-based observation cannot
    // find it. Its known PID reaches fallback only because its parent exited.
    assert!(
        !crate::reparented_detached_child_pids().contains(&orphan_pid),
        "same-session descendant must exercise runtime-wide fallback"
    );

    let mut reaper = crate::FallbackProcessReaper::for_runtime();
    for _ in 0..4 {
        assert_eq!(reaper.reap_adopted_zombies(), 0);
    }
    assert!(fallback_contains(orphan_pid));
    let adopted = fallback_events_for_pid(orphan_pid);
    assert_eq!(adopted.len(), 1, "one adoption event per process identity");
    assert_eq!(adopted[0].state, "adopted");

    // Additional scans while alive must keep one active row without
    // republishing adoption.
    for _ in 0..4 {
        assert_eq!(reaper.reap_adopted_zombies(), 0);
    }
    assert!(fallback_contains(orphan_pid));
    assert!(fallback_events_for_pid(orphan_pid).is_empty());

    unsafe {
        libc::kill(orphan_pid as libc::pid_t, libc::SIGKILL);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut reaped = false;
    while std::time::Instant::now() < deadline {
        if reaper.reap_adopted_zombies() == 1 {
            reaped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        reaped,
        "fallback must perform the exact final wait for {orphan_pid}"
    );
    assert!(!fallback_contains(orphan_pid));
    let terminal = fallback_events_for_pid(orphan_pid);
    assert_eq!(terminal.len(), 1, "one terminal event per process identity");
    assert_eq!(terminal[0].state, "reaped");
    assert!(!crate::process_may_be_alive(orphan_pid));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_managed_process_job_contains_and_kills_setsid_descendants() {
    let job = match ManagedProcessJob::create() {
        Ok(job) => job,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
            ) =>
        {
            eprintln!("skipping real cgroup-v2 containment test: {error}");
            return;
        }
        Err(error) => panic!("create managed process job: {error}"),
    };
    assert_eq!(job.backend_name(), "linux_cgroup_v2");
    let observation_note = job.observation_note().expect("Linux cgroup note");
    assert!(observation_note.starts_with("cgroup: /sys/fs/cgroup/"));
    assert!(observation_note.contains("/timem.jobs/job-"));
    let root = std::env::temp_dir().join(format!(
        "timem-linux-cgroup-job-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create cgroup test directory");
    let pid_file = root.join("escapee.pid");
    let script = format!(
        "setsid --fork sh -c 'printf %s $$ > \"{}\"; sleep 30'; sleep 30",
        pid_file.display()
    );
    let mut command = std::process::Command::new("/bin/sh");
    command.args(["-c", &script]);
    configure_child_process_group(&mut command);
    job.configure_command(&mut command)
        .expect("configure child self-placement");
    let mut leader = command.spawn().expect("spawn cgroup-owned command");
    let leader_pid = leader.id();
    let escapee_pid = wait_for_file(&pid_file, std::time::Duration::from_secs(2))
        .trim()
        .parse::<u32>()
        .expect("numeric setsid descendant pid");

    let members = job.member_pids().expect("read cgroup members");
    assert!(members.contains(&leader_pid), "members={members:?}");
    assert!(members.contains(&escapee_pid), "members={members:?}");
    assert!(!job.is_empty().expect("read populated state"));

    job.kill_all().expect("kill exactly this managed job");
    let _ = leader.wait().expect("reap cgroup leader");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !job.is_empty().unwrap_or(false) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(job.is_empty().expect("read final populated state"));
    wait_until_linux_process_stops(escapee_pid, std::time::Duration::from_secs(2));
    std::fs::remove_dir_all(root).expect("remove cgroup test directory");
}
