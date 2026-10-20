//! The query client: one cache and every operation on it.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::join_all;
use leptos::prelude::*;
use parking_lot::Mutex;
use parking_lot::RwLock;

use crate::awaitable::Refetches;
use crate::cache::CacheInner;
use crate::cache::CacheNotification;
use crate::filters::InvalidateOptions;
use crate::filters::QueryFilter;
use crate::filters::QueryKind;
use crate::filters::RefetchOptions;
use crate::filters::RefetchType;
use crate::hydration::DehydratedState;
use crate::hydration::is_server;
use crate::key::Tag;
use crate::key::hash_key;
use crate::listeners::Subscription;
use crate::managers::FocusManager;
use crate::managers::OnlineManager;
use crate::managers::install_browser_listeners;
use crate::mutation_cache::MutationCacheInner;
use crate::options::QueryDefaults;
use crate::options::ResolvedQueryOptions;
use crate::query::AnyQuery;
use crate::query::FetchFuture;
use crate::query::FetchOptions;
use crate::query::Query;
use crate::query::QueryInfo;
use crate::scope::QueryData;
use crate::scope::QueryError;
use crate::scope::QueryKey;
use crate::scope::QueryScope;
use crate::scope::ScopeLike;
use crate::state::QueryState;

/// The cache every query and mutation lives in, and the API for working with
/// it outside of components.
///
/// Create one at the root of the app and [`provide`](Self::provide) it;
/// hooks find it through context and anything else reaches it with
/// [`use_query_client`].
///
/// ```
/// use leptos::prelude::*;
/// use leptoskit_query::QueryClient;
///
/// #[component]
/// fn App() -> impl IntoView {
/// 	QueryClient::new().provide();
/// 	// ...
/// }
/// ```
///
/// Cloning is cheap: every clone is the same cache.
#[derive(Clone)]
pub struct QueryClient {
	inner: Arc<ClientInner>,
}

pub(crate) struct ClientInner {
	pub(crate) cache: Arc<CacheInner>,
	pub(crate) mutations: Arc<MutationCacheInner>,
	defaults: RwLock<QueryDefaults>,
	mounted: Mutex<Vec<Subscription>>,
}

impl Default for QueryClient {
	fn default() -> Self {
		Self::new()
	}
}

impl fmt::Debug for QueryClient {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("QueryClient")
			.field("queries", &self.inner.cache.all().len())
			.field("defaults", &*self.inner.defaults.read())
			.finish()
	}
}

// Filters are builder values built at the call site; taking them by value
// reads better than `&QueryFilter::all()`.
#[allow(clippy::needless_pass_by_value)]
impl QueryClient {
	/// An empty cache with TanStack's default options.
	#[must_use]
	pub fn new() -> Self {
		Self {
			inner: Arc::new(ClientInner {
				cache: CacheInner::new(),
				mutations: MutationCacheInner::new(),
				defaults: RwLock::new(QueryDefaults::default()),
				mounted: Mutex::new(Vec::new()),
			}),
		}
	}

	/// Sets the client-wide defaults.
	#[must_use]
	pub fn with_defaults(self, defaults: QueryDefaults) -> Self {
		self.set_defaults(defaults);
		self
	}

	/// The client-wide defaults.
	#[must_use]
	pub fn defaults(&self) -> QueryDefaults {
		*self.inner.defaults.read()
	}

	/// Replaces the client-wide defaults for queries created from now on.
	pub fn set_defaults(&self, defaults: QueryDefaults) {
		*self.inner.defaults.write() = defaults;
	}

	/// Provides the client to the component tree and wires it to window
	/// focus and network events.
	///
	/// ```
	/// # use leptos::prelude::*;
	/// # use leptoskit_query::QueryClient;
	/// #[component]
	/// fn App() -> impl IntoView {
	/// 	QueryClient::new().provide();
	/// 	// ...
	/// }
	/// ```
	pub fn provide(&self) {
		self.mount();
		provide_context(self.clone());
	}

