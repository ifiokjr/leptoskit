//! Reusable pieces of the arena screen.

use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

use crate::services::Phase;
use crate::services::Scenario;

/// The 48 pt header row: the 32 pt beacon with 8 pt margins, then the status.
pub fn header(scenario: Option<Scenario>, phase: Phase) -> Element {
	let name = scenario.map_or("arena", Scenario::id);

	View()
		.class("h-12 shrink-0 flex-row items-center")
		.children([
			View().class(format!("w-8 h-8 m-2 shrink-0 {}", phase.beacon_class())),
			Text(format!("{name} {}", phase.label())),
		])
}
