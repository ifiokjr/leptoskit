//! The mutation cache: every call in flight or recently settled, the serial
//! queues, and the client methods that read them.

use std::any::Any;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::Shared;
use futures::future::join_all;
use leptos::prelude::*;
use parking_lot::Mutex;
use parking_lot::RwLock;

use crate::client::QueryClient;
use crate::client::use_query_client;
use crate::key::ScopeId;
use crate::listeners::Listeners;
use crate::listeners::Subscription;
use crate::mutation::MutationScope;
use crate::mutation::MutationState;
use crate::mutation::MutationStatus;
use crate::scope::QueryData;
use crate::scope::QueryError;
use crate::time::BoxFuture;

/// Per-field signals of one mutation.
pub(crate) struct MutationSignals<V, T, E> {
	pub(crate) status: ArcRwSignal<MutationStatus>,
	pub(crate) data: ArcRwSignal<Option<T>>,
	pub(crate) error: ArcRwSignal<Option<E>>,
	pub(crate) variables: ArcRwSignal<Option<V>>,
	pub(crate) failure_count: ArcRwSignal<u32>,
	pub(crate) failure_reason: ArcRwSignal<Option<E>>,
	pub(crate) is_paused: ArcRwSignal<bool>,
	pub(crate) submitted_at: ArcRwSignal<Option<u64>>,
}

pub(crate) type RunFuture = Shared<BoxFuture<'static, ()>>;

pub(crate) struct Mutation<V, T, E, C> {
	pub(crate) id: u64,
	pub(crate) scope_id: ScopeId,
	pub(crate) scope_name: Arc<str>,
	pub(crate) signals: MutationSignals<V, T, E>,
	pub(crate) context: ArcRwSignal<Option<C>>,
	pub(crate) run: Mutex<Option<RunFuture>>,
}

impl<V, T, E, C> Mutation<V, T, E, C>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	pub(crate) fn state(&self) -> MutationState<V, T, E> {
		MutationState {
			status: self.signals.status.get(),
			data: self.signals.data.get(),
			error: self.signals.error.get(),
			variables: self.signals.variables.get(),
			failure_count: self.signals.failure_count.get(),
			failure_reason: self.signals.failure_reason.get(),
			is_paused: self.signals.is_paused.get(),
			submitted_at: self.signals.submitted_at.get(),
		}
	}

	pub(crate) fn state_untracked(&self) -> MutationState<V, T, E> {
		MutationState {
			status: self.signals.status.get_untracked(),
			data: self.signals.data.get_untracked(),
			error: self.signals.error.get_untracked(),
			variables: self.signals.variables.get_untracked(),
			failure_count: self.signals.failure_count.get_untracked(),
			failure_reason: self.signals.failure_reason.get_untracked(),
			is_paused: self.signals.is_paused.get_untracked(),
			submitted_at: self.signals.submitted_at.get_untracked(),
		}
	}

	pub(crate) fn set_paused(&self, paused: bool) {
		if self.signals.is_paused.get_untracked() != paused {
			self.signals.is_paused.set(paused);
		}
	}

	pub(crate) fn outcome(&self) -> Result<T, E> {
		if let Some(error) = self.signals.error.get_untracked() {
			return Err(error);
		}
		match self.signals.data.get_untracked() {
			Some(data) => Ok(data),
			None => unreachable!("a settled mutation has data or an error"),
		}
	}
}

/// A debugging snapshot of one cached mutation.
#[derive(Clone, Debug)]
pub struct MutationInfo {
	/// The mutation's id, increasing in submission order.
	pub id: u64,
	/// The scope's cache identity.
	pub scope: ScopeId,
	/// The scope's readable name.
	pub scope_name: String,
	/// See [`MutationStatus`].
	pub status: MutationStatus,
	/// Held back by the network mode or a serial scope.
	pub is_paused: bool,
	/// When `mutate` was called, in epoch milliseconds.
	pub submitted_at: Option<u64>,
	/// Consecutive failed attempts.
	pub failure_count: u32,
}

pub(crate) trait AnyMutation: Any + Send + Sync {
	fn id(&self) -> u64;
	fn scope_id(&self) -> ScopeId;
	fn status(&self) -> MutationStatus;
	fn is_paused(&self) -> bool;
	fn run(&self) -> Option<RunFuture>;
	fn info(&self) -> MutationInfo;
	fn as_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

impl<V, T, E, C> AnyMutation for Mutation<V, T, E, C>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	fn id(&self) -> u64 {
		self.id
	}

	fn scope_id(&self) -> ScopeId {
		self.scope_id
	}

	fn status(&self) -> MutationStatus {
		self.signals.status.get_untracked()
	}

	fn is_paused(&self) -> bool {
		self.signals.is_paused.get_untracked()
	}

	fn run(&self) -> Option<RunFuture> {
		self.run.lock().clone()
	}

