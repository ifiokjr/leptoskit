//! The `mount` cards.

use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

use crate::services::Card;

/// The title of a card: `Card i`, or `Card i !` once the update marked it.
pub fn title(card: &Card) -> String {
	if card.marked {
		format!("Card {} !", card.number)
	} else {
		format!("Card {}", card.number)
	}
}

/// Every card, unvirtualised, in one column.
pub fn list(cards: &[Card]) -> Element {
	View().children(cards.iter().map(|card| {
		View().class("px-4 py-2").children([
			Text(title(card)),
			Text(format!("Subtitle {}", card.number)).class("text-sm"),
		])
	}))
}
