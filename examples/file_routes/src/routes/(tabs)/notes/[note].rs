//! `/notes/:note`: one note.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use super::_note_card::note_card;
use crate::NoteId;

/// The struct is the route: nothing links here without a `NoteId`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Note {
	/// Matches the `[note]` segment, so it comes from the path.
	pub note: NoteId,
	/// Matches no segment, so it is `?highlight=`.
	pub highlight: Option<u32>,
}

/// The note, pushed over the list.
pub fn page(route: &Note) -> Element {
	note_card(route.note, route.highlight)
}
