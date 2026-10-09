//! Bounded output retention and shutdown policy for finite command consumers.
//! Platform owns interruptible pipe I/O; Agent owns the post-exit drain budget.
use crate::os::ChildOutputPipe;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

type Reader = JoinHandle<Result<Vec<u8>, String>>;

pub(crate) struct CommandOutput {
    deadline: Arc<Mutex<Option<Instant>>>,
    stdout: Option<Reader>,
    stderr: Option<Reader>,
}

impl CommandOutput {
    pub(crate) fn start(child: &mut Child, max_bytes: usize) -> Self {
        let deadline = Arc::new(Mutex::new(None));
        Self {
            stdout: child
                .stdout
                .take()
                .map(|pipe| spawn_reader(pipe.into(), max_bytes, Arc::clone(&deadline))),
            stderr: child
                .stderr
                .take()
                .map(|pipe| spawn_reader(pipe.into(), max_bytes, Arc::clone(&deadline))),
            deadline,
        }
    }

    pub(crate) fn finish(mut self) -> Result<(Vec<u8>, Vec<u8>), String> {
        self.stop();
        // Always join both, including when one pipe reports an error.
        let stdout = join_reader(self.stdout.take());
        let stderr = join_reader(self.stderr.take());
        Ok((stdout?, stderr?))
    }

    fn stop(&self) {
        if let Ok(mut deadline) = self.deadline.lock() {
            deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(250));
        }
    }
}

impl Drop for CommandOutput {
    fn drop(&mut self) {
        self.stop();
        let _ = join_reader(self.stdout.take());
        let _ = join_reader(self.stderr.take());
    }
}

fn spawn_reader(
    mut pipe: ChildOutputPipe,
    max_bytes: usize,
    deadline: Arc<Mutex<Option<Instant>>>,
) -> Reader {
    thread::spawn(move || {
        let mut retained = Vec::with_capacity(max_bytes.min(8192));
        let mut buffer = [0u8; 8192];
        loop {
            let expired = deadline
                .lock()
                .map_err(|_| "command_output_deadline_poisoned".to_string())?
                .is_some_and(|deadline| Instant::now() >= deadline);
            if expired {
                return Err("output_capture_incomplete".to_string());
            }
            match pipe.read_with_timeout(&mut buffer, Duration::from_millis(20)) {
                Ok(Some(0)) => return Ok(retained),
                Ok(Some(read)) => {
                    let keep = read.min(max_bytes.saturating_sub(retained.len()));
                    retained.extend_from_slice(&buffer[..keep]);
                }
                Ok(None) => {}
                Err(error) => return Err(format!("command_output_read_failed:{error}")),
            }
        }
    })
}

fn join_reader(reader: Option<Reader>) -> Result<Vec<u8>, String> {
    match reader {
        Some(reader) => reader
            .join()
            .map_err(|_| "command_output_reader_panicked".to_string())?,
        None => Ok(Vec::new()),
    }
}
