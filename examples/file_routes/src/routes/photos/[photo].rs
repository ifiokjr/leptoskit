//! `/photos/:photo`: one photo, presented as a modal once navigators land.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// A photo's id.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Photo {
	pub photo: u64,
}

/// The photo, full screen.
#[component]
pub fn Page(route: &Photo) -> Element {
	view! { <Text class="p-6">{format!("Photo {}", route.photo)}</Text> }
}
