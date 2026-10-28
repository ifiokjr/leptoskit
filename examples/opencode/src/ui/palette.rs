//! The command palette: a search field and grouped commands over a scrim
//! that dims the screen behind it.

use leptoskit_native::Element;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;

use super::PALETTE_SEARCH_ID;
use crate::Action;
use crate::Dispatch;
use crate::Opencode;
use crate::Palette;

/// One palette command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
	/// The group it is listed under.
	pub group: &'static str,
	/// What it does.
	pub title: &'static str,
	/// Its keyboard shortcut, if it has one.
	pub shortcut: &'static str,
}

const fn command(group: &'static str, title: &'static str, shortcut: &'static str) -> Command {
	Command {
		group,
		title,
		shortcut,
	}
}

/// Every command, in display order.
const COMMANDS: &[Command] = &[
	command("Suggested", "Switch session", "ctrl+x l"),
	command("Suggested", "New session", "ctrl+x n"),
	command("Suggested", "Switch model", "ctrl+x m"),
	command("Suggested", "Connect an integration", ""),
	command("Suggested", "Open settings", ""),
	command("Suggested", "Share session", ""),
	command("Session", "Switch session", "ctrl+x l"),
	command("Session", "New session", "ctrl+x n"),
	command("Session", "Clear session", ""),
	command("Session", "Open session or project", "ctrl+o"),
	command("Session", "Close tab", "ctrl+x w"),
	command("Session", "Reopen closed tab", "ctrl+shift+t"),
	command("Session", "Share session", ""),
	command("Session", "Rename session", "ctrl+r"),
	command("Session", "Jump to message", "ctrl+x g"),
	command("Session", "Fork session", ""),
	command("Session", "Compact session", "ctrl+x c"),
	command("Session", "Undo previous message", "ctrl+x u"),
	command("Session", "Copy last assistant message", "ctrl+x y"),
	command("Session", "Copy session ID", ""),
	command("Session", "Copy session transcript", ""),
	command("Session", "Export session transcript", "ctrl+x x"),
	command("Session", "Toggle subagent picker", "↓"),
	command("Agent", "Switch agent", "shift+tab"),
	command("Agent", "Switch model variant", "ctrl+t"),
	command("System", "Toggle theme", ""),
	command("System", "Help", ""),
	command("System", "Exit the app", "ctrl+c"),
];

/// The commands whose titles contain the search text, ignoring case.
#[must_use]
pub fn matching_commands(query: &str) -> Vec<Command> {
	let query = query.to_lowercase();
	COMMANDS
		.iter()
		.filter(|command| command.title.to_lowercase().contains(&query))
		.copied()
		.collect()
}

/// One row of the palette's list.
#[derive(Clone, Copy)]
enum Row {
	Group(&'static str),
	Gap,
	Command { command: Command, selected: bool },
}

/// The scrim and the dialog. The dialog sits a quarter of the way down, and
/// its list shows half the screen's rows less its own chrome, scrolled to
/// keep the selection in view.
pub fn palette(state: &Opencode, palette: &Palette, dispatch: &Dispatch) -> Element {
	let rows = state.size.1;
	let visible = usize::from((rows / 2).saturating_sub(6)).max(1);
	let list = rows_for(palette);
	let selected_row = list
		.iter()
		.position(|row| matches!(row, Row::Command { selected: true, .. }))
		.unwrap_or(0);
	let first = (selected_row + 1).saturating_sub(visible);
	let search = dispatch.clone();

	View()
		.class("absolute inset-0 bg-[#00000096] items-center")
		.child(
			View()
				.class(format!(
					"mt-[{}px] w-[480px] max-w-full bg-[#141414] pt-4 pb-8 px-2",
					rows / 4 * 16
				))
				.child(
					View()
						.class("flex-row justify-between px-6")
						.child(Text("Commands").class("font-bold"))
						.child(Text("esc").class("text-[#808080]")),
				)
				.child(
					View().class("mt-4 px-6").child(
						TextInput()
							.id(PALETTE_SEARCH_ID)
							.class("bg-[#141414] text-[#eeeeee]")
							.placeholder("Search")
							.value(palette.query.clone())
							.aria_label("Search commands")
							.on_change(move |text| search(Action::Search(text.to_owned()))),
					),
				)
				.child(
					View()
						.class(format!("mt-4 h-[{}px]", visible * 16))
						.children(list.into_iter().skip(first).take(visible).map(row)),
				),
		)
}

/// The list rows: each group's title, its commands, and a gap between groups.
fn rows_for(palette: &Palette) -> Vec<Row> {
	let mut rows = Vec::new();
	let mut group = "";

	for (index, command) in matching_commands(&palette.query).into_iter().enumerate() {
		if command.group != group {
			if !group.is_empty() {
				rows.push(Row::Gap);
			}
			group = command.group;
			rows.push(Row::Group(group));
		}

		rows.push(Row::Command {
			command,
			selected: index == palette.selected,
		});
	}

	rows
}

fn row(row: Row) -> Element {
	match row {
		Row::Group(title) => {
			View()
				.class("px-6")
				.child(Text(title).class("font-bold text-[#9d7cd8]"))
		}
		Row::Gap => View().class("h-4"),
		Row::Command {
			command,
			selected: true,
		} => {
			View()
				.class("flex-row justify-between px-6 bg-[#fab283] text-[#0a0a0a]")
				.child(Text(command.title).class("font-bold"))
				.child(Text(command.shortcut))
		}
		Row::Command { command, .. } => {
			View()
				.class("flex-row justify-between px-6")
				.child(Text(command.title))
				.child(Text(command.shortcut).class("text-[#808080]"))
		}
	}
}
