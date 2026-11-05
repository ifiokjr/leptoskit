//! <!-- {=p_single_instanceSummary|trim|linePrefix:"//! ":true} -->
//!
//! Native hosts construct it explicitly; it is not a guest plugin and installs
//! nothing by default. Call `InstanceGuard::acquire` before constructing the
//! app and keep the guard alive; macOS, Linux, and Windows release the OS lock
//! on drop or process exit, including a crash.
//!
//! <!-- {/p_single_instanceSummary} -->
//!
//! Acquire before creating application windows; retain the guard for the whole
//! application lifetime. A second process receives `None`. This crate provides
//! mutual exclusion only: it does **not** forward arguments, focus windows, or
//! deliver activation events to the first process.

#[cfg(any(
	all(target_os = "linux", not(target_env = "ohos")),
	target_os = "macos",
	target_os = "windows"
))]
mod desktop;
#[cfg(any(
	all(target_os = "linux", not(target_env = "ohos")),
	target_os = "macos",
	target_os = "windows"
))]
pub use desktop::InstanceGuard;