	/// Subscribes the cache to window focus and network status, so queries
	/// refetch on focus and reconnect and paused mutations resume. Idempotent;
	/// [`provide`](Self::provide) calls it.
	pub fn mount(&self) {
		let mut mounted = self.inner.mounted.lock();
		if !mounted.is_empty() {
			return;
		}
		install_browser_listeners();
		let cache = Arc::clone(&self.inner.cache);
		mounted.push(FocusManager::subscribe(move |focused| {
			if focused {
				cache.on_focus();
			}
		}));
		let cache = Arc::clone(&self.inner.cache);
		mounted.push(OnlineManager::subscribe(move |online| {
			if online {
				cache.on_online();
			}
		}));
	}

	/// Undoes [`mount`](Self::mount).
	pub fn unmount(&self) {
		self.inner.mounted.lock().clear();
	}

	pub(crate) fn mutations(&self) -> &Arc<MutationCacheInner> {
		&self.inner.mutations
	}

	// ---- internal lookup ---------------------------------------------------

	/// Returns the query for `key`, creating it with the scope's fetcher.
	pub(crate) fn get_or_create<K, T, E>(
		&self,
		scope: &QueryScope<K, T, E>,
		key: K,
	) -> Arc<Query<K, T, E>>
	where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		let key_hash = hash_key(&key);
		let cache = &self.inner.cache;
		let query = cache.get_or_insert_with(scope.id, key_hash, || {
			let options = ResolvedQueryOptions::resolve(
				&self.inner.defaults.read(),
				&scope.options,
				is_server(),
			);
			Query::new(cache, scope, key, options)
		});
		query
			.as_any()
			.downcast::<Query<K, T, E>>()
			.unwrap_or_else(|_| {
				panic!(
					"query scope {:?} is shared by two scopes with different key, data, or error \
					 types; give them different names",
					scope.name
				)
			})
	}

	/// The query for `key` if it exists.
	pub(crate) fn find<S: ScopeLike>(&self, scope: &S, key: &S::Key) -> Option<ScopedQuery<S>> {
		self.inner
			.cache
			.get(scope.scope_id(), hash_key(key))
			.and_then(|query| query.as_any().downcast().ok())
	}

	fn typed_queries<S: ScopeLike>(&self, filter: &QueryFilter) -> Vec<ScopedQuery<S>> {
		self.inner
			.cache
			.find_all(filter)
			.into_iter()
			.filter_map(|query| query.as_any().downcast().ok())
			.collect()
	}

	// ---- fetching ----------------------------------------------------------

	/// Returns fresh cached data, or fetches and caches it.
	///
	/// Data is fresh when it is younger than the scope's stale time. A fetch
	/// already in flight is joined, never duplicated. If a fetch is cancelled
	/// before the query has any data, the call fetches again: a caller that
	/// asked for data gets data or an error, never a silent gap.
	pub async fn fetch_query<K, T, E>(&self, scope: &QueryScope<K, T, E>, key: K) -> Result<T, E>
	where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		let query = self.get_or_create(scope, key);
		if !query.is_stale_for(query.options.stale_time) {
			if let Some(data) = query.signals.data.get_untracked() {
				return Ok(data);
			}
		}
		query.fetch(FetchOptions::JOIN).await;
		Self::settle(&query).await
	}

	/// Resolves with the query's outcome, fetching only while it has neither
	/// data nor an error.
	///
	/// Callers that need a fresh fetch start one first, then settle; this
	/// handles cancellation (nothing to show, so fetch again) and joins any
	/// fetch already in flight.
	pub(crate) async fn settle<K, T, E>(query: &Arc<Query<K, T, E>>) -> Result<T, E>
	where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		loop {
			if let Some(error) = query.signals.error.get_untracked() {
				return Err(error);
			}
			if let Some(data) = query.signals.data.get_untracked() {
				return Ok(data);
			}
			let fetch = query
				.in_flight()
				.unwrap_or_else(|| query.fetch(FetchOptions::JOIN));
			fetch.await;
		}
	}

	/// Like [`fetch_query`](Self::fetch_query) but discards the outcome:
	/// warm the cache before the data is needed.
	pub async fn prefetch_query<K, T, E>(&self, scope: &QueryScope<K, T, E>, key: K)
	where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		let _ = self.fetch_query(scope, key).await;
	}

	/// Returns cached data however stale, fetching only when there is none.
	pub async fn ensure_query_data<K, T, E>(
		&self,
		scope: &QueryScope<K, T, E>,
		key: K,
	) -> Result<T, E>
	where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		let query = self.get_or_create(scope, key);
		if let Some(data) = query.signals.data.get_untracked() {
			return Ok(data);
		}
		Self::settle(&query).await
	}

	/// Like [`ensure_query_data`](Self::ensure_query_data), but stale cached
	/// data also starts a background refetch.
	pub async fn ensure_query_data_revalidating<K, T, E>(
		&self,
		scope: &QueryScope<K, T, E>,
		key: K,
	) -> Result<T, E>
	where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		let query = self.get_or_create(scope, key);
		if let Some(data) = query.signals.data.get_untracked() {
			if query.is_stale_for(query.options.stale_time) {
				drop(query.fetch(FetchOptions::JOIN));
			}
			return Ok(data);
		}
		Self::settle(&query).await
	}

	// ---- reading -----------------------------------------------------------

	/// The cached data for `key`, without subscribing. For callbacks such as
	/// optimistic updates; views should use [`use_query`](crate::use_query).
	#[must_use]
	pub fn get_query_data<S: ScopeLike>(&self, scope: &S, key: &S::Key) -> Option<S::Data> {
		self.find(scope, key)?.signals.data.get_untracked()
	}

	/// The cached state for `key`, without subscribing.
	#[must_use]
	pub fn get_query_state<S: ScopeLike>(
		&self,
		scope: &S,
		key: &S::Key,
	) -> Option<QueryState<S::Data, S::Error>> {
		Some(self.find(scope, key)?.state_untracked())
	}

	/// Every cached key of a scope with its data, without subscribing.
	#[must_use]
	pub fn get_queries_data<S: ScopeLike>(&self, scope: &S) -> Vec<(S::Key, Option<S::Data>)> {
		self.typed_queries::<S>(&QueryFilter::scope(scope))
			.into_iter()
			.map(|query| (query.key.clone(), query.signals.data.get_untracked()))
			.collect()
	}

	/// Whether `key` is cached.
	#[must_use]
	pub fn query_exists<S: ScopeLike>(&self, scope: &S, key: &S::Key) -> bool {
		self.find(scope, key).is_some()
	}

	// ---- writing -----------------------------------------------------------

	/// Writes data into the cache as if a fetch had just succeeded, creating
	/// the query if needed. Every observer sees it immediately.
	pub fn set_query_data<K, T, E>(&self, scope: &QueryScope<K, T, E>, key: K, data: T)
	where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		self.get_or_create(scope, key).set_data(data, None);
	}

	/// Like [`set_query_data`](Self::set_query_data) with an explicit fetch
	/// time, so staleness is judged from when the data was really produced.
	pub fn set_query_data_at<K, T, E>(
		&self,
		scope: &QueryScope<K, T, E>,
		key: K,
		data: T,
		updated_at: u64,
	) where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		self.get_or_create(scope, key)
			.set_data(data, Some(updated_at));
	}

	/// Updates cached data from its current value.
	///
	/// Returning `None` leaves the cache untouched and creates no query, as
	/// an `undefined` updater result does in TanStack. Returns the written
	/// data.
	pub fn update_query_data<K, T, E>(
		&self,
		scope: &QueryScope<K, T, E>,
		key: K,
		updater: impl FnOnce(Option<T>) -> Option<T>,
	) -> Option<T>
	where
		K: QueryKey,
		T: QueryData,
		E: QueryError,
	{
		let current = self.get_query_data(scope, &key);
		let next = updater(current)?;
		self.get_or_create(scope, key).set_data(next.clone(), None);
		Some(next)
	}

	/// Updates every cached query of a scope; `None` skips a query.
	/// Returns how many were written.
	pub fn update_queries_data<S: ScopeLike>(
		&self,
		scope: &S,
		updater: impl Fn(&S::Key, S::Data) -> Option<S::Data>,
	) -> usize {
		let mut written = 0;
		for query in self.typed_queries::<S>(&QueryFilter::scope(scope)) {
			let Some(current) = query.signals.data.get_untracked() else {
				continue;
			};
			if let Some(next) = updater(&query.key, current) {
				query.set_data(next, None);
				written += 1;
			}
		}
		written
	}

	// ---- invalidation ------------------------------------------------------

	/// Marks matching queries stale and refetches the observed ones.
	///
	/// The refetches start immediately; await the returned [`Refetches`] to
	/// wait for them, or drop it to fire and forget.
	pub fn invalidate_queries(&self, filter: QueryFilter) -> Refetches {
		self.invalidate_queries_with(filter, InvalidateOptions::default())
	}

	/// Like [`invalidate_queries`](Self::invalidate_queries) with control over
	/// which queries refetch.
	pub fn invalidate_queries_with(
		&self,
		filter: QueryFilter,
		options: InvalidateOptions,
	) -> Refetches {
		let queries = self.inner.cache.find_all(&filter);
		for query in &queries {
			query.invalidate();
		}
		let kind = match options.refetch_type {
			RefetchType::Active => Some(QueryKind::Active),
			RefetchType::Inactive => Some(QueryKind::Inactive),
			RefetchType::All => Some(QueryKind::All),
			RefetchType::None => None,
		};
		let fetches = kind.map_or_else(Vec::new, |kind| {
			let queries: Vec<_> = queries
				.into_iter()
				.filter(|query| {
					match kind {
						QueryKind::All => true,
						QueryKind::Active => query.is_active(),
						QueryKind::Inactive => !query.is_active(),
					}
				})
				.collect();
			refetch_all(&queries, options.cancel_refetch)
		});
		Refetches::new(Box::pin(join_all(fetches).map(|_| ())))
	}

	/// Invalidates one query.
	pub fn invalidate_query<S: ScopeLike>(&self, scope: &S, key: &S::Key) -> Refetches {
		self.invalidate_queries(QueryFilter::key(scope, key))
	}

	/// Invalidates every query of a scope.
	pub fn invalidate_scope(&self, scope: &impl ScopeLike) -> Refetches {
		self.invalidate_queries(QueryFilter::scope(scope))
	}

	/// Invalidates every query tagged at or below `tag`.
	pub fn invalidate_tag(&self, tag: impl Into<Tag>) -> Refetches {
		self.invalidate_queries(QueryFilter::tag(tag))
	}

	/// Invalidates the whole cache.
	pub fn invalidate_all(&self) -> Refetches {
		self.invalidate_queries(QueryFilter::all())
	}

	/// Refetches matching queries whether or not they are stale, skipping
	/// disabled and static ones.
	pub fn refetch_queries(&self, filter: QueryFilter) -> Refetches {
		self.refetch_queries_with(filter, RefetchOptions::default())
	}

	/// Like [`refetch_queries`](Self::refetch_queries) with control over
	/// fetches already in flight.
	pub fn refetch_queries_with(&self, filter: QueryFilter, options: RefetchOptions) -> Refetches {
		let queries = self.inner.cache.find_all(&filter);
		Refetches::new(Box::pin(
			join_all(refetch_all(&queries, options.cancel_refetch)).map(|_| ()),
		))
	}

	/// Cancels the fetches of matching queries, restoring the state from
	/// before each fetch began.
	pub fn cancel_queries(&self, filter: QueryFilter) {
		for query in self.inner.cache.find_all(&filter) {
			query.cancel();
		}
	}

	/// Drops matching queries from the cache. Observers still mounted rebind
	/// to fresh queries.
	pub fn remove_queries(&self, filter: QueryFilter) {
		for query in self.inner.cache.find_all(&filter) {
			self.inner.cache.remove(query.scope_id(), query.key_hash());
		}
	}

	/// Returns matching queries to their never-fetched state, then refetches
	/// the observed ones.
	pub fn reset_queries(
		&self,
		filter: QueryFilter,
	) -> impl Future<Output = ()> + Send + 'static + use<> {
		let queries = self.inner.cache.find_all(&filter);
		for query in &queries {
			query.reset();
		}
		let active: Vec<_> = queries
			.into_iter()
			.filter(|query| query.is_active())
			.collect();
		join_all(refetch_all(&active, true)).map(|_| ())
	}

	// ---- counters ----------------------------------------------------------

	/// How many queries are fetching, without subscribing.
	#[must_use]
	pub fn fetching_count(&self) -> usize {
		self.inner.cache.fetching.get_untracked()
	}

	/// How many matching queries are fetching, without subscribing.
	#[must_use]
	pub fn fetching_count_matching(&self, filter: QueryFilter) -> usize {
		self.inner.cache.find_all(&filter.fetching()).len()
	}

	/// How many queries are fetching, as a signal.
	/// [`use_is_fetching`](crate::use_is_fetching) wraps this for
	/// components.
	#[must_use]
	pub fn fetching_count_signal(&self) -> ArcReadSignal<usize> {
		self.inner.cache.fetching.read_only()
	}

	// ---- cache -------------------------------------------------------------

	/// Empties both caches.
	pub fn clear(&self) {
		self.inner.cache.clear();
		self.inner.mutations.clear();
	}

	/// A debugging snapshot of every cached query.
	#[must_use]
	pub fn query_infos(&self) -> Vec<QueryInfo> {
		self.inner
			.cache
			.all()
			.iter()
			.map(|query| query.info())
			.collect()
	}

	/// Calls `listener` on every cache event until the subscription drops.
	pub fn subscribe(
		&self,
		listener: impl Fn(&CacheNotification) + Send + Sync + 'static,
	) -> Subscription {
		self.inner.cache.subscribe(listener)
	}

	// ---- hydration ---------------------------------------------------------

	/// Serializes every successful query whose scope has a codec.
	#[must_use]
	pub fn dehydrate(&self) -> DehydratedState {
		self.dehydrate_matching(QueryFilter::all())
	}

	/// Serializes the matching successful queries whose scopes have a codec.
	#[must_use]
	pub fn dehydrate_matching(&self, filter: QueryFilter) -> DehydratedState {
		DehydratedState {
			queries: self
				.inner
				.cache
				.find_all(&filter)
				.iter()
				.filter_map(|query| query.dehydrate())
				.collect(),
		}
	}

	/// Restores dehydrated data, now for queries that exist and as soon as
	/// they are created for the rest. Fresher local data is kept.
	pub fn hydrate(&self, state: DehydratedState) {
		for query in state.queries {
			self.inner.cache.hydrate(query);
		}
	}
}

