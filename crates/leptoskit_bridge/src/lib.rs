//! Runtime plugin transport, independent of build hooks and UI frameworks.
//!
//! A host owns a [`Bridge`] on its event-loop thread. Guests submit calls;
//! the host drains requests and completes them after native work finishes.
//! No native work executes while the guest is on the stack. Native callbacks
//! must be marshalled back to that owning thread.
//!
//! Payloads are [`Bytes`]: Rust-to-Rust handoff moves ownership without copying
//! the buffer. Crossing a wasm linear-memory boundary requires a copy. There
//! is no JSON envelope, base64, global registry, mutex, or executor in this
//! crate. Plugin crates define their own typed API and binary payload format.
//!
//! Registration and grants belong to the installed host, never the guest.
//! An OTA bundle cannot add native capabilities by choosing another ID.

use std::collections::BTreeMap;
use std::collections::VecDeque;

pub use bytes::Bytes;

pub mod codec;

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
#[allow(unsafe_code)]
pub mod guest;

/// Stable protocol version, negotiated independently of the rendering ABI.
pub const VERSION: u32 = 1;

/// A synchronous Rust plugin handler for short, nonblocking operations.
///
/// Call this directly from a native Rust host, or dispatch drained [`Request`]s
/// to it. Slow I/O and UI interactions should use the host's asynchronous
/// native APIs and [`Bridge::complete`] rather than block this method.
pub trait Plugin {
	/// Stable name used during registration.
	fn name(&self) -> &'static str;
	/// Versioned method IDs supported by this handler.
	fn methods(&self) -> &'static [MethodId];
	/// Handles one operation, taking ownership of its binary arguments.
	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error>;
}

/// An installed plugin's host-assigned identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginId(pub u32);

/// A method identifier defined by a plugin's versioned contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MethodId(pub u32);

/// A call identifier. Never reused during a bridge's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallId(pub u64);

/// Transport errors have stable wire codes. Direct plugin APIs may expose
/// additional platform-specific error details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[repr(i32)]
pub enum Error {
	/// Plugin is not installed or does not implement this method.
	#[error("plugin or method is unavailable")]
	Unavailable = 1,
	/// The installed host did not grant this operation.
	#[error("permission denied")]
	PermissionDenied = 2,
	/// Payload does not match the method's contract.
	#[error("invalid argument")]
	InvalidArgument = 3,
	/// The bridge's configured capacity has been reached.
	#[error("bridge capacity exceeded")]
	Busy = 4,
	/// The payload is larger than the configured limit.
	#[error("payload too large")]
	TooLarge = 5,
	/// The call was cancelled or the native dialog was dismissed.
	#[error("cancelled")]
	Cancelled = 6,
	/// The host or plugin encountered a native failure.
	#[error("native operation failed")]
	Native = 7,
	/// Unknown, completed, or cancelled call ID.
	#[error("invalid call")]
	InvalidCall = 8,
	/// No more calls can be assigned an ID.
	#[error("call identifiers exhausted")]
	Exhausted = 9,
}

impl Error {
	/// Decodes a v1 wire status (`-(code + 1)`).
	pub fn from_status(status: i64) -> Self {
		match status {
			-2 => Self::Unavailable,
			-3 => Self::PermissionDenied,
			-4 => Self::InvalidArgument,
			-5 => Self::Busy,
			-6 => Self::TooLarge,
			-7 => Self::Cancelled,
			-9 => Self::InvalidCall,
			-10 => Self::Exhausted,
			_ => Self::Native,
		}
	}
}

/// Limits apply before a payload is accepted, including completed responses.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
	/// Maximum outstanding calls, including unread replies.
	pub calls: usize,
	/// Maximum buffered native events.
	pub events: usize,
	/// Maximum bytes in a single request, reply, or event.
	pub payload_bytes: usize,
	/// Maximum total bytes retained by this bridge.
	pub buffered_bytes: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			calls: 128,
			events: 256,
			payload_bytes: 8 * 1024 * 1024,
			buffered_bytes: 32 * 1024 * 1024,
		}
	}
}

/// A call handed to the native host. Buffers remain valid until dropped.
#[derive(Debug)]
pub struct Request {
	/// Correlation ID for completion or cancellation.
	pub id: CallId,
	/// Registered plugin.
	pub plugin: PluginId,
	/// Granted method.
	pub method: MethodId,
	/// Plugin-defined binary arguments.
	pub payload: Bytes,
}

/// A native event. Subscribe/unsubscribe operations belong to plugin methods.
#[derive(Debug)]
pub struct Event {
	/// Installed plugin that produced the event.
	pub plugin: PluginId,
	/// Plugin-defined event kind.
	pub kind: u32,
	/// Plugin-defined binary event payload.
	pub payload: Bytes,
}

#[derive(Debug)]
struct Registration {
	name: String,
	methods: BTreeMap<MethodId, bool>,
}

#[derive(Debug)]
enum Call {
	Queued,
	Running,
	Ready(Result<Bytes, Error>),
}

