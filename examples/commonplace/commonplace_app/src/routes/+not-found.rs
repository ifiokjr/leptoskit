//! Any address that isn't one of Commonplace's screens.

use leptoskit::prelude::*;

use crate::routes::Library;
use crate::routes::Navigation;
use crate::ui;

#[component]
pub fn Page(navigation: &Navigation) -> Element {
	view! {
		<View class="gap-3">
			{view! { <ui::Title text="Nothing here" /> }}
			<Text>"This page doesn't exist, or it moved."</Text>
			{navigation
				.link(Library, "Back to your library")
				.child(view! { <Text class="text-amber-700">"Back to your library"</Text> })}
		</View>
	}
}
