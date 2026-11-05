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
#[component]
pub fn Page(navigation: &crate::routes::Navigation) -> Element {
	let draft = reactive_graph::owner::use_context::<crate::Draft>()
		.unwrap_or_default()
		.0;
	let value = draft.get();
	let back = navigation.clone();
	view! {
		<View class="gap-4 p-4">
			<TextInput
				value=value
				on:change=move |value: &str| draft.set(value.to_owned())
				accessibility_label="Note draft"
				placeholder="Write a note"
				class="p-3 rounded-lg"
			/>
			<Button on:press=move || {
				back.back();
			}>
				"Back"
			</Button>
		</View>
	}
}
