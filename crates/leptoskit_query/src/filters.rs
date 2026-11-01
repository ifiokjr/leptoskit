//! Selecting queries for bulk operations.
//!
//! A [`QueryFilter`] is TanStack's `QueryFilters` object: by scope, by exact
//! key, by [`Tag`] prefix, by whether anyone observes the query, by
//! staleness, by fetch status, or by an arbitrary predicate over typed keys.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use crate::key::ScopeId;
use crate::key::Tag;
use crate::key::hash_key;
use crate::query::AnyQuery;
use crate::query::Query;
use crate::scope::ScopeLike;
use crate::state::FetchStatus;

/// Whether a query currently has an enabled observer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryKind {
	/// Observed or not.
	#[default]
	All,
	/// Has at least one enabled observer.
	Active,
	/// Has no enabled observer.
	Inactive,
}

/// Which invalidated queries are refetched right away.
///
/// The rest are only marked stale and refetch when next observed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RefetchType {
	/// Queries with an enabled observer; the default.
	#[default]
	Active,
	/// Queries without one.
	Inactive,
	/// Every matching query.
	All,
	/// None: only mark stale.
	None,
}

type Predicate = Arc<dyn Fn(&Arc<dyn AnyQuery>) -> bool + Send + Sync>;

/// Selects queries for [`QueryClient`](crate::QueryClient) bulk operations.
///
/// ```
/// use leptoskit_query::QueryFilter;
/// use leptoskit_query::QueryScope;
///
/// # #[derive(Clone, Debug)] struct Todo;
/// # #[derive(Clone, Debug)] struct ApiError;
/// # async fn fetch_todo(id: u32) -> Result<Todo, ApiError> { let _ = id; Ok(Todo) }
/// # fn todo() -> QueryScope<u32, Todo, ApiError> { QueryScope::new(|id| async move { fetch_todo(id).await }) }
/// // Every todo query.
/// let _ = QueryFilter::scope(&todo());
/// // One todo.
/// let _ = QueryFilter::key(&todo(), &7);
/// // Odd-numbered todos that are currently stale.
/// let _ = QueryFilter::keys_where(&todo(), |id| id % 2 == 1).stale(true);
/// // Everything tagged under "users".
/// let _ = QueryFilter::tag("users");
/// ```
#[derive(Clone, Default)]
pub struct QueryFilter {
	pub(crate) scope: Option<ScopeId>,
	pub(crate) key_hash: Option<u64>,
	pub(crate) tag: Option<Tag>,
	pub(crate) kind: QueryKind,
	pub(crate) stale: Option<bool>,
	pub(crate) fetch_status: Option<FetchStatus>,
	pub(crate) predicate: Option<Predicate>,
}

impl QueryFilter {
	/// Every query in the cache.
	#[must_use]
	pub fn all() -> Self {
		Self::default()
	}

	/// Every query of one scope.
	#[must_use]
	pub fn scope(scope: &impl ScopeLike) -> Self {
		Self {
			scope: Some(scope.scope_id()),
			..Self::default()
		}
	}

	/// Exactly one query.
	#[must_use]
	pub fn key<S: ScopeLike>(scope: &S, key: &S::Key) -> Self {
		let key = key.clone();
		Self {
			scope: Some(scope.scope_id()),
			key_hash: Some(hash_key(&key)),
			// The hash narrows the search; equality decides, so a key whose
			// hash collides with another's is not caught up in it.
			predicate: Some(Arc::new(move |query: &Arc<dyn AnyQuery>| {
				let query: &dyn Any = &**query;
				query
					.downcast_ref::<Query<S::Key, S::Data, S::Error>>()
					.is_some_and(|query| query.key == key)
			})),
			..Self::default()
		}
	}

	/// The queries of one scope whose keys satisfy `predicate`.
	#[must_use]
	pub fn keys_where<S: ScopeLike>(
		scope: &S,
		predicate: impl Fn(&S::Key) -> bool + Send + Sync + 'static,
	) -> Self {
		Self {
			scope: Some(scope.scope_id()),
			predicate: Some(Arc::new(move |query: &Arc<dyn AnyQuery>| {
				Arc::clone(query)
					.as_any()
					.downcast::<Query<S::Key, S::Data, S::Error>>()
					.is_ok_and(|query| predicate(&query.key))
			})),
			..Self::default()
		}
	}

	/// Exactly `query`, by identity: a query that later replaced it under
	/// the same key is not caught up in it.
	#[cfg(feature = "devtools")]
	pub(crate) fn exactly(query: &Arc<dyn AnyQuery>) -> Self {
		let target = Arc::clone(query);
		Self {
			scope: Some(query.scope_id()),
			key_hash: Some(query.key_hash()),
			predicate: Some(Arc::new(move |candidate: &Arc<dyn AnyQuery>| {
				Arc::ptr_eq(candidate, &target)
			})),
			..Self::default()
		}
	}

