//! `/about` and `/fr/about`: an optional language segment.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// `lang` may be absent, so it is an `Option`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct About {
	pub lang: Option<String>,
}

/// About the app, in the requested language.
#[component]
pub fn Page(route: &About) -> Element {
	let greeting = match route.lang.as_deref() {
		Some("fr") => "À propos de Field",
		_ => "About Field",
	};
	view! { <Text class="p-6">{greeting}</Text> }
}
