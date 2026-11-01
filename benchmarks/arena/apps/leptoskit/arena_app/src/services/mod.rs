//! Application state shared by every destination.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use leptoskit::native::ScrollOffset;
use leptoskit::prelude::Color;
use leptoskit::prelude::Get;
use leptoskit::prelude::RwSignal;
use leptoskit::prelude::Set;

/// The scenarios the arena contract defines, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
	Cold,
	Mount,
	Echo,
	Ticker,
	Autoscroll,
	Busy,
}

impl Scenario {
	/// Every scenario, in the order the home screen lists them.
	pub const ALL: [Self; 6] = [
		Self::Cold,
		Self::Mount,
		Self::Echo,
		Self::Ticker,
		Self::Autoscroll,
		Self::Busy,
	];

	/// The scenario's contract id.
	pub fn id(self) -> &'static str {
		match self {
			Self::Cold => "cold",
			Self::Mount => "mount",
			Self::Echo => "echo",
			Self::Ticker => "ticker",
			Self::Autoscroll => "autoscroll",
			Self::Busy => "busy",
		}
	}

	/// Parses a contract id.
	pub fn parse(id: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|scenario| scenario.id() == id)
	}
}

/// The beacon's phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
	#[default]
	Idle,
	Running,
	Done,
	Failed,
}

impl Phase {
	/// The beacon's colour in this phase.
	pub fn colour(self) -> Color {
		match self {
			Self::Idle => Color::rgba(0, 0, 0, 255),
			Self::Running => Color::rgba(255, 0, 0, 255),
			Self::Done => Color::rgba(0, 255, 0, 255),
			Self::Failed => Color::rgba(0, 0, 255, 255),
		}
	}

	/// The phase's name in the status text.
	pub fn label(self) -> &'static str {
		match self {
			Self::Idle => "idle",
			Self::Running => "running",
			Self::Done => "done",
			Self::Failed => "failed",
		}
	}
}

/// Which body the screen shows under the header.
#[derive(Debug, Clone, Default)]
pub enum Screen {
	/// The scenario is not known yet.
	#[default]
	Starting,
	/// No scenario was given: one button per scenario.
	Home,
	/// `cold`: twenty text rows.
	Ready,
	/// `mount`: the cards in a non-virtualised scroll view.
	///
	/// A signal, so creating or clearing the list updates the `For` that
	/// renders it and leaves the rest of the screen alone.
	Cards(RwSignal<Vec<Card>>),
	/// `echo`: a status line, untouched while calls are timed.
	Echo(String),
	/// `ticker`: the 300 tiles, showing `frame + i`.
	Ticker(u64),
	/// `autoscroll` and `busy`: the 2,000-item feed in a virtual list.
	Feed(Feed),
	/// A scenario leptoskit cannot run, with the reason.
	Unsupported(String),
}

/// One `mount` card.
#[derive(Debug, Clone)]
pub struct Card {
	/// The card's 1-based number.
	pub number: usize,
	/// Whether the update step marked its title.
	///
	/// A signal rather than a field: the title text reads it, so marking a
	/// card rewrites that label and leaves the other 999 alone.
	pub marked: RwSignal<bool>,
}

impl Card {
	/// A card whose title is not marked yet.
	pub fn new(number: usize) -> Self {
		Self {
			number,
			marked: RwSignal::new(false),
		}
	}

	/// Whether this card's title is marked.
	pub fn is_marked(&self) -> bool {
		self.marked.get()
	}
}

/// Where the feed is scrolled.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Feed {
	/// The offset the list's window is built around.
	pub offset: f64,
	/// The offset the list last reported.
	pub reported: f64,
	/// Where the scenario scrolls the list, once it does.
	pub target: Option<ScrollOffset>,
}

/// Everything the screen renders from.
#[derive(Debug)]
pub struct Model {
	pub scenario: Option<Scenario>,
	/// The beacon's phase; a signal, so the header follows it without a
	/// rebuild.
	pub phase: RwSignal<Phase>,
	pub screen: Screen,
}

impl Default for Model {
	fn default() -> Self {
		Self {
			scenario: None,
			phase: RwSignal::new(Phase::Idle),
			screen: Screen::default(),
		}
	}
}

/// Shared state; each destination decides how a change reaches the screen.
#[derive(Clone)]
pub struct Arena {
	model: Arc<Mutex<Model>>,
	on_change: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Default for Arena {
	fn default() -> Self {
		Self {
			model: Arc::new(Mutex::new(Model::default())),
			on_change: None,
		}
	}
}

impl Arena {
	/// Registers how the destination renders after a change.
	#[must_use]
	pub fn on_change(mut self, notify: impl Fn() + Send + Sync + 'static) -> Self {
		self.on_change = Some(Arc::new(notify));
		self
	}

	/// Locks the model for a render.
	pub fn read(&self) -> MutexGuard<'_, Model> {
		self.model
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	/// Changes the model, then renders once. The lock is released before the
	/// render reads the model again.
	pub fn update(&self, change: impl FnOnce(&mut Model)) {
		change(&mut self.read());
		self.render();
	}

	/// Renders the model again.
	///
	/// For a change the tree holds structurally — a row added, a screen
	/// replaced — which no binding can express.
	pub fn render(&self) {
		if let Some(notify) = &self.on_change {
			notify();
		}
	}

	/// Changes the beacon's phase.
	///
	/// A phase is a value the header follows, not a screen: the header binds
	/// the beacon's colour and its label, so writing the signal is the whole
	/// change. The native host settles bound values after the turn; the
	/// browser renders again.
	pub fn set_phase(&self, phase: Phase) {
		self.read().phase.set(phase);

		#[cfg(not(feature = "native"))]
		if let Some(notify) = &self.on_change {
			notify();
		}
	}
}
