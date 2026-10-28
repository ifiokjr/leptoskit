//! An opencode-style coding agent interface, written once as a shared element
//! tree and rendered both in an interactive terminal and in the browser.
//!
//! The screens match opencode 2's terminal interface cell for cell: the
//! two-tone logo, the prompt with its agent bar, a session with markdown,
//! a diff, and shell output, the context sidebar, and the command palette
//! over a dimming scrim. Every model reply is scripted, so the example runs
//! offline.
//!
//! [`Opencode`] holds the state and [`Opencode::render`] describes the
//! interface; the `opencode` binary drives it in a terminal and
//! `opencode-web` in a browser.

use std::sync::Arc;

use leptoskit_native::Element;

mod demo;
mod highlight;
mod logo;
mod markdown;
#[cfg(not(target_arch = "wasm32"))]
pub mod terminal;
pub mod theme;
mod ui;

/// A no-break space: a blank that never wraps or collapses, so indentation
/// and the logo's spacing survive in the terminal and the browser alike.
pub const NBSP: char = '\u{a0}';

/// The version the interface shows in its corner.
pub const VERSION: &str = "2.0.15";

/// Sends an [`Action`] from an event handler to whoever owns the state.
pub type Dispatch = Arc<dyn Fn(Action) + Send + Sync>;

/// Everything the interface can be asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
	/// The prompt's text changed.
	Draft(String),
	/// The prompt was submitted.
	Submit,
	/// Switch to the next agent.
	CycleAgent,
	/// Open the command palette, or close it when it is open.
	TogglePalette,
	/// Close whatever dialog is open.
	Dismiss,
	/// The palette's search text changed.
	Search(String),
	/// Move the palette selection by this many commands.
	MoveSelection(i32),
	/// Run the selected palette command.
	RunSelection,
}

/// Which full screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
	/// The logo and an empty prompt.
	Home,
	/// A conversation.
	Session,
}

/// The agent that answers the next prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
	/// Makes changes.
	Build,
	/// Plans without changing files.
	Plan,
}

impl Agent {
	/// The agent's display name.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Build => "Build",
			Self::Plan => "Plan",
		}
	}

	/// The agent's color, used by the prompt bar and replies.
	#[must_use]
	pub const fn color(self) -> leptoskit_native::Color {
		match self {
			Self::Build => theme::SECONDARY,
			Self::Plan => theme::ACCENT,
		}
	}

	/// The same color as a hex string, for Tailwind classes.
	#[must_use]
	pub const fn hex(self) -> &'static str {
		match self {
			Self::Build => "#5c9cf5",
			Self::Plan => "#9d7cd8",
		}
	}

	const fn next(self) -> Self {
		match self {
			Self::Build => Self::Plan,
			Self::Plan => Self::Build,
		}
	}
}

/// The model answering prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
	/// The model's name.
	pub name: String,
	/// The provider serving it.
	pub provider: String,
	/// The context window, in tokens.
	pub context: u32,
}

/// A conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
	/// The session's title, shown in its tab and the sidebar.
	pub title: String,
	/// The messages, oldest first.
	pub messages: Vec<Message>,
	/// The tokens in the context window so far.
	pub tokens: u32,
}

/// One message in a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
	/// What the user asked.
	User(String),
	/// An agent's turn: its steps in order, then how it ran.
	Assistant {
		/// The text and tool calls, in order.
		parts: Vec<Part>,
		/// The agent that answered.
		agent: Agent,
		/// How long the turn took and how fast it streamed.
		timing: String,
	},
}

/// One step of an agent's turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
	/// Markdown prose.
	Text(String),
	/// Read-only tool calls, collapsed to a summary.
	Explored(String),
	/// A file edit, with the diff it made.
	Edit {
		/// The edited file.
		path: String,
		/// The changed lines and their context.
		lines: Vec<DiffLine>,
	},
	/// A shell command and what it printed.
	Shell {
		/// The command line.
		command: String,
		/// The command's output.
		output: String,
	},
}

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
	/// The line's number in the file.
	pub number: u32,
	/// Whether the line was kept, added, or removed.
	pub change: Change,
	/// The line's code.
	pub code: String,
}

/// How a diff line changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Unchanged context.
	Kept,
	/// Added by the edit.
	Added,
	/// Removed by the edit.
	Removed,
}

/// The command palette's state while it is open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Palette {
	/// The search text.
	pub query: String,
	/// The index of the selected command among those that match.
	pub selected: usize,
}

