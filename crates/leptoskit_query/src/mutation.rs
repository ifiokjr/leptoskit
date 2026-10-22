//! Mutations: writes with lifecycle callbacks, retries, serial scopes, and
//! optimistic updates.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures::FutureExt;
use leptos::prelude::*;
use parking_lot::Mutex;

use crate::client::QueryClient;
use crate::client::use_query_client;
use crate::hydration::is_server;
use crate::key::ScopeId;
use crate::listeners::spawn;
use crate::managers::OnlineManager;
use crate::mutation_cache::AnyMutation;
use crate::mutation_cache::Mutation;
use crate::mutation_cache::MutationCacheInner;
use crate::mutation_cache::MutationEvent;
use crate::mutation_cache::MutationSignals;
use crate::mutation_cache::RunFuture;
use crate::options::GcTime;
use crate::options::NetworkMode;
use crate::options::QueryOptions;
use crate::retry::Retry;
use crate::retry::RetryDelay;
use crate::scope::QueryData;
use crate::scope::QueryError;
use crate::time::BoxFuture;
use crate::time::now_ms;
use crate::time::sleep;

/// Where a mutation is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MutationStatus {
	/// Never run, or reset.
	#[default]
	Idle,
	/// Running, including while paused or waiting for a serial scope.
	Pending,
	/// The last run failed.
	Error,
	/// The last run succeeded.
	Success,
}

/// A snapshot of one mutation.
#[derive(Clone, Debug)]
pub struct MutationState<V, T, E> {
	/// See [`MutationStatus`].
	pub status: MutationStatus,
	/// The result of the last successful run.
	pub data: Option<T>,
	/// The error of the last failed run.
	pub error: Option<E>,
	/// What the mutation was called with.
	pub variables: Option<V>,
	/// Consecutive failed attempts in the current or last run.
	pub failure_count: u32,
	/// The error of the most recent failed attempt, including attempts still
	/// being retried.
	pub failure_reason: Option<E>,
	/// Held back by the network mode or waiting its turn in a serial scope.
	pub is_paused: bool,
	/// When `mutate` was called, in epoch milliseconds.
	pub submitted_at: Option<u64>,
}

impl<V, T, E> MutationState<V, T, E> {
	/// Never run, or reset.
	#[must_use]
	pub fn is_idle(&self) -> bool {
		self.status == MutationStatus::Idle
	}

	/// Running.
	#[must_use]
	pub fn is_pending(&self) -> bool {
		self.status == MutationStatus::Pending
	}

	/// The last run succeeded.
	#[must_use]
	pub fn is_success(&self) -> bool {
		self.status == MutationStatus::Success
	}

	/// The last run failed.
	#[must_use]
	pub fn is_error(&self) -> bool {
		self.status == MutationStatus::Error
	}
}

/// What `on_mutate` receives.
#[derive(Clone, Debug)]
pub struct MutateArgs<V> {
	/// What the mutation was called with.
	pub variables: V,
	/// The client, for optimistic cache updates.
	pub client: QueryClient,
}

/// What `on_success` receives.
#[derive(Clone, Debug)]
pub struct SuccessArgs<T, V, C> {
	/// The mutation's result.
	pub data: T,
	/// What the mutation was called with.
	pub variables: V,
	/// What `on_mutate` returned.
	pub context: C,
	/// The client, for invalidating affected queries.
	pub client: QueryClient,
}

/// What `on_error` receives.
#[derive(Clone, Debug)]
pub struct ErrorArgs<E, V, C> {
	/// Why the mutation failed.
	pub error: E,
	/// What the mutation was called with.
	pub variables: V,
	/// What `on_mutate` returned — usually what to roll back to.
	pub context: C,
	/// The client, for rolling back optimistic updates.
	pub client: QueryClient,
}

/// What `on_settled` receives.
#[derive(Clone, Debug)]
pub struct SettledArgs<T, E, V, C> {
	/// The mutation's outcome.
	pub result: Result<T, E>,
	/// What the mutation was called with.
	pub variables: V,
	/// What `on_mutate` returned.
	pub context: C,
	/// The client, for invalidating affected queries either way.
	pub client: QueryClient,
}

