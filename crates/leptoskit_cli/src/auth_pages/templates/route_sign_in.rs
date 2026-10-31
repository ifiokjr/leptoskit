//! `/sign-in`, written by `leptoskit generate auth`.

use leptoskit::prelude::*;

/// Signs someone in with a code emailed to them, or shows that they are.
pub fn page() -> Element {
	crate::auth::screens().sign_in.screen()
}
