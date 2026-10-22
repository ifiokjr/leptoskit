use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Once;
use std::sync::OnceLock;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use global_hotkey::GlobalHotKeyEvent;
use global_hotkey::GlobalHotKeyManager;
use global_hotkey::HotKeyState;
use global_hotkey::hotkey::HotKey;
use parking_lot::Mutex;

use crate::EVENT_CAPACITY;
use crate::Error;
use crate::Event;
use crate::EventBatch;
use crate::KeyState;
use crate::REGISTRATION_CAPACITY;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static HANDLER: Once = Once::new();
static ROUTER: OnceLock<Mutex<Weak<Events>>> = OnceLock::new();

trait Backend {
	fn register(&self, key: HotKey) -> Result<(), Error>;
	fn unregister(&self, key: HotKey) -> Result<(), Error>;
}

struct Native(GlobalHotKeyManager);
impl Backend for Native {
	fn register(&self, key: HotKey) -> Result<(), Error> {
		self.0
			.register(key)
			.map_err(|error| Error::Conflict(error.to_string()))
	}

	fn unregister(&self, key: HotKey) -> Result<(), Error> {
		self.0
			.unregister(key)
			.map_err(|error| Error::Native(error.to_string()))
	}
}

struct Queue {
	keys: BTreeMap<u32, HotKey>,
	events: VecDeque<Event>,
	lost: u64,
	wake_pending: bool,
}

impl Default for Queue {
	fn default() -> Self {
		Self {
			keys: BTreeMap::new(),
			events: VecDeque::with_capacity(EVENT_CAPACITY),
			lost: 0,
			wake_pending: false,
		}
	}
}

impl Queue {
	fn push(&mut self, event: Event) -> bool {
		if !self.keys.contains_key(&event.id) {
			return false;
		}
		if self.events.len() == EVENT_CAPACITY {
			self.lost = self.lost.saturating_add(1);
		} else {
			self.events.push_back(event);
		}
		!std::mem::replace(&mut self.wake_pending, true)
	}

	fn remove(&mut self, id: u32) {
		self.keys.remove(&id);
		self.events.retain(|event| event.id != id);
	}

	fn drain(&mut self) -> EventBatch {
		self.wake_pending = false;
		EventBatch {
			events: self.events.drain(..).collect(),
			lost: std::mem::take(&mut self.lost),
		}
	}
}

struct Events {
	queue: Mutex<Queue>,
	wake: Box<dyn Fn() + Send + Sync>,
	callback_failed: AtomicBool,
}

impl Events {
	fn push(&self, event: Event) {
		let accepted = self.queue.lock().push(event);
		if accepted
			&& std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.wake)())).is_err()
		{
			self.callback_failed.store(true, Ordering::Release);
		}
	}
}

struct Exclusive;
impl Exclusive {
	fn acquire() -> Result<Self, Error> {
		ACTIVE
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.map(|_| Self)
			.map_err(|_| Error::AlreadyOwned)
	}
}
impl Drop for Exclusive {
	fn drop(&mut self) {
		ACTIVE.store(false, Ordering::Release);
	}
}

struct Inner {
	// Drop the native backend before releasing the process-wide owner slot.
	backend: Box<dyn Backend>,
	events: Arc<Events>,
	cleanup_error: RefCell<Option<Error>>,
	_exclusive: Option<Exclusive>,
}

impl Inner {
	fn unregister(&self, key: HotKey) -> Result<(), Error> {
		self.backend.unregister(key)?;
		self.events.queue.lock().remove(key.id());
		Ok(())
	}
}

/// One process-wide backend owner, confined to its creating event-loop thread.
///
/// This crate exclusively owns `global-hotkey`'s process-wide event callback.
/// Do not initialize/use that dependency separately in the same application.
/// Native Windows messages and macOS AppKit events must continue to be pumped.
/// X11 is supported; Wayland sessions are rejected rather than claiming global
/// shortcuts work through `XWayland`.
pub struct GlobalShortcuts {
	inner: Rc<Inner>,
}

impl GlobalShortcuts {
	/// Creates an owner whose host explicitly polls [`Self::drain`].
	pub fn new() -> Result<Self, Error> {
		Self::with_waker(|| {})
	}