#[derive(Debug)]
struct Slot {
	id: CallId,
	call: Option<Call>,
}

/// One application's bounded transport. Drop it when unloading the guest.
///
/// A host must stop native subscriptions and cancel its native jobs on unload.
/// Late completions return [`Error::InvalidCall`] instead of reviving a call.
#[derive(Debug)]
pub struct Bridge {
	limits: Limits,
	plugins: Vec<Registration>,
	requests: VecDeque<Request>,
	calls: Vec<Slot>,
	free: Vec<usize>,
	active: usize,
	events: VecDeque<Event>,
	buffered_bytes: usize,
}

impl Default for Bridge {
	fn default() -> Self {
		Self::new(Limits::default())
	}
}

impl Bridge {
	/// Creates an empty transport with explicit resource limits.
	pub fn new(limits: Limits) -> Self {
		Self {
			limits,
			plugins: Vec::new(),
			requests: VecDeque::with_capacity(limits.calls),
			calls: (0..limits.calls)
				.map(|index| {
					Slot {
						id: CallId(index as u64 + 1),
						call: None,
					}
				})
				.collect(),
			free: (0..limits.calls).rev().collect(),
			active: 0,
			events: VecDeque::with_capacity(limits.events),
			buffered_bytes: 0,
		}
	}

	/// Registers a unique plugin and its supported methods, denied by default.
	///
	/// Register once at startup. Keep the returned ID in typed plugin clients.
	pub fn register(
		&mut self,
		name: &str,
		methods: impl IntoIterator<Item = MethodId>,
	) -> Result<PluginId, Error> {
		if name.is_empty() || name.len() > 256 || self.resolve(name).is_some() {
			return Err(Error::InvalidArgument);
		}
		let id = PluginId(u32::try_from(self.plugins.len()).map_err(|_| Error::Exhausted)?);
		self.plugins.push(Registration {
			name: name.to_owned(),
			methods: methods.into_iter().map(|method| (method, false)).collect(),
		});
		Ok(id)
	}

	/// Resolves a plugin name once during client setup, outside the call path.
	pub fn resolve(&self, name: &str) -> Option<PluginId> {
		self.plugins
			.iter()
			.position(|plugin| plugin.name == name)
			.map(|index| PluginId(index as u32))
	}

	/// Changes a host grant. Revocation prevents new calls; cancel in-flight
	/// native work separately if the application's policy requires it.
	pub fn grant(
		&mut self,
		plugin: PluginId,
		method: MethodId,
		allowed: bool,
	) -> Result<(), Error> {
		let grant = self
			.plugins
			.get_mut(plugin.0 as usize)
			.and_then(|registration| registration.methods.get_mut(&method))
			.ok_or(Error::Unavailable)?;
		*grant = allowed;
		Ok(())
	}

	/// Checks admission without allocating or copying guest bytes.
	pub fn check_call(&self, plugin: PluginId, method: MethodId, len: usize) -> Result<(), Error> {
		let allowed = self
			.plugins
			.get(plugin.0 as usize)
			.and_then(|registration| registration.methods.get(&method))
			.ok_or(Error::Unavailable)?;
		if !allowed {
			return Err(Error::PermissionDenied);
		}
		if self.free.is_empty() {
			return Err(if self.active == 0 && !self.calls.is_empty() {
				Error::Exhausted
			} else {
				Error::Busy
			});
		}
		self.check_payload(len)
	}

	/// Queues a call by moving its buffer. Does not execute native code.
	pub fn submit(
		&mut self,
		plugin: PluginId,
		method: MethodId,
		payload: Bytes,
	) -> Result<CallId, Error> {
		self.check_call(plugin, method, payload.len())?;
		let index = self.free.pop().ok_or(Error::Busy)?;
		let slot = &mut self.calls[index];
		let id = slot.id;
		slot.call = Some(Call::Queued);
		self.active += 1;
		self.buffered_bytes += payload.len();
		self.requests.push_back(Request {
			id,
			plugin,
			method,
			payload,
		});
		Ok(id)
	}

	/// Moves the next request to the native host. The host now owns the bytes
	/// and must enforce its own resource limits on long-running native jobs.
	pub fn next_request(&mut self) -> Option<Request> {
		let request = self.requests.pop_front()?;
		self.buffered_bytes -= request.payload.len();
		let slot = self
			.slot_mut(request.id)
			.expect("queued request owns a call slot");
		slot.call = Some(Call::Running);
		Some(request)
	}

	/// Completes a running call exactly once.
	///
	/// Oversized or over-budget replies become a small error reply, ensuring
	/// the caller can always consume a terminal result and release its slot.
	pub fn complete(&mut self, id: CallId, result: Result<Bytes, Error>) -> Result<(), Error> {
		if !matches!(self.call(id), Some(Call::Running)) {
			return Err(Error::InvalidCall);
		}
		let result = result.and_then(|bytes| {
			self.check_payload(bytes.len())?;
			Ok(bytes)
		});
		if let Ok(bytes) = &result {
			self.buffered_bytes += bytes.len();
		}
		self.slot_mut(id).expect("running call owns a slot").call = Some(Call::Ready(result));
		Ok(())
	}