/// A query looked up through a [`ScopeLike`], so client methods need not name
/// `K`, `T`, and `E` themselves.
pub(crate) type ScopedQuery<S> =
	Arc<Query<<S as ScopeLike>::Key, <S as ScopeLike>::Data, <S as ScopeLike>::Error>>;

/// Starts a refetch on every query that can take one.
fn refetch_all(queries: &[Arc<dyn AnyQuery>], cancel_refetch: bool) -> Vec<FetchFuture> {
	queries
		.iter()
		.filter(|query| !query.is_disabled() && !query.is_static())
		.map(|query| query.refetch(cancel_refetch))
		.collect()
}

/// The [`QueryClient`] provided by an ancestor.
///
/// # Panics
///
/// If no client was provided: call `QueryClient::new().provide()` in a
/// component above this one.
#[must_use]
#[track_caller]
pub fn use_query_client() -> QueryClient {
	use_context::<QueryClient>().expect(
		"no QueryClient in context: call `QueryClient::new().provide()` in an ancestor component",
	)
}

/// Provides a [`QueryClient`] to its children.
///
/// Equivalent to calling [`QueryClient::provide`] at the top of a component;
/// use whichever reads better.
// Component props are always passed by value.
#[allow(clippy::needless_pass_by_value)]
#[component]
pub fn QueryClientProvider(
	/// The client to provide.
	client: QueryClient,
	children: Children,
) -> impl IntoView {
	client.provide();
	children()
}