type MutateFn<V, T, E> = Arc<dyn Fn(V) -> BoxFuture<'static, Result<T, E>> + Send + Sync>;
type OnMutateFn<V, C> = Arc<dyn Fn(MutateArgs<V>) -> BoxFuture<'static, C> + Send + Sync>;
type OnSuccessFn<T, V, C> =
	Arc<dyn Fn(SuccessArgs<T, V, C>) -> BoxFuture<'static, ()> + Send + Sync>;
type OnErrorFn<E, V, C> = Arc<dyn Fn(ErrorArgs<E, V, C>) -> BoxFuture<'static, ()> + Send + Sync>;
type OnSettledFn<T, E, V, C> =
	Arc<dyn Fn(SettledArgs<T, E, V, C>) -> BoxFuture<'static, ()> + Send + Sync>;

/// A mutation function with its callbacks and options: TanStack's
/// `useMutation` options.
///
/// `C` is the context type `on_mutate` returns; it is `()` until
/// [`with_on_mutate`](Self::with_on_mutate) is called. The `on_mutate`
/// callback runs before the mutation and its return value — the *context* —
/// is handed to `on_success`, `on_error`, and `on_settled`, which is how an
/// optimistic update remembers what to roll back:
///
/// ```
/// use leptoskit_query::*;
///
/// # #[derive(Clone, Debug, PartialEq)] struct Todo { id: u32, title: String }
/// # #[derive(Clone, Debug)] struct ApiError;
/// # async fn list_todos() -> Result<Vec<Todo>, ApiError> { Ok(vec![]) }
/// # async fn create_todo(title: String) -> Result<Todo, ApiError> { Ok(Todo { id: 1, title }) }
/// fn todos() -> QueryScope<(), Vec<Todo>, ApiError> {
/// 	QueryScope::new(|()| async move { list_todos().await })
/// }
///
/// fn add_todo() -> MutationScope<String, Todo, ApiError, Option<Vec<Todo>>> {
/// 	MutationScope::new(|title: String| async move { create_todo(title).await })
/// 		.with_on_mutate_sync(|args| {
/// 			// Optimistically append; remember the previous list to roll back.
/// 			let previous = args.client.get_query_data(&todos(), &());
/// 			args.client.update_query_data(&todos(), (), |list| {
/// 				let mut list = list.unwrap_or_default();
/// 				list.push(Todo { id: 0, title: args.variables.clone() });
/// 				Some(list)
/// 			});
/// 			previous
/// 		})
/// 		.with_on_error(|args| {
/// 			if let Some(previous) = args.context {
/// 				args.client.set_query_data(&todos(), (), previous);
/// 			}
/// 		})
/// 		.with_on_settled_async(|args| async move {
/// 			args.client.invalidate_scope(&todos()).await;
/// 		})
/// }
/// ```
pub struct MutationScope<V, T, E, C = ()> {
	id: ScopeId,
	name: Arc<str>,
	mutate: MutateFn<V, T, E>,
	on_mutate: OnMutateFn<V, C>,
	on_success: Option<OnSuccessFn<T, V, C>>,
	on_error: Option<OnErrorFn<E, V, C>>,
	on_settled: Option<OnSettledFn<T, E, V, C>>,
	retry: Option<Retry<E>>,
	retry_delay: Option<RetryDelay<E>>,
	gc_time: Option<GcTime>,
	network_mode: Option<NetworkMode>,
	serial: Option<Arc<str>>,
}

impl<V, T, E, C> Clone for MutationScope<V, T, E, C> {
	fn clone(&self) -> Self {
		Self {
			id: self.id,
			name: Arc::clone(&self.name),
			mutate: Arc::clone(&self.mutate),
			on_mutate: Arc::clone(&self.on_mutate),
			on_success: self.on_success.clone(),
			on_error: self.on_error.clone(),
			on_settled: self.on_settled.clone(),
			retry: self.retry.clone(),
			retry_delay: self.retry_delay.clone(),
			gc_time: self.gc_time,
			network_mode: self.network_mode,
			serial: self.serial.clone(),
		}
	}
}

impl<V, T, E, C> fmt::Debug for MutationScope<V, T, E, C> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("MutationScope")
			.field("id", &self.id)
			.field("name", &self.name)
			.field("retry", &self.retry)
			.field("serial", &self.serial)
			.finish_non_exhaustive()
	}
}

