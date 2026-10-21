//! The query cache: the map behind every [`QueryClient`](crate::QueryClient).

use std::collections::HashMap;
use std::sync::Arc;

use leptos::prelude::*;
use parking_lot::Mutex;
use parking_lot::RwLock;

use crate::filters::QueryFilter;
use crate::hydration::DehydratedQuery;
use crate::key::ScopeId;
use crate::listeners::Listeners;
use crate::listeners::Subscription;
use crate::options::GcTime;
use crate::query::AnyQuery;
use crate::query::QueryInfo;
use crate::time::now_ms;

/// What changed in the cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CacheEvent {
	/// A query was created.
	Added,
	/// A query was garbage collected or removed.
	Removed,
	/// A query's data, error, or status changed.
	Updated,
	/// An observer mounted.
	ObserverAdded,
	/// An observer unmounted.
	ObserverRemoved,
}

/// A cache event and the query it concerns.
#[derive(Clone, Debug)]
pub struct CacheNotification {
	/// What happened.
	pub event: CacheEvent,
	/// The query it happened to.
	pub query: QueryInfo,
}

pub(crate) struct CacheInner {
	queries: RwLock<HashMap<(ScopeId, u64), Arc<dyn AnyQuery>>>,
	/// How many queries are fetching right now.
	pub(crate) fetching: ArcRwSignal<usize>,
	/// Notified whenever a query is added or removed.
	pub(crate) version: ArcTrigger,
	listeners: Listeners<CacheNotification>,
	/// Dehydrated data waiting for its query to be created.
	pending_hydration: Mutex<HashMap<(ScopeId, u64), Waiting>>,
}

impl CacheInner {
	pub(crate) fn new() -> Arc<Self> {
		Arc::new(Self {
			queries: RwLock::new(HashMap::new()),
			fetching: ArcRwSignal::new(0),
			version: ArcTrigger::new(),
			listeners: Listeners::default(),
			pending_hydration: Mutex::new(HashMap::new()),
		})
	}

	pub(crate) fn get(&self, scope: ScopeId, key_hash: u64) -> Option<Arc<dyn AnyQuery>> {
		self.queries.read().get(&(scope, key_hash)).cloned()
	}

	/// Returns the query, creating it with `create` when absent.
	///
	/// A freshly created query receives any data hydrated for it earlier.
	pub(crate) fn get_or_insert_with(
		&self,
		scope: ScopeId,
		key_hash: u64,
		create: impl FnOnce() -> Arc<dyn AnyQuery>,
	) -> Arc<dyn AnyQuery> {
		if let Some(query) = self.get(scope, key_hash) {
			return query;
		}
		let (query, created) = {
			let mut queries = self.queries.write();
			if let Some(query) = queries.get(&(scope, key_hash)) {
				(Arc::clone(query), false)
			} else {
				let query = create();
				queries.insert((scope, key_hash), Arc::clone(&query));
				(query, true)
			}
		};
		if created {
			let waiting = self.pending_hydration.lock().remove(&(scope, key_hash));
			if let Some(waiting) = waiting.filter(|waiting| !waiting.is_expired(now_ms())) {
				query.hydrate(&waiting.query);
			}
			self.emit(CacheEvent::Added, &*query);
			self.version.notify();
		}
		query
	}

	pub(crate) fn remove(&self, scope: ScopeId, key_hash: u64) {
		let removed = self.queries.write().remove(&(scope, key_hash));
		if let Some(query) = removed {
			query.cancel();
			query.on_removed();
			self.emit(CacheEvent::Removed, &*query);
			self.version.notify();
		}
	}

	pub(crate) fn all(&self) -> Vec<Arc<dyn AnyQuery>> {
		self.queries.read().values().cloned().collect()
	}

	pub(crate) fn find_all(&self, filter: &QueryFilter) -> Vec<Arc<dyn AnyQuery>> {
		self.all()
			.into_iter()
			.filter(|query| filter.matches(query))
			.collect()
	}

	pub(crate) fn clear(&self) {
		let queries: Vec<_> = self
			.queries
			.write()
			.drain()
			.map(|(_, query)| query)
			.collect();
		for query in &queries {
			query.cancel();
			query.on_removed();
			self.emit(CacheEvent::Removed, &**query);
		}
		self.pending_hydration.lock().clear();
		if !queries.is_empty() {
			self.version.notify();
		}
	}

	pub(crate) fn subscribe(
		&self,
		listener: impl Fn(&CacheNotification) + Send + Sync + 'static,
	) -> Subscription {
		self.listeners.subscribe(listener)
	}

	pub(crate) fn emit(&self, event: CacheEvent, query: &dyn AnyQuery) {
		if self.listeners.is_empty() {
			return;
		}
		self.listeners.emit(&CacheNotification {
			event,
			query: query.info(),
		});
	}

	pub(crate) fn on_focus(&self) {
		for query in self.all() {
			query.on_focus();
		}
	}

	pub(crate) fn on_online(&self) {
		for query in self.all() {
			query.on_online();
		}
	}

	/// Data hydrated for a query that has not been created yet.
	pub(crate) fn pending_hydration(
		&self,
		scope: ScopeId,
		key_hash: u64,
	) -> Option<DehydratedQuery> {
		self.pending_hydration
			.lock()
			.get(&(scope, key_hash))
			.filter(|waiting| !waiting.is_expired(now_ms()))
			.map(|waiting| waiting.query.clone())
	}

	/// Hydrates a query now if it exists, or when it is created.
	///
	/// Waiting data is dropped once `gc_time` passes without its query being
	/// created, as an unobserved query would be. Without that, a tab synced
	/// with busier peers would keep every write it never reads.
	pub(crate) fn hydrate(&self, dehydrated: DehydratedQuery, gc_time: GcTime) {
		let id = (dehydrated.scope, dehydrated.key_hash);
		if let Some(query) = self.get(id.0, id.1) {
			query.hydrate(&dehydrated);
			return;
		}
		let now = now_ms();
		let expires_at = match gc_time {
			GcTime::Duration(gc_time) => {
				Some(now.saturating_add(u64::try_from(gc_time.as_millis()).unwrap_or(u64::MAX)))
			}
			GcTime::Never => None,
		};
		let mut pending = self.pending_hydration.lock();
		pending.retain(|_, waiting| !waiting.is_expired(now));
		// Older data never replaces newer, matching `Query::hydrate`.
		if pending
			.get(&id)
			.is_some_and(|waiting| waiting.query.data_updated_at >= dehydrated.data_updated_at)
		{
			return;
		}
		pending.insert(
			id,
			Waiting {
				query: dehydrated,
				expires_at,
			},
		);
	}
}

/// Dehydrated data waiting for its query, and when to stop waiting.
struct Waiting {
	query: DehydratedQuery,
	expires_at: Option<u64>,
}

impl Waiting {
	fn is_expired(&self, now: u64) -> bool {
		self.expires_at.is_some_and(|expires_at| now >= expires_at)
	}
}
