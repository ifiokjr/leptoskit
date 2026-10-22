//! The root layout: the frame around every screen.

use leptoskit::prelude::*;

/// Wraps the matched route in the app's frame.
pub fn layout(children: Children) -> Element {
	View()
		.class("flex-1 bg-white dark:bg-slate-950")
		.children(children)
}
