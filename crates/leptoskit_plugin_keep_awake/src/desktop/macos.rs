//! The assertion identifier is owned once and released with the final lease.
#![allow(unsafe_code)]

use leptoskit_bridge::Error;
use objc2_core_foundation::CFString;
use objc2_io_kit::IOPMAssertionCreateWithName;
use objc2_io_kit::IOPMAssertionID;
use objc2_io_kit::IOPMAssertionRelease;
use objc2_io_kit::kIOPMAssertionLevelOn;
use objc2_io_kit::kIOReturnSuccess;

pub(super) struct PlatformLease(IOPMAssertionID);

impl PlatformLease {
	pub(super) fn acquire(reason: &str) -> Result<Self, Error> {
		let assertion_type = CFString::from_static_str("PreventUserIdleDisplaySleep");
		let reason = CFString::from_str(reason);
		let mut identifier = 0;
		// SAFETY: Both CFStrings and the writable output identifier remain
		// valid for the synchronous call. Ownership transfers only
		// after success.
		let status = unsafe {
			IOPMAssertionCreateWithName(
				Some(&assertion_type),
				kIOPMAssertionLevelOn,
				Some(&reason),
				&raw mut identifier,
			)
		};
		if status != kIOReturnSuccess {
			return Err(Error::Native);
		}
		Ok(Self(identifier))
	}
}

impl Drop for PlatformLease {
	fn drop(&mut self) {
		let status = IOPMAssertionRelease(self.0);
		if status != kIOReturnSuccess {
			eprintln!("LeptosKit display assertion release failed: {status}");
		}
	}
}
