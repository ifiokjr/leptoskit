//! `/settings`.

use leptoskit::prelude::*;

/// The settings list.
pub fn page() -> Element {
	Text("Settings").class("p-6 text-2xl font-semibold")
}
