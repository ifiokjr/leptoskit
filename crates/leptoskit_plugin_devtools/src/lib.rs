//! The bridge plugin that carries a native guest's `DevTools` data to its
//! host.
//!
//! A native app's host reports the element tree to the hub itself, but only
//! the guest knows its route, queries, forms, and named state. The guest's
//! `DevTools` agent (in `leptoskit_devtools`) sends those over this plugin,
//! and the host merges them into the session it already reports. Commands
//! for the guest, such as `SetState` and `Navigate`, come back the same way.
//!
//! Apps never select this plugin. The shared app runtime installs it when
//! `leptoskit dev` builds the host with `leptoskit_bridge_ffi/devtools`, and a
//! guest on any other host finds no plugin and reports nothing.
//!
//! Four methods:
//!
//! - [`HELLO`] takes a [`Hello`]: the protocol the guest speaks and what it can
//!   feed. It returns nothing.
//! - [`REPORT`] takes a [`Report`]: values that changed since the last one, or,
//!   with `reset`, every value the guest holds. It returns nothing.
//! - [`COMMANDS`] takes nothing and returns, once the hub has sent any, up to
//!   [`MAX_COMMANDS`] commands for the guest. The guest keeps one such call
//!   open.
//! - [`WAIT`] takes a little-endian `u32` of milliseconds, at most
//!   [`MAX_WAIT_MS`], and returns nothing once that long has passed on the
//!   host's clock. A guest has no timers; this is how it waits out its debounce
//!   without asking for display frames.
//!
//! The first three carry JSON.
//!
//! Every payload is bounded: a hello by [`MAX_HELLO_BYTES`], a report by
//! [`MAX_REPORT_BYTES`] ([`Report::encode`] splits a larger one), and a reply
//! to [`COMMANDS`] by [`MAX_COMMANDS_BYTES`].

use std::collections::BTreeSet;
use std::collections::VecDeque;

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::PROTOCOL_VERSION;
use serde::Deserialize;
use serde::Serialize;

#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
mod host;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::DevtoolsPlugin;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::Inbox;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::Link;

/// The plugin's name. It is reserved for the runtime, like the frame clock's
/// `leptoskit.frame`.
pub const NAME: &str = "leptoskit.devtools";
/// A [`Hello`] -> nothing.
pub const HELLO: MethodId = MethodId(0);
/// A [`Report`] -> nothing.
pub const REPORT: MethodId = MethodId(1);
/// Nothing -> the hub's commands for the guest, once there are any.
pub const COMMANDS: MethodId = MethodId(2);
/// Milliseconds -> nothing, after that long.
pub const WAIT: MethodId = MethodId(3);
/// Every method.
pub const METHODS: &[MethodId] = &[HELLO, REPORT, COMMANDS, WAIT];

/// The longest [`WAIT`].
pub const MAX_WAIT_MS: u32 = 1_000;

/// The largest encoded [`Hello`].
pub const MAX_HELLO_BYTES: usize = 4 * 1024;
/// The largest encoded [`Report`]. Values are at most 64 KiB each
/// (`MAX_VALUE_BYTES`), so a report holds at least fifteen.
pub const MAX_REPORT_BYTES: usize = 1024 * 1024;
/// The largest reply to [`COMMANDS`].
pub const MAX_COMMANDS_BYTES: usize = 1024 * 1024;
/// The most commands one reply carries, and the most the host holds for a
/// guest that isn't asking; past that the oldest are dropped.
pub const MAX_COMMANDS: usize = 64;

/// What a guest tells its host first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
	/// The `DevTools` protocol version the guest was built with.
	pub protocol: u32,
	/// What the guest can feed and accept: navigation, state, and the rest.
	pub capabilities: BTreeSet<Capability>,
}

impl Hello {
	/// A hello for this build's protocol.
	#[must_use]
	pub fn new(capabilities: impl IntoIterator<Item = Capability>) -> Self {
		Self {
			protocol: PROTOCOL_VERSION,
			capabilities: capabilities.into_iter().collect(),
		}
	}

