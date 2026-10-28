//! The screens, built from shared components and Tailwind classes.
//!
//! The terminal lays these out on its 8×16 cell grid and the browser lays
//! them out with CSS, so every size here is a whole or half cell: `px-4` is
//! two columns, `py-4` one row, and `pb-2` the half row opencode draws with
//! `▀` under its prompt.

use leptoskit_native::Element;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::TextSpan;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;

use crate::Action;
use crate::Dispatch;
use crate::Opencode;
use crate::Screen;
use crate::VERSION;
use crate::theme;

mod palette;
mod session;

pub use palette::matching_commands;

/// The prompt field's element id, which keeps keyboard focus.
pub const PROMPT_ID: &str = "prompt";

/// The palette search field's element id.
pub const PALETTE_SEARCH_ID: &str = "palette-search";

/// The placeholder suggestion in an empty prompt.
const SUGGESTION: &str = "Ask anything… \"Fix a TODO in the codebase\"";

/// The prompt's width on the home screen: 75 cells.
const HOME_PROMPT_WIDTH: &str = "w-[600px]";

/// [`HOME_PROMPT_WIDTH`] in cells.
const HOME_PROMPT_COLUMNS: usize = 75;

/// The cells the home footer's key hints take.
const HOME_HINTS_WIDTH: usize = "shift+tab agents  ctrl+p commands".len();

pub fn render(state: &Opencode, dispatch: &Dispatch) -> Element {
	let screen = match state.screen {
		Screen::Home => home(state, dispatch),
		Screen::Session => session::session(state, dispatch),
	};

	match &state.palette {
		Some(palette) => screen.child(palette::palette(state, palette, dispatch)),
		None => screen,
	}
}

/// The logo above a centered prompt, with the version in the corner.
fn home(state: &Opencode, dispatch: &Dispatch) -> Element {
	let width = usize::from(state.size.0).min(HOME_PROMPT_COLUMNS);

	View()
		.class("w-full h-full bg-[#0a0a0a] text-[#eeeeee] items-center justify-center")
		.child(crate::logo::logo())
		.child(
			View()
				.class(format!("mt-8 {HOME_PROMPT_WIDTH} max-w-full"))
				.child(prompt(state, dispatch)),
		)
		.child(
			View()
				.class(format!(
					"mt-2 {HOME_PROMPT_WIDTH} max-w-full flex-row justify-between"
				))
				.child(
					Text(fit_path(
						&state.directory,
						width.saturating_sub(HOME_HINTS_WIDTH + 2),
					))
					.class("text-[#808080]"),
				)
				.child(hints(&[("shift+tab", "agents"), ("ctrl+p", "commands")])),
		)
		.child(Text(VERSION).class("absolute bottom-4 right-4 text-[#808080]"))
}

/// The prompt: the agent's colored bar beside a panel holding the field and
/// the agent and model. The panel ends half a row down, so the terminal
/// closes it with a row of `▀` and the bar with `╹`.
fn prompt(state: &Opencode, dispatch: &Dispatch) -> Element {
	let draft = dispatch.clone();
	let submit = dispatch.clone();
	let model = &state.model;

	View()
		.class(format!("border-l-2 border-[{}] pl-1.5", state.agent.hex()))
		.child(
			View()
				.class("bg-[#1e1e1e] pt-4 pb-2 px-4")
				.child(
					TextInput()
						.id(PROMPT_ID)
						.class("bg-[#1e1e1e] text-[#eeeeee]")
						.placeholder(if state.screen == Screen::Home {
							SUGGESTION
						} else {
							""
						})
						.value(state.draft.clone())
						.aria_label("Prompt")
						.on_change(move |text| draft(Action::Draft(text.to_owned())))
						.on(NativeEventSlot::Submit, move |event| {
							if let NativeEvent::Submit { value } = event {
								submit(Action::Draft(value.clone()));
								submit(Action::Submit);
							}
							leptoskit_native::EventOutcome::Handled
						}),
				)
				.child(Text("").class("mt-4").spans([
					TextSpan::new(state.agent.name()).color(state.agent.color()),
					TextSpan::new(" "),
					TextSpan::new("·").color(theme::MUTED),
					TextSpan::new(" "),
					TextSpan::new(model.name.clone()).color(theme::TEXT),
					TextSpan::new(" "),
					TextSpan::new(model.provider.clone()).color(theme::MUTED),
				])),
		)
}

/// Key hints: each key bright, its action muted, two blanks apart.
fn hints(pairs: &[(&str, &str)]) -> Element {
	let mut spans = Vec::new();

	for (index, (key, action)) in pairs.iter().enumerate() {
		if index > 0 {
			spans.push(TextSpan::new("  "));
		}
		spans.push(TextSpan::new(format!("{key} ")).color(theme::TEXT));
		spans.push(TextSpan::new(*action).color(theme::MUTED));
	}

	Text("").spans(spans)
}

/// Shortens a path to `width` cells the way opencode does: whole trailing
/// segments after `/…/`, with the segment before them cut short by `…` to
/// fill what is left.
#[must_use]
pub fn fit_path(path: &str, width: usize) -> String {
	const ELLIPSIS: &str = "/…/";
	let cells = |text: &str| text.chars().count();

	if cells(path) <= width {
		return path.to_owned();
	}

	let segments: Vec<&str> = path
		.split('/')
		.filter(|segment| !segment.is_empty())
		.collect();
	let mut kept = segments.len();

	while kept > 0 && cells(ELLIPSIS) + cells(&segments[kept - 1..].join("/")) <= width {
		kept -= 1;
	}

	let suffix = segments[kept..].join("/");
	let Some(cut) = kept.checked_sub(1).map(|index| segments[index]) else {
		return path.to_owned();
	};
	let room = width.saturating_sub(cells(ELLIPSIS) + cells(&suffix) + 1);

	if suffix.is_empty() || room < 2 {
		return format!("{ELLIPSIS}{suffix}");
	}

	let head: String = cut.chars().take(room - 1).collect();
	format!("{ELLIPSIS}{head}…/{suffix}")
}

#[cfg(test)]
mod tests {
	use super::fit_path;

	const PATH: &str = "/tmp/runner/-Users-demo-Developer-projects-clockwork-worktrees-fix-timer/\
	                    419cee41-3e1d-4d17-b4ad-2b1c572c77a1/scratchpad/ref/clockwork:main";

	#[test]
	fn keeps_paths_that_fit() {
		assert_eq!(
			fit_path("~/code/clockwork:main", 40),
			"~/code/clockwork:main"
		);
	}

	#[test]
	fn elides_leading_segments_like_opencode() {
		assert_eq!(
			fit_path(PATH, 40),
			"/…/419cee…/scratchpad/ref/clockwork:main"
		);
		assert_eq!(fit_path(PATH, 38), "/…/419c…/scratchpad/ref/clockwork:main");
		assert_eq!(
			fit_path(PATH, 84),
			"/…/-Users-demo-D…/419cee41-3e1d-4d17-b4ad-2b1c572c77a1/scratchpad/ref/clockwork:main"
		);
	}
}
