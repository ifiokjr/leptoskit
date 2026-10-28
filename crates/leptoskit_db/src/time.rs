//! Instants as the database stores them.

/// An instant in UTC. Postgres stores it as `timestamptz`; SQLite as
/// fixed-width RFC 3339 text, so that text order is time order.
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// The current instant, to the millisecond: the precision both databases keep,
/// so a value reads back exactly as it was written.
#[cfg(feature = "server")]
pub fn now() -> Timestamp {
	use chrono::DurationRound;

	let now = chrono::Utc::now();
	now.duration_trunc(chrono::TimeDelta::milliseconds(1))
		.unwrap_or(now)
}
