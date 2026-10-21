//! `/photos/:photo`: one photo, presented as a modal once navigators land.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// A photo's id.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Photo {
	pub photo: u64,
}

/// The photo, full screen.
pub fn page(route: &Photo) -> Element {
	Text(format!("Photo {}", route.photo)).class("p-6")
}
