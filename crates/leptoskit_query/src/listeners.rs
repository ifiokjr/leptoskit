//! Subscriber lists and spawning, shared by the caches and managers.

use std::future::Future;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptos::prelude::ScopedFuture;
use parking_lot::Mutex;

type Listener<A> = Arc<dyn Fn(&A) + Send + Sync>;
type ListenerList<A> = Arc<Mutex<Vec<(u64, Listener<A>)>>>;
type WeakListenerList<A> = Weak<Mutex<Vec<(u64, Listener<A>)>>>;

/// A list of callbacks with drop-to-unsubscribe handles.
pub(crate) struct Listeners<A> {
	inner: ListenerList<A>,
	next_id: AtomicU64,
}

impl<A> Default for Listeners<A> {
	fn default() -> Self {
		Self {
			inner: Arc::default(),
			next_id: AtomicU64::new(1),
		}
	}
}

impl<A: 'static> Listeners<A> {
	pub(crate) fn subscribe(&self, listener: impl Fn(&A) + Send + Sync + 'static) -> Subscription {
		let id = self.next_id.fetch_add(1, Ordering::Relaxed);
		self.inner.lock().push((id, Arc::new(listener)));
		let inner: WeakListenerList<A> = Arc::downgrade(&self.inner);
		Subscription {
			unsubscribe: Some(Box::new(move || {
				if let Some(inner) = inner.upgrade() {
					inner.lock().retain(|(entry, _)| *entry != id);
				}
			})),
		}
	}

	pub(crate) fn emit(&self, event: &A) {
		// Listeners are cloned out so a callback may subscribe or unsubscribe
		// without deadlocking on the list.
		let listeners: Vec<Listener<A>> = self
			.inner
			.lock()
			.iter()
			.map(|(_, listener)| Arc::clone(listener))
			.collect();
		for listener in listeners {
			listener(event);
		}
	}

	pub(crate) fn is_empty(&self) -> bool {
		self.inner.lock().is_empty()
	}
}

/// Keeps a subscription alive; dropping it unsubscribes.
#[must_use = "dropping a subscription unsubscribes it; call `forget` to keep it for the program's \
              lifetime"]
pub struct Subscription {
	unsubscribe: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl Subscription {
	/// Keeps the subscription for the rest of the program.
	pub fn forget(mut self) {
		self.unsubscribe = None;
	}
}

impl Drop for Subscription {
	fn drop(&mut self) {
		if let Some(unsubscribe) = self.unsubscribe.take() {
			unsubscribe();
		}
	}
}

impl std::fmt::Debug for Subscription {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Subscription")
			.field("active", &self.unsubscribe.is_some())
			.finish()
	}
}

/// Spawns a background task that keeps the current reactive owner, so
/// fetchers can read context, but tracks nothing: whatever effect happens to
/// be running when a fetch starts must not subscribe to signals the fetcher
/// reads.
///
/// `wasm-bindgen` only runs thread-local tasks, so the browser spawns locally
/// while every other target uses the thread-safe executor.
pub(crate) fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
	let fut = ScopedFuture::new_untracked(fut);
	#[cfg(not(target_family = "wasm"))]
	any_spawner::Executor::spawn(fut);
	#[cfg(target_family = "wasm")]
	any_spawner::Executor::spawn_local(fut);
}
