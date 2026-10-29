//! The tabbed part of the app: the selected tab above a bar of links to every
//! tab. A `Tabs` navigator replaces this bar once navigators land.

use leptoskit::prelude::*;

use crate::routes::Map;
use crate::routes::Navigation;
use crate::routes::Route;
use crate::routes::Today;

/// The tab bar under the selected tab.
pub fn layout(children: Children, navigation: &Navigation) -> Element {
	let tab = |to: Route, label: &str| {
		navigation
			.link(to, label)
			.class("flex-1 items-center p-3")
			.child(view! { <Text>{label.to_owned()}</Text> })
	};
	view! {
		<View class="flex-1">
			<View class="flex-1">{children}</View>
			<View class="flex-row">
				{tab(Today.into(), "Today")} {tab(Map { near: None }.into(), "Map")}
				{tab(Route::Notes, "Notes")} {tab(Route::You, "You")}
			</View>
		</View>
	}
}
