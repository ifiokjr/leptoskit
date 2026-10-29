//! `/photos` and every URL below it that names no photo, such as
//! `/photos/beach`: a photo's id is a number.

use leptoskit::prelude::*;

/// Says the photo doesn't exist, rather than the whole page.
pub fn page() -> Element {
	Text("This photo doesn't exist.").class("p-6 text-lg")
}