impl<V, T, E> MutationScope<V, T, E, ()>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
{
	/// A scope over a mutation function.
	pub fn new<F, Fut>(mutate: F) -> Self
	where
		F: Fn(V) -> Fut + Send + Sync + 'static,
		Fut: Future<Output = Result<T, E>> + Send + 'static,
	{
		Self {
			id: ScopeId::of::<F, V, T, E>(),
			name: Arc::from(std::any::type_name::<F>()),
			mutate: Arc::new(move |variables| Box::pin(mutate(variables))),
			on_mutate: Arc::new(|_| Box::pin(std::future::ready(()))),
			on_success: None,
			on_error: None,
			on_settled: None,
			retry: None,
			retry_delay: None,
			gc_time: None,
			network_mode: None,
			serial: None,
		}
	}

	/// Runs before the mutation; what it returns is the *context* every later
	/// callback receives. Apply optimistic updates here and return what is
	/// needed to roll them back.
	///
	/// Changes the scope's context type, so call it before the other
	/// callbacks or they will not see the context.
	#[must_use]
	pub fn with_on_mutate<C2, F, Fut>(self, on_mutate: F) -> MutationScope<V, T, E, C2>
	where
		C2: QueryData,
		F: Fn(MutateArgs<V>) -> Fut + Send + Sync + 'static,
		Fut: Future<Output = C2> + Send + 'static,
	{
		MutationScope {
			id: self.id,
			name: self.name,
			mutate: self.mutate,
			on_mutate: Arc::new(move |args| Box::pin(on_mutate(args))),
			on_success: self.on_success.map(|on_success| {
				Arc::new(move |args: SuccessArgs<T, V, C2>| {
					on_success(SuccessArgs {
						data: args.data,
						variables: args.variables,
						context: (),
						client: args.client,
					})
				}) as OnSuccessFn<T, V, C2>
			}),
			on_error: self.on_error.map(|on_error| {
				Arc::new(move |args: ErrorArgs<E, V, C2>| {
					on_error(ErrorArgs {
						error: args.error,
						variables: args.variables,
						context: (),
						client: args.client,
					})
				}) as OnErrorFn<E, V, C2>
			}),
			on_settled: self.on_settled.map(|on_settled| {
				Arc::new(move |args: SettledArgs<T, E, V, C2>| {
					on_settled(SettledArgs {
						result: args.result,
						variables: args.variables,
						context: (),
						client: args.client,
					})
				}) as OnSettledFn<T, E, V, C2>
			}),
			retry: self.retry,
			retry_delay: self.retry_delay,
			gc_time: self.gc_time,
			network_mode: self.network_mode,
			serial: self.serial,
		}
	}

	/// [`with_on_mutate`](Self::with_on_mutate) for a synchronous callback.
	#[must_use]
	pub fn with_on_mutate_sync<C2>(
		self,
		on_mutate: impl Fn(MutateArgs<V>) -> C2 + Send + Sync + 'static,
	) -> MutationScope<V, T, E, C2>
	where
		C2: QueryData,
	{
		self.with_on_mutate(move |args| std::future::ready(on_mutate(args)))
	}
}

