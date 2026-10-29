//! The frame around every screen: the page colors, opencode's keyboard
//! shortcuts, and the command palette over the screen when it is open.

use leptoskit::prelude::*;
use leptoskit_native::KeyPress;

use crate::Action;
use crate::Store;
use crate::routes::Home;
use crate::routes::Navigation;
use crate::ui::palette;

/// Wraps the current screen. Keys reach this view before the focused field,
/// so ctrl+p opens the palette even while the prompt is focused.
pub fn layout(children: Children, navigation: &Navigation) -> Element {
	let store = Store::current();
	let open = store.with(|state| state.palette.clone());
	let keys = {
		let store = store.clone();
		let navigation = navigation.clone();
		move |key: &KeyPress| shortcut(&store, &navigation, key)
	};

	let root = View()
		.class("w-full h-full bg-[#0a0a0a] text-[#eeeeee]")
		.on_key(keys)
		.children(children);

	match open {
		Some(open) => root.child(store.with(|state| palette::palette(state, &open, &store))),
		None => root,
	}
}

/// What a key does before the focused field sees it: ctrl+p toggles the
/// palette, shift+tab switches agents, and while the palette is open the
/// arrows, enter, and escape drive it.
fn shortcut(store: &Store, navigation: &Navigation, key: &KeyPress) -> EventOutcome {
	let open = store.with(|state| state.palette.is_some());
	let action = if key.is("p", true, false, false) {
		Action::TogglePalette
	} else if open && key.is("Escape", false, false, false) {
		Action::Dismiss
	} else if open && key.is("ArrowUp", false, false, false) {
		Action::MoveSelection(-1)
	} else if open && key.is("ArrowDown", false, false, false) {
		Action::MoveSelection(1)
	} else if open && key.is("Enter", false, false, false) {
		let command = store.with(palette::selected_command);
		store.apply(Action::Dismiss);
		if command.is_some_and(|command| command.title == "New session") {
			store.apply(Action::NewSession);
			navigation.navigate(Home);
		}
		return EventOutcome::Handled;
	} else if !open && key.is("Tab", false, false, true) {
		Action::CycleAgent
	} else {
		return EventOutcome::Unhandled;
	};

	store.apply(action);
	EventOutcome::Handled
}
