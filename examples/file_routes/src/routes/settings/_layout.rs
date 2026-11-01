//! The frame around `/settings` and each section: a layout in a plain
//! directory, below the root layout.

use leptoskit::prelude::*;

use crate::routes::Route;

/// A heading that follows the route: `Settings`, or `Settings › privacy`.
#[component]
pub fn Layout(children: Children, route: &Route) -> Element {
	let heading = match route {
		Route::Section(section) => format!("Settings › {}", section.section),
		_ => "Settings".to_owned(),
	};
	view! {
		<View class="flex-1">
			<Text class="p-6 text-2xl font-semibold">{heading}</Text>
			<View class="flex-1">{children}</View>
		</View>
	}
}
