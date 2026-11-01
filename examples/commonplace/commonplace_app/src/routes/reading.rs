//! `/reading`: the extension's new tab page. Your projects, one tab away.

use leptoskit::prelude::*;

use crate::browser;
use crate::routes::Navigation;
use crate::ui;

#[component]
pub fn Page(navigation: &Navigation) -> Element {
	if !browser::IN_EXTENSION {
		return view! { <ui::ExtensionOnly /> };
	}
	view! {
		<View class="gap-2">
			<Text class=ui::QUIET>"A new tab, and your research where you left it."</Text>
			<crate::routes::index::Page navigation=navigation />
		</View>
	}
}
