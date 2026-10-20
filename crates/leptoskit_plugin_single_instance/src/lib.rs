//! Host-owned process locks for desktop apps.
//!
//! Acquire before creating application windows; retain the guard for the whole
//! application lifetime. A second process receives `None`. This crate provides
//! mutual exclusion only: it does **not** forward arguments, focus windows, or
//! deliver activation events to the first process.

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod desktop;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub use desktop::InstanceGuard;