impl<V, T, E, C> MutationScope<V, T, E, C>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	/// Runs after a successful mutation, before the handle reports success.
	#[must_use]
	pub fn with_on_success(
		self,
		on_success: impl Fn(SuccessArgs<T, V, C>) + Send + Sync + 'static,
	) -> Self {
		let on_success = Arc::new(on_success);
		self.with_on_success_async(move |args| {
			let on_success = Arc::clone(&on_success);
			async move {
				on_success(args);
			}
		})
	}

	/// Runs after a successful mutation and is awaited before the handle
	/// reports success — so `is_pending` covers an awaited invalidation.
	#[must_use]
	pub fn with_on_success_async<Fut>(
		mut self,
		on_success: impl Fn(SuccessArgs<T, V, C>) -> Fut + Send + Sync + 'static,
	) -> Self
	where
		Fut: Future<Output = ()> + Send + 'static,
	{
		self.on_success = Some(Arc::new(move |args| Box::pin(on_success(args))));
		self
	}

	/// Runs after a failed mutation, before the handle reports the error.
	#[must_use]
	pub fn with_on_error(
		self,
		on_error: impl Fn(ErrorArgs<E, V, C>) + Send + Sync + 'static,
	) -> Self {
		let on_error = Arc::new(on_error);
		self.with_on_error_async(move |args| {
			let on_error = Arc::clone(&on_error);
			async move {
				on_error(args);
			}
		})
	}

	/// Runs after a failed mutation and is awaited before the handle reports
	/// the error.
	#[must_use]
	pub fn with_on_error_async<Fut>(
		mut self,
		on_error: impl Fn(ErrorArgs<E, V, C>) -> Fut + Send + Sync + 'static,
	) -> Self
	where
		Fut: Future<Output = ()> + Send + 'static,
	{
		self.on_error = Some(Arc::new(move |args| Box::pin(on_error(args))));
		self
	}

	/// Runs after success or failure, after `on_success` or `on_error`.
	#[must_use]
	pub fn with_on_settled(
		self,
		on_settled: impl Fn(SettledArgs<T, E, V, C>) + Send + Sync + 'static,
	) -> Self {
		let on_settled = Arc::new(on_settled);
		self.with_on_settled_async(move |args| {
			let on_settled = Arc::clone(&on_settled);
			async move {
				on_settled(args);
			}
		})
	}

	/// Runs after success or failure and is awaited before the handle
	/// reports the outcome.
	#[must_use]
	pub fn with_on_settled_async<Fut>(
		mut self,
		on_settled: impl Fn(SettledArgs<T, E, V, C>) -> Fut + Send + Sync + 'static,
	) -> Self
	where
		Fut: Future<Output = ()> + Send + 'static,
	{
		self.on_settled = Some(Arc::new(move |args| Box::pin(on_settled(args))));
		self
	}

	/// Gives the scope a stable identity and readable name.
	#[must_use]
	pub fn with_name(mut self, name: &str) -> Self {
		self.id = ScopeId::named(name);
		self.name = Arc::from(name);
		self
	}

	/// Replaces the scope's options.
	#[must_use]
	pub fn with_options(mut self, options: QueryOptions<E>) -> Self {
		self.retry = options.retry;
		self.retry_delay = options.retry_delay;
		self.gc_time = options.gc_time;
		self.network_mode = options.network_mode;
		self
	}

	/// See [`Retry`]. Mutations do not retry by default: blindly re-sending a
	/// write is usually wrong.
	#[must_use]
	pub fn with_retry(mut self, retry: impl Into<Retry<E>>) -> Self {
		self.retry = Some(retry.into());
		self
	}

	/// See [`RetryDelay`].
	#[must_use]
	pub fn with_retry_delay(mut self, retry_delay: impl Into<RetryDelay<E>>) -> Self {
		self.retry_delay = Some(retry_delay.into());
		self
	}

	/// How long a settled mutation stays in the cache for
	/// [`use_mutation_state`](crate::use_mutation_state); five minutes by
	/// default.
	#[must_use]
	pub fn with_gc_time(mut self, gc_time: impl Into<GcTime>) -> Self {
		self.gc_time = Some(gc_time.into());
		self
	}

	/// See [`NetworkMode`].
	#[must_use]
	pub fn with_network_mode(mut self, network_mode: NetworkMode) -> Self {
		self.network_mode = Some(network_mode);
		self
	}

	/// Mutations sharing a serial scope run one at a time in submission
	/// order; the rest wait as paused.
	#[must_use]
	pub fn with_serial_scope(mut self, scope: &str) -> Self {
		self.serial = Some(Arc::from(scope));
		self
	}

	/// The scope's cache identity.
	#[must_use]
	pub fn id(&self) -> ScopeId {
		self.id
	}

	/// The scope's readable name.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.name
	}
}

/// Callbacks for one `mutate` call, fired after the scope's own.
///
/// They run only for this call, which is where "close the dialog on success"
/// belongs when the scope is shared.
// The `on_` prefixes are the point: they match TanStack's callback names.
#[allow(clippy::struct_field_names)]
pub struct MutateCallbacks<V, T, E, C = ()> {
	on_success: CallOnSuccess<V, T, C>,
	on_error: CallOnError<E, V, C>,
	on_settled: CallOnSettled<T, E, V, C>,
}

