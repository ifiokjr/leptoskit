use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;

use futures::channel::oneshot;
use futures::executor::block_on;
use futures::task::noop_waker_ref;
use leptoskit_use::*;
use parking_lot::Mutex;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
	Read,
	Write,
	Remove,
}

type Gate = Option<(Kind, oneshot::Receiver<()>)>;

#[derive(Clone, Default)]
struct Controlled {
	values: MemoryStorage<i32>,
	gate: Arc<Mutex<Gate>>,
	fail_read: Arc<AtomicBool>,
	fail_write: Arc<AtomicBool>,
	fail_remove: Arc<AtomicBool>,
	reads: Arc<AtomicUsize>,
}

impl Controlled {
	fn suspend(&self, kind: Kind) -> oneshot::Sender<()> {
		let (send, receive) = oneshot::channel();
		*self.gate.lock() = Some((kind, receive));
		send
	}

	async fn wait(&self, kind: Kind) {
		let gate = {
			let mut gate = self.gate.lock();
			if gate.as_ref().is_some_and(|(current, _)| *current == kind) {
				gate.take()
			} else {
				None
			}
		};

		if let Some((_, receive)) = gate {
			receive.await.unwrap();
		}
	}
}

impl AsyncStorage<i32> for Controlled {
	type Error = &'static str;

	fn get_async(&self, key: String) -> StorageFuture<'_, Result<Option<i32>, Self::Error>> {
		// Browser adapters may return non-Send futures. Keep an Rc across the
		// suspension to prove that the hook does not impose Send on futures.
		let key = Rc::new(key);
		Box::pin(async move {
			self.reads.fetch_add(1, Ordering::SeqCst);
			self.wait(Kind::Read).await;
			if self.fail_read.load(Ordering::SeqCst) {
				return Err("read failed");
			}

			Ok(self.values.get(&key).unwrap())
		})
	}

	fn set_async(&self, key: String, value: i32) -> StorageFuture<'_, Result<(), Self::Error>> {
		Box::pin(async move {
			self.wait(Kind::Write).await;
			if self.fail_write.load(Ordering::SeqCst) {
				return Err("write failed");
			}

			self.values.set(&key, &value).unwrap();
			Ok(())
		})
	}

	fn remove_async(&self, key: String) -> StorageFuture<'_, Result<(), Self::Error>> {
		Box::pin(async move {
			self.wait(Kind::Remove).await;
			if self.fail_remove.load(Ordering::SeqCst) {
				return Err("remove failed");
			}

			self.values.remove(&key).unwrap();
			Ok(())
		})
	}
}

fn pending<F: Future + ?Sized>(future: Pin<&mut F>) {
	assert!(matches!(
		future.poll(&mut Context::from_waker(noop_waker_ref())),
		Poll::Pending
	));
}

#[test]
fn async_state_keeps_its_owner_when_initialized_and_polled_outside_it() {
	let owner = Owner::new();
	let store = MemoryStorage::<i32>::default();
	let shared: Arc<dyn AsyncStorage<i32, Error = std::convert::Infallible>> =
		Arc::new(store.clone());
	let initialize = owner.with(|| use_async_storage(shared, "key", 1));
	let hook = block_on(initialize).unwrap();
	assert_eq!(hook.value.get(), 1);
	assert_eq!(store.get("key"), Ok(None));

	block_on((hook.set_value)(2)).unwrap();
	assert_eq!(hook.value.get(), 2);
	store.set("key", &3).unwrap();
	block_on((hook.reload)()).unwrap();
	assert_eq!(hook.value.get(), 3);
	block_on((hook.remove)()).unwrap();
	block_on((hook.remove)()).unwrap();
	assert_eq!(hook.value.get(), 1);
	assert_eq!(store.get("key"), Ok(None));
	block_on((hook.reload)()).unwrap();
	assert_eq!(hook.value.get(), 1);
	let other = Owner::new();
	let isolated =
		block_on(other.with(|| use_async_storage(MemoryStorage::default(), "key", 9))).unwrap();
	assert_eq!(isolated.value.get(), 9);
}

#[test]
fn pending_save_preserves_displayed_state_and_cannot_be_overtaken_by_reload() {
	let owner = Owner::new();
	let store = Controlled::default();
	let hook = block_on(owner.with(|| use_async_storage(store.clone(), "key", 1))).unwrap();
	let resume = store.suspend(Kind::Write);
	let mut write = (hook.set_value)(2);
	pending(write.as_mut());
	assert_eq!(hook.value.get(), 1);
	assert_eq!(store.values.get("key"), Ok(None));

	let mut reload = (hook.reload)();
	pending(reload.as_mut());
	assert_eq!(store.reads.load(Ordering::SeqCst), 1);
	resume.send(()).unwrap();
	let (write, read) = block_on(async { futures::join!(write, reload) });
	write.unwrap();
	read.unwrap();
	assert_eq!(hook.value.get(), 2);
	assert_eq!(store.reads.load(Ordering::SeqCst), 2);
}

