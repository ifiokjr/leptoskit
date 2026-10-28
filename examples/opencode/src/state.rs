//! The interface's state, and the store every route reads it from.

use std::cell::RefCell;

use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::Set;
use reactive_graph::traits::Update;
use reactive_graph::traits::With;

use crate::demo;
use crate::theme;
use crate::ui::palette;

/// Everything the interface can be asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
	/// The prompt's text changed.
	Draft(String),
	/// The home prompt was submitted: start the scripted session.
	StartSession,
	/// The session prompt was submitted: append the prompt and a reply.
	Reply,
	/// Clear the session for a fresh start.
	NewSession,
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
	/// The interface's size in cells, which sizes the palette's list.
	pub size: (u16, u16),
}

impl Default for Opencode {
	fn default() -> Self {
		Self {
			draft: String::new(),
			agent: Agent::Build,
			model: Model {
				name: "Demo".to_owned(),
				provider: "Local".to_owned(),
				context: 200_000,
			},
			directory: "~/code/clockwork:main".to_owned(),
			session: Session {
				title: String::new(),
				messages: Vec::new(),
				tokens: 0,
			},
			palette: None,
			size: (120, 36),
		}
	}
}

impl Opencode {
	/// Applies one action.
	pub fn apply(&mut self, action: Action) {
		match action {
			Action::Draft(text) => self.draft = text,
			Action::StartSession => {
				if let Some(prompt) = self.take_prompt() {
					self.session = demo::session(&prompt, self.agent);
				}
			}
			Action::Reply => {
				if let Some(prompt) = self.take_prompt() {
					self.session.messages.push(Message::User(prompt));
					self.session.messages.push(demo::offline_reply(self.agent));
				}
			}
			Action::NewSession => {
				self.session = Self::default().session;
				self.draft.clear();
			}
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
					let count = palette::matching_commands(&palette.query).len();
					if count > 0 {
						let next = palette.selected as i64 + i64::from(step);
						palette.selected = next.rem_euclid(count as i64) as usize;
					}
				}
			}
		}
	}

	/// The trimmed prompt, leaving the field empty; `None` when it was blank.
	fn take_prompt(&mut self) -> Option<String> {
		let prompt = self.draft.trim().to_owned();
		self.draft.clear();
		(!prompt.is_empty()).then_some(prompt)
	}
}

thread_local! {
	/// One interface per thread: the browser and the native guest each run
	/// one, and every test thread gets its own.
	static STORE: RefCell<Option<Store>> = const { RefCell::new(None) };
}

/// The reactive store the routes read and the event handlers change. Reading
/// it inside a reactive closure, such as the browser's render closure,
/// subscribes the closure to changes; native hosts render again after every
/// event.
#[derive(Clone, Debug)]
pub struct Store(ArcRwSignal<Opencode>);

impl Store {
	/// This thread's store, created with the default state on first use.
	#[must_use]
	pub fn current() -> Self {
		STORE.with_borrow_mut(|store| {
			store
				.get_or_insert_with(|| Self(ArcRwSignal::new(Opencode::default())))
				.clone()
		})
	}

	/// Reads the state.
	pub fn with<T>(&self, read: impl FnOnce(&Opencode) -> T) -> T {
		self.0.with(read)
	}

	/// Applies one action.
	pub fn apply(&self, action: Action) {
		self.0.update(|state| state.apply(action));
	}

	/// Starts over from the default state, as a fresh launch does.
	pub fn reset(&self) {
		self.0.set(Opencode::default());
	}

	/// Records the interface's size in cells.
	pub fn resize(&self, cols: u16, rows: u16) {
		self.0.update(|state| state.size = (cols, rows));
	}
}