	/// Creates an owner and wakes the host after an event arrives.
	///
	/// `wake` may run on a native background thread. It must be nonblocking and
	/// post to the host event loop instead of invoking the guest or UI
	/// directly. Wakeups coalesce until drain. A panicking callback cannot
	/// unwind across the native callback boundary; `take_cleanup_error`
	/// reports that failure.
	pub fn with_waker(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, Error> {
		#[cfg(target_os = "macos")]
		if objc2::MainThreadMarker::new().is_none() {
			return Err(Error::WrongThread);
		}
		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some()
			|| std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland")
		{
			return Err(Error::Unavailable);
		}
		let exclusive = Exclusive::acquire()?;
		let events = Arc::new(Events {
			queue: Mutex::new(Queue::default()),
			wake: Box::new(wake),
			callback_failed: AtomicBool::new(false),
		});
		let router = ROUTER.get_or_init(|| Mutex::new(Weak::new()));
		HANDLER.call_once(|| {
			GlobalHotKeyEvent::set_event_handler(Some(|event: GlobalHotKeyEvent| {
				let events = ROUTER.get().and_then(|router| router.lock().upgrade());
				if let Some(events) = events {
					events.push(Event {
						id: event.id,
						state: match event.state {
							HotKeyState::Pressed => KeyState::Pressed,
							HotKeyState::Released => KeyState::Released,
						},
					});
				}
			}));
		});
		*router.lock() = Arc::downgrade(&events);
		let backend =
			Native(GlobalHotKeyManager::new().map_err(|error| Error::Native(error.to_string()))?);
		Ok(Self {
			inner: Rc::new(Inner {
				backend: Box::new(backend),
				events,
				cleanup_error: RefCell::new(None),
				_exclusive: Some(exclusive),
			}),
		})
	}

	/// Registers one accelerator without replacing another application's
	/// binding.
	///
	/// The guard owns the registration, and keeps the backend alive if this
	/// manager is dropped first. Failed registration never produces a guard.
	pub fn register(&self, accelerator: &str) -> Result<Registration, Error> {
		if accelerator.is_empty() || accelerator.len() > 256 {
			return Err(Error::InvalidShortcut);
		}
		let key: HotKey = accelerator.parse().map_err(|_| Error::InvalidShortcut)?;
		{
			let mut queue = self.inner.events.queue.lock();
			if queue.keys.contains_key(&key.id()) {
				return Err(Error::Conflict(accelerator.into()));
			}
			if queue.keys.len() == REGISTRATION_CAPACITY {
				return Err(Error::Capacity);
			}
			queue.keys.insert(key.id(), key);
		}
		if let Err(error) = self.inner.backend.register(key) {
			self.inner.events.queue.lock().remove(key.id());
			return Err(error);
		}
		Ok(Registration {
			inner: Rc::clone(&self.inner),
			key: Some(key),
			id: key.id(),
		})
	}

	/// Drains at most [`EVENT_CAPACITY`] events and reports any lost events.
	pub fn drain(&self) -> EventBatch {
		self.inner.events.queue.lock().drain()
	}

	/// Returns the most recent failed RAII unregister operation, if any.
	///
	/// Call `Registration::unregister` for synchronous error handling. Drop
	/// cannot return errors; the native backend retries remaining cleanup when
	/// its final owner is released.
	pub fn take_cleanup_error(&self) -> Option<Error> {
		if self
			.inner
			.events
			.callback_failed
			.swap(false, Ordering::AcqRel)
		{
			Some(Error::Native("event-loop wake callback panicked".into()))
		} else {
			self.inner.cleanup_error.borrow_mut().take()
		}
	}
}

/// A main-thread/event-loop-confined guard that unregisters on drop.
#[must_use = "dropping the guard unregisters the shortcut"]
pub struct Registration {
	inner: Rc<Inner>,
	key: Option<HotKey>,
	id: u32,
}

impl Registration {
	/// Identifier returned by this registration.
	pub fn id(&self) -> u32 {
		self.id
	}

