//! Executor-independent asynchronous storage with owner-scoped reactive state.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures::lock::Mutex;
use reactive_graph::prelude::*;
use reactive_graph::signal::ReadSignal;
use reactive_graph::signal::RwSignal;

use crate::MemoryStorage;
use crate::Storage;

/// A storage operation polled by the caller's executor.
///
/// Futures may be local to a browser event loop. Native adapters can offload
/// blocking work without imposing that executor on application code.
pub type StorageFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Typed asynchronous storage, independent of renderer and executor.
///
/// Implementations own encoding and persistence. Failed writes must preserve
/// the previously committed value. A cancelled operation may still commit in
/// the backend; call `reload` to observe it. The `_async` suffix lets a store
/// also implement synchronous [`Storage`] without ambiguous method calls.
pub trait AsyncStorage<T>: Send + Sync + 'static {
	/// The adapter's concrete error type.
	type Error: Send + Sync + 'static;
	/// Read a value; `None` means the key is absent.
	fn get_async(&self, key: String) -> StorageFuture<'_, Result<Option<T>, Self::Error>>;
	/// Persist a value.
	fn set_async(&self, key: String, value: T) -> StorageFuture<'_, Result<(), Self::Error>>;
	/// Remove a key. Removing a missing key succeeds.
	fn remove_async(&self, key: String) -> StorageFuture<'_, Result<(), Self::Error>>;
}

impl<T, S: AsyncStorage<T> + ?Sized> AsyncStorage<T> for Arc<S> {
	type Error = S::Error;

	fn get_async(&self, key: String) -> StorageFuture<'_, Result<Option<T>, Self::Error>> {
		self.as_ref().get_async(key)
	}

	fn set_async(&self, key: String, value: T) -> StorageFuture<'_, Result<(), Self::Error>> {
		self.as_ref().set_async(key, value)
	}

	fn remove_async(&self, key: String) -> StorageFuture<'_, Result<(), Self::Error>> {
		self.as_ref().remove_async(key)
	}
}

impl<T: Clone + Send + Sync + 'static> AsyncStorage<T> for MemoryStorage<T> {
	type Error = std::convert::Infallible;

	fn get_async(&self, key: String) -> StorageFuture<'_, Result<Option<T>, Self::Error>> {
		Box::pin(std::future::ready(self.get(&key)))
	}

	fn set_async(&self, key: String, value: T) -> StorageFuture<'_, Result<(), Self::Error>> {
		Box::pin(std::future::ready(self.set(&key, &value)))
	}

	fn remove_async(&self, key: String) -> StorageFuture<'_, Result<(), Self::Error>> {
		Box::pin(std::future::ready(self.remove(&key)))
	}
}

/// An asynchronous storage failure, including disposal while I/O was pending.
#[derive(Debug, thiserror::Error)]
pub enum AsyncStorageError<E> {
	/// The underlying storage operation failed.
	#[error("storage backend failed: {0}")]
	Backend(E),
	/// The reactive owner was cleaned up before the operation committed to
	/// state.
	#[error("storage owner was disposed")]
	Disposed,
}

/// An asynchronous action the caller awaits or schedules on its host executor.
pub type AsyncStorageAction<E> =
	Arc<dyn Fn() -> StorageFuture<'static, Result<(), AsyncStorageError<E>>> + Send + Sync>;

/// An asynchronous setter the caller awaits or schedules on its host executor.
pub type AsyncStorageSetter<T, E> =
	Arc<dyn Fn(T) -> StorageFuture<'static, Result<(), AsyncStorageError<E>>> + Send + Sync>;

/// Reactive storage state whose operations use the host's async storage
/// adapter.
pub struct UseAsyncStorageReturn<T: Send + Sync + 'static, E> {
	/// Last successfully loaded or persisted value.
	pub value: ReadSignal<T>,
	/// Persist first, then update the signal.
	pub set_value: AsyncStorageSetter<T, E>,
	/// Remove the key, then reset to the initial fallback.
	pub remove: AsyncStorageAction<E>,
	/// Refresh after a change made by another instance or host notification.
	pub reload: AsyncStorageAction<E>,
}

impl<T: Send + Sync + 'static, E> Clone for UseAsyncStorageReturn<T, E> {
	fn clone(&self) -> Self {
		Self {
			value: self.value,
			set_value: Arc::clone(&self.set_value),
			remove: Arc::clone(&self.remove),
			reload: Arc::clone(&self.reload),
		}
	}
}

struct State<T: Send + Sync + 'static, S> {
	storage: S,
	key: String,
	initial: T,
	value: RwSignal<T>,
	operation: Mutex<()>,
}

enum Operation<T> {
	Set(T),
	Remove,
	Reload,
}

impl<T, S> State<T, S>
where
	T: Clone + Send + Sync + 'static,
	S: AsyncStorage<T>,
{
	/// Serialize this hook's operations across awaits so reload cannot overtake
	/// a save.
	async fn apply(&self, operation: Operation<T>) -> Result<(), AsyncStorageError<S::Error>> {
		let _guard = self.operation.lock().await;

		if self.value.is_disposed() {
			return Err(AsyncStorageError::Disposed);
		}

		let next = match operation {
			Operation::Set(next) => {
				self.storage
					.set_async(self.key.clone(), next.clone())
					.await
					.map_err(AsyncStorageError::Backend)?;
				next
			}
			Operation::Remove => {
				self.storage
					.remove_async(self.key.clone())
					.await
					.map_err(AsyncStorageError::Backend)?;
				self.initial.clone()
			}
			Operation::Reload => {
				self.storage
					.get_async(self.key.clone())
					.await
					.map_err(AsyncStorageError::Backend)?
					.unwrap_or_else(|| self.initial.clone())
			}
		};

		if self.value.try_set(next).is_some() {
			return Err(AsyncStorageError::Disposed);
		}

		Ok(())
	}
}

/// Load reactive state from asynchronous host storage.
///
/// Call this function inside an owner, then await the returned future on the
/// host executor. Signals are allocated before the first await, preserving
/// owner scope even when initialization resumes outside it. The initial read
/// must succeed before the handle is returned; absent keys use `initial`
/// without writing. Operations on a handle are serialized. Errors preserve
/// its last successful value. Dropping a future prevents its later signal
/// update; backend writes already in progress may still finish.
///
/// No task is spawned internally. Browser, native, SSR, and custom hosts can
/// supply their own [`AsyncStorage`] and choose how to run each action.
pub fn use_async_storage<T, S>(
	storage: S,
	key: impl Into<String>,
	initial: T,
) -> impl Future<Output = Result<UseAsyncStorageReturn<T, S::Error>, AsyncStorageError<S::Error>>>
where
	T: Clone + Send + Sync + 'static,
	S: AsyncStorage<T>,
{
	let state = Arc::new(State {
		storage,
		key: key.into(),
		value: RwSignal::new(initial.clone()),
		initial,
		operation: Mutex::new(()),
	});

	async move {
		state.apply(Operation::Reload).await?;
		let writer = Arc::clone(&state);
		let remover = Arc::clone(&state);
		let value = state.value.read_only();

		Ok(UseAsyncStorageReturn {
			value,
			set_value: Arc::new(move |next| {
				let state = Arc::clone(&writer);
				Box::pin(async move { state.apply(Operation::Set(next)).await })
			}),
			remove: Arc::new(move || {
				let state = Arc::clone(&remover);
				Box::pin(async move { state.apply(Operation::Remove).await })
			}),
			reload: Arc::new(move || {
				let state = Arc::clone(&state);
				Box::pin(async move { state.apply(Operation::Reload).await })
			}),
		})
	}
}
