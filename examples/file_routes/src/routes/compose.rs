//! `/compose`: a new note, presented as a sheet once navigators land.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::NoteId;

/// A reply names the note it answers.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Compose {
	pub reply_to: Option<NoteId>,
}

/// The editor.
pub fn page() -> Element {
	view! { <TextInput placeholder="Write a note" class="m-4 p-3 rounded-lg" /> }
}
