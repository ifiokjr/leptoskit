//! Keeping several caches in sync: other browser tabs, other windows of a
//! native app, or anything with a message channel.
//!
//! When a query in one cache receives data, the data is sent to every peer,
//! which hydrates its own copy of the query. A peer that does not have the
//! query yet keeps the data until something creates it. Only scopes with a
//! codec take part, since the data has to be serialized; and only successful
//! data travels, matching TanStack's `broadcastQueryClient`.
//!
//! ```
//! # use leptoskit_query::*;
//! # fn demo() {
//! let client = QueryClient::new();
//! // In the browser: every tab that joins "my-app" shares cache writes.
//! let _sync = client.broadcast("my-app");
//! # }
//! ```

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use parking_lot::Mutex;
use serde::Deserialize;
use serde::Serialize;

use crate::cache::CacheEvent;
use crate::client::QueryClient;
use crate::hydration::DehydratedQuery;
use crate::listeners::Subscription;

/// Carries messages between peers.
///
/// Implement this to sync caches over whatever a host offers: a
/// `BroadcastChannel` in the browser (built in), a channel between windows of
/// a native app ([`LocalBroadcast`]), or a socket.
pub trait BroadcastTransport: Send + Sync + 'static {
	/// Delivers `message` to every other peer.
	fn send(&self, message: &str);

	/// Registers the handler for messages from other peers. Called once.
	fn receive(&self, handler: Arc<dyn Fn(&str) + Send + Sync>);
}

/// What peers tell each other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BroadcastMessage {
	/// A query received data.
	Updated(DehydratedQuery),
}

/// A live sync connection. Dropping it stops syncing; the transport is
/// closed with it.
pub struct Broadcast {
	/// Keeps the cache subscription that sends updates out.
	outgoing: Option<Subscription>,
	transport: Option<Arc<dyn BroadcastTransport>>,
}

impl Broadcast {
	/// A connection that syncs nothing, for hosts without a transport.
	#[must_use]
	pub fn disconnected() -> Self {
		Self {
			outgoing: None,
			transport: None,
		}
	}

	/// Whether this connection is syncing.
	#[must_use]
	pub fn is_connected(&self) -> bool {
		self.outgoing.is_some() && self.transport.is_some()
	}
}

impl fmt::Debug for Broadcast {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Broadcast")
			.field("connected", &self.is_connected())
			.finish()
	}
}

impl QueryClient {
	/// Syncs this cache with every other browser tab on `channel`, over a
	/// `BroadcastChannel`.
	///
	/// Outside a browser build this syncs nothing; use
	/// [`broadcast_with`](Self::broadcast_with) and a transport the host
	/// provides instead.
	#[must_use]
	pub fn broadcast(&self, channel: &str) -> Broadcast {
		match browser::channel(channel) {
			Some(transport) => self.broadcast_with(transport),
			None => Broadcast::disconnected(),
		}
	}

	/// Syncs this cache with every other peer on `transport`.
	#[must_use]
	pub fn broadcast_with(&self, transport: impl BroadcastTransport) -> Broadcast {
		let transport: Arc<dyn BroadcastTransport> = Arc::new(transport);
		// Set while applying a peer's message, so the resulting cache event
		// is not echoed back to the peers that already have the data.
		let applying = Arc::new(AtomicBool::new(false));

		let outgoing = {
			let transport = Arc::clone(&transport);
			let applying = Arc::clone(&applying);
			let cache = Arc::clone(self.cache());
			self.subscribe(move |notification| {
				if notification.event != CacheEvent::Updated || applying.load(Ordering::SeqCst) {
					return;
				}
				let Some(query) = cache.get(notification.query.scope, notification.query.key_hash)
				else {
					return;
				};
				let Some(dehydrated) = query.dehydrate() else {
					return;
				};
				if let Ok(message) = serde_json::to_string(&BroadcastMessage::Updated(dehydrated)) {
					transport.send(&message);
				}
			})
		};

		let client = self.clone();
		transport.receive(Arc::new(move |raw| {
			let Ok(message) = serde_json::from_str::<BroadcastMessage>(raw) else {
				return;
			};
			applying.store(true, Ordering::SeqCst);
			match message {
				BroadcastMessage::Updated(dehydrated) => client.cache().hydrate(dehydrated),
			}
			applying.store(false, Ordering::SeqCst);
		}));

		Broadcast {
			outgoing: Some(outgoing),
			transport: Some(transport),
		}
	}
}

