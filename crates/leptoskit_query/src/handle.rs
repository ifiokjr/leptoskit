//! Reading queries from components: [`use_query`] and [`QueryHandle`].
//!
//! A handle is TanStack's `useQuery` result, made `Copy` like a leptos signal.
//! Every accessor is a reactive read, so `todos.data()` inside a view closure
//! subscribes that closure to the data — and to nothing else. Awaiting the
//! handle inside a [`Suspend`](leptos::prelude::Suspend) resolves when the
//! first load finishes, which is `useSuspenseQuery`.

use std::fmt;
use std::future::IntoFuture;
use std::sync::Arc;

use leptos::prelude::*;

use crate::awaitable::Refetch;
use crate::client::QueryClient;
use crate::client::use_query_client;
use crate::observer::Access;
use crate::observer::Observer;
use crate::options::QueryOptions;
use crate::options::RefetchInterval;
use crate::options::RefetchOn;
use crate::options::StaleTime;
use crate::query::QueryInfo;
use crate::scope::QueryData;
use crate::scope::QueryError;
use crate::scope::QueryKey;
use crate::scope::QueryScope;
use crate::state::FetchStatus;
use crate::state::QueryMeta;
use crate::state::QueryState;
use crate::state::QueryStatus;
use crate::time::BoxFuture;

/// A key, or `None` to disable the query.
///
/// Implemented for `K` and `Option<K>`, so a keyer may return either. A
/// dependent query is written `move || user.get().map(|user| user.team_id)`
/// and simply does nothing until the user is known.
pub trait IntoMaybeKey<K> {
	/// The key, if any.
	fn into_maybe_key(self) -> Option<K>;
}

impl<K> IntoMaybeKey<K> for K {
	fn into_maybe_key(self) -> Option<K> {
		Some(self)
	}
}

impl<K> IntoMaybeKey<K> for Option<K> {
	fn into_maybe_key(self) -> Option<K> {
		self
	}
}

type PlaceholderFn<T> = Arc<dyn Fn(Option<&T>) -> Option<T> + Send + Sync>;

/// What to show while a query has no data of its own.
#[derive(Clone)]
pub enum Placeholder<T> {
	/// Nothing: `data()` is `None` until the fetch lands.
	None,
	/// The data of the previous key, so paginated lists do not flash empty
	/// between pages. TanStack's `keepPreviousData`.
	KeepPrevious,
	/// A fixed value.
	Value(T),
	/// Computed from the previous key's data.
	Compute(PlaceholderFn<T>),
}

impl<T> fmt::Debug for Placeholder<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::None => f.write_str("None"),
			Self::KeepPrevious => f.write_str("KeepPrevious"),
			Self::Value(_) => f.write_str("Value(..)"),
			Self::Compute(_) => f.write_str("Compute(..)"),
		}
	}
}

/// Per-observer options for [`use_query_with`].
///
/// Anything set here applies to this observer only and overrides the scope.
pub struct QueryObserverOptions<T, E> {
	pub(crate) enabled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
	pub(crate) placeholder: Placeholder<T>,
	pub(crate) initial_data: Option<(T, Option<u64>)>,
	pub(crate) overlay: QueryOptions<E>,
}

impl<T, E> Default for QueryObserverOptions<T, E> {
	fn default() -> Self {
		Self {
			enabled: None,
			placeholder: Placeholder::None,
			initial_data: None,
			overlay: QueryOptions::default(),
		}
	}
}

impl<T, E> fmt::Debug for QueryObserverOptions<T, E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("QueryObserverOptions")
			.field("enabled", &self.enabled.is_some())
			.field("placeholder", &self.placeholder)
			.field("initial_data", &self.initial_data.is_some())
			.field("overlay", &self.overlay)
			.finish()
	}
}

impl<T, E> QueryObserverOptions<T, E> {
	/// Options that change nothing.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Fetches only while `enabled` returns `true`; reactive. A disabled query
	/// still serves cached data and can be refetched by hand.
	#[must_use]
	pub fn with_enabled(mut self, enabled: impl Fn() -> bool + Send + Sync + 'static) -> Self {
		self.enabled = Some(Arc::new(enabled));
		self
	}

