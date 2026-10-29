//! `/settings/:section`.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// One settings section, by name.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Section {
	pub section: String,
}

/// The section's settings. The page takes its route by value, so it can
/// keep the name; the router passes it a clone.
pub fn page(route: Section) -> Element {
	view! { <Text class="px-6">{route.section}</Text> }
}
