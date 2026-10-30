//! Runtime plugin transport, independent of build hooks and UI frameworks.
//!
//! A host owns a [`Bridge`] on its event-loop thread. Guests submit calls;
//! the host drains requests and completes them after native work finishes.
//! No native work executes while the guest is on the stack. Native callbacks
//! must be marshalled back to that owning thread.
//!
//! Payloads are [`Bytes`]: Rust-to-Rust handoff moves ownership without copying
//! the buffer. Crossing a wasm linear-memory boundary requires a copy. There
//! is no JSON envelope, base64, global registry, or executor. The default
//! transport is thread-confined; optional `async-host` permissions add shared
//! cancellation tokens. Plugins define their own typed API and binary format.
//!
//! Registration and grants belong to the installed host, never the guest.
//! An OTA bundle cannot add native capabilities by choosing another ID.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;

pub use bytes::Bytes;

pub mod codec;
pub mod frame;
mod reply;
pub use reply::Reply;
pub use reply::ReplyGuard;

#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub mod async_host;

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
#[allow(unsafe_code)]
pub mod guest;

/// Stable protocol version, negotiated independently of the rendering ABI.
pub const VERSION: u32 = 1;

const MAX_PLUGIN_NAME_BYTES: usize = 256;
const DEFAULT_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_BUFFERED_BYTES: usize = 32 * 1024 * 1024;

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
	/// Maximum outstanding calls, including unread replies and unobserved
	/// cancellations of running work.
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
			payload_bytes: DEFAULT_PAYLOAD_BYTES,
			buffered_bytes: DEFAULT_BUFFERED_BYTES,
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
	#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
	permissions: BTreeMap<MethodId, async_host::CancellationToken>,
}

#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
fn new_permission() -> async_host::CancellationToken {
	let permission = async_host::CancellationToken::new();
	// Some platforms allocate the token's mutex on its first use. Initialize
	// it during grant setup so the first submitted call remains
	// allocation-free.
	let _ = permission.is_cancelled();
	permission
}

#[derive(Debug)]
enum Call {
	Queued,
	Running,
	Cancelled,
	Ready(ReadyReply),
}

#[derive(Debug)]
struct ReadyReply {
	result: Result<Bytes, Error>,
	lease: Option<Reply>,
}

#[derive(Debug)]
struct Slot {
	id: CallId,
	call: Option<Call>,
	#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
	permission: Option<async_host::CancellationToken>,
}

/// One application's bounded transport. Drop it when unloading the guest.
///
/// A host must stop native subscriptions and cancel its native jobs on unload.
/// Late completions return [`Error::InvalidCall`] instead of reviving a call.
#[derive(Debug)]
pub struct Bridge {
	identity: BridgeIdentity,
	limits: Limits,
	plugins: Vec<Registration>,
	requests: VecDeque<Request>,
	cancelled: VecDeque<CallId>,
	calls: Vec<Slot>,
	free: Vec<usize>,
	active: usize,
	events: VecDeque<Event>,
	buffered_bytes: usize,
}

/// Opaque transport identity, stable across moves and distinct from every other
/// live identity, including retained identities of destroyed transports.
///
/// Cloning a handle retains only its identity token, never guest state.
/// Callers cannot mint an identity:
///
/// ```compile_fail,E0423
/// use leptoskit_bridge::BridgeIdentity;
/// let identity = BridgeIdentity(std::sync::Arc::new(()));
/// ```
#[derive(Debug, Clone)]
pub struct BridgeIdentity(Arc<()>);

impl PartialEq for BridgeIdentity {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.0, &other.0)
	}
}

impl Eq for BridgeIdentity {}

impl Default for Bridge {
	fn default() -> Self {
		Self::new(Limits::default())
	}
}

impl Bridge {
	/// Creates an empty transport with explicit resource limits.
	pub fn new(limits: Limits) -> Self {
		Self {
			identity: BridgeIdentity(Arc::new(())),
			limits,
			plugins: Vec::new(),
			requests: VecDeque::with_capacity(limits.calls),
			cancelled: VecDeque::with_capacity(limits.calls),
			calls: (0..limits.calls)
				.map(|index| {
					Slot {
						id: CallId(index as u64 + 1),
						call: None,
						#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
						permission: None,
					}
				})
				.collect(),
			free: (0..limits.calls).rev().collect(),
			active: 0,
			events: VecDeque::with_capacity(limits.events),
			buffered_bytes: 0,
		}
	}

	/// Identifies this transport without retaining guest state or a host
	/// borrow.
	pub fn identity(&self) -> BridgeIdentity {
		self.identity.clone()
	}

