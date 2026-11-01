//! The cached unit: one key's data, error, and fetch machinery.
//!
//! A [`Query`] is shared by every observer of the same scope and key. Its
//! state lives in per-field signals so a view that reads `data` is not woken
//! by a change to `fetch_status`, which is how TanStack's
//! `notifyOnChangeProps` and structural sharing fall out of fine-grained
//! reactivity for free.
//!
//! Fetching is deduplicated with a shared future: however many observers,
//! prefetches, and `fetch_query` calls want the same key at once, the query
//! function runs once and all of them await the same completion.

use std::any::Any;
use std::fmt;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use futures::FutureExt;
use futures::future::Shared;
use leptos::prelude::*;
use parking_lot::Mutex;

use crate::cache::CacheEvent;
use crate::cache::CacheInner;
use crate::cancel::CancelToken;
use crate::hydration::DehydratedQuery;
use crate::key::ScopeId;
use crate::key::Tag;
use crate::key::hash_key;
use crate::listeners::spawn;
use crate::managers::OnlineManager;
use crate::options::GcTime;
use crate::options::NetworkMode;
use crate::options::RefetchOn;
use crate::options::ResolvedQueryOptions;
use crate::options::StaleTime;
use crate::retry::Attempts;
use crate::retry::Settled;
use crate::retry::until_cancelled;
use crate::scope::DataCodec;
use crate::scope::DataEqFn;
use crate::scope::FetchContext;
use crate::scope::Fetcher;
use crate::scope::QueryData;
use crate::scope::QueryError;
use crate::scope::QueryKey;
use crate::scope::QueryScope;
use crate::state::FetchDirection;
use crate::state::FetchStatus;
use crate::state::QueryMeta;
use crate::state::QueryState;
use crate::state::QueryStatus;
use crate::state::status_of;
use crate::time::BoxFuture;
use crate::time::now_ms;
use crate::time::sleep;

/// Completes when the fetch it belongs to settles — with data, with an
/// error, or cancelled. Read the query afterwards for the outcome.
pub type FetchFuture = Shared<BoxFuture<'static, ()>>;

/// The per-field signals of one query.
///
/// Each field is its own signal so observers subscribe only to what they
/// read.
pub struct QuerySignals<T, E> {
	/// The last successfully fetched data.
	pub data: ArcRwSignal<Option<T>>,
	/// The error from the last failed fetch, cleared by the next success.
	pub error: ArcRwSignal<Option<E>>,
	/// See [`FetchStatus`].
	pub fetch_status: ArcRwSignal<FetchStatus>,
	/// The error from the most recent failed attempt, including attempts
	/// that are still being retried.
	pub failure_reason: ArcRwSignal<Option<E>>,
	/// Timestamps, counters, and flags.
	pub meta: ArcRwSignal<QueryMeta>,
}

impl<T, E> QuerySignals<T, E> {
	fn new() -> Self {
		Self {
			data: ArcRwSignal::new(None),
			error: ArcRwSignal::new(None),
			fetch_status: ArcRwSignal::new(FetchStatus::Idle),
			failure_reason: ArcRwSignal::new(None),
			meta: ArcRwSignal::new(QueryMeta::default()),
		}
	}
}

impl<T, E> Clone for QuerySignals<T, E> {
	fn clone(&self) -> Self {
		Self {
			data: self.data.clone(),
			error: self.error.clone(),
			fetch_status: self.fetch_status.clone(),
			failure_reason: self.failure_reason.clone(),
			meta: self.meta.clone(),
		}
	}
}

impl<T, E> fmt::Debug for QuerySignals<T, E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("QuerySignals(..)")
	}
}

/// How a fetch should start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FetchOptions {
	/// For infinite queries, which end to extend; `None` fetches the first
	/// page or refetches every page.
	pub direction: Option<FetchDirection>,
	/// Whether to cancel a fetch already in flight and start over, rather
	/// than join it. Only applies once the query has data, so a first load is
	/// never interrupted.
	pub cancel_refetch: bool,
	/// For infinite queries fetched imperatively: how many pages to load.
	pub pages: Option<usize>,
}

