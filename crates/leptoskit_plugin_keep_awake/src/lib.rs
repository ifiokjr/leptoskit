//! Scoped display wake locks. A native HOLD request remains pending for the
//! lease's entire lifetime; dropping its guest future cancels and releases it.
//! There is no separate acquire response that can race cancellation and leak a
//! token. Poll a holding future alongside work to observe failures promptly.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Runtime registration name.
pub const NAME: &str = "keep-awake";
/// Empty payload; remains pending until cancelled or the native lease is lost.
pub const HOLD: MethodId = MethodId(0);
/// Exported native operations.
pub const METHODS: &[MethodId] = &[HOLD];

/// Validates the hold operation without activating any system resource.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	if method != HOLD {
		return Err(Error::Unavailable);
	}
	if payload.is_empty() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Desktop RAII leases. OS policy can still override a display inhibition.
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop {
	use std::cell::RefCell;
	use std::rc::Rc;
	use std::rc::Weak;

	use super::*;
	#[cfg(target_os = "macos")]
	#[path = "macos.rs"]
	mod platform;
	#[cfg(target_os = "windows")]
	#[path = "windows.rs"]
	mod platform;
	#[cfg(any(target_os = "macos", target_os = "windows"))]
	use platform::PlatformLease;
	#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
	type PlatformLease = dbus::blocking::Connection;

	thread_local! {
		static DISPLAY: RefCell<Weak<PlatformLease>> = const { RefCell::new(Weak::new()) };
	}
	/// Shared ownership of one system assertion, released with the final clone.
	/// Linux requires a working desktop/session inhibitor service. This guard
	/// stays on its acquiring thread because Windows execution state is
	/// thread-local. Acquisitions on the same thread share the first active
	/// owner's reason.
	#[derive(Clone)]
	pub struct Lease(Rc<PlatformLease>);
	impl Lease {
		/// Acquires a display-only assertion with a user-visible application
		/// reason.
		pub fn acquire(application: &str, reason: &str) -> Result<Self, Error> {
			if application.trim().is_empty()
				|| reason.trim().is_empty()
				|| application.len() > 256
				|| reason.len() > 1024
				|| application
					.chars()
					.chain(reason.chars())
					.any(char::is_control)
			{
				return Err(Error::InvalidArgument);
			}
			DISPLAY.with_borrow_mut(|shared| {
				if let Some(lease) = shared.upgrade() {
					return Ok(Self(lease));
				}
				let lease = acquire_platform(application, reason)?;
				let lease = Rc::new(lease);
				*shared = Rc::downgrade(&lease);
				Ok(Self(lease))
			})
		}

		/// Number of Rust owners sharing this single assertion.
		#[must_use]
		pub fn owners(&self) -> usize {
			Rc::strong_count(&self.0)
		}
	}

	#[cfg(any(target_os = "macos", target_os = "windows"))]
	fn acquire_platform(_application: &str, reason: &str) -> Result<PlatformLease, Error> {
		PlatformLease::acquire(reason)
	}

	#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
	fn acquire_platform(application: &str, reason: &str) -> Result<PlatformLease, Error> {
		let connection =
			dbus::blocking::Connection::new_session().map_err(|_| Error::Unavailable)?;
		let proxy = connection.with_proxy(
			"org.freedesktop.ScreenSaver",
			"/org/freedesktop/ScreenSaver",
			std::time::Duration::from_secs(5),
		);
		let (_cookie,): (u32,) = proxy
			.method_call(
				"org.freedesktop.ScreenSaver",
				"Inhibit",
				(application, reason),
			)
			.map_err(|error| {
				match error.name() {
					Some("org.freedesktop.DBus.Error.AccessDenied") => Error::PermissionDenied,
					Some(
						"org.freedesktop.DBus.Error.ServiceUnknown"
						| "org.freedesktop.DBus.Error.UnknownMethod",
					) => Error::Unavailable,
					_ => Error::Native,
				}
			})?;
		// The idle-inhibit specification releases every inhibitor owned by a
		// caller when its private bus connection closes. Holding that
		// connection avoids a separate fallible UnInhibit roundtrip in drop.
		Ok(connection)
	}
}

