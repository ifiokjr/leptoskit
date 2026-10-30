//! Any address that isn't one of Commonplace's screens.

use leptoskit::prelude::*;

use crate::routes::Library;
use crate::routes::Navigation;
use crate::ui;

pub fn page(navigation: &Navigation) -> Element {
	View()
		.class("gap-3")
		.child(ui::title("Nothing here"))
		.child(Text("This page doesn't exist, or it moved."))
		.child(
			navigation
				.link(Library, "Back to your library")
				.child(Text("Back to your library").class("text-amber-700")),
		)
}
