//! The arena's screens: a header row over one body per scenario.

pub mod cards;
pub mod feed;
pub mod home;
pub mod ticker;

use leptoskit::prelude::Element;
use leptoskit::prelude::ScrollView;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

use crate::components::header;
use crate::services::Arena;
use crate::services::Screen;

/// Rows on the `cold` ready screen.
pub const READY_ROWS: usize = 20;

/// The whole screen for the current model.
pub fn screen(arena: &Arena) -> Element {
	let model = arena.read();
	let body = match &model.screen {
		Screen::Starting => View(),
		Screen::Home => home::screen(arena),
		Screen::Ready => {
			View()
				.class("px-4")
				.children((1..=READY_ROWS).map(|row| Text(format!("Row {row}"))))
		}
		Screen::Cards(cards) => ScrollView().class("flex-1").child(cards::list(cards)),
		Screen::Echo(status) => View().class("px-4").child(Text(status.clone())),
		Screen::Ticker(frame) => ticker::grid(*frame),
		Screen::Feed {
			offset,
			viewport,
			animated,
		} => {
			let arena = arena.clone();
			feed::scrolled(*offset, *viewport, *animated, move |offset| {
				arena.report_scroll(offset);
			})
		}
		Screen::Unsupported(reason) => View().class("px-4").child(Text(reason.clone())),
	};

	View()
		.class("h-screen pt-safe pb-safe bg-[#ffffff] text-[#111111]")
		.children([header(model.scenario, model.phase), body])
}
