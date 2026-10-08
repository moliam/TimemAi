use super::*;

#[cfg(unix)]
#[test]
fn idle_pipe_read_is_bounded_and_reports_eof_after_child_exit() {
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("5")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = ChildOutputPipe::from(child.stdout.take().unwrap());
    let mut buffer = [0; 32];
    let started = std::time::Instant::now();
    let idle = pipe.read_with_timeout(&mut buffer, Duration::from_millis(20));
    let elapsed = started.elapsed();
    let _ = child.kill();
    child.wait().unwrap();
    assert_eq!(idle.unwrap(), None);
    assert!(elapsed < Duration::from_secs(1));
    assert_eq!(
        pipe.read_with_timeout(&mut buffer, Duration::from_secs(1))
            .unwrap(),
        Some(0)
    );
}

#[cfg(unix)]
#[test]
fn pipe_hangup_preserves_buffered_output_before_eof() {
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "printf buffered; printf diagnostic >&2"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = ChildOutputPipe::from(child.stdout.take().unwrap());
    let mut stderr = ChildOutputPipe::from(child.stderr.take().unwrap());
    child.wait().unwrap();
    for (pipe, expected) in [
        (&mut stdout, b"buffered".as_slice()),
        (&mut stderr, b"diagnostic".as_slice()),
    ] {
        let mut buffer = [0; 32];
        let count = pipe
            .read_with_timeout(&mut buffer, Duration::ZERO)
            .unwrap()
            .unwrap();
        assert_eq!(&buffer[..count], expected);
        assert_eq!(
            pipe.read_with_timeout(&mut buffer, Duration::ZERO).unwrap(),
            Some(0)
        );
        assert_eq!(
            pipe.read_with_timeout(&mut [], Duration::ZERO)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_child_pipe_reports_data_and_eof() {
    let mut child = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "echo buffered"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = ChildOutputPipe::from(child.stdout.take().unwrap());
    child.wait().unwrap();
    let mut buffer = [0; 32];
    let read = pipe
        .read_with_timeout(&mut buffer, Duration::from_millis(20))
        .unwrap()
        .unwrap();
    assert_eq!(&buffer[..read], b"buffered\r\n");
    assert_eq!(
        pipe.read_with_timeout(&mut buffer, Duration::from_millis(20))
            .unwrap(),
        Some(0)
    );
}
