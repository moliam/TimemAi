use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) enum InputSource {
    Tty(File),
    Stdin(File),
}

impl InputSource {
    pub(crate) fn open() -> io::Result<Self> {
        if let Ok(tty) = OpenOptions::new().read(true).write(true).open("/dev/tty") {
            return Ok(Self::Tty(tty));
        }
        let fd = unsafe { libc::dup(libc::STDIN_FILENO) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self::Stdin(unsafe { File::from_raw_fd(fd) }))
    }

    fn fd(&self) -> i32 {
        match self {
            Self::Tty(file) | Self::Stdin(file) => file.as_raw_fd(),
        }
    }
}

impl Read for InputSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tty(file) | Self::Stdin(file) => file.read(buf),
        }
    }
}

pub(crate) struct ModeGuard {
    fd: i32,
    original: libc::termios,
    active: bool,
}

impl ModeGuard {
    fn enter(input: &InputSource, keep_signals: bool, immediate: bool) -> io::Result<Self> {
        let fd = input.fd();
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO);
        if !keep_signals {
            raw.c_lflag &= !libc::ISIG;
        }
        raw.c_cc[libc::VMIN] = if immediate { 0 } else { 1 };
        raw.c_cc[libc::VTIME] = if immediate { 0 } else { 1 };
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            fd,
            original,
            active: true,
        })
    }

    pub(crate) fn restore(&mut self) {
        if self.active {
            let _ = unsafe { libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.original) };
            self.active = false;
        }
    }
}

impl Drop for ModeGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub(crate) struct NonblockingGuard {
    fd: i32,
    original_flags: i32,
    active: bool,
}

impl NonblockingGuard {
    fn enter(input: &InputSource) -> io::Result<Self> {
        let fd = input.fd();
        let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if original_flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, original_flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            fd,
            original_flags,
            active: true,
        })
    }

    pub(crate) fn restore(&mut self) {
        if self.active {
            let _ = unsafe { libc::fcntl(self.fd, libc::F_SETFL, self.original_flags) };
            self.active = false;
        }
    }
}

impl Drop for NonblockingGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub(crate) fn enter_thinking_mode(
    input: &InputSource,
) -> io::Result<(ModeGuard, NonblockingGuard)> {
    let mode = ModeGuard::enter(input, true, true)?;
    let nonblocking = NonblockingGuard::enter(input)?;
    Ok((mode, nonblocking))
}

pub(crate) fn enter_interactive_mode(
    input: &InputSource,
) -> io::Result<(ModeGuard, NonblockingGuard)> {
    let mode = ModeGuard::enter(input, false, false)?;
    let nonblocking = NonblockingGuard::enter(input)?;
    Ok((mode, nonblocking))
}

pub(crate) fn drain_pending_input(
    initial_wait: Duration,
    quiet_window: Duration,
    hard_window: Duration,
) -> io::Result<Vec<u8>> {
    let mut input = InputSource::open()?;
    let (_mode, _nonblocking) = enter_thinking_mode(&input)?;
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    let initial_deadline = Instant::now() + initial_wait;
    while Instant::now() < initial_deadline {
        match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&buf[..n]);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    if bytes.is_empty() {
        return Ok(bytes);
    }
    let mut quiet_deadline = Instant::now() + quiet_window;
    let hard_deadline = Instant::now() + hard_window;
    while Instant::now() < quiet_deadline && Instant::now() < hard_deadline {
        match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&buf[..n]);
                quiet_deadline = Instant::now() + quiet_window;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(bytes)
}

static mut CANCEL_REQUESTED: Option<&'static AtomicBool> = None;

pub(crate) struct SigintGuard {
    previous: libc::sigaction,
    active: bool,
}

impl SigintGuard {
    pub(crate) fn install(cancel_requested: &'static AtomicBool) -> Option<Self> {
        unsafe extern "C" fn handle_sigint(_: libc::c_int) {
            if let Some(cancel_requested) = unsafe { CANCEL_REQUESTED } {
                cancel_requested.store(true, Ordering::SeqCst);
            }
        }

        unsafe {
            CANCEL_REQUESTED = Some(cancel_requested);
            let mut previous: libc::sigaction = std::mem::zeroed();
            let mut next: libc::sigaction = std::mem::zeroed();
            next.sa_sigaction = handle_sigint as *const () as usize;
            libc::sigemptyset(&mut next.sa_mask);
            if libc::sigaction(libc::SIGINT, &next, &mut previous) != 0 {
                CANCEL_REQUESTED = None;
                return None;
            }
            Some(Self {
                previous,
                active: true,
            })
        }
    }

    fn restore(&mut self) {
        if self.active {
            unsafe {
                let _ = libc::sigaction(libc::SIGINT, &self.previous, std::ptr::null_mut());
                CANCEL_REQUESTED = None;
            }
            self.active = false;
        }
    }
}

impl Drop for SigintGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn thinking_mode_is_noncanonical_but_keeps_sigint() {
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        original.c_lflag = libc::ICANON | libc::ECHO | libc::ISIG;
        original.c_cc[libc::VMIN] = 1;
        original.c_cc[libc::VTIME] = 1;

        let mut mode = original;
        mode.c_lflag &= !(libc::ICANON | libc::ECHO);
        mode.c_cc[libc::VMIN] = 0;
        mode.c_cc[libc::VTIME] = 0;

        assert_eq!(mode.c_lflag & libc::ICANON, 0);
        assert_eq!(mode.c_lflag & libc::ECHO, 0);
        assert_ne!(mode.c_lflag & libc::ISIG, 0);
        assert_eq!(mode.c_cc[libc::VMIN], 0);
        assert_eq!(mode.c_cc[libc::VTIME], 0);
    }
}
