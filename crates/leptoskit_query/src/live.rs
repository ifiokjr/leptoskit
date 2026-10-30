//! Refetching what changed in the database, as `leptoskit_db` live changes
//! say: tag each query with the tables and rows it reads, and apply each
//! [`Invalidation`] the server sends.
//!
//! ```
//! use leptoskit_db::live::ChangedRows;
//! use leptoskit_db::live::Invalidation;
//! use leptoskit_query::QueryClient;
//! use leptoskit_query::QueryScope;
//! use leptoskit_query::live::row_tag;
//! use leptoskit_query::live::table_tag;
//!
//! # #[derive(Clone, Debug)] struct Post;
//! # #[derive(Clone, Debug)] struct ApiError;
//! # async fn fetch_posts() -> Result<Vec<Post>, ApiError> { Ok(Vec::new()) }
//! # async fn fetch_post(id: String) -> Result<Post, ApiError> { let _ = id; Ok(Post) }
//! // Any change to a post refetches the list…
//! fn posts() -> QueryScope<(), Vec<Post>, ApiError> {
//! 	QueryScope::new(|()| fetch_posts()).with_tags_for(|()| vec![table_tag("posts")])
//! }
//!
//! // …and a change to one post refetches that post only.
//! fn post() -> QueryScope<String, Post, ApiError> {
//! 	QueryScope::new(fetch_post).with_tags_for(|id| vec![row_tag("posts", [id])])
//! }
//!
//! # fn receive(client: &QueryClient, invalidation: Invalidation) {
//! // With each invalidation the server sends:
//! let _ = client.apply_invalidation(&invalidation);
//! # }
//! ```

pub use leptoskit_db::live::ChangedRows;
pub use leptoskit_db::live::Invalidation;

use crate::QueryClient;
use crate::QueryFilter;
use crate::Refetches;
use crate::Tag;

/// The first segment of every database tag.
pub const DB_TAG: &str = "db";

/// The tag of a query that reads many rows of `table`, such as a list or a
/// count: any change to the table refetches it.
pub fn table_tag(table: &str) -> Tag {
	Tag::new([DB_TAG, table])
}

/// The tag of a query about one row of `table`, by its key: a change to
/// that row refetches it. An `Id`'s key is its hyphenated UUID, as it
/// displays.
pub fn row_tag(table: &str, key: impl IntoIterator<Item = impl ToString>) -> Tag {
	let mut tag = table_tag(table);

	for part in key {
		tag = tag.child(part.to_string());
	}

	tag
}

impl QueryClient {
	/// Marks stale, and refetches if observed, the queries a live change
	/// touches: those tagged with a changed row's [`row_tag`], or with its
	/// table's [`table_tag`]. [`Invalidation::Everything`] touches every
	/// query with a database tag.
	pub fn apply_invalidation(&self, invalidation: &Invalidation) -> Refetches {
		match invalidation {
			Invalidation::Everything => self.invalidate_tag(Tag::new([DB_TAG])),
			Invalidation::Rows(changed) => {
				let tables: Vec<Tag> = changed.iter().map(|rows| table_tag(&rows.table)).collect();
				let rows: Vec<Tag> = changed
					.iter()
					.flat_map(|rows| rows.keys.iter().map(|key| row_tag(&rows.table, key)))
					.collect();

				self.invalidate_queries(QueryFilter::tags_where(move |tags| {
					tags.iter().any(|tag| {
						tables.contains(tag) || rows.iter().any(|row| tag.starts_with(row))
					})
				}))
			}
		}
	}
}