	fn info(&self) -> MutationInfo {
		MutationInfo {
			id: self.id,
			scope: self.scope_id,
			scope_name: self.scope_name.to_string(),
			status: self.status(),
			is_paused: self.is_paused(),
			submitted_at: self.signals.submitted_at.get_untracked(),
			failure_count: self.signals.failure_count.get_untracked(),
		}
	}

	fn as_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
		self
	}
}

/// What changed in the mutation cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MutationEvent {
	/// A mutation was submitted.
	Added,
	/// A mutation's status changed.
	Updated,
	/// A mutation was garbage collected.
	Removed,
}

/// A mutation cache event and the mutation it concerns.
#[derive(Clone, Debug)]
pub struct MutationNotification {
	/// What happened.
	pub event: MutationEvent,
	/// The mutation it happened to.
	pub mutation: MutationInfo,
}

/// Selects mutations for [`QueryClient::mutating_count_matching`] and
/// [`QueryClient::mutation_infos`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MutationFilter {
	scope: Option<ScopeId>,
	status: Option<MutationStatus>,
}

impl MutationFilter {
	/// Every mutation.
	#[must_use]
	pub fn all() -> Self {
		Self::default()
	}

	/// Every mutation of one scope.
	#[must_use]
	pub fn scope<V, T, E, C>(scope: &MutationScope<V, T, E, C>) -> Self
	where
		V: QueryData,
		T: QueryData,
		E: QueryError,
		C: QueryData,
	{
		Self {
			scope: Some(scope.id()),
			status: None,
		}
	}

	/// Narrows by status.
	#[must_use]
	pub fn status(mut self, status: MutationStatus) -> Self {
		self.status = Some(status);
		self
	}

	/// Narrows to running mutations.
	#[must_use]
	pub fn pending(self) -> Self {
		self.status(MutationStatus::Pending)
	}

	fn matches(&self, mutation: &dyn AnyMutation) -> bool {
		self.scope.is_none_or(|scope| mutation.scope_id() == scope)
			&& self.status.is_none_or(|status| mutation.status() == status)
	}
}

#[derive(Default)]
struct SerialQueue {
	busy: bool,
	waiters: VecDeque<oneshot::Sender<()>>,
}

pub(crate) struct MutationCacheInner {
	mutations: RwLock<Vec<Arc<dyn AnyMutation>>>,
	pub(crate) pending: ArcRwSignal<usize>,
	pub(crate) version: ArcTrigger,
	serial: Mutex<HashMap<Arc<str>, SerialQueue>>,
	pub(crate) next_id: AtomicU64,
	listeners: Listeners<MutationNotification>,
}

impl MutationCacheInner {
	pub(crate) fn new() -> Arc<Self> {
		Arc::new(Self {
			mutations: RwLock::new(Vec::new()),
			pending: ArcRwSignal::new(0),
			version: ArcTrigger::new(),
			serial: Mutex::new(HashMap::new()),
			next_id: AtomicU64::new(1),
			listeners: Listeners::default(),
		})
	}

	pub(crate) fn add(&self, mutation: &Arc<dyn AnyMutation>) {
		self.mutations.write().push(Arc::clone(mutation));
		self.emit(MutationEvent::Added, mutation.as_ref());
		self.version.notify();
	}

	pub(crate) fn remove(&self, id: u64) {
		let removed = {
			let mut mutations = self.mutations.write();
			let index = mutations.iter().position(|mutation| mutation.id() == id);
			index.map(|index| mutations.remove(index))
		};
		if let Some(mutation) = removed {
			self.emit(MutationEvent::Removed, mutation.as_ref());
			self.version.notify();
		}
	}

	pub(crate) fn all(&self) -> Vec<Arc<dyn AnyMutation>> {
		self.mutations.read().clone()
	}

	fn find_all(&self, filter: MutationFilter) -> Vec<Arc<dyn AnyMutation>> {
		self.all()
			.into_iter()
			.filter(|mutation| filter.matches(&**mutation))
			.collect()
	}

	pub(crate) fn clear(&self) {
		let mutations: Vec<_> = self.mutations.write().drain(..).collect();
		for mutation in &mutations {
			self.emit(MutationEvent::Removed, &**mutation);
		}
		if !mutations.is_empty() {
			self.version.notify();
		}
	}

	pub(crate) fn emit(&self, event: MutationEvent, mutation: &dyn AnyMutation) {
		if self.listeners.is_empty() {
			return;
		}
		self.listeners.emit(&MutationNotification {
			event,
			mutation: mutation.info(),
		});
	}

	fn subscribe(
		&self,
		listener: impl Fn(&MutationNotification) + Send + Sync + 'static,
	) -> Subscription {
		self.listeners.subscribe(listener)
	}

