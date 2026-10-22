//! `/settings/:section`.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// One settings section, by name.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Section {
	pub section: String,
}

/// The section's settings.
pub fn page(route: &Section) -> Element {
	Text(route.section.clone()).class("p-6")
}
