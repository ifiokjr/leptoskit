//! The arena's screens: a header row over one body per scenario.

pub mod cards;
pub mod feed;
pub mod home;
pub mod ticker;

use leptoskit::prelude::Element;
use leptoskit::prelude::ScrollView;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

use crate::components::Header;
use crate::services::Arena;
use crate::services::Screen;

/// Rows on the `cold` ready screen.
pub const READY_ROWS: usize = 20;

/// The whole screen for the current model.
pub fn screen(arena: &Arena) -> Element {
	let model = arena.read();
	let body = match &model.screen {
		Screen::Starting => View(),
		Screen::Home => home::HomeScreen(arena),
		Screen::Ready => {
			View()
				.class("px-4")
				.children((1..=READY_ROWS).map(|row| Text(format!("Row {row}"))))
		}
		Screen::Cards(cards) => ScrollView().class("flex-1").child(cards::CardList(cards)),
		Screen::Echo(status) => View().class("px-4").child(Text(status.clone())),
		Screen::Ticker(frame) => ticker::TickerGrid(*frame),
		Screen::Feed(state) => feed::FeedList(arena, state),
		Screen::Unsupported(reason) => View().class("px-4").child(Text(reason.clone())),
	};

	View()
		.class("h-screen pt-safe pb-safe bg-[#ffffff] text-[#111111]")
		.children([Header(model.scenario, model.phase), body])
}
