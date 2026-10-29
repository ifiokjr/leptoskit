//! `/photos` and every URL below it that names no photo, such as
//! `/photos/beach`: a photo's id is a number.

use leptoskit::prelude::*;

/// Says the photo doesn't exist, rather than the whole page.
pub fn page() -> Element {
	view! { <Text class="p-6 text-lg">"This photo doesn't exist."</Text> }
}
