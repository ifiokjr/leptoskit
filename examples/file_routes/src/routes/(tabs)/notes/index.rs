//! `/notes`: every note.

use leptoskit::prelude::*;

/// The third tab's first screen.
pub fn page() -> Element {
	Text("Notes").class("p-6 text-3xl font-bold")
}
