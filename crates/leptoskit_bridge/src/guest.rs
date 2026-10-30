//! Async guest client for the interpreter's native plugin imports.
//!
//! Enable `guest` only in wasm bundles loaded by `leptoskit_wasm`, not browser
//! bundles. Poll calls with the app's executor; the host calls the exported
//! `leptoskit_plugins_ready` to wake registered futures. Dropping a future
//! cancels its outstanding call.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use crate::Bytes;
use crate::Error;
use crate::Event;
use crate::MethodId;
use crate::PluginId;

#[link(wasm_import_module = "leptoskit_plugins_v1")]
unsafe extern "C" {
	fn version() -> i32;
	fn resolve(ptr: *const u8, len: usize) -> i64;
	fn submit(plugin: u32, method: u32, ptr: *const u8, len: usize) -> i64;
	fn result_len(id: i64) -> i64;
	fn take_result(id: i64, ptr: *mut u8, capacity: usize) -> i64;
	fn cancel(id: i64) -> i32;
	fn event_info(ptr: *mut u8) -> i32;
	fn take_event(ptr: *mut u8, capacity: usize) -> i64;
}

thread_local! {
	static WAITERS: RefCell<BTreeMap<i64, Waker>> = const { RefCell::new(BTreeMap::new()) };
	static EVENTS_TAKEN: Cell<bool> = const { Cell::new(false) };
	static EVENT_WAITER: RefCell<Option<Waker>> = const { RefCell::new(None) };
}

/// The single consumer of this guest's native event queue.
///
/// Route returned events by `plugin` and `kind`. Subscribe and unsubscribe with
/// the plugin's methods; dropping this receiver does not stop native producers.
/// A single receiver prevents unrelated plugins from consuming each other's
/// events or building unbounded secondary queues.
#[derive(Debug)]
pub struct Events {
	// This token belongs to the guest thread whose imports and waker it uses.
	_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Events {
	/// Acquires the event consumer, returning Busy while another owner exists.
	pub fn take() -> Result<Self, Error> {
		// SAFETY: Protocol negotiation accesses no guest memory.
		if unsafe { version() } != crate::VERSION as i32 {
			return Err(Error::Unavailable);
		}

		if EVENTS_TAKEN.replace(true) {
			return Err(Error::Busy);
		}

		Ok(Self {
			_thread: std::marker::PhantomData,
		})
	}

	/// Takes the next event, or returns None when no event is queued.
	pub fn try_recv(&mut self) -> Result<Option<Event>, Error> {
		let mut info = [0_u8; 12];
		// SAFETY: The v1 import writes exactly twelve bytes when an event
		// exists.
		match unsafe { event_info(info.as_mut_ptr()) } {
			0 => return Ok(None),
			1 => {}
			_ => return Err(Error::Native),
		}

		let plugin = PluginId(u32::from_le_bytes(
			info[..4].try_into().map_err(|_| Error::Native)?,
		));
		let kind = u32::from_le_bytes(info[4..8].try_into().map_err(|_| Error::Native)?);
		let len = u32::from_le_bytes(info[8..].try_into().map_err(|_| Error::Native)?) as usize;
		let mut bytes = Vec::new();
		bytes.try_reserve_exact(len).map_err(|_| Error::TooLarge)?;
		// SAFETY: The synchronous host writes no more than len bytes into this
		// allocation. No other consumer can run between info and take_event.
		let written = unsafe { take_event(bytes.as_mut_ptr(), len) };

		if written != len as i64 {
			return Err(if written < -1 {
				Error::from_status(written)
			} else {
				Error::Native
			});
		}

		// SAFETY: The import confirmed that it initialized exactly len bytes.
		unsafe { bytes.set_len(len) };
		Ok(Some(Event {
			plugin,
			kind,
			payload: bytes.into(),
		}))
	}

	/// Waits for the next event. Dropping the wait preserves queued events.
	pub fn recv(&mut self) -> NextEvent<'_> {
		NextEvent(self)
	}
}

impl Drop for Events {
	fn drop(&mut self) {
		EVENT_WAITER.with_borrow_mut(Option::take);
		EVENTS_TAKEN.set(false);
	}
}

/// A cancellable wait which exclusively borrows the event receiver.
#[derive(Debug)]
pub struct NextEvent<'a>(&'a mut Events);

impl Future for NextEvent<'_> {
	type Output = Result<Event, Error>;

	fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		match self.0.try_recv() {
			Ok(None) => {
				let waker = context.waker().clone();
				EVENT_WAITER.with_borrow_mut(|waiter| waiter.replace(waker));
				Poll::Pending
			}
			Ok(Some(event)) => {
				EVENT_WAITER.with_borrow_mut(Option::take);
				Poll::Ready(Ok(event))
			}
			Err(error) => {
				EVENT_WAITER.with_borrow_mut(Option::take);
				Poll::Ready(Err(error))
			}
		}
	}
}

impl Drop for NextEvent<'_> {
	fn drop(&mut self) {
		EVENT_WAITER.with_borrow_mut(Option::take);
	}
}

/// A plugin name resolved once against the installed host.
#[derive(Debug, Clone, Copy)]
pub struct Client(PluginId);

