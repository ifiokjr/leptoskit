//! `/notes`: every note.

use leptoskit::prelude::*;

use crate::routes::Navigation;
use crate::routes::Note;

/// The third tab's first screen: a link to each note.
pub fn page(navigation: &Navigation) -> Element {
	let notes = [1, 2, 3].map(|note| {
		navigation
			.link(
				Note {
					note,
					highlight: None,
				},
				format!("Open note {note}"),
			)
			.class("p-4")
			.child(Text(format!("Note {note}")))
	});
	View()
		.class("gap-2 p-6")
		.child(Text("Notes").class("text-3xl font-bold"))
		.children(notes)
}
