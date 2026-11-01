//! Any URL no other route matches.

use leptoskit::prelude::*;

/// Says the page doesn't exist.
#[component]
pub fn Page() -> Element {
	view! { <Text class="p-6 text-lg">"This page doesn't exist."</Text> }
}
