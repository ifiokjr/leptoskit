//! Any URL no other route matches.

use leptoskit::prelude::*;

/// Says the page doesn't exist.
pub fn page() -> Element {
	view! { <Text class="p-6 text-lg">"This page doesn't exist."</Text> }
}
