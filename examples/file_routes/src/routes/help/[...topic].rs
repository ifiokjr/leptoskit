//! `/help/*topic`: help pages nested as deep as they go.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// Every segment after `/help`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Help {
	pub topic: Vec<String>,
}

/// The help page.
#[component]
pub fn Page(route: &Help) -> Element {
	view! { <Text class="p-6">{route.topic.join(" › ")}</Text> }
}
