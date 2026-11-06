use super::*;
use std::ffi::OsStr;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::test_support::process_lifecycle_test_lock;

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
    let _process_lifecycle = process_lifecycle_test_lock();
    let mut running = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .expect("spawn running Linux child");
    let running_pid = running.id();
    let _running_registration = crate::register_managed_child(running_pid);
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
    let _process_lifecycle = process_lifecycle_test_lock();
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
    let _leader_registration = crate::register_managed_child(leader_pid);
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
    let _process_lifecycle = process_lifecycle_test_lock();
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
    let _process_lifecycle = process_lifecycle_test_lock();
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
    let _process_lifecycle = process_lifecycle_test_lock();
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
    let _process_lifecycle = process_lifecycle_test_lock();
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
#[cfg(target_os = "macos")]
fn local_filesystem_mount_points_reports_root_and_skips_pseudo() {
    let mounts = crate::local_filesystem_mount_points();
    assert!(
        mounts.contains(&std::path::PathBuf::from("/")),
        "{mounts:?}"
    );
    // Pseudo filesystems (devfs, autofs) and system volumes (VM swap,
    // Preboot, Update, xarts, ...) must never appear; external disks live
    // directly under /Volumes only.
    for mount in &mounts {
        let text = mount.display().to_string();
        assert!(!text.starts_with("/dev"), "{text}");
        assert!(!text.starts_with("/System/Volumes/"), "{text}");
        if text != "/" {
            assert!(text.strip_prefix("/Volumes/").is_some(), "{text}");
        }
    }
}

#[test]
fn managed_command_status_preserves_owner_exit_status() {
    let _process_lifecycle = process_lifecycle_test_lock();
    let status = crate::command_status(
        std::process::Command::new(crate::POSIX_SHELL_EXECUTABLE).args(["-c", "exit 27"]),
    )
    .expect("run registered synchronous command");
    assert_eq!(status.code(), Some(27));
}

