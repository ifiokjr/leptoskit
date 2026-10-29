//! Pieces the routes share, built from shared components and Tailwind
//! classes.
//!
//! The terminal lays these out on its 8×16 cell grid and the browser lays
//! them out with CSS, so every size here is a whole or half cell: `px-4` is
//! two columns, `py-4` one row, and `pb-2` the half row opencode draws with
//! `▀` under its prompt.

use leptoskit::prelude::*;
use leptoskit_native::TextSpan;

use crate::Action;
use crate::Store;
use crate::theme;

pub mod palette;
pub mod session;

/// The prompt: the agent's colored bar beside a panel holding the field and
/// the agent and model. The panel ends half a row down, so the terminal
/// closes it with a row of `▀` and the bar with `╹`. The field holds focus
/// whenever the palette is closed.
pub fn prompt(
	store: &Store,
	placeholder: &str,
	submit: impl Fn() + Send + Sync + 'static,
) -> Element {
	let draft = store.clone();
	let submitted = store.clone();

	store.with(|state| {
		let model = &state.model;

		view! {
			<View class=format!("border-l-2 border-[{}] pl-1.5", state.agent.hex())>
				<View class="bg-[#1e1e1e] pt-4 pb-2 px-4">
					<TextInput
						id="prompt"
						class="bg-[#1e1e1e] text-[#eeeeee]"
						placeholder=placeholder
						value=state.draft.clone()
						aria_label="Prompt"
						focused=state.palette.is_none()
						on:change=move |text| draft.apply(Action::Draft(text.to_owned()))
						on=(NativeEventSlot::Submit, move |event| {
							if let NativeEvent::Submit { value } = event {
								submitted.apply(Action::Draft(value.clone()));
								submit();
							}
							EventOutcome::Handled
						})
					/>
					<Text
						class="mt-4"
						spans=[
							TextSpan::new(state.agent.name()).color(state.agent.color()),
							TextSpan::new(" "),
							TextSpan::new("·").color(theme::MUTED),
							TextSpan::new(" "),
							TextSpan::new(model.name.clone()).color(theme::TEXT),
							TextSpan::new(" "),
							TextSpan::new(model.provider.clone()).color(theme::MUTED),
						]
					/>
				</View>
			</View>
		}
	})
}

/// Key hints: each key bright, its action muted, two blanks apart.
pub fn hints(pairs: &[(&str, &str)]) -> Element {
	let mut spans = Vec::new();

	for (index, (key, action)) in pairs.iter().enumerate() {
		if index > 0 {
			spans.push(TextSpan::new("  "));
		}
		spans.push(TextSpan::new(format!("{key} ")).color(theme::TEXT));
		spans.push(TextSpan::new(*action).color(theme::MUTED));
	}

	view! { <Text spans=spans /> }
}
