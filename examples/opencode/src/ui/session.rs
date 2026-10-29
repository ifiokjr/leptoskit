//! The parts of a session: its tab, the conversation pinned to the newest
//! message, the footer, and the context sidebar.

use leptoskit::prelude::*;
use leptoskit_native::TextSpan;

use super::hints;
use crate::Change;
use crate::DiffLine;
use crate::Message;
use crate::Opencode;
use crate::Part;
use crate::highlight;
use crate::markdown;
use crate::theme;

/// The sidebar shows from 121 cells wide, as opencode's does.
const WITH_SIDEBAR: &str = "min-[968px]";

/// Diffs split side by side once the conversation is wider than 120 cells:
/// 167 cells with the sidebar.
const SPLIT_DIFF: &str = "min-[1336px]";

/// The session's tab and the new-tab button.
pub fn tabs(state: &Opencode) -> Element {
	view! {
		<View class="flex-row">
			<View class="bg-[#1e1e1e] pl-6 pr-2">
				<Text class="font-bold">{state.session.title.clone()}</Text>
			</View>
			<View class="px-2">
				<Text class="text-[#808080]">"+"</Text>
			</View>
		</View>
	}
}

/// The messages, pinned to the bottom once they outgrow the space, the way
/// a chat follows its newest message.
pub fn conversation(state: &Opencode) -> Element {
	let mut items = Vec::new();

	for message in &state.session.messages {
		match message {
			Message::User(text) => {
				items.push(view! {
					<View class="bg-[#141414] border-l-2 border-[#5c9cf5] py-4 pl-6">
						<Text>{text.clone()}</Text>
					</View>
				});
			}
			Message::Assistant {
				parts,
				agent,
				timing,
			} => {
				items.extend(parts.iter().flat_map(part));
				items.push(prose(view! {
					<Text spans=[
						TextSpan::new(agent.name()).color(agent.color()),
						TextSpan::new(format!(" · {} · {timing}", state.model.name))
							.color(theme::MUTED),
					] />
				}));
			}
		}
	}

	view! {
		<View class="flex-1 overflow-hidden justify-end">
			<View class="min-h-full shrink-0 pt-4 pb-4 gap-4">{items}</View>
		</View>
	}
}

/// One step of an agent's turn, as one or more blocks.
fn part(part: &Part) -> Vec<Element> {
	match part {
		Part::Text(source) => {
			markdown::blocks(source)
				.into_iter()
				.map(|lines| prose(view! { <View>{lines}</View> }))
				.collect()
		}
		Part::Explored(summary) => {
			vec![prose(
				view! { <Text class="text-[#808080]">{format!("→ {summary}")}</Text> },
			)]
		}
		Part::Edit { path, lines } => {
			vec![tool(
				view! {
					<Text spans=[
						TextSpan::new("← Edit").color(theme::MUTED),
						TextSpan::new(" "),
						TextSpan::new(path.clone()).color(theme::MUTED),
					] />
				},
				diff(lines),
			)]
		}
		Part::Shell { command, output } => {
			vec![tool(
				view! { <Text>{format!("$ {command}")}</Text> },
				view! { <Text class="text-[#808080]">{output.clone()}</Text> },
			)]
		}
	}
}

/// Agent prose, indented past the message bars.
fn prose(content: Element) -> Element {
	view! { <View class="pl-6">{content}</View> }
}

/// A tool call's panel. Its left border is the page color, which reads as a
/// thin notch in the terminal.
fn tool(title: Element, body: Element) -> Element {
	view! {
		<View class="bg-[#141414] border-l-2 border-[#0a0a0a] py-4 pl-6 gap-4">{title} {body}</View>
	}
}

/// A diff: a gutter of line numbers and signs, then highlighted code, with
/// changed lines on red or green. Wide conversations show it split, the old
/// file on the left and the new one on the right, pairing each removed line
/// with the added line that replaced it.
fn diff(lines: &[DiffLine]) -> Element {
	let digits = lines
		.iter()
		.map(|line| line.number.to_string().len())
		.max()
		.unwrap_or(1);

	let unified = lines
		.iter()
		.map(|line| view! { <View class="flex-row">{diff_line(line, digits)}</View> });
	let split = split_rows(lines).into_iter().map(|(old, new)| {
		view! {
			<View class="flex-row">
				<View class="flex-1 flex-row">
					{old
						.into_iter()
						.flat_map(|line| diff_line(line, digits))
						.collect::<Fragment>()}
				</View>
				<View class="flex-1 flex-row">
					{new
						.into_iter()
						.flat_map(|line| diff_line(line, digits))
						.collect::<Fragment>()}
				</View>
			</View>
		}
	});

	view! {
		<View class="ml-2">
			<View class=format!("{SPLIT_DIFF}:hidden")>{unified.collect::<Fragment>()}</View>
			<View class=format!("hidden {SPLIT_DIFF}:flex")>{split.collect::<Fragment>()}</View>
		</View>
	}
}

