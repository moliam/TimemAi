//! Cross-platform operating-system policy and process primitives used by Core.
//!
//! `api` is the stable Core-facing facade. Target-specific details stay in
//! `macos`/`linux`; Unix process primitives shared by both live in `shared`.

mod api;
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
pub use process_job::*;

#[cfg(test)]
#[path = "../tests/unit/platform_tests.rs"]
mod tests;
