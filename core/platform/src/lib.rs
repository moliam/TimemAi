//! Cross-platform operating-system policy and process primitives used by Core.
//!
//! `api` is the stable Core-facing facade. Target-specific details stay in
//! `macos`/`linux`; Unix process primitives shared by both live in `shared`.

mod api;
mod child_input;
mod child_output;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod process_job;
#[cfg(unix)]
mod shared;
#[cfg(windows)]
mod windows;

pub use api::*;
pub use child_input::ChildInputPipe;
pub use child_output::ChildOutputPipe;
pub use process_job::*;

#[cfg(test)]
#[path = "../tests/unit/platform_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) mod test_support {
    // Multi-threaded test binaries spawn real children concurrently. A child
    // in its pre-execve window briefly holds a copy of the whole fd table, so
    // a pipe's read end can stay referenced for milliseconds after wait()
    // reaped its owner. Tests that spawn children or drive process-global
    // supervisor state therefore share one lock; their timing assumptions
    // (EPIPE after wait, exact orphan event counts) then stay deterministic
    // without weakening any assertion.
    pub(crate) static PROCESS_LIFECYCLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) fn process_lifecycle_test_lock() -> std::sync::MutexGuard<'static, ()> {
        PROCESS_LIFECYCLE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