	/// Every query tagged at or below `tag`.
	#[must_use]
	pub fn tag(tag: impl Into<Tag>) -> Self {
		Self {
			tag: Some(tag.into()),
			..Self::default()
		}
	}

	/// The queries whose tags satisfy `predicate`, for matches a prefix
	/// can't express, such as a tag that is exactly `["users"]`.
	///
	/// ```
	/// use leptoskit_query::QueryFilter;
	/// use leptoskit_query::Tag;
	///
	/// let users = Tag::new(["users"]);
	/// let _ = QueryFilter::tags_where(move |tags| tags.contains(&users));
	/// ```
	#[must_use]
	pub fn tags_where(predicate: impl Fn(&[Tag]) -> bool + Send + Sync + 'static) -> Self {
		Self {
			predicate: Some(Arc::new(move |query: &Arc<dyn AnyQuery>| {
				predicate(query.tags())
			})),
			..Self::default()
		}
	}

	/// Narrows to queries with an enabled observer.
	#[must_use]
	pub fn active(self) -> Self {
		self.kind(QueryKind::Active)
	}

	/// Narrows to queries without an enabled observer.
	#[must_use]
	pub fn inactive(self) -> Self {
		self.kind(QueryKind::Inactive)
	}

	/// Narrows by observer status.
	#[must_use]
	pub fn kind(mut self, kind: QueryKind) -> Self {
		self.kind = kind;
		self
	}

	/// Narrows by staleness, judged by each scope's own stale time.
	#[must_use]
	pub fn stale(mut self, stale: bool) -> Self {
		self.stale = Some(stale);
		self
	}

	/// Narrows to queries whose query function is running.
	#[must_use]
	pub fn fetching(self) -> Self {
		self.fetch_status(FetchStatus::Fetching)
	}

	/// Narrows by fetch status.
	#[must_use]
	pub fn fetch_status(mut self, fetch_status: FetchStatus) -> Self {
		self.fetch_status = Some(fetch_status);
		self
	}

	pub(crate) fn matches(&self, query: &Arc<dyn AnyQuery>) -> bool {
		self.scope_matches(query)
			&& self.key_matches(query)
			&& self.tag_matches(query)
			&& self.kind_matches(query)
			&& self.stale.is_none_or(|stale| query.is_stale() == stale)
			&& self
				.fetch_status
				.is_none_or(|status| query.fetch_status() == status)
			&& self
				.predicate
				.as_ref()
				.is_none_or(|predicate| predicate(query))
	}

	fn scope_matches(&self, query: &Arc<dyn AnyQuery>) -> bool {
		self.scope.is_none_or(|scope| query.scope_id() == scope)
	}

	fn key_matches(&self, query: &Arc<dyn AnyQuery>) -> bool {
		self.key_hash.is_none_or(|hash| query.key_hash() == hash)
	}

	fn tag_matches(&self, query: &Arc<dyn AnyQuery>) -> bool {
		self.tag.as_ref().is_none_or(|tag| {
			query
				.tags()
				.iter()
				.any(|candidate| candidate.starts_with(tag))
		})
	}

	fn kind_matches(&self, query: &Arc<dyn AnyQuery>) -> bool {
		match self.kind {
			QueryKind::All => true,
			QueryKind::Active => query.is_active(),
			QueryKind::Inactive => !query.is_active(),
		}
	}
}

impl fmt::Debug for QueryFilter {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("QueryFilter")
			.field("scope", &self.scope)
			.field("key_hash", &self.key_hash)
			.field("tag", &self.tag)
			.field("kind", &self.kind)
			.field("stale", &self.stale)
			.field("fetch_status", &self.fetch_status)
			.field("predicate", &self.predicate.is_some())
			.finish()
	}
}

/// How [`QueryClient::invalidate_queries_with`](crate::QueryClient::invalidate_queries_with)
/// follows up an invalidation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidateOptions {
	/// Which of the invalidated queries refetch right away.
	pub refetch_type: RefetchType,
	/// Whether a refetch already in flight is restarted rather than joined.
	pub cancel_refetch: bool,
}

impl Default for InvalidateOptions {
	fn default() -> Self {
		Self {
			refetch_type: RefetchType::Active,
			cancel_refetch: true,
		}
	}
}

impl InvalidateOptions {
	/// Marks stale without refetching anything.
	pub const MARK_ONLY: Self = Self {
		refetch_type: RefetchType::None,
		cancel_refetch: false,
	};
	/// Refetches every matching query, observed or not.
	pub const REFETCH_ALL: Self = Self {
		refetch_type: RefetchType::All,
		cancel_refetch: true,
	};
}

/// How [`QueryClient::refetch_queries_with`](crate::QueryClient::refetch_queries_with)
/// starts its fetches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefetchOptions {
	/// Whether a refetch already in flight is restarted rather than joined.
	pub cancel_refetch: bool,
}

impl Default for RefetchOptions {
	fn default() -> Self {
		Self {
			cancel_refetch: true,
		}
	}
}
