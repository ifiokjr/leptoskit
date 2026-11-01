//! The command palette: a search field and grouped commands over a scrim
//! that dims the screen behind it.

use leptoskit::prelude::*;

use crate::Action;
use crate::Opencode;
use crate::Palette;
use crate::Store;

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

/// The command the selection rests on, if any match the search.
#[must_use]
pub fn selected_command(state: &Opencode) -> Option<Command> {
	let palette = state.palette.as_ref()?;
	matching_commands(&palette.query)
		.get(palette.selected)
		.copied()
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
#[component]
pub fn PaletteView(state: &Opencode, palette: &Palette, store: &Store) -> Element {
	let rows = state.size.1;
	let visible = usize::from((rows / 2).saturating_sub(6)).max(1);
	let list = rows_for(palette);
	let selected_row = list
		.iter()
		.position(|row| matches!(row, Row::Command { selected: true, .. }))
		.unwrap_or(0);
	let first = (selected_row + 1).saturating_sub(visible);
	let search = store.clone();

	let shown = list.into_iter().skip(first).take(visible).map(RowView);

	view! {
		<View class="absolute inset-0 bg-[#00000096] items-center">
			<View class=format!(
				"mt-[{}px] w-[480px] max-w-full bg-[#141414] pt-4 pb-8 px-2",
				rows / 4 * 16,
			)>
				<View class="flex-row justify-between px-6">
					<Text class="font-bold">"Commands"</Text>
					<Text class="text-[#808080]">"esc"</Text>
				</View>
				<View class="mt-4 px-6">
					<TextInput
						id="palette-search"
						focused=true
						class="bg-[#141414] text-[#eeeeee]"
						placeholder="Search"
						value=palette.query.clone()
						aria_label="Search commands"
						on:change=move |text| search.apply(Action::Search(text.to_owned()))
					/>
				</View>
				<View class=format!("mt-4 h-[{}px]", visible * 16)>
					{shown.collect::<Fragment>()}
				</View>
			</View>
		</View>
	}
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

#[component]
fn RowView(row: Row) -> Element {
	match row {
		Row::Group(title) => {
			view! {
				<View class="px-6">
					<Text class="font-bold text-[#9d7cd8]">{title}</Text>
				</View>
			}
		}
		Row::Gap => view! { <View class="h-4" /> },
		Row::Command {
			command,
			selected: true,
		} => {
			view! {
				<View class="flex-row justify-between px-6 bg-[#fab283] text-[#0a0a0a]">
					<Text class="font-bold">{command.title}</Text>
					<Text>{command.shortcut}</Text>
				</View>
			}
		}
		Row::Command { command, .. } => {
			view! {
				<View class="flex-row justify-between px-6">
					<Text>{command.title}</Text>
					<Text class="text-[#808080]">{command.shortcut}</Text>
				</View>
			}
		}
	}
}
