//! The `mount` cards.

use leptoskit::prelude::Element;
use leptoskit::prelude::Get;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit::prelude::component;

use crate::services::Card;

/// The title of a card: `Card i`, or `Card i !` once the update marked it.
pub fn title(card: &Card) -> String {
	if card.is_marked() {
		format!("Card {} !", card.number)
	} else {
		format!("Card {}", card.number)
	}
}

/// Every card, unvirtualised, in one column.
///
/// A card's title is bound to its `marked` signal, so marking every tenth card
/// rewrites a hundred labels instead of rebuilding three thousand elements.
#[component]
pub fn CardList(cards: &[Card]) -> Element {
	View().children(cards.iter().map(|card| {
		let number = card.number;
		let marked = card.marked;

		View().class("px-4 py-2").children([
			Element::text_from(move || {
				if marked.get() {
					format!("Card {number} !")
				} else {
					format!("Card {number}")
				}
			}),
			Text(format!("Subtitle {number}")).class("text-sm"),
		])
	}))
}
