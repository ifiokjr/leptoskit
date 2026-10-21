//! `/you`: the account tab.

use leptoskit::prelude::*;

/// The fourth tab.
pub fn page() -> Element {
	Text("You").class("p-6 text-3xl font-bold")
}