	/// Checks a foreign reply before copying its bytes. A capacity failure can
	/// itself be completed as an error reply, so the guest can release its
	/// slot.
	pub fn check_reply(&self, id: CallId, len: usize) -> Result<(), Error> {
		if !matches!(self.call(id), Some(Call::Running)) {
			return Err(Error::InvalidCall);
		}
		self.check_payload(len)
	}

	/// Borrows a ready result so a guest can size its destination before
	/// copying.
	pub fn result(&self, id: CallId) -> Result<Option<&Result<Bytes, Error>>, Error> {
		match self.call(id) {
			Some(Call::Ready(result)) => Ok(Some(result)),
			Some(Call::Queued | Call::Running) => Ok(None),
			None => Err(Error::InvalidCall),
		}
	}

	/// Takes a ready response without copying and frees its outstanding-call
	/// slot.
	pub fn take_result(&mut self, id: CallId) -> Result<Option<Result<Bytes, Error>>, Error> {
		if self.result(id)?.is_none() {
			return Ok(None);
		}
		let Call::Ready(result) = self.remove_call(id)? else {
			unreachable!("result was checked above");
		};
		if let Ok(bytes) = &result {
			self.buffered_bytes -= bytes.len();
		}
		Ok(Some(result))
	}

	/// Cancels any queued, running, or completed call and releases retained
	/// bytes.
	///
	/// For running work the host must also cancel the native operation. Its
	/// eventual completion will be rejected, so it cannot target another call.
	pub fn cancel(&mut self, id: CallId) -> Result<(), Error> {
		match self.remove_call(id)? {
			Call::Queued => {
				self.requests.retain(|request| {
					if request.id == id {
						self.buffered_bytes -= request.payload.len();
						false
					} else {
						true
					}
				});
			}
			Call::Ready(Ok(bytes)) => self.buffered_bytes -= bytes.len(),
			Call::Running | Call::Ready(Err(_)) => {}
		}
		Ok(())
	}

	/// Queues a native event. Full queues return an explicit error; producers
	/// can pause or coalesce their own sensor readings rather than silently
	/// lose data.
	pub fn emit(&mut self, event: Event) -> Result<(), Error> {
		self.check_event(event.plugin, event.payload.len())?;
		self.buffered_bytes += event.payload.len();
		self.events.push_back(event);
		Ok(())
	}

	/// Checks event admission before a foreign host copies its bytes.
	pub fn check_event(&self, plugin: PluginId, len: usize) -> Result<(), Error> {
		if self.plugins.get(plugin.0 as usize).is_none() {
			return Err(Error::Unavailable);
		}
		if self.events.len() >= self.limits.events {
			return Err(Error::Busy);
		}
		self.check_payload(len)
	}

	/// Borrows the next event for sizing a guest buffer.
	pub fn event(&self) -> Option<&Event> {
		self.events.front()
	}

	/// Moves the next native event out without copying its payload.
	pub fn next_event(&mut self) -> Option<Event> {
		let event = self.events.pop_front()?;
		self.buffered_bytes -= event.payload.len();
		Some(event)
	}

	/// Bytes currently retained by requests, replies, and events.
	pub fn buffered_bytes(&self) -> usize {
		self.buffered_bytes
	}

	fn index(&self, id: CallId) -> Option<usize> {
		if self.calls.is_empty() || id.0 == 0 {
			return None;
		}
		Some(((id.0 - 1) % self.calls.len() as u64) as usize)
	}

	fn call(&self, id: CallId) -> Option<&Call> {
		let slot = self.calls.get(self.index(id)?)?;
		if slot.id != id {
			return None;
		}
		slot.call.as_ref()
	}

	fn slot_mut(&mut self, id: CallId) -> Option<&mut Slot> {
		let index = self.index(id)?;
		let slot = self.calls.get_mut(index)?;
		(slot.id == id).then_some(slot)
	}

	fn remove_call(&mut self, id: CallId) -> Result<Call, Error> {
		let index = self.index(id).ok_or(Error::InvalidCall)?;
		let stride = self.calls.len() as u64;
		let slot = self.slot_mut(id).ok_or(Error::InvalidCall)?;
		let call = slot.call.take().ok_or(Error::InvalidCall)?;
		// The slot's successive IDs differ by the capacity. This lets lookup
		// use arithmetic instead of a map, without ever reusing a stale ID.
		if let Some(next) =
			id.0.checked_add(stride)
				.filter(|next| i64::try_from(*next).is_ok())
		{
			slot.id = CallId(next);
			self.free.push(index);
		}
		self.active -= 1;
		Ok(call)
	}

	fn check_payload(&self, len: usize) -> Result<(), Error> {
		if len > self.limits.payload_bytes || len > u32::MAX as usize {
			return Err(Error::TooLarge);
		}
		if len
			> self
				.limits
				.buffered_bytes
				.saturating_sub(self.buffered_bytes)
		{
			return Err(Error::Busy);
		}
		Ok(())
	}
}
