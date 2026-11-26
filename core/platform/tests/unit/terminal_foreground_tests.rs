//! Controlling-terminal foreground reclaim primitives.
//!
//! Real scenarios run inside a freshly spawned copy of this test binary
//! (`--exact` child entry) that owns a pty session of its own, so the
//! harness process and its threads are never disturbed.

#[cfg(not(unix))]
use crate::api::restore_terminal_foreground_to_current_group;
#[cfg(unix)]
use crate::api::{restore_terminal_foreground_to_current_group, terminal_foreground_pgid};
// libc 0.2 未给 apple 提供 ptsname 绑定：linux 走 libc::ptsname_r，
// macos 直接声明 libSystem 的同签名 C 函数（macOS 10.13+ 提供）。
#[cfg(all(unix, not(target_os = "macos")))]
use libc::ptsname_r;
#[cfg(target_os = "macos")]
extern "C" {
    fn ptsname_r(fd: libc::c_int, buf: *mut libc::c_char, len: libc::size_t) -> libc::c_int;
}

const CHILD_SCENARIO_ENV: &str = "TIMEM_TERMINAL_FG_CHILD_SCENARIO";
const CHILD_RESULT_MARKER: &str = "CHILD_RESULT";

#[test]
fn terminal_foreground_child_entry() {
    let Ok(scenario) = std::env::var(CHILD_SCENARIO_ENV) else {
        // Normal harness run: the parent drives the real scenarios.
        return;
    };
    match scenario.as_str() {
        #[cfg(unix)]
        "dead_group_hijack" => run_dead_group_hijack_scenario(),
        #[cfg(unix)]
        "already_owner" => run_already_owner_scenario(),
        "without_terminal" => run_without_terminal_scenario(),
        other => panic!("unknown child scenario: {other}"),
    }
}

#[cfg(unix)]
fn claim_stdin_pty_session() {
    unsafe {
        libc::setsid();
        assert_eq!(
            libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0),
            0,
            "TIOCSCTTY failed"
        );
        assert_eq!(libc::tcsetpgrp(0, libc::getpgrp()), 0, "tcsetpgrp failed");
    }
}

/// The incident shape: a command-side group takes the terminal foreground
/// and then dies, leaving the foreground group dead; the host reclaims it.
#[cfg(unix)]
fn run_dead_group_hijack_scenario() {
    claim_stdin_pty_session();
    let grand = unsafe { libc::fork() };
    assert!(grand >= 0, "fork failed");
    if grand == 0 {
        unsafe {
            libc::setpgid(0, 0);
            libc::_exit(0);
        }
    }
    unsafe {
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
        assert_eq!(libc::tcsetpgrp(0, grand), 0, "hijack tcsetpgrp failed");
        libc::waitpid(grand, std::ptr::null_mut(), 0);
    }
    let before = terminal_foreground_pgid();
    let outcome = restore_terminal_foreground_to_current_group();
    let after = terminal_foreground_pgid();
    let own = unsafe { libc::getpgrp() } as u32;
    println!(
        "{CHILD_RESULT_MARKER} before={before:?} outcome={outcome:?} after={after:?} own={own}"
    );
}

#[cfg(unix)]
fn run_already_owner_scenario() {
    claim_stdin_pty_session();
    let outcome = restore_terminal_foreground_to_current_group();
    let fg = terminal_foreground_pgid();
    let own = unsafe { libc::getpgrp() } as u32;
    println!("{CHILD_RESULT_MARKER} outcome={outcome:?} fg={fg:?} own={own}");
}

fn run_without_terminal_scenario() {
    let outcome = restore_terminal_foreground_to_current_group();
    println!("{CHILD_RESULT_MARKER} outcome={outcome:?}");
}