impl FetchOptions {
	/// Joins any fetch in flight.
	pub const JOIN: Self = Self {
		direction: None,
		cancel_refetch: false,
		pages: None,
	};
	/// Restarts a fetch in flight, as `refetch()` and invalidation do.
	pub const RESTART: Self = Self {
		direction: None,
		cancel_refetch: true,
		pages: None,
	};
}

/// One observer's standing orders, consulted when focus or the network
/// returns.
pub(crate) type EnabledFn = Arc<dyn Fn() -> bool + Send + Sync>;

pub(crate) struct ObserverEntry<E> {
	pub id: u64,
	pub options: ResolvedQueryOptions<E>,
	pub enabled: EnabledFn,
}

struct InFlight {
	future: FetchFuture,
	token: CancelToken,
	generation: u64,
}

/// What a cancelled fetch restores: the fields a fetch start had changed.
struct Revert<E> {
	error: Option<E>,
	failure_reason: Option<E>,
	failure_count: u32,
}

/// A debugging snapshot of one cached query, for devtools and logging.
#[derive(Clone, Debug)]
pub struct QueryInfo {
	/// The scope's cache identity.
	pub scope: ScopeId,
	/// The scope's readable name.
	pub scope_name: String,
	/// The key, as `Debug` prints it.
	pub key: String,
	/// The key's hash.
	pub key_hash: u64,
	/// The query's tags.
	pub tags: Vec<Tag>,
	/// See [`QueryStatus`].
	pub status: QueryStatus,
	/// See [`FetchStatus`].
	pub fetch_status: FetchStatus,
	/// How many observers are mounted.
	pub observers: usize,
	/// Whether the data is stale by the scope's own stale time.
	pub is_stale: bool,
	/// Timestamps, counters, and flags.
	pub meta: QueryMeta,
}

pub(crate) struct Query<K, T, E> {
	self_weak: Weak<Self>,
	pub(crate) scope_id: ScopeId,
	pub(crate) scope_name: Arc<str>,
	scope_named: bool,
	pub(crate) key: K,
	pub(crate) key_hash: u64,
	pub(crate) tags: Vec<Tag>,
	fetcher: Fetcher<K, T, E>,
	pub(crate) options: ResolvedQueryOptions<E>,
	pub(crate) signals: QuerySignals<T, E>,
	pub(crate) codec: Option<DataCodec<T>>,
	data_eq: Option<DataEqFn<T>>,
	observers: Mutex<Vec<ObserverEntry<E>>>,
	in_flight: Mutex<Option<InFlight>>,
	fetch_generation: AtomicU64,
	/// Cancels the cleanup waiting to drop this query, so at most one is ever
	/// pending.
	pending_gc: Mutex<Option<CancelToken>>,
	cache: Weak<CacheInner>,
	/// Notified when the query leaves the cache, so observers rebind to a
	/// fresh query instead of holding a detached one.
	pub(crate) removed: ArcTrigger,
}

impl<K, T, E> Query<K, T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
{
	pub(crate) fn new(
		cache: &Arc<CacheInner>,
		scope: &QueryScope<K, T, E>,
		key: K,
		options: ResolvedQueryOptions<E>,
	) -> Arc<Self> {
		let key_hash = hash_key(&key);
		let tags = scope.tags_for(&key);
		Arc::new_cyclic(|self_weak| {
			Self {
				self_weak: self_weak.clone(),
				scope_id: scope.id(),
				scope_name: Arc::clone(&scope.name),
				scope_named: scope.is_named(),
				key,
				key_hash,
				tags,
				fetcher: Arc::clone(&scope.fetcher),
				options,
				signals: QuerySignals::new(),
				codec: scope.codec.clone(),
				data_eq: scope.data_eq.clone(),
				observers: Mutex::new(Vec::new()),
				in_flight: Mutex::new(None),
				fetch_generation: AtomicU64::new(0),
				pending_gc: Mutex::new(None),
				cache: Arc::downgrade(cache),
				removed: ArcTrigger::new(),
			}
		})
	}

	// ---- reading -----------------------------------------------------------
	/// A snapshot that subscribes to nothing.
	pub(crate) fn state_untracked(&self) -> QueryState<T, E> {
		QueryState {
			data: self.signals.data.get_untracked(),
			error: self.signals.error.get_untracked(),
			fetch_status: self.signals.fetch_status.get_untracked(),
			failure_reason: self.signals.failure_reason.get_untracked(),
			meta: self.signals.meta.get_untracked(),
		}
	}

