//! Any URL no other route matches.

use leptoskit::prelude::*;

/// Says the page doesn't exist.
pub fn page() -> Element {
	Text("This page doesn't exist.").class("p-6 text-lg")
}
