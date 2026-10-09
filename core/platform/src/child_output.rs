//! Interruptible reads of exclusively owned child output pipes.
use std::io::{self, Read};
use std::process::{ChildStderr, ChildStdout};
use std::time::Duration;

/// Owns the read end; no other thread may read it concurrently.
#[derive(Debug)]
pub enum ChildOutputPipe {
    Stdout(ChildStdout),
    Stderr(ChildStderr),
}

impl From<ChildStdout> for ChildOutputPipe {
    fn from(pipe: ChildStdout) -> Self {
        Self::Stdout(pipe)
    }
}
impl From<ChildStderr> for ChildOutputPipe {
    fn from(pipe: ChildStderr) -> Self {
        Self::Stderr(pipe)
    }
}

impl ChildOutputPipe {
    /// `None` means no data yet, `Some(0)` means EOF. An idle writer cannot
    /// block beyond `timeout`; callers retain cancellation/deadline policy.
    pub fn read_with_timeout(
        &mut self,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> io::Result<Option<usize>> {
        if buffer.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty output buffer",
            ));
        }
        self.read_ready(buffer, timeout)
    }

    fn read_pipe(&mut self, buffer: &mut [u8]) -> io::Result<Option<usize>> {
        let result = match self {
            Self::Stdout(pipe) => pipe.read(buffer),
            Self::Stderr(pipe) => pipe.read(buffer),
        };
        match result {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(None),
            other => other.map(Some),
        }
    }

    #[cfg(unix)]
    fn read_ready(&mut self, buffer: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
        use std::os::fd::AsRawFd;
        let fd = match self {
            Self::Stdout(pipe) => pipe.as_raw_fd(),
            Self::Stderr(pipe) => pipe.as_raw_fd(),
        };
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll borrows one valid, exclusively owned pipe descriptor.
        let result = unsafe {
            libc::poll(
                &mut pollfd,
                1,
                timeout.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(None)
            } else {
                Err(error)
            };
        }
        if result == 0 {
            return Ok(None);
        }
        if pollfd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(io::Error::other("child output pipe poll failed"));
        }
        // POLLHUP can coexist with unread buffered data. Read through EOF.
        self.read_pipe(buffer)
    }

    #[cfg(windows)]
    fn read_ready(&mut self, buffer: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::ERROR_BROKEN_PIPE;
        use windows_sys::Win32::System::Pipes::PeekNamedPipe;
        let handle = match self {
            Self::Stdout(pipe) => pipe.as_raw_handle(),
            Self::Stderr(pipe) => pipe.as_raw_handle(),
        };
        let mut available = 0;
        // SAFETY: valid owned pipe; only the available-byte out pointer is used.
        let ok = unsafe {
            PeekNamedPipe(
                handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                Ok(Some(0))
            } else {
                Err(error)
            };
        }
        if available == 0 {
            std::thread::sleep(timeout);
            return Ok(None);
        }
        let count = buffer.len().min(available as usize);
        self.read_pipe(&mut buffer[..count])
    }

    #[cfg(not(any(unix, windows)))]
    fn read_ready(&mut self, _buffer: &mut [u8], _timeout: Duration) -> io::Result<Option<usize>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "child output polling unavailable",
        ))
    }
}

#[cfg(test)]
#[path = "../tests/unit/child_output_tests.rs"]
mod tests;