type PeerHandler = Arc<dyn Fn(&str) + Send + Sync>;

/// A transport for peers in the same process — the windows of a native app,
/// or several clients in a test.
///
/// ```
/// # use leptoskit_query::*;
/// # fn demo() {
/// let bus = LocalBroadcast::new();
/// let left = QueryClient::new();
/// let right = QueryClient::new();
/// let _left_sync = left.broadcast_with(bus.peer());
/// let _right_sync = right.broadcast_with(bus.peer());
/// // Data written in `left` now appears in `right`, and vice versa.
/// # }
/// ```
#[derive(Clone, Default)]
pub struct LocalBroadcast {
	peers: Arc<Mutex<Vec<(u64, PeerHandler)>>>,
	next_id: Arc<AtomicU64>,
}

impl LocalBroadcast {
	/// An empty bus.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// A new peer on this bus, to pass to
	/// [`QueryClient::broadcast_with`].
	#[must_use]
	pub fn peer(&self) -> LocalPeer {
		LocalPeer {
			bus: self.clone(),
			id: self.next_id.fetch_add(1, Ordering::Relaxed),
		}
	}

	/// How many peers are connected.
	#[must_use]
	pub fn len(&self) -> usize {
		self.peers.lock().len()
	}

	/// Whether no peer is connected.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

impl fmt::Debug for LocalBroadcast {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("LocalBroadcast")
			.field("peers", &self.len())
			.finish()
	}
}

/// One peer on a [`LocalBroadcast`].
pub struct LocalPeer {
	bus: LocalBroadcast,
	id: u64,
}

impl fmt::Debug for LocalPeer {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("LocalPeer")
			.field("id", &self.id)
			.finish_non_exhaustive()
	}
}

impl BroadcastTransport for LocalPeer {
	fn send(&self, message: &str) {
		// Handlers are cloned out so a peer may react by sending.
		let others: Vec<PeerHandler> = self
			.bus
			.peers
			.lock()
			.iter()
			.filter(|(id, _)| *id != self.id)
			.map(|(_, handler)| Arc::clone(handler))
			.collect();
		for handler in others {
			handler(message);
		}
	}

	fn receive(&self, handler: PeerHandler) {
		self.bus.peers.lock().push((self.id, handler));
	}
}

impl Drop for LocalPeer {
	fn drop(&mut self) {
		self.bus.peers.lock().retain(|(id, _)| *id != self.id);
	}
}

#[cfg(all(target_arch = "wasm32", feature = "browser"))]
mod browser {
	use std::cell::RefCell;
	use std::sync::Arc;

	use send_wrapper::SendWrapper;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen::closure::Closure;
	use web_sys::MessageEvent;

	use super::BroadcastTransport;

	type OnMessage = SendWrapper<RefCell<Option<Closure<dyn FnMut(MessageEvent)>>>>;

	/// A `BroadcastChannel`, which the browser delivers to every other tab of
	/// the same origin with a channel of the same name.
	pub(super) struct BrowserChannel {
		channel: SendWrapper<web_sys::BroadcastChannel>,
		on_message: OnMessage,
	}

	pub(super) fn channel(name: &str) -> Option<BrowserChannel> {
		web_sys::window()?;
		let channel = web_sys::BroadcastChannel::new(name).ok()?;
		Some(BrowserChannel {
			channel: SendWrapper::new(channel),
			on_message: SendWrapper::new(RefCell::new(None)),
		})
	}

	impl BroadcastTransport for BrowserChannel {
		fn send(&self, message: &str) {
			let _ = self.channel.post_message(&JsValue::from_str(message));
		}

		fn receive(&self, handler: Arc<dyn Fn(&str) + Send + Sync>) {
			let closure = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
				if let Some(text) = event.data().as_string() {
					handler(&text);
				}
			});
			self.channel
				.set_onmessage(Some(closure.as_ref().unchecked_ref()));
			*self.on_message.borrow_mut() = Some(closure);
		}
	}

	impl Drop for BrowserChannel {
		fn drop(&mut self) {
			self.channel.set_onmessage(None);
			self.channel.close();
		}
	}
}

#[cfg(not(all(target_arch = "wasm32", feature = "browser")))]
mod browser {
	use super::LocalPeer;

	/// No `BroadcastChannel` here; `QueryClient::broadcast` syncs nothing.
	pub(super) fn channel(_name: &str) -> Option<LocalPeer> {
		None
	}
}
