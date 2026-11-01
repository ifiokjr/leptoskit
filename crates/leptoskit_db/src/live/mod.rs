//! Live changes: what a write to a `#[db::table(live)]` table changed, sent
//! to whoever may see it, so clients refetch exactly the queries it touches.
//!
//! Invalidate, don't replicate: a change carries no data, only the table and
//! the keys of the rows that changed. A client refetches its queries about
//! them, and the refetch goes through the same policies as ever.
//!
//! ```ignore
//! // On the server, for each signed-in client, say over server-sent events:
//! let mut changes = request_db.changes().await?;
//! loop {
//! 	let invalidation = changes.next().await?;
//! 	send(serde_json::to_string(&invalidation)?).await?;
//! }
//!
//! // On the client, with `leptoskit_query`:
//! client.apply_invalidation(&invalidation);
//! ```
//!
//! A change reaches a subscriber only when the subscriber's `select` policy
//! allows the row as it was before the write or as it is after it, so the
//! existence of rows someone can't see never leaks.
//!
//! Changes come from the change log: capture triggers, which migrations
//! create for every live table, record each written row in the writing
//! transaction, whoever writes it, and each commit that wrote one gets the
//! next version. A feed reads the log after its version, so a subscriber
//! that falls behind, or a server whose connection to the database drops,
//! catches up rather than missing anything; a commit that rolls back never
//! reaches the log. On Postgres a `NOTIFY` at commit wakes every server
//! sharing the database; on SQLite, commits through this process wake it.

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod feed;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod images;
#[cfg(feature = "server")]
mod log;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use feed::ChangeLog;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use feed::Feed;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use feed::Hub;
#[cfg(feature = "postgres")]
pub(crate) use images::image_count;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use images::logged_values;
pub(crate) use images::visible_images;
#[cfg(feature = "server")]
pub use log::Change;
#[cfg(feature = "server")]
pub use log::ChangeOp;
#[cfg(feature = "server")]
pub use log::Commit;
#[cfg(feature = "server")]
pub use log::LogEntry;
#[cfg(feature = "sqlite")]
pub(crate) use log::seal_sqlite;
#[cfg(feature = "server")]
pub(crate) use log::tables;
use serde::Deserialize;
use serde::Serialize;

/// The Postgres channel `live.seal()` notifies with each new version.
pub const CHANNEL: &str = "leptoskit_live";

/// What a client should refetch: a subscriber's next live change.
///
/// It serializes as `{"rows":[{"table":"posts","keys":[["0192…"]]}]}`, or
/// as `"everything"`, so any client can read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Invalidation {
	/// These rows changed: inserted, updated, or deleted.
	Rows(Vec<ChangedRows>),
	/// Changes may have been missed, such as when the change log was pruned
	/// past where the subscriber was: refetch everything read from it.
	Everything,
}

/// The rows of one table a change touched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChangedRows {
	/// The table, with its namespace: `posts` or `billing.invoices`.
	pub table: String,
	/// Each changed row's key: one text per key column, as the column's
	/// value displays, such as an `Id`'s hyphenated UUID.
	pub keys: Vec<Vec<String>>,
}

/// An [`Invalidation`] with the version of the commit it came from: resume
/// a feed after it with [`Db::changes_since`](crate::Db::changes_since), or
/// compare it with a write's version to know a screen has caught up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct VersionedInvalidation {
	/// The commit's place in the version line.
	pub version: i64,
	/// What to refetch.
	pub invalidation: Invalidation,
}