#[test]
fn managed_command_output_preserves_captured_streams_and_status() {
    let _process_lifecycle = process_lifecycle_test_lock();
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
    let _process_lifecycle = process_lifecycle_test_lock();
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
    let _process_lifecycle = process_lifecycle_test_lock();
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
    let _process_lifecycle = process_lifecycle_test_lock();
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

#[test]
fn process_safety_net_initialization_is_idempotent_and_non_blocking() {
    let _process_lifecycle = process_lifecycle_test_lock();
    let first = ensure_process_safety_net();
    let first_status = first.status();
    let second = ensure_process_safety_net();
    assert_eq!(second.status(), first_status);
    assert!(matches!(
        first_status,
        ProcessSafetyNetStatus::Active
            | ProcessSafetyNetStatus::Unsupported
            | ProcessSafetyNetStatus::InitializationFailed
    ));
    drop(second);
    drop(first);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_safety_cleanup_removes_empty_current_runtime_scopes() {
    let session_id = format!(
        "cleanup-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos()
    );
    let job = match ManagedProcessJob::create_for_session(&session_id) {
        Ok(job) => job,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied
                    | std::io::ErrorKind::Unsupported
                    | std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            eprintln!("skipping real cgroup-v2 cleanup test: {error}");
            return;
        }
        Err(error) => panic!("create cleanup test process job: {error}"),
    };
    let job_path = PathBuf::from(
        job.observation_note()
            .expect("Linux cgroup note")
            .strip_prefix("cgroup: ")
            .expect("cgroup path prefix"),
    );
    let session_path = job_path.parent().expect("session parent").to_path_buf();
    let runtime_path = session_path.parent().expect("runtime parent").to_path_buf();
    drop(job);

    assert!(!job_path.exists(), "empty Job scope must be removed");
    assert!(
        session_path.exists(),
        "Session aggregate scope must remain until Runtime cleanup"
    );
    assert!(
        runtime_path.exists(),
        "Runtime aggregate scope must remain until Runtime cleanup"
    );
    cleanup_process_safety_net();
    assert!(
        !session_path.exists(),
        "empty Session scope must be removed during Runtime cleanup"
    );
    if runtime_path.exists() {
        assert!(
            std::fs::read_dir(&runtime_path)
                .expect("read surviving Runtime scope")
                .next()
                .is_some(),
            "an empty current Runtime scope must be removed"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_aggregate_scope_snapshot_exposes_runtime_and_opaque_session_directories() {
    let raw_session_id = format!("aggregate-private-session-{}", std::process::id());
    let snapshot = match process_aggregate_scope_snapshot(&raw_session_id) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => panic!("Linux must expose its cgroup aggregate scopes"),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied
                    | std::io::ErrorKind::Unsupported
                    | std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            eprintln!("skipping real aggregate cgroup-v2 test: {error}");
            return;
        }
        Err(error) => panic!("create aggregate process scopes: {error}"),
    };
    let runtime = PathBuf::from(
        snapshot
            .runtime_observation_note
            .strip_prefix("cgroup: ")
            .expect("runtime cgroup prefix"),
    );
    let session = PathBuf::from(
        snapshot
            .session_observation_note
            .strip_prefix("cgroup: ")
            .expect("session cgroup prefix"),
    );
    assert!(runtime.is_dir(), "{}", runtime.display());
    assert!(session.is_dir(), "{}", session.display());
    assert_eq!(session.parent(), Some(runtime.as_path()));
    assert!(runtime.join("cgroup.procs").is_file());
    assert!(session.join("cgroup.procs").is_file());
    assert!(!snapshot.runtime_observation_note.contains(&raw_session_id));
    assert!(!snapshot.session_observation_note.contains(&raw_session_id));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_managed_process_job_contains_and_kills_setsid_descendants() {
    let _process_lifecycle = process_lifecycle_test_lock();
    let raw_session_id = "private-session-id-must-not-appear";
    let job = match ManagedProcessJob::create_for_session(raw_session_id) {
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
    assert!(
        observation_note.contains("/timem.jobs/runtime-"),
        "{observation_note}"
    );
    assert!(observation_note.contains("/session-"), "{observation_note}");
    assert!(observation_note.contains("/job-"), "{observation_note}");
    assert!(
        !observation_note.contains(raw_session_id),
        "{observation_note}"
    );
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

#[cfg(target_os = "linux")]
#[test]
fn linux_process_observation_note_points_to_readable_membership() {
    let pid = std::process::id();
    let note = crate::process_observation_note(pid);
    assert!(note.contains(&format!("/proc/{pid}/cgroup")));
    assert!(std::fs::read_to_string(format!("/proc/{pid}/cgroup")).is_ok());
    assert!(note.contains("not task ownership"));
}

/// Keep a child unreaped until Drop, so cleanup cannot signal a reused PID.
#[cfg(target_os = "macos")]
struct MacosTestChild(std::process::Child);

#[cfg(target_os = "macos")]
impl Drop for MacosTestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_older_group_remains_visible_and_term_resistant_child_is_killed() {
    let _process_lifecycle = process_lifecycle_test_lock();
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let mut command = Command::new("/bin/sh");
    // The ready line is emitted only after SIGTERM has been ignored. exec
    // preserves that disposition without spawning an unowned descendant.
    command.args(["-c", "trap '' TERM; echo ready; exec /bin/sleep 20"]);
    command.stdout(Stdio::piped());
    configure_child_process_group(&mut command);
    let mut leader = MacosTestChild(command.spawn().expect("spawn group"));
    let mut ready = String::new();
    BufReader::new(leader.0.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    let pid = leader.0.id();
    // Darwin lists recent births first. A test of only the newest child
    // cannot detect a truncated global PID enumeration.
    let _newer: Vec<_> = (0..100)
        .map(|_| MacosTestChild(Command::new("/bin/sleep").arg("20").spawn().unwrap()))
        .collect();
    assert!(process_group_running(pid), "older live group disappeared");
    assert!(list_live_process_group_members(pid).contains(&pid));
    terminate_process_group(pid);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let status = loop {
        if let Some(status) = leader.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "SIGKILL escalation failed"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(exit_signal(&status), Some(libc::SIGKILL));
    assert!(!process_group_running(pid));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_zombie_only_group_is_not_running_before_reap() {
    let _process_lifecycle = process_lifecycle_test_lock();
    use std::io::{BufRead, BufReader};
    let mut command = std::process::Command::new("/bin/sh");
    command.args(["-c", "echo ready; exit 0"]);
    command.stdout(std::process::Stdio::piped());
    configure_child_process_group(&mut command);
    let mut child = MacosTestChild(command.spawn().unwrap());
    let pid = child.0.id();
    let mut ready = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while process_group_running(pid) {
        assert!(
            std::time::Instant::now() < deadline,
            "zombie counted as live"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // No wait/try_wait occurred above; confirm the owner can still reap its
    // exit record below. Darwin need not retain a signalable group for zombies.
    assert!(list_live_process_group_members(pid).is_empty());
    assert!(child.0.wait().unwrap().success());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_process_identity_and_unsupported_ownership_are_not_conflated() {
    let current = process_identity(std::process::id()).unwrap();
    assert!(current.starts_with("macos-start-time:"));
    assert_eq!(process_identity(std::process::id()), Some(current.clone()));
    assert_eq!(process_identity(0), None);
    assert_eq!(process_identity(u32::MAX), None);
    let mut child = MacosTestChild(
        std::process::Command::new("/bin/sleep")
            .arg("20")
            .spawn()
            .unwrap(),
    );
    let identity = process_identity(child.0.id()).unwrap();
    assert_ne!(identity, current);
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert_eq!(process_identity(child.0.id()), None);
    assert_eq!(
        ManagedProcessJob::create().unwrap_err().kind(),
        std::io::ErrorKind::Unsupported
    );
    assert!(!install_process_subreaper());
    assert!(reparented_detached_child_pids().is_empty());
}

#[test]
fn reaper_backoff_restores_fast_cadence_on_activity_and_caps_idle_growth() {
    let fast = std::time::Duration::from_millis(250);
    assert_eq!(crate::next_reaper_backoff(1, 0, fast), fast);
    assert_eq!(crate::next_reaper_backoff(0, 3, fast), fast);
    assert_eq!(
        crate::next_reaper_backoff(0, 0, fast),
        std::time::Duration::from_millis(500)
    );
    let mut backoff = fast;
    for _ in 0..10 {
        backoff = crate::next_reaper_backoff(0, 0, backoff);
    }
    assert_eq!(backoff, std::time::Duration::from_secs(4));
}

// ---------------------------------------------------------------------------
// Instance resource caps

#[test]
fn instance_resource_caps_validation_rejects_out_of_range() {
    assert!(InstanceResourceCaps::validated(true, 0.0, true, 85.0).is_err());
    assert!(InstanceResourceCaps::validated(true, -1.0, true, 85.0).is_err());
    assert!(InstanceResourceCaps::validated(true, f64::NAN, true, 85.0).is_err());
    assert!(InstanceResourceCaps::validated(true, f64::INFINITY, true, 85.0).is_err());
    assert!(InstanceResourceCaps::validated(true, 85.0, true, 100.0001).is_err());
    assert!(InstanceResourceCaps::validated(true, 1.0, true, 100.0).is_ok());
    assert!(InstanceResourceCaps::validated(true, 85.0, true, 85.0).is_ok());
}

#[cfg(target_os = "linux")]
#[test]
fn cpu_max_formats_percent_of_cores_with_kernel_minimum() {
    assert_eq!(crate::linux::format_cpu_max(85.0, 16), "1360000 100000");
    assert_eq!(crate::linux::format_cpu_max(100.0, 4), "400000 100000");
    // Kernel minimum quota is 1000 microseconds.
    assert_eq!(crate::linux::format_cpu_max(1.0, 1), "1000 100000");
}

#[cfg(target_os = "linux")]
#[test]
fn memory_max_scales_system_total_and_high_tracks_eighty_five_percent() {
    assert_eq!(crate::linux::format_memory_max(85.0, 1_000_000), 850_000);
    assert_eq!(crate::linux::format_memory_high(850_000), 722_500);
    assert_eq!(crate::linux::format_memory_max(50.0, 3), 2);
    assert_eq!(crate::linux::format_memory_high(2), 1);
    assert_eq!(crate::linux::format_memory_max(100.0, 0), 0);
    assert_eq!(crate::linux::format_memory_high(0), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn meminfo_total_parses_kb_scale() {
    let content =
        "MemFree:         123 kB\nMemTotal:       32878012 kB\nMemAvailable:   9876543 kB\n";
    assert_eq!(
        crate::linux::parse_meminfo_total_bytes(content),
        Some(32_878_012u64 * 1024)
    );
    assert_eq!(
        crate::linux::parse_meminfo_total_bytes("SwapTotal: 1 kB\n"),
        None
    );
}

#[cfg(target_os = "linux")]
#[test]
fn instance_resource_caps_apply_is_idempotent_and_update_rewrites_kernel_bound() {
    // One test drives the process-global state: apply fixes the first
    // outcome, update rewrites it, invalid updates keep the old bound.
    let first = apply_instance_resource_caps(InstanceResourceCaps {
        cpu_enabled: true,
        cpu_percent: 85.0,
        mem_enabled: true,
        mem_percent: 85.0,
    });
    let replay = apply_instance_resource_caps(InstanceResourceCaps {
        cpu_enabled: true,
        cpu_percent: 90.0,
        mem_enabled: true,
        mem_percent: 90.0,
    });
    assert!(
        std::sync::Arc::ptr_eq(&first, &replay),
        "later applies return the recorded outcome"
    );
    assert_eq!(
        first.caps,
        InstanceResourceCaps {
            cpu_enabled: true,
            cpu_percent: 85.0,
            mem_enabled: true,
            mem_percent: 85.0,
        }
    );
    let (
        crate::ResourceCapStatus::Applied { limit: cpu_limit },
        crate::ResourceCapStatus::Applied { limit: mem_limit },
    ) = (&first.cpu, &first.mem)
    else {
        eprintln!(
            "skipping real cgroup-v2 caps write test: cpu {:?}; mem {:?}",
            first.cpu, first.mem
        );
        return;
    };
    let period = 100_000.0f64;
    let (quota, read_period) = cpu_limit.split_once(' ').expect("cpu.max shape");
    assert_eq!(read_period, "100000");
    let cores = system_cpu_cores().expect("online cores");
    let expected_quota = ((85.0 / 100.0) * f64::from(cores) * period).round() as u64;
    assert_eq!(quota.parse::<u64>().expect("quota number"), expected_quota);
    let mem_total = system_mem_total_bytes().expect("mem total");
    let expected_max = (0.85 * mem_total as f64).round() as u64;
    let expected_high = expected_max * 85 / 100;
    assert_eq!(
        mem_limit.as_str(),
        format!("high={expected_high}; max={expected_max}").as_str()
    );
    let scope = PathBuf::from(&first.scope);
    assert!(scope.ends_with("timem.jobs"), "{}", first.scope);
    assert_eq!(
        std::fs::read_to_string(scope.join("cpu.max")).expect("cpu.max readable"),
        format!("{expected_quota} 100000\n")
    );
    assert_eq!(
        std::fs::read_to_string(scope.join("memory.high")).expect("memory.high readable"),
        format!("{expected_high}\n")
    );
    assert_eq!(
        std::fs::read_to_string(scope.join("memory.max")).expect("memory.max readable"),
        format!("{expected_max}\n")
    );

    // Runtime update rewrites both kernel files and the recorded outcome.
    let updated_cpu = ((70.0 / 100.0) * f64::from(cores) * period).round() as u64;
    let updated_max = (0.70 * mem_total as f64).round() as u64;
    let updated_high = updated_max * 85 / 100;
    let updated = update_instance_resource_caps(InstanceResourceCaps {
        cpu_enabled: true,
        cpu_percent: 70.0,
        mem_enabled: true,
        mem_percent: 70.0,
    })
    .expect("runtime update applies");
    assert_eq!(updated.caps.cpu_percent, 70.0);
    assert_eq!(
        std::fs::read_to_string(scope.join("cpu.max")).expect("cpu.max readable"),
        format!("{updated_cpu} 100000\n")
    );
    assert_eq!(
        std::fs::read_to_string(scope.join("memory.high")).expect("memory.high readable"),
        format!("{updated_high}\n")
    );
    assert_eq!(
        std::fs::read_to_string(scope.join("memory.max")).expect("memory.max readable"),
        format!("{updated_max}\n")
    );

    // Invalid updates change nothing.
    assert!(update_instance_resource_caps(InstanceResourceCaps {
        cpu_enabled: true,
        cpu_percent: 150.0,
        mem_enabled: true,
        mem_percent: 70.0,
    })
    .is_err());
    assert_eq!(
        std::fs::read_to_string(scope.join("cpu.max")).expect("cpu.max readable"),
        format!("{updated_cpu} 100000\n"),
        "invalid update keeps the previous kernel bound"
    );

    // Restore unbounded parent defaults so later job-scoped tests inherit no quota.
    let _ = std::fs::write(scope.join("cpu.max"), "max 100000\n");
    let _ = std::fs::write(scope.join("memory.high"), "max\n");
    let _ = std::fs::write(scope.join("memory.max"), "max\n");
}

#[cfg(target_os = "linux")]
#[test]
fn escalation_script_renders_whitelisted_actions_only() {
    let actions = vec![
        crate::ResourceCapEscalationAction::MigrateSelf {
            cgroup: "/sys/fs/cgroup/user.slice/timem.self".to_string(),
            pid: 4242,
        },
        crate::ResourceCapEscalationAction::EnableController {
            cgroup: "/sys/fs/cgroup/user.slice".to_string(),
            controller: "cpu".to_string(),
        },
        crate::ResourceCapEscalationAction::MakeWritable {
            cgroup: "/sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice"
                .to_string(),
            uid: 1000,
            gid: 1000,
        },
    ];
    let script = crate::linux::render_escalation_script(&actions).expect("render");
    assert!(script.contains(
        "mkdir -p '/sys/fs/cgroup/user.slice/timem.self' && echo '4242' > '/sys/fs/cgroup/user.slice/timem.self/cgroup.procs' || exit 77"
    ));
    assert!(script.contains(
        "grep -qw 'cpu' '/sys/fs/cgroup/user.slice/cgroup.subtree_control' 2>/dev/null || echo '+cpu' > '/sys/fs/cgroup/user.slice/cgroup.subtree_control' || exit 77"
    ));
    assert!(script.contains(
        "chown -R '1000:1000' '/sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice' || exit 77"
    ));

    for bad in [
        "/sys/fs/cgroup/x; rm -rf /",
        "/sys/fs/cgroup/../..",
        "/etc/passwd",
        "/sys/fs/cgroup/a b",
    ] {
        let plan = vec![crate::ResourceCapEscalationAction::EnableController {
            cgroup: bad.to_string(),
            controller: "cpu".to_string(),
        }];
        assert!(
            crate::linux::render_escalation_script(&plan).is_err(),
            "unsafe path must be rejected: {bad}"
        );
    }
    let plan = vec![crate::ResourceCapEscalationAction::EnableController {
        cgroup: "/sys/fs/cgroup/user.slice".to_string(),
        controller: "cpu;reboot".to_string(),
    }];
    assert!(crate::linux::render_escalation_script(&plan).is_err());
    let plan = vec![crate::ResourceCapEscalationAction::MigrateSelf {
        cgroup: "/etc/passwd".to_string(),
        pid: 1,
    }];
    assert!(crate::linux::render_escalation_script(&plan).is_err());
    assert!(crate::linux::render_escalation_script(&[]).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn probe_reports_well_formed_readiness() {
    let readiness = crate::probe_instance_resource_caps();
    for status in [&readiness.cpu, &readiness.mem] {
        match status {
            crate::ResourceCapReadiness::NeedsEscalation { actions } => {
                assert!(!actions.is_empty());
                for action in actions {
                    match action {
                        crate::ResourceCapEscalationAction::EnableController {
                            cgroup,
                            controller,
                        } => {
                            assert!(cgroup.starts_with("/sys/fs/cgroup/"));
                            assert!(matches!(controller.as_str(), "cpu" | "memory"));
                        }
                        crate::ResourceCapEscalationAction::MakeWritable { cgroup, .. } => {
                            assert!(cgroup.starts_with("/sys/fs/cgroup/"));
                        }
                        crate::ResourceCapEscalationAction::MigrateSelf { cgroup, pid } => {
                            assert!(cgroup.starts_with("/sys/fs/cgroup/"));
                            assert!(*pid > 0);
                        }
                    }
                }
            }
            crate::ResourceCapReadiness::Ready | crate::ResourceCapReadiness::Unsupported => {}
            crate::ResourceCapReadiness::Unavailable { reason } => {
                assert!(!reason.is_empty());
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn dimension_bound_disable_reports_disabled_or_unavailable() {
    use crate::linux::{apply_dimension_bound, MEMORY_LIMIT_UNBOUNDED};
    let root = std::env::temp_dir().join(format!("timem-caps-bound-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    // Disabling a fresh dimension writes the unbounded value and reports Disabled.
    let status = apply_dimension_bound(&root, "memory.high", false, MEMORY_LIMIT_UNBOUNDED, || {
        "1".to_string()
    });
    assert_eq!(status, crate::ResourceCapStatus::Disabled);
    assert_eq!(
        std::fs::read_to_string(root.join("memory.high"))
            .unwrap()
            .trim(),
        MEMORY_LIMIT_UNBOUNDED
    );
    // Clearing a bound inside a missing directory fails closed as Unavailable.
    let status = apply_dimension_bound(
        &root.join("missing"),
        "memory.high",
        false,
        MEMORY_LIMIT_UNBOUNDED,
        || "1".to_string(),
    );
    assert!(matches!(
        status,
        crate::ResourceCapStatus::Unavailable { .. }
    ));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn instance_resource_caps_validate_percent_independently_of_enabled() {
    assert!(InstanceResourceCaps::validated(true, 85.0, false, 85.0).is_ok());
    assert!(InstanceResourceCaps::validated(false, 0.0, false, 150.0).is_err());
    assert!(InstanceResourceCaps::validated(false, -1.0, true, 100.0).is_err());
    assert!(InstanceResourceCaps::validated(false, f64::NAN, false, 85.0).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn escalation_plan_streams_script_and_password_through_sudo() {
    // Development-time privilege simulation: a fake `sudo` on PATH records
    // argv and stdin, proving the real call chain (spawn, script argument,
    // password stream, exit-code handling) without any root power.
    let _guard = crate::test_support::process_lifecycle_test_lock();
    let fake_dir = std::env::temp_dir().join(format!(
        "timem-fake-sudo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&fake_dir).unwrap();
    let sudo = fake_dir.join("sudo");
    std::fs::write(
        &sudo,
        "#!/bin/sh\n{ printf '%s\\n' \"$@\"; echo '--STDIN--'; cat; } > \"$FAKE_SUDO_LOG\"\nexit 0\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&sudo, std::fs::Permissions::from_mode(0o755)).unwrap();

    let log = fake_dir.join("invocation.log");
    let original_path = std::env::var_os("PATH").unwrap();
    // Serialized by the lifecycle lock: PATH and the log target are process-global.
    std::env::set_var(
        "PATH",
        format!("{}:{}", fake_dir.display(), original_path.to_string_lossy()),
    );
    std::env::set_var("FAKE_SUDO_LOG", &log);
    let outcome = crate::apply_resource_cap_escalation_plan(
        &[crate::ResourceCapEscalationAction::EnableController {
            cgroup: "/sys/fs/cgroup/user.slice".to_string(),
            controller: "cpu".to_string(),
        }],
        b"stub-root-password",
    );
    std::env::set_var("PATH", &original_path);
    std::env::remove_var("FAKE_SUDO_LOG");
    outcome.expect("fake sudo invocation succeeds");
    let invocation = std::fs::read_to_string(&log).unwrap();
    assert!(invocation.contains("-S"), "sudo stdin mode: {invocation}");
    assert!(invocation.contains("sh"), "script shell: {invocation}");
    assert!(
        invocation.contains("cgroup.subtree_control"),
        "controller enablement: {invocation}"
    );
    assert!(
        invocation.contains("+cpu"),
        "enable action reaches the script: {invocation}"
    );
    assert!(
        invocation.contains("stub-root-password"),
        "password streams through stdin: {invocation}"
    );
    std::fs::remove_dir_all(&fake_dir).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn escalation_plan_separates_auth_failure_from_script_failure() {
    // Development-time privilege simulation, serialized by the lifecycle lock
    // like the other fake-sudo test: PATH swaps are process-global.
    let _guard = crate::test_support::process_lifecycle_test_lock();
    let fake_dir = std::env::temp_dir().join(format!(
        "timem-fake-sudo-rc-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&fake_dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let actions = [crate::ResourceCapEscalationAction::EnableController {
        cgroup: "/sys/fs/cgroup/user.slice".to_string(),
        controller: "memory".to_string(),
    }];
    let original_path = std::env::var_os("PATH").unwrap();
    std::env::set_var(
        "PATH",
        format!("{}:{}", fake_dir.display(), original_path.to_string_lossy()),
    );

    // sudo itself refuses before running the script: its own exit code 1
    // must keep the fixed authorization-rejected attribution.
    std::fs::write(
        fake_dir.join("sudo"),
        "#!/bin/sh\ncat >/dev/null\necho 'sudo: 1 incorrect password attempt' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(
        fake_dir.join("sudo"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let error = crate::apply_resource_cap_escalation_plan(&actions, b"stub")
        .expect_err("auth failure must surface");
    assert_eq!(
        error, "root authorization rejected",
        "sudo-level rc=1: {error}"
    );

    // The rendered script exits 77 when an action fails; the kernel sh error
    // tail is disclosed, and the stub password can never appear anywhere.
    std::fs::write(
        fake_dir.join("sudo"),
        "#!/bin/sh\ncat >/dev/null\necho \"sh: echo: write error: Operation not permitted\" >&2\nexit 77\n",
    )
    .unwrap();
    std::fs::set_permissions(
        fake_dir.join("sudo"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let error = crate::apply_resource_cap_escalation_plan(&actions, b"stub")
        .expect_err("script failure must surface");
    assert!(
        error.starts_with("escalation script failed:"),
        "script failure attribution: {error}"
    );
    assert!(
        error.contains("Operation not permitted"),
        "kernel error tail disclosed: {error}"
    );
    assert!(!error.contains("stub"), "password never leaks: {error}");

    std::env::set_var("PATH", &original_path);
    std::fs::remove_dir_all(&fake_dir).unwrap();
}

#[test]
fn shared_root_is_detected_by_foreign_pids() {
    let own = 4242u32;
    assert!(!crate::linux::pids_indicate_shared_root(&[], own));
    assert!(!crate::linux::pids_indicate_shared_root(&[own], own));
    assert!(crate::linux::pids_indicate_shared_root(&[own, 7], own));
    assert!(crate::linux::pids_indicate_shared_root(&[7], own));
}

#[test]
fn instance_root_is_the_parent_of_the_self_subtree() {
    let plain = std::path::Path::new("/sys/fs/cgroup/a/b");
    assert_eq!(crate::linux::instance_root_cgroup(plain), plain);
    let migrated = std::path::Path::new("/sys/fs/cgroup/a/b/timem.self");
    assert_eq!(
        crate::linux::instance_root_cgroup(migrated),
        std::path::Path::new("/sys/fs/cgroup/a/b")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn escalation_plan_runs_migration_before_controller_enablement() {
    // Caller merges per-dimension plans in arbitrary order; the apply entry
    // must render the migration ahead of every controller enablement, since
    // enabling under a populated root force-converts the cgroup to a threaded
    // subtree that carries only cpu.
    let _guard = crate::test_support::process_lifecycle_test_lock();
    let fake_dir = std::env::temp_dir().join(format!(
        "timem-fake-sudo-order-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&fake_dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let sudo = fake_dir.join("sudo");
    std::fs::write(
        &sudo,
        "#!/bin/sh\n{ printf '%s\\n' \"$@\"; echo '--STDIN--'; cat; } > \"$FAKE_SUDO_LOG\"\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&sudo, std::fs::Permissions::from_mode(0o755)).unwrap();

    let log = fake_dir.join("invocation.log");
    let original_path = std::env::var_os("PATH").unwrap();
    std::env::set_var(
        "PATH",
        format!("{}:{}", fake_dir.display(), original_path.to_string_lossy()),
    );
    std::env::set_var("FAKE_SUDO_LOG", &log);
    let outcome = crate::apply_resource_cap_escalation_plan(
        &[
            crate::ResourceCapEscalationAction::EnableController {
                cgroup: "/sys/fs/cgroup/user.slice".to_string(),
                controller: "memory".to_string(),
            },
            crate::ResourceCapEscalationAction::MakeWritable {
                cgroup: "/sys/fs/cgroup/user.slice".to_string(),
                uid: 1000,
                gid: 1000,
            },
            crate::ResourceCapEscalationAction::MigrateSelf {
                cgroup: "/sys/fs/cgroup/user.slice/timem.self".to_string(),
                pid: 4242,
            },
        ],
        b"stub-root-password",
    );
    std::env::set_var("PATH", &original_path);
    std::env::remove_var("FAKE_SUDO_LOG");
    outcome.expect("fake sudo invocation succeeds");
    let invocation = std::fs::read_to_string(&log).unwrap();
    let migration = invocation
        .find("mkdir -p '/sys/fs/cgroup/user.slice/timem.self'")
        .expect("migration rendered");
    let enablement = invocation
        .find("echo '+memory'")
        .expect("enablement rendered");
    let ownership = invocation
        .find("chown -R '1000:1000'")
        .expect("ownership repair rendered");
    assert!(
        migration < enablement && enablement < ownership,
        "layers must run migration, enablement, then ownership repair: {invocation}"
    );
    std::fs::remove_dir_all(&fake_dir).unwrap();
}
