//! Thread execution state is restored on the same thread that acquired it.
#![allow(unsafe_code)]

use leptoskit_bridge::Error;
use windows::Win32::System::Power::ES_CONTINUOUS;
use windows::Win32::System::Power::ES_DISPLAY_REQUIRED;
use windows::Win32::System::Power::EXECUTION_STATE;
use windows::Win32::System::Power::SetThreadExecutionState;

pub(super) struct PlatformLease(EXECUTION_STATE);

impl PlatformLease {
	pub(super) fn acquire(_reason: &str) -> Result<Self, Error> {
		// SAFETY: Valid flags; no pointers. The public Rc guard is !Send, so
		// release always runs on this same thread.
		let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED) };
		if previous == EXECUTION_STATE(0) {
			return Err(Error::Native);
		}
		let lease = Self(previous);
		// Preserve requirements established by other users of this thread.
		// SAFETY: The returned flags are valid and are combined with valid
		// flags.
		let result =
			unsafe { SetThreadExecutionState(previous | ES_CONTINUOUS | ES_DISPLAY_REQUIRED) };
		if result == EXECUTION_STATE(0) {
			return Err(Error::Native);
		}
		Ok(lease)
	}
}

impl Drop for PlatformLease {
	fn drop(&mut self) {
		// SAFETY: This guard cannot leave its acquiring thread; the flags were
		// returned by a successful call on that thread.
		if unsafe { SetThreadExecutionState(self.0 | ES_CONTINUOUS) } == EXECUTION_STATE(0) {
			eprintln!("LeptosKit display execution state restoration failed");
		}
	}
}
