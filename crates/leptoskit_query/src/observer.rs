//! The observer behind a [`QueryHandle`](crate::QueryHandle): what one
//! `use_query` call knows that the shared [`Query`] does not.
//!
//! A query is shared by everyone reading the same key. An observer is one
//! reader: it decides whether *it* is enabled, which placeholder *it* shows,
//! how stale is too stale for *it*, and when *it* mounted. Several observers
//! of one key hold the same `Arc<Query>` and differ only here.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use futures::StreamExt;
use futures::channel::mpsc;
use leptos::prelude::*;
use parking_lot::Mutex;

use crate::client::QueryClient;
use crate::handle::Placeholder;
use crate::handle::QueryObserverOptions;
use crate::handle::QueryResult;
use crate::hydration::DehydratedQuery;
use crate::hydration::is_server;
use crate::listeners::Subscription;
use crate::listeners::spawn;
use crate::managers::FocusManager;
use crate::options::QueryOptions;
use crate::options::RefetchInterval;
use crate::options::RefetchOn;
use crate::options::ResolvedQueryOptions;
use crate::options::StaleTime;
use crate::query::EnabledFn;
use crate::query::FetchFuture;
use crate::query::FetchOptions;
use crate::query::ObserverEntry;
use crate::query::Query;
use crate::query::QueryInfo;
use crate::query::QuerySignals;
use crate::scope::QueryData;
use crate::scope::QueryError;
use crate::scope::QueryKey;
use crate::scope::QueryScope;
use crate::state::FetchStatus;
use crate::state::QueryMeta;
use crate::state::QueryState;
use crate::state::QueryStatus;
use crate::state::status_of;

static NEXT_OBSERVER_ID: AtomicU64 = AtomicU64::new(1);

/// The key-erased face of a [`Query`], so an observer need not know `K`.
pub(crate) trait QueryAccess<T, E>: Send + Sync {
	fn signals(&self) -> &QuerySignals<T, E>;
	fn options(&self) -> &ResolvedQueryOptions<E>;
	fn fetch(&self, options: FetchOptions) -> FetchFuture;
	fn in_flight(&self) -> Option<FetchFuture>;
	fn is_stale_for(&self, stale_time: StaleTime) -> bool;
	fn is_stale_for_tracked(&self, stale_time: StaleTime) -> bool;
	fn add_observer(&self, entry: ObserverEntry<E>);
	fn remove_observer(&self, id: u64);
	fn set_data(&self, data: T, updated_at: Option<u64>);
	fn has_data_untracked(&self) -> bool;
	fn status_untracked(&self) -> QueryStatus;
	fn state_untracked(&self) -> QueryState<T, E>;
	fn removed(&self) -> &ArcTrigger;
	fn invalidate(&self);
	fn dehydrate(&self) -> Option<DehydratedQuery>;
	fn hydrate(&self, dehydrated: &DehydratedQuery);
	fn has_codec(&self) -> bool;
	fn info(&self) -> QueryInfo;
	fn ptr_id(&self) -> usize;
}

impl<K, T, E> QueryAccess<T, E> for Query<K, T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
{
	fn signals(&self) -> &QuerySignals<T, E> {
		&self.signals
	}

	fn options(&self) -> &ResolvedQueryOptions<E> {
		&self.options
	}

	fn fetch(&self, options: FetchOptions) -> FetchFuture {
		Query::fetch(self, options)
	}

	fn in_flight(&self) -> Option<FetchFuture> {
		Query::in_flight(self)
	}

	fn is_stale_for(&self, stale_time: StaleTime) -> bool {
		Query::is_stale_for(self, stale_time)
	}

	fn is_stale_for_tracked(&self, stale_time: StaleTime) -> bool {
		Query::is_stale_for_tracked(self, stale_time)
	}

	fn add_observer(&self, entry: ObserverEntry<E>) {
		Query::add_observer(self, entry);
	}

	fn remove_observer(&self, id: u64) {
		Query::remove_observer(self, id);
	}

