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
//! existence of rows someone can't see never leaks. Changes are published
//! when their transaction commits, and never for one that rolls back. On
//! Postgres they travel through `LISTEN`/`NOTIFY`, so every server sharing
//! the database hears every change; on SQLite they stay in the process.

#[cfg(feature = "server")]
mod capture;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod feed;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod images;
#[cfg(feature = "postgres")]
mod wire;

#[cfg(feature = "server")]
pub(crate) use capture::RowChange;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use feed::Event;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use feed::Feed;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use feed::Hub;
#[cfg(feature = "postgres")]
pub(crate) use images::image_count;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use images::visible_images;
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "postgres")]
pub(crate) use wire::Assembler;
#[cfg(feature = "postgres")]
pub(crate) use wire::encode;

/// The Postgres channel live changes are sent on.
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
	/// Changes may have been missed, such as while the server reconnected to
	/// the database: refetch everything read from it.
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
