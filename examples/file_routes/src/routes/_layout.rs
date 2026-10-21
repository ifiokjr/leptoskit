//! The root layout: chrome around every screen.

use leptoskit::prelude::*;

/// Wraps a screen in the app's frame.
pub fn layout(screen: Element) -> Element {
	View()
		.class("flex-1 bg-white dark:bg-slate-950")
		.child(screen)
}
