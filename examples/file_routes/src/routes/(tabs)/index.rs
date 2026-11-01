//! `/`: today's notes.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// Names the route `Today` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Today;

/// The first tab.
#[component]
pub fn Page() -> Element {
	view! { <Text class="p-6 text-3xl font-bold">"Today"</Text> }
}
