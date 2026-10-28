//! Uses the native dispatcher's real registration ABI beside the Rust service.

use leptoskit_bridge::Error;
use leptoskit_bridge_ffi::app::App;
use leptoskit_bridge_ffi::lk_bridge_register;

const NAME: &[u8] = b"fixture-platform";
const METHODS: [u32; 1] = [1];

pub fn register(app: &mut App) {
	app.with_host(|host| {
		// SAFETY: The host lends the bridge exclusively; arrays outlive the
		// call.
		let identifier = unsafe {
			lk_bridge_register(
				host.plugins(),
				NAME.as_ptr(),
				NAME.len(),
				METHODS.as_ptr(),
				METHODS.len(),
			)
		};
		assert!(identifier >= 0);
	});
}

pub fn assert_distinct_identities(app: &mut App) {
	app.with_host(|host| {
		let bridge = host.plugins();
		let session = bridge.resolve("fixture-session").unwrap();
		let platform = bridge.resolve("fixture-platform").unwrap();
		assert_ne!(session, platform);

		// SAFETY: The host lends the bridge exclusively; arrays outlive the
		// call.
		let duplicate = unsafe {
			lk_bridge_register(
				bridge,
				NAME.as_ptr(),
				NAME.len(),
				METHODS.as_ptr(),
				METHODS.len(),
			)
		};
		assert_eq!(duplicate, -i64::from(Error::InvalidArgument as i32));
		assert_eq!(bridge.resolve("fixture-platform"), Some(platform));
		assert_eq!(bridge.resolve("fixture-session"), Some(session));
	});
}
