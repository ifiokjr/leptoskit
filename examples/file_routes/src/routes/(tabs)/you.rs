//! `/you`: the account tab, written as a component with `view!`.

use leptoskit::prelude::*;

use crate::routes::Navigation;

/// The fourth tab, with a link to settings.
#[component]
pub fn Page(navigation: &Navigation) -> impl IntoView {
	let settings = navigation
		.link(crate::routes::Route::Settings, "Open settings")
		.child(Text("Settings"));
	view! {
		<View class="gap-4 p-6">
			<Text class="text-3xl font-bold">"You"</Text>
			{settings}
		</View>
	}
}
