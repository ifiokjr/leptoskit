//! `/connected-apps`, written by `leptoskit generate auth --idp`.

use leptoskit::prelude::*;

/// The apps the signed-in person let sign in with their account.
pub fn page() -> Element {
	let screens = crate::auth::screens();

	if screens.sign_in.is_signed_in() {
		screens.connected_apps.screen()
	} else {
		screens.sign_in.screen()
	}
}