type CallOnSuccess<V, T, C> = Option<Box<dyn FnOnce(SuccessArgs<T, V, C>) + Send>>;
type CallOnError<E, V, C> = Option<Box<dyn FnOnce(ErrorArgs<E, V, C>) + Send>>;
type CallOnSettled<T, E, V, C> = Option<Box<dyn FnOnce(SettledArgs<T, E, V, C>) + Send>>;

impl<V, T, E, C> Default for MutateCallbacks<V, T, E, C> {
	fn default() -> Self {
		Self {
			on_success: None,
			on_error: None,
			on_settled: None,
		}
	}
}

impl<V, T, E, C> fmt::Debug for MutateCallbacks<V, T, E, C> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("MutateCallbacks")
			.field("on_success", &self.on_success.is_some())
			.field("on_error", &self.on_error.is_some())
			.field("on_settled", &self.on_settled.is_some())
			.finish()
	}
}

impl<V, T, E, C> MutateCallbacks<V, T, E, C> {
	/// No callbacks.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Runs if this call succeeds.
	#[must_use]
	pub fn on_success(
		mut self,
		on_success: impl FnOnce(SuccessArgs<T, V, C>) + Send + 'static,
	) -> Self {
		self.on_success = Some(Box::new(on_success));
		self
	}

	/// Runs if this call fails.
	#[must_use]
	pub fn on_error(mut self, on_error: impl FnOnce(ErrorArgs<E, V, C>) + Send + 'static) -> Self {
		self.on_error = Some(Box::new(on_error));
		self
	}

	/// Runs when this call settles either way.
	#[must_use]
	pub fn on_settled(
		mut self,
		on_settled: impl FnOnce(SettledArgs<T, E, V, C>) + Send + 'static,
	) -> Self {
		self.on_settled = Some(Box::new(on_settled));
		self
	}
}

type CurrentMutation<V, T, E, C> = ArcRwSignal<Option<Arc<Mutation<V, T, E, C>>>>;

struct MutationHandleInner<V, T, E, C> {
	client: QueryClient,
	scope: MutationScope<V, T, E, C>,
	current: CurrentMutation<V, T, E, C>,
}

/// A live view of a mutation scope, returned by [`use_mutation`].
///
/// `Copy`, like a leptos signal; every copy reports the most recent call.
/// After the owning component unmounts, reads report an idle mutation and
/// the fire-and-forget `mutate` methods do nothing.
pub struct MutationHandle<V, T, E, C = ()> {
	inner: StoredValue<Arc<MutationHandleInner<V, T, E, C>>>,
}

impl<V, T, E, C> Clone for MutationHandle<V, T, E, C> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<V, T, E, C> Copy for MutationHandle<V, T, E, C> {}

impl<V, T, E, C> fmt::Debug for MutationHandle<V, T, E, C>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.inner() {
			Some(inner) => {
				f.debug_struct("MutationHandle")
					.field("scope", &inner.scope)
					.finish()
			}
			None => f.write_str("MutationHandle(disposed)"),
		}
	}
}