	pub(crate) fn has_data_untracked(&self) -> bool {
		self.signals.data.with_untracked(Option::is_some)
	}

	pub(crate) fn status_untracked(&self) -> QueryStatus {
		status_of(
			self.has_data_untracked(),
			self.signals.error.with_untracked(Option::is_some),
		)
	}

	/// Whether the data is stale under `stale_time`, without subscribing.
	///
	/// Missing data is always stale; static data never is, not even when
	/// invalidated; otherwise an invalidation or age decides.
	pub(crate) fn is_stale_for(&self, stale_time: StaleTime) -> bool {
		Self::stale(
			self.has_data_untracked(),
			self.signals.meta.get_untracked(),
			stale_time,
		)
	}

	/// Whether the data is stale under `stale_time`, subscribing to the
	/// signals that decide it.
	pub(crate) fn is_stale_for_tracked(&self, stale_time: StaleTime) -> bool {
		Self::stale(
			self.signals.data.with(Option::is_some),
			self.signals.meta.get(),
			stale_time,
		)
	}

	fn stale(has_data: bool, meta: QueryMeta, stale_time: StaleTime) -> bool {
		if !has_data {
			return true;
		}

		if stale_time == StaleTime::Static {
			return false;
		}

		if meta.is_invalidated {
			return true;
		}

		meta.data_updated_at
			.is_none_or(|updated_at| stale_time.is_stale_at(updated_at, now_ms()))
	}

	// ---- observers ---------------------------------------------------------
	pub(crate) fn add_observer(&self, entry: ObserverEntry<E>) {
		self.cancel_gc();
		self.observers.lock().push(entry);
		self.emit(CacheEvent::ObserverAdded);
	}

	pub(crate) fn remove_observer(&self, id: u64) {
		let empty = {
			let mut observers = self.observers.lock();
			observers.retain(|entry| entry.id != id);
			observers.is_empty()
		};
		self.emit(CacheEvent::ObserverRemoved);

		if empty {
			self.schedule_gc();
		}
	}

	pub(crate) fn observer_count(&self) -> usize {
		self.observers.lock().len()
	}

	/// Whether any observer is enabled.
	///
	/// Untracked: `enabled` closures read the observers' own signals, which
	/// whoever asks — an effect that invalidates, say — must not subscribe to.
	pub(crate) fn is_active(&self) -> bool {
		let enabled: Vec<_> = self
			.observers
			.lock()
			.iter()
			.map(|entry| Arc::clone(&entry.enabled))
			.collect();
		untrack(|| enabled.iter().any(|enabled| enabled()))
	}

	/// Disabled queries are skipped by refetches: every observer is disabled,
	/// or nobody observes it and it has never fetched.
	pub(crate) fn is_disabled(&self) -> bool {
		if self.observer_count() > 0 {
			return !self.is_active();
		}

		let meta = self.signals.meta.get_untracked();
		meta.data_update_count + meta.error_update_count == 0
	}

