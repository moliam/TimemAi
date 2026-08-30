use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct InputSource {
    pending: VecDeque<u8>,
}

impl InputSource {
    pub(crate) fn open() -> io::Result<Self> {
        Ok(Self {
            pending: VecDeque::new(),
        })
    }

    fn fill_pending(&mut self) -> io::Result<()> {
        if !event::poll(Duration::ZERO)? {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                encode_key(key, &mut self.pending)
            }
            Event::Paste(text) => self.pending.extend(text.into_bytes()),
            _ => {}
        }
        Ok(())
    }
}

impl Read for InputSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            self.fill_pending()?;
        }
        let count = buf.len().min(self.pending.len());
        for slot in &mut buf[..count] {
            *slot = self.pending.pop_front().expect("pending length checked");
        }
        Ok(count)
    }
}

fn encode_key(key: KeyEvent, out: &mut VecDeque<u8>) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c' | 'C') => {
                out.push_back(3);
                return;
            }
            KeyCode::Char('d' | 'D') => {
                out.push_back(4);
                return;
            }
            _ => {}
        }
    }
    match key.code {
        KeyCode::Enter => out.push_back(b'\r'),
        KeyCode::Esc => out.push_back(27),
        KeyCode::Backspace => out.push_back(127),
        KeyCode::Tab => out.push_back(b'\t'),
        KeyCode::Up => out.extend([27, b'[', b'A']),
        KeyCode::Down => out.extend([27, b'[', b'B']),
        KeyCode::Right => out.extend([27, b'[', b'C']),
        KeyCode::Left => out.extend([27, b'[', b'D']),
        KeyCode::Char(ch) => out.extend(ch.to_string().into_bytes()),
        _ => {}
    }
}

pub(crate) struct ModeGuard {
    active: bool,
}

impl ModeGuard {
    fn enter() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self { active: true })
    }

    pub(crate) fn restore(&mut self) {
        if self.active {
            let _ = crossterm::terminal::disable_raw_mode();
            self.active = false;
        }
    }
}

impl Drop for ModeGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub(crate) struct NonblockingGuard;

impl NonblockingGuard {
    pub(crate) fn restore(&mut self) {}
}

pub(crate) fn enter_thinking_mode(
    _input: &InputSource,
) -> io::Result<(ModeGuard, NonblockingGuard)> {
    Ok((ModeGuard::enter()?, NonblockingGuard))
}

pub(crate) fn enter_interactive_mode(
    _input: &InputSource,
) -> io::Result<(ModeGuard, NonblockingGuard)> {
    Ok((ModeGuard::enter()?, NonblockingGuard))
}

pub(crate) fn drain_pending_input(
    initial_wait: Duration,
    quiet_window: Duration,
    hard_window: Duration,
) -> io::Result<Vec<u8>> {
    let mut input = InputSource::open()?;
    let _mode = ModeGuard::enter()?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let initial_deadline = Instant::now() + initial_wait;
    while Instant::now() < initial_deadline {
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&buffer[..read]);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }
    if bytes.is_empty() {
        return Ok(bytes);
    }
    let mut quiet_deadline = Instant::now() + quiet_window;
    let hard_deadline = Instant::now() + hard_window;
    while Instant::now() < quiet_deadline && Instant::now() < hard_deadline {
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&buffer[..read]);
                quiet_deadline = Instant::now() + quiet_window;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(bytes)
}

pub(crate) struct SigintGuard;

impl SigintGuard {
    pub(crate) fn install(_cancel_requested: &'static AtomicBool) -> Option<Self> {
        Some(Self)
    }
}