impl<V, T, E, C> MutationHandle<V, T, E, C>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	fn new(client: QueryClient, scope: MutationScope<V, T, E, C>) -> Self {
		Self {
			inner: StoredValue::new(Arc::new(MutationHandleInner {
				client,
				scope,
				current: ArcRwSignal::new(None),
			})),
		}
	}

	fn inner(&self) -> Option<Arc<MutationHandleInner<V, T, E, C>>> {
		self.inner.try_get_value()
	}

	fn expect_inner(&self) -> Arc<MutationHandleInner<V, T, E, C>> {
		self.inner()
			.expect("this MutationHandle belongs to a component that has been disposed")
	}

	// ---- acting ------------------------------------------------------------

	/// Runs the mutation. Fire and forget; the handle reports progress. Does
	/// nothing once the owning component is disposed.
	pub fn mutate(&self, variables: V) {
		if let Some(inner) = self.inner() {
			drop(Self::start(&inner, variables, MutateCallbacks::default()));
		}
	}

	/// Runs the mutation and resolves with its outcome.
	///
	/// # Panics
	///
	/// If the owning component has been disposed.
	pub fn mutate_async(
		&self,
		variables: V,
	) -> impl Future<Output = Result<T, E>> + Send + 'static + use<V, T, E, C> {
		Self::start(&self.expect_inner(), variables, MutateCallbacks::default())
	}

	/// Runs the mutation with callbacks for this call only. Does nothing once
	/// the owning component is disposed.
	pub fn mutate_with(&self, variables: V, callbacks: MutateCallbacks<V, T, E, C>) {
		if let Some(inner) = self.inner() {
			drop(Self::start(&inner, variables, callbacks));
		}
	}

	/// Runs the mutation with callbacks for this call only and resolves with
	/// its outcome.
	///
	/// # Panics
	///
	/// If the owning component has been disposed.
	pub fn mutate_with_async(
		&self,
		variables: V,
		callbacks: MutateCallbacks<V, T, E, C>,
	) -> impl Future<Output = Result<T, E>> + Send + 'static + use<V, T, E, C> {
		Self::start(&self.expect_inner(), variables, callbacks)
	}

	fn start(
		inner: &Arc<MutationHandleInner<V, T, E, C>>,
		variables: V,
		callbacks: MutateCallbacks<V, T, E, C>,
	) -> impl Future<Output = Result<T, E>> + Send + 'static + use<V, T, E, C> {
		let client = inner.client.clone();
		let cache = Arc::clone(client.mutations());
		let scope = inner.scope.clone();
		let mutation = Arc::new(Mutation {
			id: cache.next_id.fetch_add(1, Ordering::Relaxed),
			scope_id: scope.id,
			scope_name: Arc::clone(&scope.name),
			signals: MutationSignals {
				status: ArcRwSignal::new(MutationStatus::Pending),
				data: ArcRwSignal::new(None),
				error: ArcRwSignal::new(None),
				variables: ArcRwSignal::new(Some(variables.clone())),
				failure_count: ArcRwSignal::new(0),
				failure_reason: ArcRwSignal::new(None),
				is_paused: ArcRwSignal::new(false),
				submitted_at: ArcRwSignal::new(Some(now_ms())),
			},
			context: ArcRwSignal::new(None),
			run: Mutex::new(None),
		});
		let boxed: BoxFuture<'static, ()> = Box::pin(run_mutation(
			Arc::clone(&mutation),
			scope,
			variables,
			client,
			Arc::clone(&cache),
			callbacks,
		));
		let run: RunFuture = boxed.shared();
		*mutation.run.lock() = Some(run.clone());
		// Counted before the task runs, so `is_mutating` is right the moment
		// `mutate` returns.
		cache.pending.update(|count| *count += 1);
		cache.add(&(Arc::clone(&mutation) as Arc<dyn AnyMutation>));
		inner.current.set(Some(Arc::clone(&mutation)));
		spawn(run.clone());
		async move {
			run.await;
			mutation.outcome()
		}
	}

	/// Forgets the last call: the handle is idle again.
	pub fn reset(&self) {
		if let Some(inner) = self.inner() {
			inner.current.set(None);
		}
	}

	// ---- reading -----------------------------------------------------------

	fn with_current<R>(&self, read: impl FnOnce(Option<&Arc<Mutation<V, T, E, C>>>) -> R) -> R {
		match self.inner() {
			Some(inner) => inner.current.with(|current| read(current.as_ref())),
			None => read(None),
		}
	}

	/// See [`MutationStatus`].
	pub fn status(&self) -> MutationStatus {
		self.with_current(|mutation| {
			mutation.map_or(MutationStatus::Idle, |mutation| {
				mutation.signals.status.get()
			})
		})
	}

	/// The result of the last successful call.
	pub fn data(&self) -> Option<T> {
		self.with_current(|mutation| mutation.and_then(|mutation| mutation.signals.data.get()))
	}

	/// The error of the last failed call.
	pub fn error(&self) -> Option<E> {
		self.with_current(|mutation| mutation.and_then(|mutation| mutation.signals.error.get()))
	}

	/// What the last call was made with.
	pub fn variables(&self) -> Option<V> {
		self.with_current(|mutation| mutation.and_then(|mutation| mutation.signals.variables.get()))
	}

	/// What `on_mutate` returned for the last call.
	pub fn context(&self) -> Option<C> {
		self.with_current(|mutation| mutation.and_then(|mutation| mutation.context.get()))
	}

	/// Consecutive failed attempts in the current or last call.
	pub fn failure_count(&self) -> u32 {
		self.with_current(|mutation| {
			mutation.map_or(0, |mutation| mutation.signals.failure_count.get())
		})
	}

	/// The error of the most recent failed attempt, including attempts still
	/// being retried.
	pub fn failure_reason(&self) -> Option<E> {
		self.with_current(|mutation| {
			mutation.and_then(|mutation| mutation.signals.failure_reason.get())
		})
	}

	/// When the last call was made, in epoch milliseconds.
	pub fn submitted_at(&self) -> Option<u64> {
		self.with_current(|mutation| {
			mutation.and_then(|mutation| mutation.signals.submitted_at.get())
		})
	}

	/// Never called, or reset.
	pub fn is_idle(&self) -> bool {
		self.status() == MutationStatus::Idle
	}

	/// Running.
	pub fn is_pending(&self) -> bool {
		self.status() == MutationStatus::Pending
	}

	/// The last call succeeded.
	pub fn is_success(&self) -> bool {
		self.status() == MutationStatus::Success
	}

	/// The last call failed.
	pub fn is_error(&self) -> bool {
		self.status() == MutationStatus::Error
	}

	/// Held back by the network mode or a serial scope.
	pub fn is_paused(&self) -> bool {
		self.with_current(|mutation| {
			mutation.is_some_and(|mutation| mutation.signals.is_paused.get())
		})
	}

	/// Everything at once, as one reactive read.
	pub fn get(&self) -> MutationState<V, T, E> {
		self.with_current(|mutation| {
			mutation.map_or_else(
				|| {
					MutationState {
						status: MutationStatus::Idle,
						data: None,
						error: None,
						variables: None,
						failure_count: 0,
						failure_reason: None,
						is_paused: false,
						submitted_at: None,
					}
				},
				|mutation| mutation.state(),
			)
		})
	}

	/// The client this handle belongs to, if the handle is still alive.
	#[must_use]
	pub fn client(&self) -> Option<QueryClient> {
		self.inner().map(|inner| inner.client.clone())
	}

	/// The scope this handle runs, if the handle is still alive.
	#[must_use]
	pub fn scope(&self) -> Option<MutationScope<V, T, E, C>> {
		self.inner().map(|inner| inner.scope.clone())
	}
}

