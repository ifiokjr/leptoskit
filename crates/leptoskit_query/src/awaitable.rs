//! Handles to work that is already running.
//!
//! Invalidation, refetching, and paging start their fetches immediately; the
//! value they return only lets a caller *wait* for the outcome. These types
//! implement [`IntoFuture`] rather than [`Future`](std::future::Future) so
//! that dropping one is not a mistake the compiler warns about:
//!
//! ```
//! # use leptoskit_query::*;
//! # async fn demo(client: QueryClient, todos: QueryScope<(), Vec<String>, String>) {
//! // Fire and forget: the refetches run in the background.
//! client.invalidate_scope(&todos);
//! // Or wait for them to settle.
//! client.invalidate_scope(&todos).await;
//! # }
//! ```

use std::fmt;
use std::future::IntoFuture;

use crate::time::BoxFuture;

/// Refetches started by an invalidation, refetch, or reset. Await it to wait
/// for all of them to settle.
pub struct Refetches {
	settled: BoxFuture<'static, ()>,
}

impl Refetches {
	pub(crate) fn new(settled: BoxFuture<'static, ()>) -> Self {
		Self { settled }
	}
}

impl IntoFuture for Refetches {
	type IntoFuture = BoxFuture<'static, ()>;
	type Output = ();

	fn into_future(self) -> Self::IntoFuture {
		self.settled
	}
}

impl fmt::Debug for Refetches {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("Refetches(..)")
	}
}

/// A refetch of one query, started by
/// [`QueryHandle::refetch`](crate::QueryHandle::refetch). Await it for the
/// outcome: `None` only if the handle had no query.
pub struct Refetch<T, E> {
	outcome: BoxFuture<'static, Option<Result<T, E>>>,
}

impl<T, E> Refetch<T, E> {
	pub(crate) fn new(outcome: BoxFuture<'static, Option<Result<T, E>>>) -> Self {
		Self { outcome }
	}
}

impl<T, E> IntoFuture for Refetch<T, E> {
	type IntoFuture = BoxFuture<'static, Option<Result<T, E>>>;
	type Output = Option<Result<T, E>>;

	fn into_future(self) -> Self::IntoFuture {
		self.outcome
	}
}

impl<T, E> fmt::Debug for Refetch<T, E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("Refetch(..)")
	}
}

/// A page fetch started by an
/// [`InfiniteQueryHandle`](crate::InfiniteQueryHandle). Await it to wait for
/// the page to land.
pub struct PageFetch {
	settled: BoxFuture<'static, ()>,
}

impl PageFetch {
	pub(crate) fn new(settled: BoxFuture<'static, ()>) -> Self {
		Self { settled }
	}
}

impl IntoFuture for PageFetch {
	type IntoFuture = BoxFuture<'static, ()>;
	type Output = ();

	fn into_future(self) -> Self::IntoFuture {
		self.settled
	}
}

impl fmt::Debug for PageFetch {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("PageFetch(..)")
	}
}
