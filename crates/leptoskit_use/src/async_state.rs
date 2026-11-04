//! Executor-independent asynchronous state with owner-scoped reactive
//! signals.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;

/// The state of one asynchronous operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncState<T, E> {
	/// An operation is running; the previous outcome is no longer current.
	Pending,
	/// The last operation succeeded with this value, or no operation has run
	/// and this is the initial value.
	Success(T),
	/// The last operation failed with this error.
	Error(E),
}

/// An asynchronous action the caller awaits or schedules on its host
/// executor.
pub type AsyncAction = Pin<Box<dyn Future<Output = ()> + 'static>>;

/// Reactive asynchronous state whose operation the caller drives.
pub struct UseAsyncStateReturn<T: Send + Sync + 'static, E> {
	/// Current state, starting as `Success(initial)`.
	pub state: RwSignal<AsyncState<T, E>>,
	/// Run the operation once: set `Pending`, await the fetch, then publish
	/// its outcome.
	pub execute: Arc<dyn Fn() -> AsyncAction + Send + Sync>,
}

/// Load reactive state from an asynchronous operation.
///
/// Call this function inside an owner and await the returned future to obtain
/// the handle; the signal is allocated before the first await, preserving
/// owner scope. Await each `execute` future on the host executor. The
/// state starts as `Success(initial)`. Every execution sets `Pending` before
/// awaiting and publishes `Success` or `Error` after. Only the latest
/// execution publishes: one that settles after a later one started is
/// ignored, so a slow first request cannot overwrite a newer result. Dropping
/// an execution
/// future leaves the state at `Pending`; if the owner is disposed while an
/// operation is in flight, the fetch still runs but the signal update is
/// skipped.
///
/// No task is spawned internally. Browser, native, SSR, and custom hosts
/// choose how to run each action.
pub fn use_async_state<T, E, F, Fut>(
	initial: T,
	fetch: F,
) -> impl Future<Output = UseAsyncStateReturn<T, E>>
where
	T: Send + Sync + 'static,
	E: Send + Sync + 'static,
	F: Fn() -> Fut + Send + Sync + 'static,
	Fut: Future<Output = Result<T, E>> + 'static,
{
	let state = RwSignal::new(AsyncState::Success(initial));
	async move {
		let fetch = Arc::new(fetch);
		let latest = Arc::new(AtomicU64::new(0));
		UseAsyncStateReturn {
			state,
			execute: Arc::new(move || {
				let fetch = Arc::clone(&fetch);
				let latest = Arc::clone(&latest);
				Box::pin(async move {
					let execution = latest.fetch_add(1, Ordering::Relaxed) + 1;

					if state.try_set(AsyncState::Pending).is_some() {
						return;
					}

					let outcome = fetch().await;

					if latest.load(Ordering::Relaxed) != execution {
						return;
					}

					match outcome {
						Ok(value) => {
							state.try_set(AsyncState::Success(value));
						}
						Err(error) => {
							state.try_set(AsyncState::Error(error));
						}
					}
				})
			}),
		}
	}
}
