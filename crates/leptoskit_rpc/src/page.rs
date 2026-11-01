//! Pages of a live query whose boundaries stay put while rows move.
//!
//! A chat's history, an order queue, a feed: too long to load at once, and
//! changing while someone scrolls. A paginated query takes a [`PageArgs`]
//! argument and returns a [`Page`]:
//!
//! ```ignore
//! #[rpc::query]
//! pub async fn history(cx: &rpc::QueryCx, channel: Id<Channel>, page: rpc::PageArgs) -> rpc::Result<rpc::Page<Message>> {
//! 	let query = Message::query().filter(|m| m.channel_id.eq(channel));
//! 	rpc::paginate(cx.db(), query, &page, |m| m.id.desc(), |m| m.id).await
//! }
//!
//! // Any client: every loaded page stays live.
//! let history = rpc::use_live_paginated(move |page| History { channel: channel.get(), page }, 50);
//! history.load_more();
//! ```
//!
//! Each page is an ordinary live query, subscribed with its own arguments,
//! so its read set covers exactly its own range. The last page reads up to
//! `limit` rows after the page before it. Once a later page is loaded, the
//! one before it is pinned at the row it ended on (`until`), as Convex's
//! paginated queries are: a row inserted into it makes it longer rather
//! than pushing a row onto the next page, and a deleted row makes it
//! shorter rather than pulling one back. So a row is on exactly one page
//! however the data moves.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::Operation;

/// Which rows one page of a paginated query reads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct PageArgs {
	/// Where the page before ended: this page starts after it. `None` for
	/// the first page.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub after: Option<String>,
	/// Where this page ends, once a later page was loaded: it reads every
	/// row up to and including it, however many there are. `None` for the
	/// last page, which reads `limit` rows.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub until: Option<String>,
	/// How many rows the last page reads.
	pub limit: u32,
}

impl PageArgs {
	/// The first page, of `limit` rows.
	#[must_use]
	pub fn first(limit: u32) -> Self {
		Self {
			after: None,
			until: None,
			limit,
		}
	}

	/// The page after one that ended at `end`, of `limit` rows.
	#[must_use]
	pub fn after(end: Option<String>, limit: u32) -> Self {
		Self {
			after: end,
			until: None,
			limit,
		}
	}

	/// The same page, pinned to end at `end`.
	#[must_use]
	pub fn pinned(mut self, end: Option<String>) -> Self {
		self.until = end;
		self
	}

	/// Whether this page ends at a fixed row.
	#[must_use]
	pub fn is_pinned(&self) -> bool {
		self.until.is_some()
	}
}

/// One page of a paginated query's rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Page<T> {
	/// The page's rows, in the query's order.
	pub items: Vec<T>,
	/// Where the page ended: the next page starts after it. `None` when
	/// the first page is empty.
	pub end: Option<String>,
	/// Whether rows follow this page.
	pub has_more: bool,
}

impl<T> Default for Page<T> {
	fn default() -> Self {
		Self {
			items: Vec::new(),
			end: None,
			has_more: false,
		}
	}
}

/// A query that reads a page at a time: the arguments carry a
/// [`PageArgs`], and the result is a [`Page`]. `#[rpc::query]` implements
/// it for a query with a `PageArgs` argument that returns `rpc::Page<T>`.
pub trait Paginated: Operation<Output = Page<Self::Item>> {
	/// What each row is.
	type Item: Serialize + serde::de::DeserializeOwned + Clone + Send + Sync + 'static;

	/// The same query, reading `page` instead.
	#[must_use]
	fn with_page(self, page: PageArgs) -> Self;
}

/// Reads one page of `query`, ordered by `order`, which must be a unique
/// column such as the row's id, with `key` reading that column from a row.
///
/// ```ignore
/// rpc::paginate(cx.db(), Message::query().filter(|m| m.channel_id.eq(channel)), &page, |m| m.id.desc(), |m| m.id).await
/// ```
///
/// Leave `query` unordered: `order` is the page's order, and a row's place
/// in it is its cursor. The last page reads `limit` rows and one more, to
/// tell whether more follow; a pinned page reads everything up to its end,
/// and says more follow.
///
/// # Errors
///
/// A cursor that isn't one this column's values make, and whatever the
/// read fails with.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub async fn paginate<T, V>(
	db: &impl leptoskit_db::Read,
	query: leptoskit_db::Query<T>,
	page: &PageArgs,
	order: impl FnOnce(&T::Columns) -> leptoskit_db::Order,
	key: impl Fn(&T) -> V,
) -> crate::Result<Page<T>>
where
	T: leptoskit_db::Table,
	V: leptoskit_db::Column + Serialize + serde::de::DeserializeOwned,
{
	let order = order(T::columns());
	let column = leptoskit_db::Col::<T, V>::new(order.column());
	let descending = order.is_descending();
	let cursor = |text: &str| {
		serde_json::from_str::<V>(text)
			.map_err(|_| crate::Error::invalid(format!("`{text}` isn't a cursor of this query")))
	};
	let mut query = query.order_by(|_| order);

	if let Some(after) = page.after.as_deref() {
		let after = cursor(after)?;
		query = query.filter(|_| {
			if descending {
				column.lt(after)
			} else {
				column.gt(after)
			}
		});
	}

	if let Some(until) = page.until.as_deref() {
		let until_value = cursor(until)?;
		query = query.filter(|_| {
			if descending {
				column.gte(until_value)
			} else {
				column.lte(until_value)
			}
		});
		let items = query.all(db).await?;
		return Ok(Page {
			items,
			end: Some(until.to_owned()),
			has_more: true,
		});
	}

	let limit = usize::try_from(page.limit.max(1)).unwrap_or(usize::MAX);
	let mut items = query
		.limit(u64::from(page.limit.max(1)) + 1)
		.all(db)
		.await?;
	let has_more = items.len() > limit;
	items.truncate(limit);
	let end = match items.last() {
		Some(row) => {
			Some(
				serde_json::to_string(&key(row))
					.map_err(|_| crate::Error::server("a page's cursor didn't serialize"))?,
			)
		}
		None => page.after.clone(),
	};

	Ok(Page {
		items,
		end,
		has_more,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn pages_follow_and_pin() {
		let first = PageArgs::first(20);
		assert!(!first.is_pinned());
		let pinned = first.clone().pinned(Some("\"b\"".into()));
		assert_eq!(pinned.until.as_deref(), Some("\"b\""));
		assert!(pinned.is_pinned());
		let next = PageArgs::after(pinned.until.clone(), 20);
		assert_eq!(next.after.as_deref(), Some("\"b\""));
		assert_eq!(
			serde_json::to_value(&first).unwrap(),
			serde_json::json!({ "limit": 20 })
		);
		assert_eq!(Page::<u8>::default().items, Vec::<u8>::new());
	}
}
