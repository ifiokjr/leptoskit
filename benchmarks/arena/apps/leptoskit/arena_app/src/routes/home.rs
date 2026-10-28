//! The home screen: one button per scenario, for running them by hand.

use leptoskit::prelude::Button;
use leptoskit::prelude::Element;
use leptoskit::prelude::ScrollView;
use leptoskit::prelude::View;

use crate::services::Arena;
use crate::services::Scenario;

/// Lists every scenario as a button that starts it.
pub fn screen(arena: &Arena) -> Element {
	ScrollView()
		.class("flex-1")
		.child(
			View()
				.class("gap-2 p-4")
				.children(Scenario::ALL.map(|scenario| {
					let arena = arena.clone();

					Button(scenario.id())
						.class("h-12 rounded-lg bg-slate-200 px-4")
						.on_press(move || crate::start(&arena, scenario))
				})),
		)
}
