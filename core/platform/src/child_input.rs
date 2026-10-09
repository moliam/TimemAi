//! Exclusively owned, nonblocking child stdin. Callers own deadline policy.
use std::io::{self, Write};
use std::process::ChildStdin;

#[derive(Debug)]
pub struct ChildInputPipe {
    pipe: ChildStdin,
}

impl ChildInputPipe {
    /// Configure only the parent's write end. Failure never falls back to a
    /// blocking write, which could prevent the caller from enforcing a deadline.
    pub fn new(pipe: ChildStdin) -> io::Result<Self> {
        set_nonblocking(&pipe)?;
        Ok(Self { pipe })
    }

    /// Makes one write attempt. WouldBlock means retry later; no bytes were
    /// accepted. Partial writes are ordinary progress, not complete delivery.
    pub fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        match self.pipe.write(bytes) {
            // A nonblocking Windows byte pipe can report success with zero
            // bytes when full. Treat it as backpressure, not delivered EOF.
            Ok(0) => Err(io::ErrorKind::WouldBlock.into()),
            result => result,
        }
    }
}

#[cfg(unix)]
fn set_nonblocking(pipe: &ChildStdin) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = pipe.as_raw_fd();
    // SAFETY: pipe exclusively owns this valid descriptor for both calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn set_nonblocking(pipe: &ChildStdin) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::{SetNamedPipeHandleState, PIPE_NOWAIT};
    // This is synchronous nonblocking polling, not overlapped async I/O.
    // SetNamedPipeHandleState explicitly supports anonymous pipe handles.
    // SAFETY: owned write handle; the mode pointer is valid for the call.
    let ok = unsafe {
        SetNamedPipeHandleState(
            pipe.as_raw_handle(),
            &PIPE_NOWAIT,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn set_nonblocking(_pipe: &ChildStdin) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "nonblocking child input unavailable",
    ))
}

#[cfg(test)]
#[path = "../tests/unit/child_input_tests.rs"]
mod tests;