	fn set_data(&self, data: T, updated_at: Option<u64>) {
		Query::set_data(self, data, updated_at);
	}

	fn has_data_untracked(&self) -> bool {
		Query::has_data_untracked(self)
	}

	fn status_untracked(&self) -> QueryStatus {
		Query::status_untracked(self)
	}

	fn state_untracked(&self) -> QueryState<T, E> {
		Query::state_untracked(self)
	}

	fn removed(&self) -> &ArcTrigger {
		&self.removed
	}

	fn invalidate(&self) {
		Query::invalidate(self);
	}

	fn dehydrate(&self) -> Option<DehydratedQuery> {
		crate::query::AnyQuery::dehydrate(self)
	}

	fn hydrate(&self, dehydrated: &DehydratedQuery) {
		crate::query::AnyQuery::hydrate(self, dehydrated);
	}

	fn has_codec(&self) -> bool {
		self.codec.is_some()
	}

	fn info(&self) -> QueryInfo {
		Query::info(self)
	}

	fn ptr_id(&self) -> usize {
		std::ptr::from_ref(self).cast::<()>() as usize
	}
}

pub(crate) type Access<T, E> = Arc<dyn QueryAccess<T, E>>;

/// One reader of a query. Always behind an `Arc`, shared between the handle,
/// the effect that rebinds it, and the cleanup that unbinds it.
pub(crate) struct Observer<T, E> {
	client: QueryClient,
	/// The query being watched; `None` while the key is `None`.
	current: ArcRwSignal<Option<Access<T, E>>>,
	/// Keeps this observer registered with the current query; dropping it
	/// unregisters. Held here rather than undone by hand so that an observer
	/// dropped without an explicit unbind still leaves its query, which can
	/// then be refetched less and garbage collected.
	registration: Mutex<Option<Subscription>>,
	enabled: EnabledFn,
	overlay: QueryOptions<E>,
	placeholder: Placeholder<T>,
	initial_data: Mutex<Option<(T, Option<u64>)>>,
	/// Data from the last query watched, for `Placeholder::KeepPrevious`.
	previous_data: ArcRwSignal<Option<T>>,
	/// Update counts when the current query was bound, so
	/// `is_fetched_after_mount` can tell new fetches from old.
	bound_counts: ArcRwSignal<(u32, u32)>,
	id: u64,
	last_enabled: AtomicBool,
	/// Bumped to retire the interval task of a previous binding.
	interval_generation: AtomicU64,
	/// Woken whenever `current` changes, for handles awaiting a key.
	rebinds: Mutex<Vec<mpsc::UnboundedSender<()>>>,
}

impl<T: 'static, E: 'static> fmt::Debug for Observer<T, E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Observer")
			.field("id", &self.id)
			.field("bound", &self.current.try_with_untracked(Option::is_some))
			.finish_non_exhaustive()
	}
}

