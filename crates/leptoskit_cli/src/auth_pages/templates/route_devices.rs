//! `/devices`, written by `leptoskit generate auth`.

use leptoskit::prelude::*;

/// Where the signed-in person is signed in, with buttons to end each one.
pub fn page() -> Element {
	let screens = crate::auth::screens();

	if screens.sign_in.is_signed_in() {
		screens.devices.screen()
	} else {
		screens.sign_in.screen()
	}
}
