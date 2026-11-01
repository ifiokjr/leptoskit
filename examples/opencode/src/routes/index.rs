//! `/`: the logo above a centered prompt, with the version in the corner.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::Action;
use crate::Store;
use crate::VERSION;
use crate::routes::Navigation;
use crate::routes::Session;
use crate::ui::Hints;
use crate::ui::Prompt;

/// Names the route `Home` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Home;

/// The placeholder suggestion in the empty prompt.
const SUGGESTION: &str = "Ask anything… \"Fix a TODO in the codebase\"";

/// The prompt's width: 75 cells.
const PROMPT_WIDTH: &str = "w-[600px] max-w-full";

/// Submitting a prompt starts the scripted session and opens it.
#[component]
pub fn Page(navigation: &Navigation) -> Element {
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

	view! {
		<View class="flex-1 items-center justify-center">
			<crate::logo::Logo />
			<View class=format!("mt-8 {PROMPT_WIDTH}")>
				<Prompt store=&store placeholder=SUGGESTION submit=submit />
			</View>
			<View class=format!("mt-2 {PROMPT_WIDTH} flex-row justify-between")>
				<Text class="text-[#808080]">{directory}</Text>
				<Hints pairs=&[("shift+tab", "agents"), ("ctrl+p", "commands")] />
			</View>
			<Text class="absolute bottom-4 right-4 text-[#808080]">{VERSION}</Text>
		</View>
	}
}
