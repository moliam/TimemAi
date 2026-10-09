use super::*;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn nonreading_child_fixture() {
    if std::env::var_os("TIMEM_TEST_NONREADING_STDIN").is_some() {
        // Finite even if the parent test aborts. No inherited shell descendants.
        std::thread::sleep(Duration::from_secs(4));
    }
}

struct Fixture(Child);
impl Fixture {
    fn new() -> Self {
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "child_input::tests::nonreading_child_fixture"])
                .env("TIMEM_TEST_NONREADING_STDIN", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn full_stdin_pipe_returns_without_waiting_for_reader() {
    let mut child = Fixture::new();
    let mut input = ChildInputPipe::new(child.0.stdin.take().unwrap()).unwrap();
    let block = [b'x'; 8192];
    let started = Instant::now();
    let mut blocked = false;
    for _ in 0..2048 {
        match input.try_write(&block) {
            Ok(written) => assert!(written > 0 && written <= block.len()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                blocked = true;
                break;
            }
            Err(error) => panic!("unexpected input failure: {error}"),
        }
    }
    let elapsed = started.elapsed();
    drop(input);
    drop(child);
    assert!(blocked, "16 MiB must encounter pipe backpressure");
    assert!(
        elapsed < Duration::from_secs(1),
        "write blocked: {elapsed:?}"
    );
}

#[test]
fn closed_stdin_pipe_reports_failure_not_success() {
    let mut child = Fixture::new();
    let mut input = ChildInputPipe::new(child.0.stdin.take().unwrap()).unwrap();
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let started = Instant::now();
    let error = input.try_write(b"undelivered").unwrap_err();
    assert_ne!(error.kind(), io::ErrorKind::WouldBlock, "{error}");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn reading_child_fixture() {
    if let Some(path) = std::env::var_os("TIMEM_TEST_STDIN_CAPTURE") {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

#[test]
fn partial_input_delivery_preserves_bytes_and_drop_delivers_eof() {
    let path = std::env::temp_dir().join(format!(
        "timem_stdin_capture_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut child = Fixture(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_input::tests::reading_child_fixture"])
            .env("TIMEM_TEST_STDIN_CAPTURE", &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut input = ChildInputPipe::new(child.0.stdin.take().unwrap()).unwrap();
    let bytes: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut offset = 0;
    let mut partial = false;
    while offset < bytes.len() && Instant::now() < deadline {
        match input.try_write(&bytes[offset..]) {
            Ok(written) => {
                partial |= written < bytes.len() - offset;
                offset += written;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => panic!("input failed at {offset}: {error}"),
        }
    }
    drop(input);
    let mut status = None;
    while Instant::now() < deadline {
        status = child.0.try_wait().unwrap();
        if status.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(child);
    let actual = std::fs::read(&path);
    let _ = std::fs::remove_file(path);
    assert_eq!(offset, bytes.len(), "delivery did not complete");
    assert!(partial, "large input must exercise partial writes");
    assert!(
        status.is_some_and(|status| status.success()),
        "EOF did not complete child: {status:?}"
    );
    assert_eq!(actual.unwrap(), bytes);
}
