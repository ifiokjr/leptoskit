//! Application state shared by every destination.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use leptoskit::native::ScrollOffset;

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
	/// The Tailwind background utility for the beacon in this phase.
	pub fn beacon_class(self) -> &'static str {
		match self {
			Self::Idle => "bg-[#000000]",
			Self::Running => "bg-[#ff0000]",
			Self::Done => "bg-[#00ff00]",
			Self::Failed => "bg-[#0000ff]",
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
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Screen {
	/// The scenario is not known yet.
	#[default]
	Starting,
	/// No scenario was given: one button per scenario.
	Home,
	/// `cold`: twenty text rows.
	Ready,
	/// `mount`: the cards in a non-virtualised scroll view.
	Cards(Vec<Card>),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
	/// The card's 1-based number.
	pub number: usize,
	/// Whether the update step marked its title.
	pub marked: bool,
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
#[derive(Debug, Default)]
pub struct Model {
	pub scenario: Option<Scenario>,
	pub phase: Phase,
	pub screen: Screen,
}

/// Shared state; each destination decides how a change reaches the screen.
#[derive(Clone, Default)]
pub struct Arena {
	model: Arc<Mutex<Model>>,
	on_change: Option<Arc<dyn Fn() + Send + Sync>>,
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

		if let Some(notify) = &self.on_change {
			notify();
		}
	}
}