/// The whole life of one call: serial turn, `on_mutate`, network gating, the
/// retry loop, then callbacks in TanStack's order.
async fn run_mutation<V, T, E, C>(
	mutation: Arc<Mutation<V, T, E, C>>,
	scope: MutationScope<V, T, E, C>,
	variables: V,
	client: QueryClient,
	cache: Arc<MutationCacheInner>,
	callbacks: MutateCallbacks<V, T, E, C>,
) where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	let _permit = match &scope.serial {
		Some(name) => {
			mutation.set_paused(true);
			let permit = Arc::clone(&cache).acquire(Arc::clone(name)).await;
			mutation.set_paused(false);
			Some(permit)
		}
		None => None,
	};

	let context = (scope.on_mutate)(MutateArgs {
		variables: variables.clone(),
		client: client.clone(),
	})
	.await;
	mutation.context.set(Some(context.clone()));

	let network_mode = scope.network_mode.unwrap_or_default();
	if network_mode == NetworkMode::Online && !OnlineManager::is_online_untracked() {
		mutation.set_paused(true);
		OnlineManager::wait_until_online().await;
		mutation.set_paused(false);
	}

	let retry = scope.retry.clone().unwrap_or_else(Retry::never);
	let retry_delay = scope.retry_delay.clone().unwrap_or_default();
	let mut failures = 0_u32;
	let result = loop {
		match (scope.mutate)(variables.clone()).await {
			Ok(data) => break Ok(data),
			Err(error) => {
				let should_retry = retry.should_retry(failures, &error);
				let delay = retry_delay.delay(failures, &error);
				failures += 1;
				mutation.signals.failure_count.set(failures);
				mutation.signals.failure_reason.set(Some(error.clone()));
				if !should_retry {
					break Err(error);
				}
				sleep(delay).await;
				if network_mode != NetworkMode::Always && !OnlineManager::is_online_untracked() {
					mutation.set_paused(true);
					OnlineManager::wait_until_online().await;
					mutation.set_paused(false);
				}
			}
		}
	};

	// Scope callbacks run before the status flips, so `is_pending` covers an
	// awaited invalidation, exactly as in TanStack.
	match &result {
		Ok(data) => {
			if let Some(on_success) = &scope.on_success {
				on_success(SuccessArgs {
					data: data.clone(),
					variables: variables.clone(),
					context: context.clone(),
					client: client.clone(),
				})
				.await;
			}
		}
		Err(error) => {
			if let Some(on_error) = &scope.on_error {
				on_error(ErrorArgs {
					error: error.clone(),
					variables: variables.clone(),
					context: context.clone(),
					client: client.clone(),
				})
				.await;
			}
		}
	}
	if let Some(on_settled) = &scope.on_settled {
		on_settled(SettledArgs {
			result: result.clone(),
			variables: variables.clone(),
			context: context.clone(),
			client: client.clone(),
		})
		.await;
	}

	match &result {
		Ok(data) => {
			mutation.signals.data.set(Some(data.clone()));
			mutation.signals.error.set(None);
			mutation.signals.failure_reason.set(None);
			mutation.signals.status.set(MutationStatus::Success);
		}
		Err(error) => {
			mutation.signals.error.set(Some(error.clone()));
			mutation.signals.status.set(MutationStatus::Error);
		}
	}
	cache
		.pending
		.update(|count| *count = count.saturating_sub(1));
	cache.emit(MutationEvent::Updated, &*mutation);

	let MutateCallbacks {
		on_success,
		on_error,
		on_settled,
	} = callbacks;
	match &result {
		Ok(data) => {
			if let Some(on_success) = on_success {
				on_success(SuccessArgs {
					data: data.clone(),
					variables: variables.clone(),
					context: context.clone(),
					client: client.clone(),
				});
			}
		}
		Err(error) => {
			if let Some(on_error) = on_error {
				on_error(ErrorArgs {
					error: error.clone(),
					variables: variables.clone(),
					context: context.clone(),
					client: client.clone(),
				});
			}
		}
	}
	if let Some(on_settled) = on_settled {
		on_settled(SettledArgs {
			result,
			variables,
			context,
			client,
		});
	}

	// Settled mutations linger for `use_mutation_state`, then go.
	let gc_time = scope.gc_time.unwrap_or(if is_server() {
		GcTime::Never
	} else {
		GcTime::DEFAULT
	});
	if let GcTime::Duration(gc_time) = gc_time {
		let id = mutation.id;
		spawn(async move {
			sleep(gc_time).await;
			cache.remove(id);
		});
	}
}