	// ---- fetching ----------------------------------------------------------
	/// Starts a fetch, or joins the one in flight.
	pub(crate) fn fetch(&self, options: FetchOptions) -> FetchFuture {
		let mut in_flight = self.in_flight.lock();

		if let Some(current) = in_flight.as_ref() {
			// A fetch that was already cancelled, by `cancel_queries` say, is
			// settling rather than loading: joining it would hand the caller
			// nothing.
			let restart = current.token.is_cancelled()
				|| (options.cancel_refetch && self.has_data_untracked());

			if !restart {
				return current.future.clone();
			}

			current.token.cancel();
		}

		let Some(this) = self.self_weak.upgrade() else {
			// The query is being dropped; nothing can observe the result.
			let done: BoxFuture<'static, ()> = Box::pin(std::future::ready(()));

			return done.shared();
		};

		let token = CancelToken::new();
		let generation = self.fetch_generation.fetch_add(1, Ordering::SeqCst) + 1;
		let (revert, paused) = self.begin_fetch(options.direction);

		let boxed: BoxFuture<'static, ()> = Box::pin(Self::run_fetch(
			this,
			token.clone(),
			options,
			generation,
			revert,
			paused,
		));
		let future: FetchFuture = boxed.shared();
		*in_flight = Some(InFlight {
			future: future.clone(),
			token,
			generation,
		});
		drop(in_flight);
		spawn(future.clone());
		future
	}

	/// The fetch in flight, if any.
	pub(crate) fn in_flight(&self) -> Option<FetchFuture> {
		self.in_flight
			.lock()
			.as_ref()
			.map(|current| current.future.clone())
	}

	/// Applies the start transition synchronously — before the spawned task
	/// first runs — so `is_loading()` is true the instant a fetch is requested
	/// and a query with no data goes back to pending at once.
	///
	/// Returns what a cancelled fetch should restore, and whether the fetch
	/// starts paused because the network mode forbids fetching offline.
	fn begin_fetch(&self, direction: Option<FetchDirection>) -> (Revert<E>, bool) {
		let had_data = self.has_data_untracked();
		let revert = Revert {
			error: self.signals.error.get_untracked(),
			failure_reason: self.signals.failure_reason.get_untracked(),
			failure_count: self.signals.meta.get_untracked().failure_count,
		};
		self.update_meta(|meta| {
			meta.failure_count = 0;
			meta.fetch_direction = direction;
		});
		Self::set_signal(&self.signals.failure_reason, None);

		if !had_data {
			Self::set_signal(&self.signals.error, None);
		}

		let paused = self.options.network_mode == NetworkMode::Online
			&& !OnlineManager::is_online_untracked();
		self.set_fetch_status(if paused {
			FetchStatus::Paused
		} else {
			FetchStatus::Fetching
		});

		(revert, paused)
	}

	async fn run_fetch(
		query: Arc<Self>,
		token: CancelToken,
		options: FetchOptions,
		generation: u64,
		revert: Revert<E>,
		paused: bool,
	) {
		let settled = query.attempt(&token, options, paused).await;
		// Cancellation wins any tie: a cancel that lands after the last
		// attempt resolved but before its outcome is written still cancels.
		let settled = if token.is_cancelled() {
			Settled::Cancelled
		} else {
			settled
		};

		query.finish(settled, revert, options.direction, generation);
	}

	/// Waits for the network if the fetch starts paused, then runs the
	/// fetcher under the retry policy until it succeeds, gives up, or is
	/// cancelled.
	async fn attempt(
		&self,
		token: &CancelToken,
		options: FetchOptions,
		paused: bool,
	) -> Settled<T, E> {
		// A fetch cancelled or retired before its task first ran never
		// starts: `reset` may already have put the query back to idle.
		if token.is_cancelled() {
			return Settled::Cancelled;
		}

		if paused {
			if until_cancelled(token, OnlineManager::wait_until_online())
				.await
				.is_none()
			{
				return Settled::Cancelled;
			}
			self.set_fetch_status(FetchStatus::Fetching);
		}

		let attempts = Attempts {
			retry: &self.options.retry,
			retry_delay: &self.options.retry_delay,
			network_mode: self.options.network_mode,
			token,
		};
		attempts
			.run(
				|| {
					(self.fetcher)(FetchContext {
						key: self.key.clone(),
						direction: options.direction,
						current: self.signals.data.get_untracked(),
						signal: token.clone(),
						pages: options.pages,
					})
				},
				|failures, error| {
					self.update_meta(|meta| meta.failure_count = failures);
					Self::set_signal(&self.signals.failure_reason, Some(error.clone()));
				},
				|paused| {
					self.set_fetch_status(if paused {
						FetchStatus::Paused
					} else {
						FetchStatus::Fetching
					});
				},
			)
			.await
	}

	fn finish(
		&self,
		settled: Settled<T, E>,
		revert: Revert<E>,
		direction: Option<FetchDirection>,
		generation: u64,
	) {
		// A fetch that a newer one replaced, or that a reset retired, owns
		// nothing any more: whatever it ended with is discarded, and the
		// status belongs to whatever came after it.
		let superseded = self.fetch_generation.load(Ordering::SeqCst) != generation;

		match settled {
			_ if superseded => {}
			Settled::Success(data) => self.on_success(data),
			Settled::Error(error) => self.on_error(error, direction),
			Settled::Cancelled => {
				Self::set_signal(&self.signals.error, revert.error);
				Self::set_signal(&self.signals.failure_reason, revert.failure_reason);
				self.update_meta(|meta| meta.failure_count = revert.failure_count);
			}
		}

		if !superseded {
			let mut in_flight = self.in_flight.lock();

			if in_flight
				.as_ref()
				.is_some_and(|current| current.generation == generation)
			{
				*in_flight = None;
			}
			drop(in_flight);
			self.set_fetch_status(FetchStatus::Idle);

			self.update_meta(|meta| meta.fetch_direction = None);
		}

		if self.observers.lock().is_empty() {
			self.schedule_gc();
		}
	}

	fn on_success(&self, data: T) {
		let keep_existing = self.data_eq.as_ref().is_some_and(|eq| {
			self.signals.data.with_untracked(|existing| {
				existing
					.as_ref()
					.is_some_and(|existing| eq(existing, &data))
			})
		});

		if !keep_existing {
			self.signals.data.set(Some(data));
		}

		Self::set_signal(&self.signals.error, None);
		Self::set_signal(&self.signals.failure_reason, None);
		self.update_meta(|meta| {
			meta.data_updated_at = Some(now_ms());
			meta.data_update_count += 1;
			meta.failure_count = 0;
			meta.is_invalidated = false;
			meta.failed_direction = None;
		});
		self.emit(CacheEvent::Updated);
	}

	fn on_error(&self, error: E, direction: Option<FetchDirection>) {
		self.signals.error.set(Some(error));
		self.update_meta(|meta| {
			meta.error_updated_at = Some(now_ms());
			meta.error_update_count += 1;
			meta.failed_direction = direction;
		});
		self.emit(CacheEvent::Updated);
	}

	/// Cancels the fetch in flight, restoring the state from before it began.
	pub(crate) fn cancel(&self) {
		let token = self
			.in_flight
			.lock()
			.as_ref()
			.map(|current| current.token.clone());

		if let Some(token) = token {
			token.cancel();
		}
	}

	// ---- writing -----------------------------------------------------------
	/// Writes data as if a fetch had succeeded.
	pub(crate) fn set_data(&self, data: T, updated_at: Option<u64>) {
		self.signals.data.set(Some(data));
		Self::set_signal(&self.signals.error, None);
		self.update_meta(|meta| {
			meta.data_updated_at = Some(updated_at.unwrap_or_else(now_ms));
			meta.data_update_count += 1;
			meta.is_invalidated = false;
		});
		self.emit(CacheEvent::Updated);
	}

	/// Marks the data stale regardless of age.
	pub(crate) fn invalidate(&self) {
		self.update_meta(|meta| meta.is_invalidated = true);
	}

	/// Returns to the never-fetched state, cancelling any fetch.
	pub(crate) fn reset(&self) {
		self.cancel();
		// Retire the cancelled fetch. It settles later and must neither
		// restore the state it saved nor be joined by the next fetch.
		self.fetch_generation.fetch_add(1, Ordering::SeqCst);
		*self.in_flight.lock() = None;
		Self::set_signal(&self.signals.data, None);
		Self::set_signal(&self.signals.error, None);
		Self::set_signal(&self.signals.failure_reason, None);
		self.update_meta(|meta| *meta = QueryMeta::default());
		self.set_fetch_status(FetchStatus::Idle);
		self.emit(CacheEvent::Updated);
	}

	fn update_meta(&self, update: impl FnOnce(&mut QueryMeta)) {
		let mut meta = self.signals.meta.get_untracked();
		let before = meta;
		update(&mut meta);

		if meta != before {
			self.signals.meta.set(meta);
		}
	}

	/// Sets an `Option` signal only when it would change whether a value is
	/// present or which value it is, to spare subscribers spurious wake-ups.
	fn set_signal<V: Send + Sync + 'static>(signal: &ArcRwSignal<Option<V>>, value: Option<V>) {
		if value.is_none() && signal.with_untracked(Option::is_none) {
			return;
		}

		signal.set(value);
	}

	fn set_fetch_status(&self, status: FetchStatus) {
		let previous = self.signals.fetch_status.get_untracked();

		if previous == status {
			return;
		}

		self.signals.fetch_status.set(status);

		if let Some(cache) = self.cache.upgrade() {
			match (previous, status) {
				(FetchStatus::Fetching, _) => {
					cache
						.fetching
						.update(|count| *count = count.saturating_sub(1));
				}
				(_, FetchStatus::Fetching) => cache.fetching.update(|count| *count += 1),
				_ => {}
			}
		}
	}

	fn emit(&self, event: CacheEvent) {
		if let Some(cache) = self.cache.upgrade() {
			cache.emit(event, self);
		}
	}

	// ---- lifecycle ---------------------------------------------------------
	/// Drops the query from the cache once it has gone unobserved for its gc
	/// time. A fetch still in flight defers removal until it settles.
	///
	/// Replaces any cleanup already pending, so a query never has more than
	/// one timer waiting.
	pub(crate) fn schedule_gc(&self) {
		let GcTime::Duration(gc_time) = self.options.gc_time else {
			return;
		};
		let token = CancelToken::new();

		if let Some(previous) = self.pending_gc.lock().replace(token.clone()) {
			previous.cancel();
		}

		let weak = self.self_weak.clone();
		spawn(async move {
			if until_cancelled(&token, sleep(gc_time)).await.is_none() {
				return;
			}

			let Some(query) = weak.upgrade() else {
				return;
			};
			if !query.observers.lock().is_empty()
				|| query.signals.fetch_status.get_untracked() != FetchStatus::Idle
			{
				return;
			}
			if let Some(cache) = query.cache.upgrade() {
				cache.remove(&(query as Arc<dyn AnyQuery>));
			}
		});
	}

	fn cancel_gc(&self) {
		if let Some(pending) = self.pending_gc.lock().take() {
			pending.cancel();
		}
	}

	/// The window regained focus.
	pub(crate) fn on_focus(&self) {
		self.refetch_for_event(|options| options.refetch_on_window_focus);
	}

	/// The network came back.
	pub(crate) fn on_online(&self) {
		self.refetch_for_event(|options| options.refetch_on_reconnect);
	}

	fn refetch_for_event(&self, policy: impl Fn(&ResolvedQueryOptions<E>) -> RefetchOn) {
		let entries: Vec<(RefetchOn, StaleTime, EnabledFn)> = self
			.observers
			.lock()
			.iter()
			.map(|entry| {
				(
					policy(&entry.options),
					entry.options.stale_time,
					Arc::clone(&entry.enabled),
				)
			})
			.collect();
		let should_fetch = untrack(|| {
			entries.iter().any(|(policy, stale_time, enabled)| {
				enabled()
					&& match policy {
						RefetchOn::Always => true,
						RefetchOn::Stale => self.is_stale_for(*stale_time),
						RefetchOn::Never => false,
					}
			})
		});

		if should_fetch {
			drop(self.fetch(FetchOptions::JOIN));
		}
	}

	pub(crate) fn on_removed(&self) {
		self.cancel_gc();
		self.removed.notify();
	}

	pub(crate) fn info(&self) -> QueryInfo {
		QueryInfo {
			scope: self.scope_id,
			scope_name: self.scope_name.to_string(),
			key: format!("{:?}", self.key),
			key_hash: self.key_hash,
			tags: self.tags.clone(),
			status: self.status_untracked(),
			fetch_status: self.signals.fetch_status.get_untracked(),
			observers: self.observer_count(),
			is_stale: self.is_stale_for(self.options.stale_time),
			meta: self.signals.meta.get_untracked(),
		}
	}
}

