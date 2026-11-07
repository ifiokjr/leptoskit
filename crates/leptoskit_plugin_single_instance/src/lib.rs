//! <!-- {=p_single_instanceSummary|trim|linePrefix:"//! ":true} -->
//!
//! Native hosts construct it explicitly; it is not a guest plugin and installs
//! nothing by default. Call `InstanceGuard::acquire` before constructing the
//! app and keep the guard alive; macOS, Linux, and Windows release the OS lock
//! on drop or process exit, including a crash. When acquire returns `None`,
//! call `notify_running` to leave a bounded activation for the owner to
//! `poll_activation`.
//!
//! <!-- {/p_single_instanceSummary} -->
//!
//! Acquire before creating application windows; retain the guard for the whole
//! application lifetime. A second process receives `None` and may forward its
//! launch arguments through [`notify_running`]. Window focus remains the host's
//! responsibility after it observes an activation.

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
pub use desktop::Activation;
#[cfg(any(
	all(target_os = "linux", not(target_env = "ohos")),
	target_os = "macos",
	target_os = "windows"
))]
pub use desktop::InstanceGuard;
#[cfg(any(
	all(target_os = "linux", not(target_env = "ohos")),
	target_os = "macos",
	target_os = "windows"
))]
pub use desktop::notify_running;
