//! Query status types and state snapshots.
//!
//! TanStack Query separates two independent questions: *do we have data?*
//! ([`QueryStatus`]) and *is the network busy?* ([`FetchStatus`]). A query
//! can be `Success` + `Fetching` during a background refetch, or `Error` with
//! stale data still on screen after a refetch failed.

use serde::Deserialize;
use serde::Serialize;

/// Whether a query has data, an error, or neither yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueryStatus {
	/// No data and no error yet.
	Pending,
	/// The last attempt failed; stale data may still be present.
	Error,
	/// Data is available.
	Success,
}

/// Whether the query function is running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FetchStatus {
	/// Nothing in flight.
	#[default]
	Idle,
	/// The query function is running, whether first load or background refetch.
	Fetching,
	/// A fetch wants to run but the network mode holds it back while offline.
	Paused,
}

/// Which end of an infinite query a fetch extends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FetchDirection {
	/// Appending the next page.
	Forward,
	/// Prepending the previous page.
	Backward,
}

/// The bookkeeping half of a query's state: timestamps, counters, flags.
///
/// Kept in one signal because its fields change together and are all `Copy`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryMeta {
	/// When data last arrived, in epoch milliseconds.
	pub data_updated_at: Option<u64>,
	/// How many times data has arrived.
	pub data_update_count: u32,
	/// When an error last arrived, in epoch milliseconds.
	pub error_updated_at: Option<u64>,
	/// How many times a fetch has ended in error.
	pub error_update_count: u32,
	/// Consecutive failed attempts in the current or last fetch.
	pub failure_count: u32,
	/// Whether an invalidation marked the data stale regardless of age.
	pub is_invalidated: bool,
	/// The direction of the fetch in flight, for infinite queries.
	pub fetch_direction: Option<FetchDirection>,
	/// The direction whose last fetch failed, for infinite queries.
	pub failed_direction: Option<FetchDirection>,
}

/// A snapshot of one cached query.
#[derive(Clone, Debug)]
pub struct QueryState<T, E> {
	/// The last successfully fetched data.
	pub data: Option<T>,
	/// The error from the last failed fetch, cleared by the next success.
	pub error: Option<E>,
	/// See [`FetchStatus`].
	pub fetch_status: FetchStatus,
	/// The error from the most recent failed attempt, including attempts that
	/// are still being retried.
	pub failure_reason: Option<E>,
	/// Timestamps, counters, and flags.
	pub meta: QueryMeta,
}

impl<T, E> QueryState<T, E> {
	/// See [`QueryStatus`].
	#[must_use]
	pub fn status(&self) -> QueryStatus {
		status_of(self.data.is_some(), self.error.is_some())
	}

	/// No data and no error yet.
	#[must_use]
	pub fn is_pending(&self) -> bool {
		self.status() == QueryStatus::Pending
	}

	/// Data is available.
	#[must_use]
	pub fn is_success(&self) -> bool {
		self.status() == QueryStatus::Success
	}

	/// The last attempt failed.
	#[must_use]
	pub fn is_error(&self) -> bool {
		self.status() == QueryStatus::Error
	}

	/// The query function is running.
	#[must_use]
	pub fn is_fetching(&self) -> bool {
		self.fetch_status == FetchStatus::Fetching
	}

	/// A fetch is held back by the network mode.
	#[must_use]
	pub fn is_paused(&self) -> bool {
		self.fetch_status == FetchStatus::Paused
	}

	/// The first load is in flight: pending *and* fetching. A disabled query
	/// that has never fetched is pending but not loading.
	#[must_use]
	pub fn is_loading(&self) -> bool {
		self.is_pending() && self.is_fetching()
	}

	/// A background refetch is in flight behind existing data or error.
	#[must_use]
	pub fn is_refetching(&self) -> bool {
		self.is_fetching() && !self.is_pending()
	}

	/// The very first fetch failed, so there is no data to fall back on.
	#[must_use]
	pub fn is_loading_error(&self) -> bool {
		self.is_error() && self.meta.data_updated_at.is_none()
	}

	/// A refetch failed while older data is still available.
	#[must_use]
	pub fn is_refetch_error(&self) -> bool {
		self.is_error() && self.meta.data_updated_at.is_some()
	}

	/// At least one fetch has finished, with data or with an error.
	#[must_use]
	pub fn is_fetched(&self) -> bool {
		self.meta.data_update_count > 0 || self.meta.error_update_count > 0
	}

	/// Data as a `Result`, for rendering inside an `ErrorBoundary`.
	///
	/// `None` while pending; `Err` while the latest fetch failed even if stale
	/// data is present.
	#[must_use]
	pub fn as_result(&self) -> Option<Result<&T, &E>> {
		match (&self.error, &self.data) {
			(Some(error), _) => Some(Err(error)),
			(None, Some(data)) => Some(Ok(data)),
			(None, None) => None,
		}
	}
}

pub(crate) fn status_of(has_data: bool, has_error: bool) -> QueryStatus {
	if has_error {
		QueryStatus::Error
	} else if has_data {
		QueryStatus::Success
	} else {
		QueryStatus::Pending
	}
}