	/// The payload for [`HELLO`].
	///
	/// # Errors
	///
	/// [`Error::TooLarge`] past [`MAX_HELLO_BYTES`].
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		let bytes = serde_json::to_vec(self).map_err(|_| Error::InvalidArgument)?;
		bounded(bytes, MAX_HELLO_BYTES)
	}

	/// Reads a [`HELLO`] payload.
	///
	/// # Errors
	///
	/// [`Error::TooLarge`] past [`MAX_HELLO_BYTES`], and
	/// [`Error::InvalidArgument`] for anything that isn't a hello for this
	/// build's protocol.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() > MAX_HELLO_BYTES {
			return Err(Error::TooLarge);
		}

		let hello: Self = serde_json::from_slice(bytes).map_err(|_| Error::InvalidArgument)?;

		if hello.protocol != PROTOCOL_VERSION {
			return Err(Error::InvalidArgument);
		}

		Ok(hello)
	}
}

/// Values a guest reports.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
	/// Whether these values replace every value the host holds for the
	/// guest, rather than updating them. A guest resets when it starts, when
	/// the hub asks for a snapshot, after a report the host refused, and when
	/// a value went away.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub reset: bool,
	/// The values, as the events the hub receives: `RouteChanged`,
	/// `QueryChanged`, `MutationChanged`, `FormChanged`, and `StateChanged`.
	pub events: Vec<Event>,
}

impl Report {
	/// The payloads for [`REPORT`], each at most [`MAX_REPORT_BYTES`], to send
	/// in order. A reset rides in the first. An event that can't fit in a
	/// payload on its own is left out, which values bounded by
	/// `MAX_VALUE_BYTES` never are.
	///
	/// # Errors
	///
	/// [`Error::InvalidArgument`] for an event that isn't a value.
	pub fn encode(&self) -> Result<Vec<Vec<u8>>, Error> {
		const OPEN_RESET: &[u8] = br#"{"reset":true,"events":["#;
		const OPEN: &[u8] = br#"{"events":["#;
		const CLOSE: &[u8] = b"]}";

		let mut payloads = Vec::new();

		let mut payload: Vec<u8> = (if self.reset { OPEN_RESET } else { OPEN }).to_vec();
		let mut empty = true;

		for event in &self.events {
			if !is_value(event) {
				return Err(Error::InvalidArgument);
			}

			let encoded = serde_json::to_vec(event).map_err(|_| Error::InvalidArgument)?;

			if OPEN_RESET.len() + encoded.len() + CLOSE.len() > MAX_REPORT_BYTES {
				continue;
			}

			if !empty && payload.len() + 1 + encoded.len() + CLOSE.len() > MAX_REPORT_BYTES {
				payload.extend_from_slice(CLOSE);

				payloads.push(std::mem::replace(&mut payload, OPEN.to_vec()));
				empty = true;
			}

			if !empty {
				payload.push(b',');
			}

			payload.extend_from_slice(&encoded);
			empty = false;
		}

		if !empty || payloads.is_empty() {
			payload.extend_from_slice(CLOSE);
			payloads.push(payload);
		}

		Ok(payloads)
	}

	/// Reads a [`REPORT`] payload.
	///
	/// # Errors
	///
	/// [`Error::TooLarge`] past [`MAX_REPORT_BYTES`], and
	/// [`Error::InvalidArgument`] for anything that isn't a report of values.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() > MAX_REPORT_BYTES {
			return Err(Error::TooLarge);
		}

		let report: Self = serde_json::from_slice(bytes).map_err(|_| Error::InvalidArgument)?;

		if !report.events.iter().all(is_value) {
			return Err(Error::InvalidArgument);
		}

		Ok(report)
	}
}

/// Whether `event` is one of the values a guest reports.
#[must_use]
pub fn is_value(event: &Event) -> bool {
	matches!(
		event,
		Event::RouteChanged { .. }
			| Event::QueryChanged { .. }
			| Event::MutationChanged { .. }
			| Event::FormChanged { .. }
			| Event::StateChanged { .. }
	)
}

/// Whether `command` is for the guest, rather than the host: navigating,
/// editing state and queries, and a snapshot request, which the guest
/// answers with a reset.
#[must_use]
pub fn is_for_guest(command: &Command) -> bool {
	matches!(
		command,
		Command::Navigate { .. }
			| Command::InvalidateQuery { .. }
			| Command::RefetchQuery { .. }
			| Command::SetQueryData { .. }
			| Command::SetState { .. }
			| Command::RequestSnapshot
	)
}

