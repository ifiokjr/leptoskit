//! Reusable pieces of the arena screen.

use leptoskit::prelude::Element;
use leptoskit::prelude::Get;
use leptoskit::prelude::RwSignal;
use leptoskit::prelude::View;

use crate::services::Phase;
use crate::services::Scenario;

/// The 48 pt header row: the 32 pt beacon with 8 pt margins, then the status.
///
/// The beacon's colour and the status text follow `phase` as bindings, so a
/// scenario that moves from running to done rewrites two nodes rather than
/// rebuilding the screen around them.
pub fn header(scenario: Option<Scenario>, phase: RwSignal<Phase>) -> Element {
	let name = scenario.map_or("arena", Scenario::id);

	View()
		.class("h-12 shrink-0 flex-row items-center")
		.children([
			Element::background_from(move || phase.get().colour()).class("w-8 h-8 m-2 shrink-0"),
			Element::text_from(move || format!("{name} {}", phase.get().label())),
		])
}
