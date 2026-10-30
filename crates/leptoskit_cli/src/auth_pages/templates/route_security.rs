//! `/security`, written by `leptoskit generate auth`.

use leptoskit::prelude::*;

/// Two-step verification: the authenticator app and recovery codes.
pub fn page() -> Element {
	let screens = crate::auth::screens();

	if screens.sign_in.is_signed_in() {
		screens.security.screen()
	} else {
		screens.sign_in.screen()
	}
}
