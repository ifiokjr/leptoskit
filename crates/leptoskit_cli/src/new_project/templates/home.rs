//! The home screen, shared across every platform.

use leptoskit::prelude::Button;
use leptoskit::prelude::Element;
use leptoskit::prelude::Get;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

use crate::Counter;

/// Builds the shared counter screen with a platform-neutral event handler.
pub fn screen(state: &Counter) -> Element {
	let counter = state.clone();
	let value = state.signal();

	View()
		.class("min-h-screen items-center justify-center gap-6 p-6 bg-slate-950 text-white")
		.child(Text("Hello, leptoskit").class("text-3xl font-bold"))
		.child(Text(move || format!("Count: {}", value.get())).class("text-xl"))
		.child(
			Button("Add one")
				.class("rounded-lg bg-indigo-500 px-6 py-3 font-semibold")
				.on_press(move || counter.increment()),
		)
}
