//! The root layout: the frame around every screen.

use leptoskit::prelude::*;

/// Wraps the matched route in the app's frame.
#[component]
pub fn Layout(children: Children) -> Element {
	view! { <View class="flex-1 bg-white dark:bg-slate-950">{children}</View> }
}
