//! `/session`: the conversation, its prompt, and the context sidebar on wide
//! screens.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::Action;
use crate::Store;
use crate::ui::prompt;
use crate::ui::session;

/// The open session.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Session;

/// A tab, the conversation pinned to its newest message, the prompt, and the
/// sidebar once the terminal is wider than 120 cells.
pub fn page() -> Element {
	let store = Store::current();
	let reply = {
		let store = store.clone();
		move || store.apply(Action::Reply)
	};

	store.with(|state| {
		view! {
			<View class="flex-1">
				{session::tabs(state)}
				<View class="flex-1 flex-row">
					<View class="flex-1 px-4 pb-4">
						{session::conversation(state)} {prompt(&store, "", reply)}
						{session::footer(state)}
					</View>
					{session::sidebar(state)}
				</View>
			</View>
		}
	})
}