	/// Explicit, idempotent unregister with native error reporting.
	pub fn unregister(&mut self) -> Result<(), Error> {
		if let Some(key) = self.key {
			self.inner.unregister(key)?;
			self.key = None;
		}
		Ok(())
	}
}

impl Drop for Registration {
	fn drop(&mut self) {
		if let Err(error) = self.unregister() {
			*self.inner.cleanup_error.borrow_mut() = Some(error);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[cfg(target_os = "macos")]
	#[test]
	fn native_owner_rejects_non_main_threads_before_registering_anything() {
		assert!(
			std::thread::spawn(|| matches!(GlobalShortcuts::new(), Err(Error::WrongThread)))
				.join()
				.unwrap()
		);
	}

	#[test]
	fn process_owner_slot_is_exclusive_and_released() {
		let owner = Exclusive::acquire().unwrap();
		assert!(matches!(Exclusive::acquire(), Err(Error::AlreadyOwned)));
		drop(owner);
		assert!(Exclusive::acquire().is_ok());
	}
	#[derive(Default)]
	struct Mock {
		unregisters: Mutex<Vec<u32>>,
		conflict: AtomicBool,
		unregister_fails: AtomicBool,
	}
	impl Backend for Arc<Mock> {
		fn register(&self, _key: HotKey) -> Result<(), Error> {
			if self.conflict.load(Ordering::Relaxed) {
				Err(Error::Conflict("occupied".into()))
			} else {
				Ok(())
			}
		}

		fn unregister(&self, key: HotKey) -> Result<(), Error> {
			self.unregisters.lock().push(key.id());
			if self.unregister_fails.load(Ordering::Relaxed) {
				Err(Error::Native("unregister failed".into()))
			} else {
				Ok(())
			}
		}
	}
	fn manager(backend: Arc<Mock>) -> GlobalShortcuts {
		GlobalShortcuts {
			inner: Rc::new(Inner {
				backend: Box::new(backend),
				events: Arc::new(Events {
					queue: Mutex::new(Queue::default()),
					wake: Box::new(|| {}),
					callback_failed: AtomicBool::new(false),
				}),
				cleanup_error: RefCell::new(None),
				_exclusive: None,
			}),
		}
	}
	#[test]
	fn guard_unregisters_once_and_removes_pending_events() {
		let backend = Arc::new(Mock::default());
		let owner = manager(Arc::clone(&backend));
		let guard = owner.register("Control+Shift+KeyK").unwrap();
		let id = guard.id();
		owner.inner.events.push(Event {
			id,
			state: KeyState::Pressed,
		});
		drop(guard);
		assert_eq!(*backend.unregisters.lock(), [id]);
		assert_eq!(owner.drain(), EventBatch::default());
		let mut guard = owner.register("Control+Shift+KeyK").unwrap();
		guard.unregister().unwrap();
		guard.unregister().unwrap();
		drop(guard);
		assert_eq!(backend.unregisters.lock().len(), 2);
	}
	#[test]
	fn conflict_does_not_leak_capacity_or_release_someone_elses_registration() {
		let backend = Arc::new(Mock::default());
		let owner = manager(Arc::clone(&backend));
		backend.conflict.store(true, Ordering::Relaxed);
		assert!(matches!(
			owner.register("Control+KeyK"),
			Err(Error::Conflict(_))
		));
		assert!(owner.inner.events.queue.lock().keys.is_empty());
		assert!(backend.unregisters.lock().is_empty());
		backend.conflict.store(false, Ordering::Relaxed);
		let guard = owner.register("Control+KeyK").unwrap();
		assert!(matches!(
			owner.register("Control+KeyK"),
			Err(Error::Conflict(_))
		));
		drop(owner);
		assert!(backend.unregisters.lock().is_empty());
		drop(guard);
		assert_eq!(backend.unregisters.lock().len(), 1);
	}
	#[test]
	fn bounded_queue_reports_overflow_and_ignores_unowned_events() {
		let owner = manager(Arc::new(Mock::default()));
		let guard = owner.register("Control+KeyK").unwrap();
		for _ in 0..EVENT_CAPACITY + 7 {
			owner.inner.events.push(Event {
				id: guard.id(),
				state: KeyState::Pressed,
			});
		}
		let batch = owner.drain();
		assert_eq!(batch.events.len(), EVENT_CAPACITY);
		assert_eq!(batch.lost, 7);
		assert_eq!(owner.drain(), EventBatch::default());
		owner.inner.events.push(Event {
			id: 0,
			state: KeyState::Pressed,
		});
		assert_eq!(owner.drain(), EventBatch::default());
	}

	#[test]
	fn unregister_failure_preserves_the_guard_for_retry_and_drop_reports_failure() {
		let backend = Arc::new(Mock::default());
		let owner = manager(Arc::clone(&backend));
		let mut guard = owner.register("Control+KeyK").unwrap();
		backend.unregister_fails.store(true, Ordering::Relaxed);
		assert!(matches!(guard.unregister(), Err(Error::Native(_))));
		assert!(matches!(
			owner.register("Control+KeyK"),
			Err(Error::Conflict(_))
		));
		backend.unregister_fails.store(false, Ordering::Relaxed);
		guard.unregister().unwrap();
		drop(guard);
		let guard = owner.register("Control+KeyK").unwrap();
		backend.unregister_fails.store(true, Ordering::Relaxed);
		drop(guard);
		assert!(matches!(owner.take_cleanup_error(), Some(Error::Native(_))));
		assert!(owner.take_cleanup_error().is_none());
	}

	#[test]
	fn wakeups_are_coalesced_and_panics_do_not_cross_native_callbacks() {
		use std::sync::atomic::AtomicUsize;
		let wakes = Arc::new(AtomicUsize::new(0));
		let count = Arc::clone(&wakes);
		let events = Events {
			queue: Mutex::new(Queue::default()),
			wake: Box::new(move || {
				count.fetch_add(1, Ordering::Relaxed);
			}),
			callback_failed: AtomicBool::new(false),
		};
		let key: HotKey = "Control+KeyK".parse().unwrap();
		events.queue.lock().keys.insert(key.id(), key);
		let event = Event {
			id: key.id(),
			state: KeyState::Pressed,
		};
		for _ in 0..200 {
			events.push(event);
		}
		assert_eq!(wakes.load(Ordering::Relaxed), 1);
		events.queue.lock().drain();
		events.push(event);
		assert_eq!(wakes.load(Ordering::Relaxed), 2);
		let panicking = Events {
			queue: events.queue,
			wake: Box::new(|| panic!("host wake failed")),
			callback_failed: AtomicBool::new(false),
		};
		panicking.queue.lock().drain();
		panicking.push(event);
		assert!(panicking.callback_failed.load(Ordering::Acquire));
	}

	#[test]
	fn bridge_adapter_registers_drains_and_unregisters_through_its_owner() {
		use leptoskit_bridge::Bytes;
		use leptoskit_bridge::Error as BridgeError;
		use leptoskit_bridge::MethodId;
		use leptoskit_bridge::Plugin;

		use crate::EVENTS;
		use crate::REGISTER;
		use crate::ShortcutPlugin;
		use crate::UNREGISTER;

		let backend = Arc::new(Mock::default());
		let owner = manager(Arc::clone(&backend));
		let events = Arc::clone(&owner.inner.events);
		let mut plugin = ShortcutPlugin::new(owner);
		assert_eq!(plugin.name(), "global-shortcut");
		assert_eq!(plugin.methods(), &[REGISTER, UNREGISTER, EVENTS]);
		let reply = plugin
			.invoke(REGISTER, Bytes::from_static(b"Control+Shift+KeyP"))
			.unwrap();
		let id = u32::from_le_bytes(reply.as_ref().try_into().unwrap());
		events.push(Event {
			id,
			state: KeyState::Pressed,
		});
		let batch = EventBatch::decode(&plugin.invoke(EVENTS, Bytes::new()).unwrap()).unwrap();
		assert_eq!(
			batch.events,
			[Event {
				id,
				state: KeyState::Pressed
			}]
		);
		assert_eq!(
			plugin.invoke(REGISTER, Bytes::from_static(b"Control+Shift+KeyP")),
			Err(BridgeError::Busy),
			"the same accelerator conflicts"
		);
		assert_eq!(
			plugin.invoke(REGISTER, Bytes::from_static(&[0xff])),
			Err(BridgeError::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(REGISTER, Bytes::from_static(b"not a shortcut")),
			Err(BridgeError::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(EVENTS, Bytes::from_static(&[0])),
			Err(BridgeError::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(MethodId(9), Bytes::new()),
			Err(BridgeError::Unavailable)
		);
		let unknown = 404_u32.to_le_bytes().to_vec();
		assert_eq!(
			plugin.invoke(UNREGISTER, Bytes::from(unknown)),
			Err(BridgeError::InvalidCall)
		);
		let known = id.to_le_bytes().to_vec();
		assert_eq!(
			plugin.invoke(UNREGISTER, Bytes::from(known)),
			Ok(Bytes::new())
		);
		assert_eq!(backend.unregisters.lock().as_slice(), [id]);

		// A failed native unregister keeps the guard and surfaces on the next
		// drain.
		backend.unregister_fails.store(true, Ordering::Relaxed);
		let reply = plugin
			.invoke(REGISTER, Bytes::from_static(b"Control+KeyQ"))
			.unwrap();
		let second = reply.to_vec();
		assert_eq!(
			plugin.invoke(UNREGISTER, Bytes::from(second.clone())),
			Err(BridgeError::Native)
		);
		assert!(plugin.take_cleanup_error().is_none());
		backend.unregister_fails.store(false, Ordering::Relaxed);
		assert_eq!(
			plugin.invoke(UNREGISTER, Bytes::from(second)),
			Ok(Bytes::new())
		);
	}

	#[test]
	fn shortcut_errors_map_to_stable_bridge_errors() {
		use leptoskit_bridge::Error as BridgeError;
		for (error, expected) in [
			(Error::Unavailable, BridgeError::Unavailable),
			(Error::WrongThread, BridgeError::InvalidArgument),
			(Error::InvalidShortcut, BridgeError::InvalidArgument),
			(Error::AlreadyOwned, BridgeError::Busy),
			(Error::Conflict("taken".into()), BridgeError::Busy),
			(Error::Capacity, BridgeError::Busy),
			(Error::Native("failed".into()), BridgeError::Native),
		] {
			assert_eq!(BridgeError::from(error), expected);
		}
	}
}