/// The type-erased face of a query, for the cache's maps and filters.
pub(crate) trait AnyQuery: Any + Send + Sync {
	fn scope_id(&self) -> ScopeId;
	fn is_scope_named(&self) -> bool;
	fn key_hash(&self) -> u64;
	fn tags(&self) -> &[Tag];
	fn is_active(&self) -> bool;
	fn is_disabled(&self) -> bool;
	fn is_stale(&self) -> bool;
	fn is_static(&self) -> bool;
	fn fetch_status(&self) -> FetchStatus;
	fn invalidate(&self);
	fn refetch(&self, cancel_refetch: bool) -> FetchFuture;
	fn cancel(&self);
	fn reset(&self);
	fn on_focus(&self);
	fn on_online(&self);
	fn on_removed(&self);
	fn schedule_gc(&self);
	fn dehydrate(&self) -> Option<DehydratedQuery>;
	fn hydrate(&self, dehydrated: &DehydratedQuery);
	fn info(&self) -> QueryInfo;
	fn as_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
	#[cfg(feature = "devtools")]
	fn data_type(&self) -> &'static str;
	#[cfg(feature = "devtools")]
	fn has_codec(&self) -> bool;
	#[cfg(feature = "devtools")]
	fn encode_data(&self) -> Option<Result<serde_json::Value, serde_json::Error>>;
	#[cfg(feature = "devtools")]
	fn set_json(&self, json: serde_json::Value) -> Result<(), crate::devtools::EditError>;
}