#[test]
fn asynchronous_errors_preserve_last_successful_value() {
	let owner = Owner::new();
	let store = Controlled::default();
	store.fail_read.store(true, Ordering::SeqCst);
	assert!(matches!(
		block_on(owner.with(|| use_async_storage(store.clone(), "key", 1))),
		Err(AsyncStorageError::Backend("read failed"))
	));
	store.fail_read.store(false, Ordering::SeqCst);
	let hook = block_on(owner.with(|| use_async_storage(store.clone(), "key", 1))).unwrap();
	block_on((hook.set_value)(2)).unwrap();
	store.fail_write.store(true, Ordering::SeqCst);

	assert!(matches!(
		block_on((hook.set_value)(3)),
		Err(AsyncStorageError::Backend("write failed"))
	));
	assert_eq!(hook.value.get(), 2);
	store.fail_remove.store(true, Ordering::SeqCst);
	assert!(matches!(
		block_on((hook.remove)()),
		Err(AsyncStorageError::Backend("remove failed"))
	));
	assert_eq!(hook.value.get(), 2);
	store.fail_read.store(true, Ordering::SeqCst);
	assert!(matches!(
		block_on((hook.reload)()),
		Err(AsyncStorageError::Backend("read failed"))
	));
	assert_eq!(hook.value.get(), 2);
	assert_eq!(store.values.get("key"), Ok(Some(2)));
	assert_eq!(
		AsyncStorageError::Backend("failure").to_string(),
		"storage backend failed: failure"
	);
}

#[test]
fn disposal_before_first_poll_performs_no_io() {
	let owner = Owner::new();
	let store = Controlled::default();
	let initialize = owner.with(|| use_async_storage(store.clone(), "key", 1));
	owner.cleanup();
	assert!(matches!(
		block_on(initialize),
		Err(AsyncStorageError::Disposed)
	));
	assert_eq!(store.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn disposal_during_initial_read_returns_a_typed_error() {
	let owner = Owner::new();
	let store = Controlled::default();
	let resume = store.suspend(Kind::Read);
	let mut initialize = Box::pin(owner.with(|| use_async_storage(store, "key", 1)));
	pending(initialize.as_mut());
	owner.cleanup();
	resume.send(()).unwrap();
	assert!(matches!(
		block_on(initialize),
		Err(AsyncStorageError::Disposed)
	));

	assert_eq!(
		AsyncStorageError::<&str>::Disposed.to_string(),
		"storage owner was disposed"
	);
}

#[test]
fn disposed_handles_reject_io_and_pending_writes_do_not_touch_disposed_signals() {
	let owner = Owner::new();
	let store = Controlled::default();
	let hook = block_on(owner.with(|| use_async_storage(store.clone(), "key", 1))).unwrap();
	let resume = store.suspend(Kind::Write);
	let mut write = (hook.set_value)(2);
	pending(write.as_mut());
	owner.cleanup();
	resume.send(()).unwrap();

	assert!(matches!(block_on(write), Err(AsyncStorageError::Disposed)));
	assert_eq!(store.values.get("key"), Ok(Some(2)));
	assert!(matches!(
		block_on((hook.set_value)(3)),
		Err(AsyncStorageError::Disposed)
	));
	assert!(matches!(
		block_on((hook.remove)()),
		Err(AsyncStorageError::Disposed)
	));
	assert!(matches!(
		block_on((hook.reload)()),
		Err(AsyncStorageError::Disposed)
	));
	assert_eq!(store.values.get("key"), Ok(Some(2)));
}

#[test]
fn dropping_a_pending_operation_releases_serialization_and_never_updates_state() {
	let owner = Owner::new();
	let store = Controlled::default();
	let hook = block_on(owner.with(|| use_async_storage(store.clone(), "key", 1))).unwrap();
	let resume = store.suspend(Kind::Write);
	let mut write = (hook.set_value)(2);
	pending(write.as_mut());
	drop(write);
	assert!(resume.send(()).is_err());

	assert_eq!(hook.value.get(), 1);
	block_on((hook.set_value)(3)).unwrap();
	assert_eq!(hook.value.get(), 3);
	let resume = store.suspend(Kind::Remove);
	let mut remove = (hook.remove)();
	pending(remove.as_mut());
	drop(remove);
	assert!(resume.send(()).is_err());
	assert_eq!(hook.value.get(), 3);
}

#[test]
fn suspended_reads_and_removal_complete_on_the_callers_executor() {
	let owner = Owner::new();
	let store = Controlled::default();
	store.values.set("key", &7).unwrap();
	let resume = store.suspend(Kind::Read);
	let mut initialize = Box::pin(owner.with(|| use_async_storage(store.clone(), "key", 1)));
	pending(initialize.as_mut());
	resume.send(()).unwrap();
	let hook = block_on(initialize).unwrap();

	assert_eq!(hook.value.get(), 7);
	store.values.set("key", &8).unwrap();
	let resume = store.suspend(Kind::Read);
	let mut reload = (hook.reload)();
	pending(reload.as_mut());
	assert_eq!(hook.value.get(), 7);
	resume.send(()).unwrap();
	block_on(reload).unwrap();
	assert_eq!(hook.value.get(), 8);
	let resume = store.suspend(Kind::Remove);
	let mut remove = (hook.remove)();
	pending(remove.as_mut());
	assert_eq!(hook.value.get(), 8);
	resume.send(()).unwrap();
	block_on(remove).unwrap();
	assert_eq!(hook.value.get(), 1);
}
