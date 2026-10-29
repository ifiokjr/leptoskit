//! What the postcard says about the trip: reactive values the controls
//! change and every message reads.

use reactive_graph::signal::RwSignal;
use reactive_graph::traits::Update;

/// Where the postcard is sent from.
pub const PLACE: &str = "Lisbon";

/// The sky, as a select over `sunny`, `rainy`, and anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sky {
	/// Sun every day.
	Sunny,
	/// Rain every afternoon.
	Rainy,
	/// Neither: the messages' `other` case.
	Changeable,
}

impl Sky {
	/// The word the `weather` message selects on.
	pub fn key(self) -> &'static str {
		match self {
			Self::Sunny => "sunny",
			Self::Rainy => "rainy",
			Self::Changeable => "changeable",
		}
	}

	/// The next sky, round and round.
	#[must_use]
	pub fn next(self) -> Self {
		match self {
			Self::Sunny => Self::Rainy,
			Self::Rainy => Self::Changeable,
			Self::Changeable => Self::Sunny,
		}
	}
}

/// Who signs the postcard, as a select over `female`, `male`, and anyone
/// else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Traveler {
	/// She.
	Female,
	/// He.
	Male,
	/// They: the messages' `other` case.
	Other,
}

impl Traveler {
	/// The word the `signOff` message selects on.
	pub fn key(self) -> &'static str {
		match self {
			Self::Female => "female",
			Self::Male => "male",
			Self::Other => "other",
		}
	}

	/// The next traveler, round and round.
	#[must_use]
	pub fn next(self) -> Self {
		match self {
			Self::Female => Self::Male,
			Self::Male => Self::Other,
			Self::Other => Self::Female,
		}
	}
}

/// The trip's state. Every field is a signal, so the struct is `Copy` and a
/// component that reads a field inside a closure renders again when it
/// changes.
#[derive(Debug, Clone, Copy)]
pub struct Trip {
	/// Days since leaving home.
	pub days: RwSignal<i64>,
	/// Photos of the sea taken so far.
	pub photos: RwSignal<i64>,
	/// Letters waiting at home.
	pub letters: RwSignal<i64>,
	/// The weather.
	pub sky: RwSignal<Sky>,
	/// Who signs.
	pub traveler: RwSignal<Traveler>,
}

impl Trip {
	/// A trip three days in, under the sun. Call inside a reactive owner.
	pub fn new() -> Self {
		Self {
			days: RwSignal::new(3),
			photos: RwSignal::new(22),
			letters: RwSignal::new(2),
			sky: RwSignal::new(Sky::Sunny),
			traveler: RwSignal::new(Traveler::Other),
		}
	}

	/// Moves the weather on to the next sky.
	pub fn change_sky(&self) {
		self.sky.update(|sky| *sky = sky.next());
	}

	/// Hands the pen to the next traveler.
	pub fn change_traveler(&self) {
		self.traveler.update(|traveler| *traveler = traveler.next());
	}
}

impl Default for Trip {
	fn default() -> Self {
		Self::new()
	}
}
