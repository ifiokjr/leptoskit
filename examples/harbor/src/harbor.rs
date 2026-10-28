//! The harbour behind the server functions: a week of ferry sailings with
//! seats to book, and the harbour master's log, kept in memory.

use std::sync::LazyLock;
use std::time::Duration;

use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// How many days the timetable covers, numbered from 1.
pub const DAYS: u16 = 7;

/// How many log entries one page holds.
pub const LOG_PAGE: u32 = 4;

/// How long the harbour takes to answer, as a request over a network would:
/// long enough that a board can show a booking before the answer arrives.
pub const LATENCY: Duration = Duration::from_millis(50);

/// One ferry departure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Sailing {
	/// Stable across days and bookings.
	pub id: u32,
	/// Where it sails to.
	pub route: String,
	/// The departure time, as `HH:MM`.
	pub departs: String,
	/// Seats still free.
	pub seats_left: u32,
	/// Whether it is on time.
	pub status: SailingStatus,
}

/// Where a sailing stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SailingStatus {
	/// Leaving on time.
	OnTime,
	/// Leaving late.
	Delayed {
		/// How late, in minutes.
		minutes: u32,
	},
	/// Already gone.
	Departed,
}

/// One line of the harbour master's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogEntry {
	/// Its place in the log, oldest first.
	pub id: u32,
	/// What happened.
	pub text: String,
}

/// A page of the log, newest first, and where the next older page starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogPage {
	/// The entries on this page.
	pub entries: Vec<LogEntry>,
	/// The cursor of the next older page, absent once the page holds the
	/// first entry.
	pub next: Option<u32>,
}

/// Why a booking failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookingError {
	/// No sailing has that id.
	Unknown,
	/// Every seat is taken.
	SoldOut,
	/// The ferry has already left.
	Departed,
}

/// The harbour's state.
#[derive(Debug)]
pub struct Harbour {
	sailings: Vec<(u16, Sailing)>,
	log: Vec<LogEntry>,
}

const ROUTES: [(&str, &str, u32); 5] = [
	("Kelp Island", "06:40", 3),
	("Lighthouse Point", "08:15", 24),
	("The Old Quay", "11:30", 40),
	("Seal Rocks", "14:05", 12),
	("Mainland", "18:50", 60),
];

impl Harbour {
	/// The harbour as every run starts: the same week, the same log.
	pub fn new() -> Self {
		let mut sailings = Vec::new();
		for day in 1..=DAYS {
			for (index, &(route, departs, seats)) in ROUTES.iter().enumerate() {
				let id = u32::from(day) * 100 + index as u32;
				let status = match (day, index) {
					(1, 0) => SailingStatus::Departed,
					(_, 3) if day % 2 == 0 => {
						SailingStatus::Delayed {
							minutes: 10 * u32::from(day),
						}
					}
					_ => SailingStatus::OnTime,
				};
				sailings.push((
					day,
					Sailing {
						id,
						route: route.to_owned(),
						departs: departs.to_owned(),
						seats_left: seats.saturating_sub(u32::from(day) % 3),
						status,
					},
				));
			}
		}
		let log = [
			"Harbour opens for the season.",
			"Fog horn tested and loud.",
			"Kelp Island pier repainted.",
			"Seal colony counted: 41.",
			"Mainland ferry back from its refit.",
			"Lighthouse Point bell replaced.",
			"Night tide higher than charted.",
			"A swan nesting on the slipway; give it room.",
			"Ticket office moved beside the café.",
			"Storm shutters down until Thursday.",
		]
		.iter()
		.zip(1..)
		.map(|(text, id)| {
			LogEntry {
				id,
				text: (*text).to_owned(),
			}
		})
		.collect();
		Self { sailings, log }
	}

	/// One day's sailings, earliest first.
	pub fn sailings(&self, day: u16) -> Vec<Sailing> {
		self.sailings
			.iter()
			.filter(|(on, _)| *on == day)
			.map(|(_, sailing)| sailing.clone())
			.collect()
	}

	/// Books one seat on a sailing.
	pub fn book(&mut self, sailing: u32) -> Result<Sailing, BookingError> {
		let (_, found) = self
			.sailings
			.iter_mut()
			.find(|(_, candidate)| candidate.id == sailing)
			.ok_or(BookingError::Unknown)?;
		if found.status == SailingStatus::Departed {
			return Err(BookingError::Departed);
		}
		found.seats_left = found
			.seats_left
			.checked_sub(1)
			.ok_or(BookingError::SoldOut)?;
		Ok(found.clone())
	}

	/// The page of the log ending just before `cursor`, newest first. A
	/// cursor of zero starts from the newest entry.
	pub fn log_page(&self, cursor: u32) -> LogPage {
		let end = if cursor == 0 {
			self.log.len()
		} else {
			(cursor as usize - 1).min(self.log.len())
		};
		let start = end.saturating_sub(LOG_PAGE as usize);
		let entries: Vec<LogEntry> = self.log[start..end].iter().rev().cloned().collect();
		let next = (start > 0).then(|| self.log[start].id);
		LogPage { entries, next }
	}
}

impl Default for Harbour {
	fn default() -> Self {
		Self::new()
	}
}

/// The one harbour the server functions read and write, as a server's
/// database would be.
pub static HARBOUR: LazyLock<Mutex<Harbour>> = LazyLock::new(|| Mutex::new(Harbour::new()));