	/// Shows the previous key's data while the new key loads.
	#[must_use]
	pub fn keep_previous_data(mut self) -> Self {
		self.placeholder = Placeholder::KeepPrevious;
		self
	}

	/// Shows `data` while the query has none. Unlike initial data it is not
	/// written to the cache.
	#[must_use]
	pub fn with_placeholder_data(mut self, data: T) -> Self {
		self.placeholder = Placeholder::Value(data);
		self
	}

	/// Computes placeholder data from the previous key's data.
	#[must_use]
	pub fn with_placeholder_fn(
		mut self,
		placeholder: impl Fn(Option<&T>) -> Option<T> + Send + Sync + 'static,
	) -> Self {
		self.placeholder = Placeholder::Compute(Arc::new(placeholder));
		self
	}

	/// Seeds the cache when the query has no data yet. Considered fetched
	/// just now, so it is stale under the default stale time and refetches
	/// in the background on mount.
	#[must_use]
	pub fn with_initial_data(mut self, data: T) -> Self {
		self.initial_data = Some((data, None));
		self
	}

	/// Seeds the cache with data fetched at `updated_at` (epoch
	/// milliseconds), so staleness is judged from the real fetch time.
	#[must_use]
	pub fn with_initial_data_at(mut self, data: T, updated_at: u64) -> Self {
		self.initial_data = Some((data, Some(updated_at)));
		self
	}

	/// Overrides scope options for this observer.
	#[must_use]
	pub fn with_options(mut self, options: QueryOptions<E>) -> Self {
		self.overlay = options;
		self
	}

	/// See [`StaleTime`].
	#[must_use]
	pub fn with_stale_time(mut self, stale_time: impl Into<StaleTime>) -> Self {
		self.overlay.stale_time = Some(stale_time.into());
		self
	}

	/// See [`RefetchInterval`].
	#[must_use]
	pub fn with_refetch_interval(mut self, interval: impl Into<RefetchInterval>) -> Self {
		self.overlay.refetch_interval = Some(interval.into());
		self
	}

	/// Refetch policy when this observer mounts.
	#[must_use]
	pub fn with_refetch_on_mount(mut self, refetch: RefetchOn) -> Self {
		self.overlay.refetch_on_mount = Some(refetch);
		self
	}

	/// Refetch policy when the window regains focus.
	#[must_use]
	pub fn with_refetch_on_window_focus(mut self, refetch: RefetchOn) -> Self {
		self.overlay.refetch_on_window_focus = Some(refetch);
		self
	}

	/// Refetch policy when the network reconnects.
	#[must_use]
	pub fn with_refetch_on_reconnect(mut self, refetch: RefetchOn) -> Self {
		self.overlay.refetch_on_reconnect = Some(refetch);
		self
	}
}

/// The observer's view of a query: the cached state plus what only this
/// observer knows.
///
/// Derefs to [`QueryState`] for `is_loading()` and friends.
// The flags are TanStack's flat observer booleans; grouping them would break
// the one-to-one mapping with its API.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct QueryResult<T, E> {
	/// The cached state, with placeholder data substituted when shown.
	pub state: QueryState<T, E>,
	/// Whether `data` is placeholder data rather than this key's own.
	pub is_placeholder_data: bool,
	/// Whether the data is stale by this observer's stale time.
	pub is_stale: bool,
	/// Whether this observer is enabled.
	pub is_enabled: bool,
	/// Whether a fetch finished since this observer mounted. Useful for
	/// hiding cached data until it has been confirmed.
	pub is_fetched_after_mount: bool,
}

impl<T, E> std::ops::Deref for QueryResult<T, E> {
	type Target = QueryState<T, E>;

	fn deref(&self) -> &QueryState<T, E> {
		&self.state
	}
}

/// A live view of one query, returned by [`use_query`].
///
/// `Copy`, like a leptos signal: use it in as many closures as you like.
/// Every accessor is a reactive read. After the owning component unmounts,
/// the handle reads as an idle, pending query.
pub struct QueryHandle<T, E> {
	observer: StoredValue<Arc<Observer<T, E>>>,
}

impl<T, E> Clone for QueryHandle<T, E> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T, E> Copy for QueryHandle<T, E> {}