#[cfg(unix)]
fn spawn_child_on_pty(scenario: &str) -> (std::process::Child, std::fs::File) {
    use std::os::unix::io::FromRawFd;
    use std::process::{Command, Stdio};
    unsafe {
        let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master_fd >= 0, "posix_openpt failed");
        assert_eq!(libc::grantpt(master_fd), 0, "grantpt failed");
        assert_eq!(libc::unlockpt(master_fd), 0, "unlockpt failed");
        let mut name = [0 as libc::c_char; 128];
        assert_eq!(
            ptsname_r(master_fd, name.as_mut_ptr(), name.len()),
            0,
            "ptsname_r failed"
        );
        let path = std::ffi::CStr::from_ptr(name.as_ptr())
            .to_string_lossy()
            .into_owned();
        let slave_fd = libc::open(
            std::ffi::CString::new(path).unwrap().as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY,
        );
        assert!(slave_fd >= 0, "open pty slave failed");
        let slave = std::fs::File::from_raw_fd(slave_fd);
        let stdin = slave.try_clone().expect("clone slave for stdin");
        let stdout = slave.try_clone().expect("clone slave for stdout");
        let stderr = slave.try_clone().expect("clone slave for stderr");
        let mut child_command = Command::new(std::env::current_exe().expect("current_exe"));
        child_command
            .arg("terminal_foreground_tests::terminal_foreground_child_entry")
            .arg("--exact")
            .arg("--nocapture")
            .env(CHILD_SCENARIO_ENV, scenario)
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let child = child_command.spawn().expect("spawn child test binary");
        drop(slave); // master EOFs once the child (last slave holder) exits
        (child, std::fs::File::from_raw_fd(master_fd))
    }
}

#[cfg(unix)]
fn child_output_until_result(master: &mut std::fs::File) -> String {
    use std::io::Read;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut collected = String::new();
    let mut buffer = [0_u8; 8192];
    while collected.find(CHILD_RESULT_MARKER).is_none() {
        if std::time::Instant::now() >= deadline {
            collected.push_str("\n[deadline reached before the child result marker]");
            break;
        }
        let read = match master.read(&mut buffer) {
            Ok(0) => break, // child exited without the marker; assertions report it
            Ok(read) => read,
            // The child owns the last slave descriptors; once it exits the
            // kernel reports EIO on the master instead of a clean EOF.
            Err(err) => {
                collected.push_str(&format!("\n[master read stopped: {err}]"));
                break;
            }
        };
        collected.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    collected
}

/// Spawns the scenario child, collects its output, and always reaps it.
///
/// Every scenario child must be waited on before any assertion can fail, so a
/// broken scenario can never leak zombie children into the shared test
/// process: the runtime-wide fallback reaper would otherwise reap them as
/// adopted zombies and break unrelated lifecycle tests.
#[cfg(unix)]
fn run_pty_child(scenario: &str) -> (String, std::process::ExitStatus) {
    let (mut child, mut master) = spawn_child_on_pty(scenario);
    let output = child_output_until_result(&mut master);
    drop(master);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("try_wait scenario child") {
            Some(status) => break status,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    break child.wait().expect("wait scenario child after kill");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    };
    (output, status)
}

/// The incident regression: a background command group steals the terminal
/// foreground, dies, and the host must reclaim it from its own background
/// group with SIGTTOU blocked.
#[cfg(unix)]
#[test]
fn restore_reclaims_foreground_from_dead_hijacking_group() {
    let (output, status) = run_pty_child("dead_group_hijack");
    assert!(status.success(), "child failed: {status}; output: {output}");
    let line = output
        .lines()
        .find(|line| line.contains(CHILD_RESULT_MARKER))
        .expect("child result line");
    let grand: u32 = line
        .split("before=Some(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .and_then(|value| value.parse().ok())
        .expect("parse the hijacking group id");
    let own: u32 = line
        .split("own=")
        .nth(1)
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .expect("parse the host group id");
    assert!(
        line.contains(&format!("outcome=Restored {{ from_pgid: {grand} }}")),
        "hijack was not reclaimed: {line}"
    );
    assert!(line.contains(&format!("after=Some({own})")), "{line}");
}

#[cfg(unix)]
#[test]
fn restore_is_not_applicable_when_current_group_already_owns_foreground() {
    let (output, status) = run_pty_child("already_owner");
    assert!(status.success(), "child failed: {status}; output: {output}");
    assert!(output.contains("outcome=NotApplicable"), "{output}");
    assert!(output.contains("fg=Some("), "{output}");
}

#[test]
fn restore_is_not_applicable_without_terminal() {
    use std::process::{Command, Stdio};
    let output = Command::new(std::env::current_exe().expect("current_exe"))
        .arg("terminal_foreground_tests::terminal_foreground_child_entry")
        .arg("--exact")
        .arg("--nocapture")
        .env(CHILD_SCENARIO_ENV, "without_terminal")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn child test binary");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "child failed: {}", output.status);
    assert!(text.contains("outcome=NotApplicable"), "{text}");
}
