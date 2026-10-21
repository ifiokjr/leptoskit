//! A private module beside the routes that use it. It is never a route.

use leptoskit::prelude::*;

use crate::NoteId;

/// A note's summary card.
pub fn note_card(note: NoteId, highlight: Option<u32>) -> Element {
	let title = match highlight {
		Some(line) => format!("Note {note}, line {line}"),
		None => format!("Note {note}"),
	};
	View().class("gap-2 p-4 rounded-xl").child(Text(title))
}