/// The whole interface's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opencode {
	/// The screen showing.
	pub screen: Screen,
	/// The prompt's current text.
	pub draft: String,
	/// The agent the next prompt goes to.
	pub agent: Agent,
	/// The model answering.
	pub model: Model,
	/// The working directory and branch, as the footer shows them.
	pub directory: String,
	/// The open session.
	pub session: Session,
	/// The command palette, when open.
	pub palette: Option<Palette>,
	/// The interface's size in cells, which sizes the palette and decides
	/// whether the sidebar fits.
	pub size: (u16, u16),
}

impl Opencode {
	/// A fresh interface on the home screen, in a directory.
	#[must_use]
	pub fn new(directory: impl Into<String>) -> Self {
		Self {
			screen: Screen::Home,
			draft: String::new(),
			agent: Agent::Build,
			model: Model {
				name: "Demo".to_owned(),
				provider: "Local".to_owned(),
				context: 200_000,
			},
			directory: directory.into(),
			session: Session {
				title: String::new(),
				messages: Vec::new(),
				tokens: 0,
			},
			palette: None,
			size: (120, 36),
		}
	}

	/// The element id of the widget that should hold keyboard focus.
	#[must_use]
	pub const fn focus_target(&self) -> &'static str {
		if self.palette.is_some() {
			ui::PALETTE_SEARCH_ID
		} else {
			ui::PROMPT_ID
		}
	}

	/// Applies one action.
	pub fn apply(&mut self, action: Action) {
		match action {
			Action::Draft(text) => self.draft = text,
			Action::Submit => self.submit(),
			Action::CycleAgent => self.agent = self.agent.next(),
			Action::TogglePalette => {
				self.palette = match self.palette {
					Some(_) => None,
					None => Some(Palette::default()),
				};
			}
			Action::Dismiss => self.palette = None,
			Action::Search(query) => {
				if let Some(palette) = &mut self.palette {
					palette.query = query;
					palette.selected = 0;
				}
			}
			Action::MoveSelection(step) => {
				if let Some(palette) = &mut self.palette {
					let count = ui::matching_commands(&palette.query).len();
					if count > 0 {
						let next = palette.selected as i64 + i64::from(step);
						palette.selected = next.rem_euclid(count as i64) as usize;
					}
				}
			}
			Action::RunSelection => self.run_selection(),
		}
	}

	fn submit(&mut self) {
		let prompt = self.draft.trim().to_owned();
		if prompt.is_empty() {
			return;
		}
		self.draft.clear();

		match self.screen {
			Screen::Home => {
				self.session = demo::session(&prompt, self.agent);
				self.screen = Screen::Session;
			}
			Screen::Session => {
				self.session.messages.push(Message::User(prompt));
				self.session.messages.push(demo::offline_reply(self.agent));
			}
		}
	}

	fn run_selection(&mut self) {
		let Some(palette) = self.palette.take() else {
			return;
		};
		let commands = ui::matching_commands(&palette.query);

		if let Some(command) = commands.get(palette.selected)
			&& command.title == "New session"
		{
			self.screen = Screen::Home;
			self.draft.clear();
		}
	}

	/// Describes the whole interface for the current state.
	#[must_use]
	pub fn render(&self, dispatch: &Dispatch) -> Element {
		ui::render(self, dispatch)
	}
}

/// A key the interface handles before the focused field sees it. Each host
/// translates its own key events into these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKey {
	/// A character key.
	Char(char),
	/// Escape.
	Escape,
	/// The up arrow.
	Up,
	/// The down arrow.
	Down,
	/// Enter.
	Enter,
	/// Shift and tab.
	BackTab,
}

/// The action a key press triggers before the focused field sees it:
/// ctrl+p toggles the palette, shift+tab switches agents, and while the
/// palette is open the arrows, enter, and escape drive it.
#[must_use]
pub fn shortcut(state: &Opencode, key: ShortcutKey, ctrl: bool) -> Option<Action> {
	let palette = state.palette.is_some();

	match key {
		ShortcutKey::Char('p') if ctrl => Some(Action::TogglePalette),
		ShortcutKey::Escape if palette => Some(Action::Dismiss),
		ShortcutKey::Up if palette => Some(Action::MoveSelection(-1)),
		ShortcutKey::Down if palette => Some(Action::MoveSelection(1)),
		ShortcutKey::Enter if palette => Some(Action::RunSelection),
		ShortcutKey::BackTab if !palette => Some(Action::CycleAgent),
		_ => None,
	}
}
