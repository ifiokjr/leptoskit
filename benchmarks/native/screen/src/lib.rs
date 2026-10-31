//! The screen guest: the arena's `ticker` and `mount` screens rendered through
//! the element-tree adapter a shipped app uses.
//!
//! The hand-written guest beside it calls the rendering ABI directly, so it
//! measures the ABI. This one measures the layer above: `Element` builders,
//! Tailwind resolution, the retained tree, reconciliation, and the wire
//! format — the work a real leptoskit app does per frame.
//!
//! Events drive the steps, so the harness needs no second entry point:
//!
//! - `Press` ticks the ticker one frame (300 tiles rewritten);
//! - `Change` creates 1,000 cards;
//! - `Submit` marks every tenth card;
//! - `Focus` clears the cards.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use std::cell::RefCell;

use leptoskit_native::Element;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::guest::mount;

/// Tiles across the ticker wall.
const COLUMNS: u64 = 15;
/// Rows of tiles.
const ROWS: u64 = 20;
/// Cards the `mount` screen creates.
const CARDS: usize = 1_000;

/// One card of the `mount` screen.
struct Card {
	number: usize,
	marked: bool,
}

/// What the screen shows.
enum Screen {
	Ticker(u64),
	Cards(Vec<Card>),
}

impl Screen {
	fn empty_cards() -> Self {
		Self::Cards(Vec::new())
	}
}

thread_local! {
	static SCREEN: RefCell<Screen> = const { RefCell::new(Screen::Ticker(0)) };
}

/// The whole screen: a header row over the scenario's body.
fn screen() -> Element {
	SCREEN.with_borrow(|screen| {
		let body = match screen {
			Screen::Ticker(frame) => ticker(*frame),
			Screen::Cards(cards) => cards_list(cards),
		};
		View()
			.class("h-screen bg-[#ffffff] text-[#111111]")
			.children([header(), body])
	})
}

fn header() -> Element {
	View()
		.class("h-12 flex-row items-center bg-[#ffffff]")
		.children([View()
			.class("w-8 h-8 m-2 bg-[#000000]")
			.child(Text("ticker idle").class("text-sm"))])
}

fn ticker(frame: u64) -> Element {
	View().class("flex-1").children((0..ROWS).map(|row| {
		View()
			.class("h-8 flex-row")
			.children((0..COLUMNS).map(|column| {
				let tile = row * COLUMNS + column;

				View()
					.class("flex-1 items-center justify-center")
					.child(Text((frame + tile).to_string()).class("text-[10px]"))
			}))
	}))
}

fn cards_list(cards: &[Card]) -> Element {
	View().class("flex-1").children(cards.iter().map(|card| {
		View().class("px-4 py-2").children([
			Text(card_title(card)),
			Text(format!("Subtitle {}", card.number)).class("text-sm"),
		])
	}))
}

fn card_title(card: &Card) -> String {
	if card.marked {
		format!("Card {} !", card.number)
	} else {
		format!("Card {}", card.number)
	}
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	mount(screen);
}

/// Drives one step of the current screen. See the module docs for the mapping.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(id: i64, kind: i32, pointer: i32, length: i32) -> i32 {
	const PRESS: i32 = 0;
	const CHANGE: i32 = 1;
	const SUBMIT: i32 = 2;
	const FOCUS: i32 = 3;
	let _ = (id, pointer, length);

	let step = SCREEN.with_borrow_mut(|screen| -> Option<bool> {
		Some(match kind {
			PRESS => {
				match screen {
					Screen::Ticker(frame) => *frame += 1,
					Screen::Cards(_) => *screen = Screen::Ticker(1),
				}

				true
			}
			CHANGE => {
				*screen = Screen::Cards(
					(1..=CARDS)
						.map(|number| {
							Card {
								number,
								marked: false,
							}
						})
						.collect(),
				);

				true
			}
			SUBMIT => {
				let Screen::Cards(cards) = &mut *screen else {
					return None;
				};

				for card in cards.iter_mut().filter(|card| card.number % 10 == 0) {
					card.marked = true;
				}

				true
			}
			FOCUS => {
				*screen = Screen::empty_cards();

				true
			}
			_ => return None,
		})
	});

	let Some(step) = step else {
		return 0;
	};

	if step {
		leptoskit_native::guest::render();
	}

	1
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(length: i32) -> i32 {
	leptoskit_native::guest::alloc(length)
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit_native::guest::ABI_VERSION as i32
}