impl<K, T, E> AnyQuery for Query<K, T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
{
	fn scope_id(&self) -> ScopeId {
		self.scope_id
	}

	fn is_scope_named(&self) -> bool {
		self.scope_named
	}

	fn key_hash(&self) -> u64 {
		self.key_hash
	}

	fn tags(&self) -> &[Tag] {
		&self.tags
	}

	fn is_active(&self) -> bool {
		Query::is_active(self)
	}

	fn is_disabled(&self) -> bool {
		Query::is_disabled(self)
	}

	fn is_stale(&self) -> bool {
		self.is_stale_for(self.options.stale_time)
	}

	fn is_static(&self) -> bool {
		self.options.stale_time == StaleTime::Static
	}

	fn fetch_status(&self) -> FetchStatus {
		self.signals.fetch_status.get_untracked()
	}

	fn invalidate(&self) {
		Query::invalidate(self);
	}

	fn refetch(&self, cancel_refetch: bool) -> FetchFuture {
		self.fetch(FetchOptions {
			direction: None,
			cancel_refetch,
			pages: None,
		})
	}

	fn cancel(&self) {
		Query::cancel(self);
	}

	fn reset(&self) {
		Query::reset(self);
	}

	fn on_focus(&self) {
		Query::on_focus(self);
	}

