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
			.child(view! { <Text>{format!("Note {note}")}</Text> })
	});
	view! {
		<View class="gap-2 p-6">
			<Text class="text-3xl font-bold">"Notes"</Text>
			{notes.into_iter().collect::<Fragment>()}
		</View>
	}
}