impl Client {
	/// Returns this client's host-assigned plugin identity for event routing.
	pub fn plugin_id(&self) -> PluginId {
		self.0
	}

	/// Verifies the bridge protocol and resolves an installed plugin.
	pub fn new(name: &str) -> Result<Self, Error> {
		// SAFETY: The imported functions synchronously borrow the supplied
		// slice.
		if unsafe { version() } != crate::VERSION as i32 {
			return Err(Error::Unavailable);
		}

		let id = unsafe { resolve(name.as_ptr(), name.len()) };

		if id < 0 {
			return Err(Error::from_status(id));
		}

		Ok(Self(PluginId(
			u32::try_from(id).map_err(|_| Error::Native)?,
		)))
	}

	/// Submits bytes synchronously, then returns a cancellable response future.
	pub fn call(self, method: MethodId, payload: &[u8]) -> Result<Call, Error> {
		// SAFETY: The host copies payload during submit; it retains no guest
		// pointer.
		let id = unsafe { submit(self.0.0, method.0, payload.as_ptr(), payload.len()) };

		if id < 0 {
			return Err(Error::from_status(id));
		}

		Ok(Call { id: Some(id) })
	}
}

/// An in-flight native call. Cancelled automatically on drop.
#[derive(Debug)]
pub struct Call {
	id: Option<i64>,
}

impl Future for Call {
	type Output = Result<Bytes, Error>;

	fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		let Some(id) = self.id else {
			return Poll::Ready(Err(Error::InvalidCall));
		};
		// SAFETY: No guest memory is accessed by this status query.
		let len = unsafe { result_len(id) };

		if len == -1 {
			let waker = context.waker().clone();
			WAITERS.with_borrow_mut(|waiters| waiters.insert(id, waker));

			return Poll::Pending;
		}

		WAITERS.with_borrow_mut(|waiters| waiters.remove(&id));

		if len < 0 {
			// SAFETY: Error replies do not write to the destination.
			unsafe { take_result(id, std::ptr::null_mut(), 0) };
			self.id = None;

			return Poll::Ready(Err(Error::from_status(len)));
		}

		let Ok(len) = usize::try_from(len) else {
			self.cancel();

			return Poll::Ready(Err(Error::TooLarge));
		};
		let mut bytes = Vec::new();

		if bytes.try_reserve_exact(len).is_err() {
			self.cancel();

			return Poll::Ready(Err(Error::TooLarge));
		}

		// SAFETY: The host writes at most capacity bytes. The vector length is
		// set only after the import reports exactly how many bytes it wrote.
		let written = unsafe { take_result(id, bytes.as_mut_ptr(), len) };

		if written != len as i64 {
			self.cancel();

			return Poll::Ready(Err(Error::Native));
		}

		unsafe { bytes.set_len(len) };
		self.id = None;
		Poll::Ready(Ok(bytes.into()))
	}
}

impl Drop for Call {
	fn drop(&mut self) {
		self.cancel();
	}
}

impl Call {
	/// Returns the host-assigned identity while this handle owns the call.
	///
	/// A host reply does not clear the identity until this future is polled
	/// to completion. Successful and error completion both return `None`
	/// thereafter. Dropping an outstanding handle cancels this same identity.
	pub fn id(&self) -> Option<crate::CallId> {
		self.id
			.map(|id| crate::CallId(u64::try_from(id).expect("submitted call IDs are nonnegative")))
	}

	fn cancel(&mut self) {
		if let Some(id) = self.id.take() {
			WAITERS.with_borrow_mut(|waiters| waiters.remove(&id));
			// SAFETY: Cancellation takes only the host-issued identifier.
			unsafe { cancel(id) };
		}
	}
}

/// Wakes native-call futures after the host finishes work. Wakers are invoked
/// outside the registry borrow so executors may immediately poll them.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_plugins_ready() {
	let waiters = WAITERS.with_borrow_mut(std::mem::take);
	let event_waiter = EVENT_WAITER.with_borrow_mut(Option::take);

	for waker in waiters.into_values() {
		waker.wake();
	}

	if let Some(waker) = event_waiter {
		waker.wake();
	}
}

/// Waits for the next display frame, like `requestAnimationFrame`: await it,
/// update state, and the render reaches the screen on the next vsync when
/// it fits in a frame.
///
/// Every waiter resumes on the same frame. On a host built before the frame
/// clock existed this returns [`Error::Unavailable`]; fall back to a timer
/// there rather than failing.
///
/// # Errors
///
/// [`Error::Unavailable`] when the host has no frame clock, or the error the
/// bridge reports for the call.
pub async fn next_frame() -> Result<crate::frame::Frame, Error> {
	thread_local! {
		static FRAMES: std::cell::OnceCell<Result<Client, Error>> = const { std::cell::OnceCell::new() };
	}
	let client =
		FRAMES.with(|frames| *frames.get_or_init(|| Client::new(crate::frame::FRAME_PLUGIN)))?;
	let reply = client.call(crate::frame::NEXT_FRAME, &[])?.await?;
	crate::frame::Frame::decode(&reply).ok_or(Error::Native)
}
