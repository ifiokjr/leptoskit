//! `/`: the logo above a centered prompt, with the version in the corner.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::Action;
use crate::Store;
use crate::VERSION;
use crate::routes::Navigation;
use crate::routes::Session;
use crate::ui::hints;
use crate::ui::prompt;

/// Names the route `Home` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Home;

/// The placeholder suggestion in the empty prompt.
const SUGGESTION: &str = "Ask anything… \"Fix a TODO in the codebase\"";

/// The prompt's width: 75 cells.
const PROMPT_WIDTH: &str = "w-[600px] max-w-full";

/// Submitting a prompt starts the scripted session and opens it.
pub fn page(navigation: &Navigation) -> Element {
	let store = Store::current();
	let directory = store.with(|state| state.directory.clone());
	let submit = {
		let store = store.clone();
		let navigation = navigation.clone();
		move || {
			store.apply(Action::StartSession);
			navigation.navigate(Session);
		}
	};

	View()
		.class("flex-1 items-center justify-center")
		.child(crate::logo::logo())
		.child(
			View()
				.class(format!("mt-8 {PROMPT_WIDTH}"))
				.child(prompt(&store, SUGGESTION, submit)),
		)
		.child(
			View()
				.class(format!("mt-2 {PROMPT_WIDTH} flex-row justify-between"))
				.child(Text(directory).class("text-[#808080]"))
				.child(hints(&[("shift+tab", "agents"), ("ctrl+p", "commands")])),
		)
		.child(Text(VERSION).class("absolute bottom-4 right-4 text-[#808080]"))
}