	/// Waits for the turn of `name`'s serial queue.
	pub(crate) async fn acquire(self: Arc<Self>, name: Arc<str>) -> SerialPermit {
		let wait = {
			let mut queues = self.serial.lock();
			let queue = queues.entry(Arc::clone(&name)).or_default();
			if queue.busy {
				let (sender, receiver) = oneshot::channel();
				queue.waiters.push_back(sender);
				Some(receiver)
			} else {
				queue.busy = true;
				None
			}
		};
		if let Some(receiver) = wait {
			let _ = receiver.await;
		}
		SerialPermit { cache: self, name }
	}
}

/// Holding the turn of a serial scope; dropping it hands the turn on.
pub(crate) struct SerialPermit {
	cache: Arc<MutationCacheInner>,
	name: Arc<str>,
}

impl Drop for SerialPermit {
	fn drop(&mut self) {
		let mut queues = self.cache.serial.lock();
		let Some(queue) = queues.get_mut(&self.name) else {
			return;
		};
		while let Some(next) = queue.waiters.pop_front() {
			if next.send(()).is_ok() {
				// The turn passes directly; the queue stays busy.
				return;
			}
		}
		queues.remove(&self.name);
	}
}

/// How many mutations are running, as a signal.
#[must_use]
#[track_caller]
pub fn use_is_mutating() -> Signal<usize> {
	let count = use_query_client().mutating_count_signal();
	Signal::derive(move || count.get())
}

/// The state of every cached mutation of a scope, newest last, as a signal.
///
/// Running mutations are always present; settled ones linger for the scope's
/// gc time. Render `variables` of pending entries for optimistic UI without
/// touching the query cache.
#[must_use]
#[track_caller]
pub fn use_mutation_state<V, T, E, C>(
	scope: MutationScope<V, T, E, C>,
) -> Signal<Vec<MutationState<V, T, E>>>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	let client = use_query_client();
	Signal::derive(move || client.mutation_states_tracked(&scope))
}

impl QueryClient {
	/// How many mutations are running, without subscribing.
	#[must_use]
	pub fn mutating_count(&self) -> usize {
		self.mutations().pending.get_untracked()
	}

	/// How many matching mutations are running, without subscribing.
	#[must_use]
	pub fn mutating_count_matching(&self, filter: MutationFilter) -> usize {
		self.mutations().find_all(filter.pending()).len()
	}

	/// How many mutations are running, as a signal. [`use_is_mutating`] wraps
	/// this for components.
	#[must_use]
	pub fn mutating_count_signal(&self) -> ArcReadSignal<usize> {
		self.mutations().pending.read_only()
	}

	/// A debugging snapshot of matching cached mutations.
	#[must_use]
	pub fn mutation_infos(&self, filter: MutationFilter) -> Vec<MutationInfo> {
		self.mutations()
			.find_all(filter)
			.iter()
			.map(|mutation| mutation.info())
			.collect()
	}

	/// The state of every cached mutation of a scope, without subscribing.
	#[must_use]
	pub fn mutation_states<V, T, E, C>(
		&self,
		scope: &MutationScope<V, T, E, C>,
	) -> Vec<MutationState<V, T, E>>
	where
		V: QueryData,
		T: QueryData,
		E: QueryError,
		C: QueryData,
	{
		self.typed_mutations(scope)
			.iter()
			.map(|mutation| mutation.state_untracked())
			.collect()
	}

	fn mutation_states_tracked<V, T, E, C>(
		&self,
		scope: &MutationScope<V, T, E, C>,
	) -> Vec<MutationState<V, T, E>>
	where
		V: QueryData,
		T: QueryData,
		E: QueryError,
		C: QueryData,
	{
		self.mutations().version.track();
		self.typed_mutations(scope)
			.iter()
			.map(|mutation| mutation.state())
			.collect()
	}

	fn typed_mutations<V, T, E, C>(
		&self,
		scope: &MutationScope<V, T, E, C>,
	) -> Vec<Arc<Mutation<V, T, E, C>>>
	where
		V: QueryData,
		T: QueryData,
		E: QueryError,
		C: QueryData,
	{
		self.mutations()
			.find_all(MutationFilter::scope(scope))
			.into_iter()
			.filter_map(|mutation| mutation.as_any().downcast().ok())
			.collect()
	}

	/// Calls `listener` on every mutation cache event until the subscription
	/// drops.
	pub fn subscribe_mutations(
		&self,
		listener: impl Fn(&MutationNotification) + Send + Sync + 'static,
	) -> Subscription {
		self.mutations().subscribe(listener)
	}

	/// Resolves when every mutation paused by the network mode has settled.
	///
	/// Paused mutations resume on their own when
	/// [`OnlineManager::set_online`](crate::OnlineManager::set_online) reports
	/// the network back; this waits for them.
	pub fn resume_paused_mutations(&self) -> impl Future<Output = ()> + Send + 'static + use<> {
		let runs: Vec<RunFuture> = self
			.mutations()
			.all()
			.iter()
			.filter(|mutation| mutation.is_paused())
			.filter_map(|mutation| mutation.run())
			.collect();
		join_all(runs).map(|_| ())
	}
}
