//! The tabbed part of the app. A `Tabs` navigator replaces this layout once
//! navigators land.

use leptoskit::prelude::*;

/// Puts the tab bar under the selected tab.
pub fn layout(tab: Element) -> Element {
	View()
		.class("flex-1")
		.children([tab, Text("Today · Map · Notes · You").class("p-3")])
}