impl<T, E> Observer<T, E>
where
	T: QueryData,
	E: QueryError,
{
	pub(crate) fn new(client: QueryClient, options: QueryObserverOptions<T, E>) -> Arc<Self> {
		let QueryObserverOptions {
			enabled,
			placeholder,
			initial_data,
			overlay,
		} = options;
		Arc::new(Self {
			client,
			current: ArcRwSignal::new(None),
			registration: Mutex::new(None),
			enabled: enabled.unwrap_or_else(|| Arc::new(|| true)),
			overlay,
			placeholder,
			initial_data: Mutex::new(initial_data),
			previous_data: ArcRwSignal::new(None),
			bound_counts: ArcRwSignal::new((0, 0)),
			id: NEXT_OBSERVER_ID.fetch_add(1, Ordering::Relaxed),
			last_enabled: AtomicBool::new(false),
			interval_generation: AtomicU64::new(0),
			rebinds: Mutex::new(Vec::new()),
		})
	}

	// ---- binding -----------------------------------------------------------
	/// Points the observer at the query for `key`, or at nothing.
	///
	/// Called synchronously on creation so the first render and SSR see the
	/// query, then from an effect whenever the key or `enabled` changes.
	pub(crate) fn bind<K: QueryKey>(self: &Arc<Self>, scope: &QueryScope<K, T, E>, key: Option<K>) {
		let next: Option<Access<T, E>> = key.map(|key| {
			let query: Arc<Query<K, T, E>> = self.client.get_or_create(scope, key);
			query as Access<T, E>
		});
		let previous = self.current.get_untracked();
		let enabled = (self.enabled)();
		let was_enabled = self.last_enabled.swap(enabled, Ordering::SeqCst);

		if Self::same_query(previous.as_ref(), next.as_ref()) {
			// Re-enabling behaves like mounting afresh.
			if enabled
				&& !was_enabled
				&& let Some(query) = &next
			{
				self.fetch_on_mount(query);
				self.start_interval(query);
			}

			return;
		}

		self.leave_previous(previous.as_ref());

		match &next {
			Some(query) => self.join(scope, query, enabled),
			None => self.stop_interval(),
		}

		self.current.set(next);
		self.rebinds
			.lock()
			.retain(|waiter| waiter.unbounded_send(()).is_ok());
	}

	/// Whether `next` is the query already being watched.
	fn same_query(previous: Option<&Access<T, E>>, next: Option<&Access<T, E>>) -> bool {
		match (previous, next) {
			(Some(previous), Some(next)) => previous.ptr_id() == next.ptr_id(),
			(None, None) => true,
			_ => false,
		}
	}

	/// Leaves the previous query before joining the next, keeping its data for
	/// a placeholder.
	fn leave_previous(&self, previous: Option<&Access<T, E>>) {
		let left = self.registration.lock().take();
		drop(left);

		let Some(previous) = previous else {
			return;
		};

		let data = previous.signals().data.get_untracked();

		if data.is_some() {
			self.previous_data.set(data);
		}
	}

	/// Starts watching `query`, with the data it was given before it existed.
	fn join<K: QueryKey>(
		self: &Arc<Self>,
		scope: &QueryScope<K, T, E>,
		query: &Access<T, E>,
		enabled: bool,
	) {
		if let Some((data, updated_at)) = self.initial_data.lock().take()
			&& !query.has_data_untracked()
		{
			query.set_data(data, updated_at);
		}

		query.add_observer(ObserverEntry {
			id: self.id,
			options: self.effective_options(query),
			enabled: Arc::clone(&self.enabled),
		});
		let registered = Arc::clone(query);
		let id = self.id;
		*self.registration.lock() = Some(Subscription::new(move || registered.remove_observer(id)));

		let meta = query.signals().meta.get_untracked();
		self.bound_counts
			.set((meta.data_update_count, meta.error_update_count));
		Self::hook_up_ssr(scope, query);

		if enabled {
			self.fetch_on_mount(query);
		}

		self.start_interval(query);
	}

	/// Stops observing; the observer keeps serving its last state. Calling it
	/// again does nothing.
	pub(crate) fn unbind(&self) {
		self.stop_interval();
		let left = self.registration.lock().take();
		drop(left);
	}

	pub(crate) fn current_query(&self) -> Option<Access<T, E>> {
		self.current.get_untracked()
	}

	/// Tracks the signals that should re-run the binding effect.
	pub(crate) fn track_rebind_sources(&self) {
		let _ = (self.enabled)();

		if let Some(query) = self.current.get_untracked() {
			// A removed query is replaced by a fresh one on the next run.
			query.removed().track();
		}
	}

	fn effective_options(&self, query: &Access<T, E>) -> ResolvedQueryOptions<E> {
		query.options().overlay(&self.overlay)
	}

	/// TanStack's `shouldFetchOnMount`: load when there is no data (unless the
	/// last attempt failed and `retry_on_mount` is off), otherwise consult the
	/// refetch-on-mount policy. Client-only queries never fetch during SSR.
	fn fetch_on_mount(&self, query: &Access<T, E>) {
		if is_server() && !query.has_codec() {
			return;
		}

		let options = self.effective_options(query);
		let should_fetch = if query.has_data_untracked() {
			match options.refetch_on_mount {
				RefetchOn::Never => false,
				RefetchOn::Always => true,
				RefetchOn::Stale => query.is_stale_for(options.stale_time),
			}
		} else {
			query.status_untracked() != QueryStatus::Error || options.retry_on_mount
		};

		if should_fetch {
			drop(query.fetch(FetchOptions::JOIN));
		}
	}

	/// Streams this query's data from the server to the browser, the way a
	/// leptos `Resource` does, when the scope has a codec.
	fn hook_up_ssr<K: QueryKey>(scope: &QueryScope<K, T, E>, query: &Access<T, E>) {
		if scope.codec.is_none() {
			return;
		}

		let Some(context) = Owner::current_shared_context() else {
			return;
		};
		// Ids are handed out in render order on both sides, so consuming one
		// here only when the scope has a codec keeps server and browser in
		// step.
		let id = context.next_id();

		if context.is_browser() {
			if let Some(serialized) = context.read_data(&id)
				&& let Ok(Some(dehydrated)) =
					serde_json::from_str::<Option<DehydratedQuery>>(&serialized)
			{
				query.hydrate(&dehydrated);
			}
		} else if context.get_is_hydrating() {
			let settled = query
				.in_flight()
				.unwrap_or_else(|| query.fetch(FetchOptions::JOIN));
			let query = Arc::clone(query);
			context.write_async(
				id,
				Box::pin(async move {
					settled.await;
					serde_json::to_string(&query.dehydrate()).unwrap_or_else(|_| "null".to_owned())
				}),
			);
		}
	}

	fn start_interval(self: &Arc<Self>, query: &Access<T, E>) {
		let generation = self.interval_generation.fetch_add(1, Ordering::SeqCst) + 1;
		let options = self.effective_options(query);
		let RefetchInterval::Every(interval) = options.refetch_interval else {
			return;
		};
		// Without a real timer every sleep resolves at once, and the loop
		// below would spin without ever yielding.
		if is_server() || !crate::time::has_timer() {
			return;
		}

		let in_background = options.refetch_interval_in_background;
		let observer = Arc::downgrade(self);

		let query = Arc::clone(query);
		spawn(async move {
			loop {
				crate::time::sleep(interval).await;
				let Some(observer) = observer.upgrade() else {
					break;
				};
				if observer.interval_generation.load(Ordering::SeqCst) != generation {
					break;
				}
				if !(observer.enabled)() {
					continue;
				}
				if !in_background && !FocusManager::is_focused_untracked() {
					continue;
				}

				drop(query.fetch(FetchOptions::JOIN));
			}
		});
	}

	fn stop_interval(&self) {
		self.interval_generation.fetch_add(1, Ordering::SeqCst);
	}

	// ---- reading -----------------------------------------------------------
	fn with_query<R>(&self, read: impl FnOnce(Option<&Access<T, E>>) -> R) -> R {
		self.current.with(|query| read(query.as_ref()))
	}

	fn own_data(&self) -> Option<T> {
		self.with_query(|query| query.and_then(|query| query.signals().data.get()))
	}

	fn placeholder_data(&self) -> Option<T> {
		match &self.placeholder {
			Placeholder::None => None,
			Placeholder::KeepPrevious => self.previous_data.get(),
			Placeholder::Value(value) => Some(value.clone()),
			Placeholder::Compute(compute) => {
				self.previous_data
					.with(|previous| compute(previous.as_ref()))
			}
		}
	}

	pub(crate) fn data(&self) -> Option<T> {
		self.own_data().or_else(|| self.placeholder_data())
	}

	pub(crate) fn with_data<R>(&self, read: impl FnOnce(&T) -> R) -> Option<R> {
		self.with_query(|query| {
			query.and_then(|query| query.signals().data.with(|data| data.as_ref().map(read)))
		})
	}

	pub(crate) fn error(&self) -> Option<E> {
		self.with_query(|query| query.and_then(|query| query.signals().error.get()))
	}

	pub(crate) fn status(&self) -> QueryStatus {
		status_of(self.data().is_some(), self.error().is_some())
	}

	pub(crate) fn fetch_status(&self) -> FetchStatus {
		self.with_query(|query| {
			query.map_or(FetchStatus::Idle, |query| {
				query.signals().fetch_status.get()
			})
		})
	}

	pub(crate) fn meta(&self) -> QueryMeta {
		self.with_query(|query| {
			query.map_or_else(QueryMeta::default, |query| query.signals().meta.get())
		})
	}

	pub(crate) fn failure_reason(&self) -> Option<E> {
		self.with_query(|query| query.and_then(|query| query.signals().failure_reason.get()))
	}

	pub(crate) fn is_fetched_after_mount(&self) -> bool {
		let meta = self.meta();
		let (data, error) = self.bound_counts.get();
		meta.data_update_count > data || meta.error_update_count > error
	}

	pub(crate) fn is_placeholder_data(&self) -> bool {
		self.own_data().is_none() && self.placeholder_data().is_some()
	}

	pub(crate) fn is_stale(&self) -> bool {
		self.with_query(|query| {
			query.is_none_or(|query| {
				let stale_time = self.effective_options(query).stale_time;
				query.is_stale_for_tracked(stale_time)
			})
		})
	}

	pub(crate) fn is_enabled(&self) -> bool {
		self.with_query(|query| query.is_some()) && (self.enabled)()
	}

	pub(crate) fn get(&self) -> QueryResult<T, E> {
		let own = self.own_data();
		let placeholder = if own.is_none() {
			self.placeholder_data()
		} else {
			None
		};

		QueryResult {
			is_placeholder_data: own.is_none() && placeholder.is_some(),
			state: QueryState {
				data: own.or(placeholder),
				error: self.error(),
				fetch_status: self.fetch_status(),
				failure_reason: self.failure_reason(),
				meta: self.meta(),
			},
			is_stale: self.is_stale(),
			is_enabled: self.is_enabled(),
			is_fetched_after_mount: self.is_fetched_after_mount(),
		}
	}

	// ---- acting ------------------------------------------------------------
	pub(crate) fn refetch(
		&self,
	) -> impl Future<Output = Option<Result<T, E>>> + Send + 'static + use<T, E> {
		let query = self.current.get_untracked();
		let fetch = query
			.as_ref()
			.map(|query| query.fetch(FetchOptions::RESTART));
		async move {
			let query = query?;
			fetch?.await;
			let state = query.state_untracked();

			match (state.error, state.data) {
				(Some(error), _) => Some(Err(error)),
				(None, Some(data)) => Some(Ok(data)),
				(None, None) => None,
			}
		}
	}

	pub(crate) fn invalidate(&self) {
		if let Some(query) = self.current.get_untracked() {
			query.invalidate();

			if untrack(|| (self.enabled)()) {
				drop(query.fetch(FetchOptions::RESTART));
			}
		}
	}

	pub(crate) fn set_data(&self, data: T) {
		if let Some(query) = self.current.get_untracked() {
			query.set_data(data, None);
		}
	}

	pub(crate) fn client(&self) -> QueryClient {
		self.client.clone()
	}

	pub(crate) fn info(&self) -> Option<QueryInfo> {
		self.current.get_untracked().map(|query| query.info())
	}

	/// Resolves when the query has data or an error, fetching if it has to.
	pub(crate) async fn wait(&self) -> Result<T, E> {
		loop {
			let Some(query) = self.current.get_untracked() else {
				// No key yet: sleep until the next rebind, re-checking after
				// subscribing so a bind in between is not missed.
				let (sender, mut receiver) = mpsc::unbounded();
				self.rebinds.lock().push(sender);

				if self.current.get_untracked().is_none() {
					receiver.next().await;
				}

				continue;
			};
			let state = query.state_untracked();

			if let Some(error) = state.error {
				return Err(error);
			}

			if let Some(data) = state.data {
				return Ok(data);
			}

			let fetch = query
				.in_flight()
				.unwrap_or_else(|| query.fetch(FetchOptions::JOIN));
			fetch.await;
		}
	}
}
