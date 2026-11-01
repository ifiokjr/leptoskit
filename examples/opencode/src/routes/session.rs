//! `/session`: the conversation, its prompt, and the context sidebar on wide
//! screens.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::Action;
use crate::Store;
use crate::ui::Prompt;
use crate::ui::session;

/// The open session.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Session;

/// A tab, the conversation pinned to its newest message, the prompt, and the
/// sidebar once the terminal is wider than 120 cells.
#[component]
pub fn Page() -> Element {
	let store = Store::current();
	let reply = {
		let store = store.clone();
		move || store.apply(Action::Reply)
	};

	store.with(|state| {
		view! {
			<View class="flex-1">
				{view! { <session::Tabs state=state /> }}
				<View class="flex-1 flex-row">
					<View class="flex-1 px-4 pb-4">
						{view! { <session::Conversation state=state /> }}
						{view! { <Prompt store=&store placeholder="" submit=reply /> }}
						{view! { <session::Footer state=state /> }}
					</View>
					{view! { <session::Sidebar state=state /> }}
				</View>
			</View>
		}
	})
}