	fn on_online(&self) {
		Query::on_online(self);
	}

	fn on_removed(&self) {
		Query::on_removed(self);
	}

	fn schedule_gc(&self) {
		Query::schedule_gc(self);
	}

	fn dehydrate(&self) -> Option<DehydratedQuery> {
		let codec = self.codec.as_ref()?;

		if self.status_untracked() != QueryStatus::Success {
			return None;
		}

		let data = self
			.signals
			.data
			.with_untracked(|data| data.as_ref().map(|data| codec.encode(data)))?
			.ok()?;
		Some(DehydratedQuery {
			scope: self.scope_id,
			scope_name: self.scope_name.to_string(),
			key: format!("{:?}", self.key),
			key_hash: self.key_hash,
			data,
			data_updated_at: self
				.signals
				.meta
				.get_untracked()
				.data_updated_at
				.unwrap_or_else(now_ms),
		})
	}

	fn hydrate(&self, dehydrated: &DehydratedQuery) {
		let Some(codec) = self.codec.as_ref() else {
			return;
		};
		// Fresher local data wins over what was dehydrated earlier.
		let current = self.signals.meta.get_untracked().data_updated_at;

		if current.is_some_and(|current| current >= dehydrated.data_updated_at) {
			return;
		}

		if let Ok(data) = codec.decode(dehydrated.data.clone()) {
			self.set_data(data, Some(dehydrated.data_updated_at));
		}
	}

	fn info(&self) -> QueryInfo {
		Query::info(self)
	}

	fn as_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
		self
	}

	#[cfg(feature = "devtools")]
	fn data_type(&self) -> &'static str {
		std::any::type_name::<T>()
	}

	#[cfg(feature = "devtools")]
	fn has_codec(&self) -> bool {
		self.codec.is_some()
	}

	#[cfg(feature = "devtools")]
	fn encode_data(&self) -> Option<Result<serde_json::Value, serde_json::Error>> {
		let codec = self.codec.as_ref()?;
		self.signals
			.data
			.with_untracked(|data| data.as_ref().map(|data| codec.encode(data)))
	}

	#[cfg(feature = "devtools")]
	fn set_json(&self, json: serde_json::Value) -> Result<(), crate::devtools::EditError> {
		let codec = self
			.codec
			.as_ref()
			.ok_or(crate::devtools::EditError::NoCodec)?;
		let data = codec
			.decode(json)
			.map_err(crate::devtools::EditError::Decode)?;
		self.set_data(data, None);
		Ok(())
	}
}
