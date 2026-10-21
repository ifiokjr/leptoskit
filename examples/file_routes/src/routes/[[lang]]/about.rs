//! `/about` and `/fr/about`: an optional language segment.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// `lang` may be absent, so it is an `Option`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct About {
	pub lang: Option<String>,
}

/// About the app, in the requested language.
pub fn page(route: &About) -> Element {
	let greeting = match route.lang.as_deref() {
		Some("fr") => "À propos de Field",
		_ => "About Field",
	};
	Text(greeting).class("p-6")
}
