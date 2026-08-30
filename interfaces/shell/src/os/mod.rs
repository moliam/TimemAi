use std::io;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::{InputSource, ModeGuard, NonblockingGuard, SigintGuard};
#[cfg(windows)]
pub(crate) use windows::{InputSource, ModeGuard, NonblockingGuard, SigintGuard};

pub(crate) fn enter_thinking_mode(
    input: &InputSource,
) -> io::Result<(ModeGuard, NonblockingGuard)> {
    platform_enter_thinking_mode(input)
}

pub(crate) fn enter_interactive_mode(
    input: &InputSource,
) -> io::Result<(ModeGuard, NonblockingGuard)> {
    platform_enter_interactive_mode(input)
}

#[cfg(unix)]
fn platform_enter_thinking_mode(input: &InputSource) -> io::Result<(ModeGuard, NonblockingGuard)> {
    unix::enter_thinking_mode(input)
}

#[cfg(windows)]
fn platform_enter_thinking_mode(input: &InputSource) -> io::Result<(ModeGuard, NonblockingGuard)> {
    windows::enter_thinking_mode(input)
}

#[cfg(unix)]
fn platform_enter_interactive_mode(
    input: &InputSource,
) -> io::Result<(ModeGuard, NonblockingGuard)> {
    unix::enter_interactive_mode(input)
}

#[cfg(windows)]
fn platform_enter_interactive_mode(
    input: &InputSource,
) -> io::Result<(ModeGuard, NonblockingGuard)> {
    windows::enter_interactive_mode(input)
}

pub(crate) fn drain_pending_input(
    initial_wait: Duration,
    quiet_window: Duration,
    hard_window: Duration,
) -> io::Result<Vec<u8>> {
    platform_drain_pending_input(initial_wait, quiet_window, hard_window)
}

#[cfg(unix)]
fn platform_drain_pending_input(
    initial_wait: Duration,
    quiet_window: Duration,
    hard_window: Duration,
) -> io::Result<Vec<u8>> {
    unix::drain_pending_input(initial_wait, quiet_window, hard_window)
}

#[cfg(windows)]
fn platform_drain_pending_input(
    initial_wait: Duration,
    quiet_window: Duration,
    hard_window: Duration,
) -> io::Result<Vec<u8>> {
    windows::drain_pending_input(initial_wait, quiet_window, hard_window)
}

pub(crate) fn terminal_width() -> usize {
    crossterm::terminal::size()
        .ok()
        .map(|(width, _)| usize::from(width))
        .filter(|width| *width > 0)
        .unwrap_or(80)
}

pub(crate) fn install_sigint_guard(cancel_requested: &'static AtomicBool) -> Option<SigintGuard> {
    SigintGuard::install(cancel_requested)
}