	/// Registers a unique plugin and its supported methods, denied by default.
	///
	/// Register once at startup. Keep the returned ID in typed plugin clients.
	pub fn register(
		&mut self,
		name: &str,
		methods: impl IntoIterator<Item = MethodId>,
	) -> Result<PluginId, Error> {
		if name.is_empty() || name.len() > MAX_PLUGIN_NAME_BYTES || self.resolve(name).is_some() {
			return Err(Error::InvalidArgument);
		}

		let id = PluginId(u32::try_from(self.plugins.len()).map_err(|_| Error::Exhausted)?);
		let methods: BTreeMap<_, _> = methods.into_iter().map(|method| (method, false)).collect();
		self.plugins.push(Registration {
			name: name.to_owned(),
			#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
			permissions: methods
				.keys()
				.map(|method| (*method, new_permission()))
				.collect(),
			methods,
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

	/// Changes a host grant. With `async-host`, revocation also cancels the
	/// permission token retained by queued and running async requests. A later
	/// grant creates a new token and cannot revive earlier work.
	pub fn grant(
		&mut self,
		plugin: PluginId,
		method: MethodId,
		allowed: bool,
	) -> Result<(), Error> {
		let registration = self
			.plugins
			.get_mut(plugin.0 as usize)
			.ok_or(Error::Unavailable)?;
		let grant = registration
			.methods
			.get_mut(&method)
			.ok_or(Error::Unavailable)?;
		#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
		if *grant != allowed {
			let permission = registration
				.permissions
				.get_mut(&method)
				.expect("registered permission");
			if allowed {
				*permission = new_permission();
			} else {
				permission.cancel();
			}
		}
		*grant = allowed;
		#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
		if !allowed {
			for slot in &mut self.calls {
				if slot
					.permission
					.as_ref()
					.is_some_and(async_host::CancellationToken::is_cancelled)
					&& let Some(Call::Ready(reply)) = &mut slot.call
				{
					if let Ok(bytes) = &reply.result {
						self.buffered_bytes -= bytes.len();
					}
					*reply = ReadyReply {
						result: Err(Error::PermissionDenied),
						lease: None,
					};
				}
			}
		}
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
			if self.active == 0 && !self.calls.is_empty() {
				return Err(Error::Exhausted);
			}

			return Err(Error::Busy);
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
		#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
		{
			slot.permission = Some(self.plugins[plugin.0 as usize].permissions[&method].clone());
		}
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
		self.complete_reply(id, result.map(Reply::from))
	}

	/// Completes a call while retaining a provisional resource until the
	/// response is consumed. Every rejection drops its lease.
	pub fn complete_reply(
		&mut self,
		id: CallId,
		result: Result<Reply, Error>,
	) -> Result<(), Error> {
		if !matches!(self.call(id), Some(Call::Running)) {
			return Err(Error::InvalidCall);
		}

		#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
		let result = if self.permission(id)?.is_cancelled() {
			Err(Error::PermissionDenied)
		} else {
			result
		};
		let result = result.and_then(|reply| {
			self.check_payload(reply.bytes.len())?;
			Ok(reply)
		});
		let (result, lease) = match result {
			Ok(mut reply) => {
				self.buffered_bytes += reply.bytes.len();
				let bytes = std::mem::take(&mut reply.bytes);
				(Ok(bytes), reply.guard.is_some().then_some(reply))
			}
			Err(error) => (Err(error), None),
		};
		self.slot_mut(id).expect("running call owns a slot").call =
			Some(Call::Ready(ReadyReply { result, lease }));
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
			Some(Call::Ready(reply)) => Ok(Some(&reply.result)),
			Some(Call::Queued | Call::Running) => Ok(None),
			Some(Call::Cancelled) | None => Err(Error::InvalidCall),
		}
	}

	/// Takes a ready response without copying and frees its outstanding-call
	/// slot.
	pub fn take_result(&mut self, id: CallId) -> Result<Option<Result<Bytes, Error>>, Error> {
		if self.result(id)?.is_none() {
			return Ok(None);
		}

		let Call::Ready(mut reply) = self.remove_call(id)? else {
			unreachable!("result was checked above");
		};
		if let Some(guard) = reply.lease.as_mut().and_then(|lease| lease.guard.as_mut()) {
			guard.commit();
		}
		if let Ok(bytes) = &reply.result {
			self.buffered_bytes -= bytes.len();
		}
		Ok(Some(reply.result))
	}

	/// Cancels any queued, running, or completed call and releases retained
	/// bytes.
	///
	/// Running work retains its slot until the host takes its cancellation
	/// through [`Self::next_cancelled`]. This bounds both outstanding work and
	/// cancellation notifications. Eventual completions are rejected.
	pub fn cancel(&mut self, id: CallId) -> Result<(), Error> {
		match self.call(id) {
			Some(Call::Running) => {
				self.slot_mut(id).expect("running call owns a slot").call = Some(Call::Cancelled);
				self.cancelled.push_back(id);
				return Ok(());
			}
			Some(Call::Cancelled) | None => return Err(Error::InvalidCall),
			Some(Call::Queued | Call::Ready(_)) => {}
		}

		match self.remove_call(id)? {
			Call::Queued => self.remove_queued_request(id),
			Call::Ready(reply) => {
				if let Ok(bytes) = &reply.result {
					self.buffered_bytes -= bytes.len();
				}
			}
			Call::Running | Call::Cancelled => unreachable!("handled above"),
		}

		Ok(())
	}

	/// Transfers one cancellation to the host and releases its bridge slot.
	///
	/// Drain these after each guest entry point, before admitting new native
	/// jobs. Stop the matching task/subscription; if cancellation itself is
	/// asynchronous, continue counting that job against the host's own budget
	/// until its resources are actually released.
	pub fn next_cancelled(&mut self) -> Option<CallId> {
		let id = self.cancelled.pop_front()?;
		let call = self
			.remove_call(id)
			.expect("pending cancellation owns a slot");
		debug_assert!(matches!(call, Call::Cancelled));
		Some(id)
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

	/// Retains the exact permission generation captured when a call was
	/// submitted. Revoking then regranting cannot authorize that old call.
	#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
	pub fn permission(&self, id: CallId) -> Result<async_host::CancellationToken, Error> {
		let slot = self
			.calls
			.get(self.index(id).ok_or(Error::InvalidCall)?)
			.ok_or(Error::InvalidCall)?;
		if slot.id != id || slot.call.is_none() {
			return Err(Error::InvalidCall);
		}
		slot.permission.clone().ok_or(Error::InvalidCall)
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
		#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
		{
			slot.permission = None;
		}
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

	fn remove_queued_request(&mut self, id: CallId) {
		self.requests.retain(|request| {
			if request.id != id {
				return true;
			}

			self.buffered_bytes -= request.payload.len();
			false
		});
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
