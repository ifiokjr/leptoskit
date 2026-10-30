//! `/notes/:note`: one note.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use super::_note_card::note_card;
use crate::NoteId;
use crate::routes::Compose;
use crate::routes::Navigation;

/// The struct is the route: nothing links here without a `NoteId`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Note {
	/// Matches the `[note]` segment, so it comes from the path.
	pub note: NoteId,
	/// Matches no segment, so it is `?highlight=`.
	pub highlight: Option<u32>,
}

/// The note, with a way back and a way to reply.
pub fn page(route: &Note, navigation: &Navigation) -> Element {
	let back = navigation.clone();
	let reply = Compose {
		reply_to: Some(route.note),
	};
	view! {
		<View class="gap-4 p-4">
			{note_card(route.note, route.highlight)}
			{navigation
				.link(reply, "Reply")
				.class("p-3")
				.child(view! { <Text>"Reply"</Text> })}
			<Button on:press=move || {
				back.back();
			}>
				"Back"
			</Button>
		</View>
	}
}