/// The gutter and the code of one diff line.
fn diff_line(line: &DiffLine, digits: usize) -> [Element; 2] {
	let (gutter, code, sign, sign_color) = match line.change {
		Change::Kept => ("", "", ' ', theme::TEXT),
		Change::Added => ("bg-[#1b2b34]", "bg-[#20303b]", '+', theme::DIFF_ADDED_SIGN),
		Change::Removed => {
			(
				"bg-[#2d1f26]",
				"bg-[#37222c]",
				'-',
				theme::DIFF_REMOVED_SIGN,
			)
		}
	};

	[
		view! {
			<View class=format!("{gutter} shrink-0")>
				<Text spans=[
					TextSpan::new(no_break(&format!(" {:>digits$}", line.number)))
						.color(theme::DIFF_LINE_NUMBER),
					TextSpan::new(no_break(&format!(" {sign} "))).color(sign_color),
				] />
			</View>
		},
		view! {
			<View class=format!("{code} flex-1")>
				<Text spans=highlight::rust(&line.code) />
			</View>
		},
	]
}

/// Side-by-side rows: kept lines on both sides, and each run of removed
/// lines beside the run of added lines that follows it.
fn split_rows(lines: &[DiffLine]) -> Vec<(Option<&DiffLine>, Option<&DiffLine>)> {
	let mut rows = Vec::new();
	let mut index = 0;

	while index < lines.len() {
		if lines[index].change == Change::Kept {
			rows.push((Some(&lines[index]), Some(&lines[index])));
			index += 1;
			continue;
		}

		let removed: Vec<&DiffLine> = lines[index..]
			.iter()
			.take_while(|line| line.change == Change::Removed)
			.collect();
		index += removed.len();
		let added: Vec<&DiffLine> = lines[index..]
			.iter()
			.take_while(|line| line.change == Change::Added)
			.collect();
		index += added.len();

		for row in 0..removed.len().max(added.len()) {
			rows.push((removed.get(row).copied(), added.get(row).copied()));
		}
	}

	rows
}

/// The path on the left; context use and the palette hint on the right.
pub fn footer(state: &Opencode) -> Element {
	view! {
		<View class="mt-2 flex-row justify-between">
			<Text class="text-[#808080]">{state.directory.clone()}</Text>
			<View class="flex-row gap-4">
				<Text class="text-[#808080]">{context_summary(state)}</Text>
				{hints(&[("ctrl+p", "commands")])}
			</View>
		</View>
	}
}

/// `19.3K (10%)`: the tokens in context and the share of the window.
fn context_summary(state: &Opencode) -> String {
	let tokens = f64::from(state.session.tokens);
	format!(
		"{:.1}K ({:.0}%)",
		tokens / 1000.0,
		tokens / f64::from(state.model.context) * 100.0
	)
}

/// The session title, context use, and the getting-started card.
pub fn sidebar(state: &Opencode) -> Element {
	let tokens = state.session.tokens;
	let used = f64::from(tokens) / f64::from(state.model.context) * 100.0;

	view! {
		<View class=format!("hidden {WITH_SIDEBAR}:flex w-[336px] shrink-0 bg-[#141414] pt-4 px-4 pb-4",)>
			<Text class="font-bold">{state.session.title.clone()}</Text>
			<View class="mt-4">
				<Text class="font-bold">"Context"</Text>
				<Text class="text-[#808080]">{format!("{} tokens", thousands(tokens))}</Text>
				<Text class="text-[#808080]">{format!("{used:.0}% used")}</Text>
			</View>
			<View class="grow" />
			{getting_started()}
			<Text class="mt-4 text-[#808080]">{state.directory.clone()}</Text>
		</View>
	}
}

/// The card that points a new user at more models.
fn getting_started() -> Element {
	view! {
		<View class="bg-[#1e1e1e] p-4 gap-4">
			<View class="flex-row justify-between">
				<Text spans=[TextSpan::new("⬖ "), TextSpan::new("Getting started").bold()] />
				<Text class="text-[#808080]">"✕"</Text>
			</View>
			<View class="pl-4 gap-4">
				<Text class="text-[#808080]">
					"OpenCode includes free models so you can start immediately."
				</Text>
				<Text class="text-[#808080]">
					"Connect from 75+ providers to use other models, including Claude, GPT, \
					 Gemini etc"
				</Text>
				<View class="flex-row justify-between">
					<Text>"Connect provider"</Text>
					<Text class="text-[#808080]">"/connect"</Text>
				</View>
			</View>
		</View>
	}
}

/// `19254` as `19,254`.
fn thousands(value: u32) -> String {
	let digits = value.to_string();
	let mut grouped = String::new();

	for (index, digit) in digits.chars().enumerate() {
		if index > 0 && (digits.len() - index) % 3 == 0 {
			grouped.push(',');
		}
		grouped.push(digit);
	}

	grouped
}

/// Spaces as no-break spaces, so a gutter keeps its leading blanks.
fn no_break(text: &str) -> String {
	text.replace(' ', &crate::NBSP.to_string())
}

#[cfg(test)]
mod tests {
	use super::thousands;

	#[test]
	fn groups_thousands() {
		assert_eq!(thousands(7), "7");
		assert_eq!(thousands(19_254), "19,254");
		assert_eq!(thousands(1_200_000), "1,200,000");
	}
}