impl<T, E> fmt::Debug for QueryHandle<T, E>
where
	T: QueryData,
	E: QueryError,
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.observer() {
			Some(observer) => observer.fmt(f),
			None => f.write_str("QueryHandle(disposed)"),
		}
	}
}

impl<T, E> QueryHandle<T, E>
where
	T: QueryData,
	E: QueryError,
{
	pub(crate) fn new(observer: Arc<Observer<T, E>>) -> Self {
		Self {
			observer: StoredValue::new(observer),
		}
	}

	pub(crate) fn observer(&self) -> Option<Arc<Observer<T, E>>> {
		self.observer.try_get_value()
	}

	/// Releases the observer early, before its owner is cleaned up; every
	/// copy of the handle then reads as disposed.
	pub(crate) fn dispose(self) {
		self.observer.dispose();
	}

	pub(crate) fn current_query(&self) -> Option<Access<T, E>> {
		self.observer()
			.and_then(|observer| observer.current_query())
	}

	// ---- data --------------------------------------------------------------

	/// The data to show: the query's own, or placeholder data while it has
	/// none.
	pub fn data(&self) -> Option<T> {
		self.observer().and_then(|observer| observer.data())
	}

	/// Reads the data without cloning it; `None` when there is none.
	pub fn with_data<R>(&self, read: impl FnOnce(&T) -> R) -> Option<R> {
		self.observer()
			.and_then(|observer| observer.with_data(read))
	}

	/// The error of the last failed fetch.
	pub fn error(&self) -> Option<E> {
		self.observer().and_then(|observer| observer.error())
	}

	/// Data as a `Result`, for rendering inside an `ErrorBoundary`: `None`
	/// while pending.
	pub fn result(&self) -> Option<Result<T, E>> {
		match (self.error(), self.data()) {
			(Some(error), _) => Some(Err(error)),
			(None, Some(data)) => Some(Ok(data)),
			(None, None) => None,
		}
	}

	/// A derived signal over the data, TanStack's `select`.
	///
	/// ```
	/// # use leptos::prelude::*;
	/// # use leptoskit_query::*;
	/// # fn demo(todos: QueryHandle<Vec<String>, String>) {
	/// let count: Signal<Option<usize>> = todos.select(|todos| todos.len());
	/// # }
	/// ```
	pub fn select<U>(&self, select: impl Fn(&T) -> U + Send + Sync + 'static) -> Signal<Option<U>>
	where
		U: Send + Sync + 'static,
	{
		let handle = *self;
		Signal::derive(move || handle.data().map(|data| select(&data)))
	}

	/// Like [`select`](Self::select), but subscribers only wake when the
	/// selected value changes.
	pub fn select_memo<U>(
		&self,
		select: impl Fn(&T) -> U + Send + Sync + 'static,
	) -> Memo<Option<U>>
	where
		U: PartialEq + Send + Sync + 'static,
	{
		let handle = *self;
		Memo::new(move |_| handle.data().map(|data| select(&data)))
	}

	// ---- status ------------------------------------------------------------

	/// See [`QueryStatus`]. Placeholder data counts as success.
	pub fn status(&self) -> QueryStatus {
		self.observer()
			.map_or(QueryStatus::Pending, |observer| observer.status())
	}

	/// See [`FetchStatus`].
	pub fn fetch_status(&self) -> FetchStatus {
		self.observer()
			.map_or(FetchStatus::Idle, |observer| observer.fetch_status())
	}

	/// No data and no error yet.
	pub fn is_pending(&self) -> bool {
		self.status() == QueryStatus::Pending
	}

	/// Data is available (its own or placeholder).
	pub fn is_success(&self) -> bool {
		self.status() == QueryStatus::Success
	}

	/// The last fetch failed.
	pub fn is_error(&self) -> bool {
		self.status() == QueryStatus::Error
	}

	/// The query function is running.
	pub fn is_fetching(&self) -> bool {
		self.fetch_status() == FetchStatus::Fetching
	}

	/// A fetch is held back by the network mode.
	pub fn is_paused(&self) -> bool {
		self.fetch_status() == FetchStatus::Paused
	}

	/// The first load is in flight. Prefer this over `is_pending` for a
	/// spinner: a disabled query is pending but not loading.
	pub fn is_loading(&self) -> bool {
		self.is_pending() && self.is_fetching()
	}

	/// A background refetch is in flight behind existing data.
	pub fn is_refetching(&self) -> bool {
		self.is_fetching() && !self.is_pending()
	}

	/// The very first fetch failed.
	pub fn is_loading_error(&self) -> bool {
		self.is_error() && self.meta().data_updated_at.is_none()
	}

	/// A refetch failed while older data is still available.
	pub fn is_refetch_error(&self) -> bool {
		self.is_error() && self.meta().data_updated_at.is_some()
	}

	/// At least one fetch has finished.
	pub fn is_fetched(&self) -> bool {
		let meta = self.meta();
		meta.data_update_count > 0 || meta.error_update_count > 0
	}

	/// A fetch finished since this observer mounted.
	pub fn is_fetched_after_mount(&self) -> bool {
		self.observer()
			.is_some_and(|observer| observer.is_fetched_after_mount())
	}

	/// `data()` is placeholder data, not this key's own.
	pub fn is_placeholder_data(&self) -> bool {
		self.observer()
			.is_some_and(|observer| observer.is_placeholder_data())
	}

	/// Whether the data is stale by this observer's stale time.
	pub fn is_stale(&self) -> bool {
		self.observer().is_none_or(|observer| observer.is_stale())
	}

	/// Whether this observer is enabled and has a key.
	pub fn is_enabled(&self) -> bool {
		self.observer()
			.is_some_and(|observer| observer.is_enabled())
	}

	// ---- bookkeeping -------------------------------------------------------

	/// Timestamps, counters, and flags.
	pub fn meta(&self) -> QueryMeta {
		self.observer()
			.map_or_else(QueryMeta::default, |observer| observer.meta())
	}

	/// When data last arrived, in epoch milliseconds.
	pub fn data_updated_at(&self) -> Option<u64> {
		self.meta().data_updated_at
	}

	/// When an error last arrived, in epoch milliseconds.
	pub fn error_updated_at(&self) -> Option<u64> {
		self.meta().error_updated_at
	}

	/// Consecutive failed attempts in the current or last fetch.
	pub fn failure_count(&self) -> u32 {
		self.meta().failure_count
	}

	/// The error of the most recent failed attempt, including attempts still
	/// being retried.
	pub fn failure_reason(&self) -> Option<E> {
		self.observer()
			.and_then(|observer| observer.failure_reason())
	}

	/// Everything at once, as one reactive read.
	pub fn get(&self) -> QueryResult<T, E> {
		self.observer().map_or_else(
			|| {
				QueryResult {
					state: QueryState {
						data: None,
						error: None,
						fetch_status: FetchStatus::Idle,
						failure_reason: None,
						meta: QueryMeta::default(),
					},
					is_placeholder_data: false,
					is_stale: true,
					is_enabled: false,
					is_fetched_after_mount: false,
				}
			},
			|observer| observer.get(),
		)
	}

	// ---- acting ------------------------------------------------------------

	/// Fetches again, restarting any fetch in flight. The fetch runs whether
	/// or not the returned [`Refetch`] is awaited; await it for the outcome.
	pub fn refetch(&self) -> Refetch<T, E> {
		let refetch = self.observer().map(|observer| observer.refetch());
		Refetch::new(Box::pin(async move {
			match refetch {
				Some(refetch) => refetch.await,
				None => None,
			}
		}))
	}

	/// Marks the data stale and refetches it.
	pub fn invalidate(&self) {
		if let Some(observer) = self.observer() {
			observer.invalidate();
		}
	}

	/// Writes data into the cache for this key, as an optimistic update
	/// would.
	pub fn set_data(&self, data: T) {
		if let Some(observer) = self.observer() {
			observer.set_data(data);
		}
	}

	/// Resolves when the query has data or an error, fetching if it has to.
	///
	/// This is what awaiting the handle does. Like `useSuspenseQuery` it
	/// ignores `enabled`, and a refetch error resolves to `Err` even when
	/// older data exists. A disposed handle never resolves.
	pub async fn wait(&self) -> Result<T, E> {
		match self.observer() {
			Some(observer) => observer.wait().await,
			None => std::future::pending().await,
		}
	}

	/// The client this handle belongs to, while the handle is alive.
	#[must_use]
	pub fn client(&self) -> Option<QueryClient> {
		self.observer().map(|observer| observer.client())
	}

	/// A debugging snapshot of the observed query.
	#[must_use]
	pub fn info(&self) -> Option<QueryInfo> {
		self.observer().and_then(|observer| observer.info())
	}
}

impl<T, E> IntoFuture for QueryHandle<T, E>
where
	T: QueryData,
	E: QueryError,
{
	type IntoFuture = BoxFuture<'static, Result<T, E>>;
	type Output = Result<T, E>;

	fn into_future(self) -> Self::IntoFuture {
		Box::pin(async move { self.wait().await })
	}
}

impl<T, E> IntoFuture for &QueryHandle<T, E>
where
	T: QueryData,
	E: QueryError,
{
	type IntoFuture = BoxFuture<'static, Result<T, E>>;
	type Output = Result<T, E>;

	fn into_future(self) -> Self::IntoFuture {
		(*self).into_future()
	}
}

/// Observes a query whose key is reactive.
///
/// The key closure may return `K` or `Option<K>`; `None` disables the query
/// until a key is available. The handle rebinds whenever the key changes and
/// unsubscribes when the component unmounts.
///
/// ```
/// use leptos::prelude::*;
/// use leptoskit_query::*;
///
/// # #[derive(Clone, Debug)] struct Todo { title: String }
/// # #[derive(Clone, Debug)] struct ApiError;
/// # async fn fetch_todo(id: u32) -> Result<Todo, ApiError> { let _ = id; Ok(Todo { title: String::new() }) }
/// fn todo() -> QueryScope<u32, Todo, ApiError> {
/// 	QueryScope::new(|id| async move { fetch_todo(id).await })
/// }
///
/// #[component]
/// fn TodoView(id: RwSignal<u32>) -> impl IntoView {
/// 	let todo = use_query(todo(), move || id.get());
/// 	view! {
/// 		<Show when=move || todo.is_loading()>"Loading…"</Show>
/// 		{move || todo.data().map(|todo| todo.title)}
/// 	}
/// }
/// ```
#[track_caller]
pub fn use_query<K, T, E, R>(
	scope: QueryScope<K, T, E>,
	key: impl Fn() -> R + Send + Sync + 'static,
) -> QueryHandle<T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
	R: IntoMaybeKey<K>,
{
	use_query_with(scope, key, QueryObserverOptions::default())
}

/// [`use_query`] with per-observer [`QueryObserverOptions`].
#[track_caller]
pub fn use_query_with<K, T, E, R>(
	scope: QueryScope<K, T, E>,
	key: impl Fn() -> R + Send + Sync + 'static,
	options: QueryObserverOptions<T, E>,
) -> QueryHandle<T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
	R: IntoMaybeKey<K>,
{
	use_query_in(use_query_client(), scope, key, options)
}

/// [`use_query_with`] against an explicit client instead of the one in
/// context.
// Scopes are values built at the call site; taking one by value reads better
// than borrowing a temporary.
#[allow(clippy::needless_pass_by_value)]
#[track_caller]
pub fn use_query_in<K, T, E, R>(
	client: QueryClient,
	scope: QueryScope<K, T, E>,
	key: impl Fn() -> R + Send + Sync + 'static,
	options: QueryObserverOptions<T, E>,
) -> QueryHandle<T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
	R: IntoMaybeKey<K>,
{
	let key = ArcMemo::new(move |_| key().into_maybe_key());
	let observer = Observer::new(client, options);
	observer.bind(&scope, key.get_untracked());

	let effect_observer = Arc::clone(&observer);
	let effect_scope = scope.clone();
	Effect::new_isomorphic(move |_| {
		let key = key.get();
		effect_observer.bind(&effect_scope, key);
		effect_observer.track_rebind_sources();
	});

	let cleanup_observer = Arc::clone(&observer);
	on_cleanup(move || cleanup_observer.unbind());
	QueryHandle::new(observer)
}

/// How many queries are fetching, as a signal: the global loading indicator.
#[must_use]
#[track_caller]
pub fn use_is_fetching() -> Signal<usize> {
	let count = use_query_client().fetching_count_signal();
	Signal::derive(move || count.get())
}
