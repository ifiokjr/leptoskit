//! `/sign-in`. The `(auth)` group adds nothing to the URL.

use leptoskit::prelude::*;

/// The sign-in form.
pub fn page() -> Element {
	Button("Sign in").class("m-6 px-4 py-2 rounded-lg")
}
