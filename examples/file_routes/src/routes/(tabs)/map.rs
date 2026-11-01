//! `/map?near=`: notes on a map.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// `near` matches no segment, so it comes from the query string.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Map {
	pub near: Option<String>,
}

/// The second tab.
#[component]
pub fn Page() -> Element {
	view! { <Text class="p-6 text-3xl font-bold">"Map"</Text> }
}