/// Native guest leases use bridge cancellation as their release mechanism.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use std::convert::Infallible;
	use std::future::Future;
	use std::pin::Pin;
	use std::task::Context;
	use std::task::Poll;

	use super::*;
	/// Resolved host plugin, cached across lease requests.
	#[derive(Debug, Clone, Copy)]
	pub struct KeepAwake(leptoskit_bridge::guest::Client);
	impl KeepAwake {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Submits a scoped lease. Retain and poll it alongside work. Pending
		/// means no failure has been reported, not an acknowledgement
		/// of acquisition.
		pub fn hold(self) -> Result<Lease, Error> {
			self.0.call(HOLD, &[]).map(Lease)
		}
	}
	/// A holding future that never completes successfully. Drop releases it via
	/// host cancellation, including when it has not been polled yet.
	#[must_use = "dropping the lease cancels the wake lock request"]
	#[derive(Debug)]
	pub struct Lease(leptoskit_bridge::guest::Call);
	impl Future for Lease {
		type Output = Result<Infallible, Error>;

		fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
			match Pin::new(&mut self.0).poll(context) {
				Poll::Pending => Poll::Pending,
				Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
				Poll::Ready(Ok(_)) => Poll::Ready(Err(Error::Native)),
			}
		}
	}
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn hold_has_no_ambiguous_payload_or_success_message() {
		assert_eq!(validate(HOLD, &[]), Ok(()));
		assert_eq!(validate(HOLD, &[0]), Err(Error::InvalidArgument));
		assert_eq!(validate(MethodId(1), &[]), Err(Error::Unavailable));
	}

	#[cfg(any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	#[test]
	fn desktop_rejects_invalid_descriptions_before_contacting_the_os() {
		for (application, reason) in [
			("", "reason"),
			("application", " "),
			("application\n", "reason"),
			("application", "reason\0"),
			(&"a".repeat(257), "reason"),
			("application", &"r".repeat(1025)),
		] {
			assert!(matches!(
				desktop::Lease::acquire(application, reason),
				Err(Error::InvalidArgument)
			));
		}
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn desktop_assertion_survives_out_of_order_release_and_ends_with_last_owner() {
		fn assertions() -> String {
			let output = std::process::Command::new("/usr/bin/pmset")
				.args(["-g", "assertions"])
				.output()
				.unwrap();
			assert!(output.status.success());
			String::from_utf8(output.stdout).unwrap()
		}
		let reason = format!("LeptosKit scoped assertion test {}", std::process::id());
		let first = desktop::Lease::acquire("LeptosKit test", &reason).unwrap();
		let second =
			desktop::Lease::acquire("LeptosKit test", "shares the first assertion").unwrap();
		assert_eq!(second.owners(), 2);
		assert!(assertions().contains(&reason));
		drop(first);
		assert_eq!(second.owners(), 1);
		assert!(assertions().contains(&reason));
		drop(second);
		assert!(!assertions().contains(&reason));
	}

	#[cfg(target_os = "windows")]
	#[test]
	fn desktop_execution_state_requires_the_display_until_the_last_owner() {
		use windows::Win32::System::Power::ES_CONTINUOUS;
		use windows::Win32::System::Power::ES_DISPLAY_REQUIRED;
		use windows::Win32::System::Power::SetThreadExecutionState;

		/// Reads this thread's execution state, then restores it unchanged.
		#[allow(unsafe_code)]
		fn display_required() -> bool {
			// SAFETY: Valid flags and no pointers. The previous state is
			// reinstated immediately on this same thread.
			let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
			assert_ne!(previous.0, 0, "read the thread execution state");
			// SAFETY: As above, with flags returned by the successful call.
			assert_ne!(
				unsafe { SetThreadExecutionState(previous | ES_CONTINUOUS) }.0,
				0
			);
			previous.0 & ES_DISPLAY_REQUIRED.0 != 0
		}

		assert!(!display_required());
		let first = desktop::Lease::acquire("LeptosKit test", "keep the display on").unwrap();
		let second =
			desktop::Lease::acquire("LeptosKit test", "shares the first assertion").unwrap();
		assert_eq!(second.owners(), 2);
		assert!(display_required());
		drop(first);
		assert!(display_required());
		drop(second);
		assert!(!display_required());
	}
}