/// A handle to run a [`MutationScope`] from a component.
///
/// ```
/// # use leptos::prelude::*;
/// # use leptoskit_query::*;
/// # #[derive(Clone, Debug)] struct Todo;
/// # #[derive(Clone, Debug)] struct ApiError;
/// # async fn create_todo(title: String) -> Result<Todo, ApiError> { let _ = title; Ok(Todo) }
/// # fn add_todo() -> MutationScope<String, Todo, ApiError> { MutationScope::new(|title: String| async move { create_todo(title).await }) }
/// #[component]
/// fn AddTodo() -> impl IntoView {
/// 	let add = use_mutation(add_todo());
/// 	view! {
/// 		<button
/// 			disabled=move || add.is_pending()
/// 			on:click=move |_| add.mutate("Buy milk".to_owned())
/// 		>
/// 			"Add"
/// 		</button>
/// 	}
/// }
/// ```
#[track_caller]
pub fn use_mutation<V, T, E, C>(scope: MutationScope<V, T, E, C>) -> MutationHandle<V, T, E, C>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	MutationHandle::new(use_query_client(), scope)
}

/// [`use_mutation`] against an explicit client instead of the one in
/// context.
pub fn use_mutation_in<V, T, E, C>(
	client: QueryClient,
	scope: MutationScope<V, T, E, C>,
) -> MutationHandle<V, T, E, C>
where
	V: QueryData,
	T: QueryData,
	E: QueryError,
	C: QueryData,
{
	MutationHandle::new(client, scope)
}