/// The reply to [`COMMANDS`]: the oldest queued commands, at most
/// [`MAX_COMMANDS`] of them and [`MAX_COMMANDS_BYTES`] in all, taken out of
/// `queue`. A command too large for a reply on its own is dropped.
#[must_use]
pub fn encode_commands(queue: &mut VecDeque<Command>) -> Vec<u8> {
	let mut reply = vec![b'['];
	let mut count = 0;

	while count < MAX_COMMANDS {
		let Some(command) = queue.front() else {
			break;
		};
		let Ok(encoded) = serde_json::to_vec(command) else {
			queue.pop_front();
			continue;
		};

		if encoded.len() + 2 > MAX_COMMANDS_BYTES {
			queue.pop_front();

			continue;
		}

		if reply.len() + 1 + encoded.len() + 1 > MAX_COMMANDS_BYTES {
			break;
		}

		if count > 0 {
			reply.push(b',');
		}

		reply.extend_from_slice(&encoded);
		queue.pop_front();
		count += 1;
	}

	reply.push(b']');
	reply
}

/// Reads a reply to [`COMMANDS`], keeping only the commands for the guest.
///
/// # Errors
///
/// [`Error::TooLarge`] past [`MAX_COMMANDS_BYTES`], and
/// [`Error::InvalidArgument`] for anything that isn't a list of commands.
pub fn decode_commands(bytes: &[u8]) -> Result<Vec<Command>, Error> {
	if bytes.len() > MAX_COMMANDS_BYTES {
		return Err(Error::TooLarge);
	}

	let commands: Vec<Command> =
		serde_json::from_slice(bytes).map_err(|_| Error::InvalidArgument)?;
	Ok(commands.into_iter().filter(is_for_guest).collect())
}

/// The payload for [`WAIT`].
#[must_use]
pub fn encode_wait(ms: u32) -> [u8; 4] {
	ms.min(MAX_WAIT_MS).to_le_bytes()
}

/// Reads a [`WAIT`] payload.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for anything but four bytes of at most
/// [`MAX_WAIT_MS`].
pub fn decode_wait(bytes: &[u8]) -> Result<u32, Error> {
	let bytes: [u8; 4] = bytes.try_into().map_err(|_| Error::InvalidArgument)?;
	let ms = u32::from_le_bytes(bytes);

	if ms > MAX_WAIT_MS {
		return Err(Error::InvalidArgument);
	}

	Ok(ms)
}

fn bounded(bytes: Vec<u8>, limit: usize) -> Result<Vec<u8>, Error> {
	if bytes.len() > limit {
		Err(Error::TooLarge)
	} else {
		Ok(bytes)
	}
}

/// The guest's client.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	use leptoskit_bridge::Error;
	use leptoskit_bridge::guest::Call;
	use leptoskit_bridge::guest::Client;

	use super::COMMANDS;
	use super::HELLO;
	use super::Hello;
	use super::NAME;
	use super::REPORT;
	use super::WAIT;
	use super::encode_wait;

	/// The host's `DevTools` plugin, resolved once.
	#[derive(Debug, Clone, Copy)]
	pub struct Devtools(Client);

	impl Devtools {
		/// Resolves the plugin.
		///
		/// # Errors
		///
		/// [`Error::Unavailable`] on a host that doesn't report to a hub:
		/// any host outside `leptoskit dev`.
		pub fn new() -> Result<Self, Error> {
			Client::new(NAME).map(Self)
		}

		/// Says what the guest feeds. The call resolves once the host has it.
		///
		/// # Errors
		///
		/// When the hello is too large, or the bridge refuses the call.
		pub fn hello(self, hello: &Hello) -> Result<Call, Error> {
			self.0.call(HELLO, &hello.encode()?)
		}

		/// Sends one payload from [`super::Report::encode`].
		///
		/// # Errors
		///
		/// When the bridge refuses the call.
		pub fn report(self, payload: &[u8]) -> Result<Call, Error> {
			self.0.call(REPORT, payload)
		}

		/// Waits for the hub's next commands; read the reply with
		/// [`super::decode_commands`].
		///
		/// # Errors
		///
		/// When the bridge refuses the call.
		pub fn commands(self) -> Result<Call, Error> {
			self.0.call(COMMANDS, &[])
		}

		/// Resolves after `ms` milliseconds, at most [`super::MAX_WAIT_MS`],
		/// on the host's clock.
		///
		/// # Errors
		///
		/// When the bridge refuses the call.
		pub fn wait(self, ms: u32) -> Result<Call, Error> {
			self.0.call(WAIT, &encode_wait(ms))
		}
	}
}

#[cfg(test)]
mod tests;
