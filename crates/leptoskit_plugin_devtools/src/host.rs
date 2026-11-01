//! The host's half: hands each report to the host's `DevTools` reporter, and
//! holds the guest's [`COMMANDS`] call open until the hub sends something.
//!
//! The plugin is async so that call can wait without a thread: the shared
//! app runtime runs it on its async host, and the platform resumes the guest
//! when the reply is ready, as it does for any async plugin. `WAIT` sleeps
//! on the same runtime's clock.

use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::pin::pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::task::Poll;
use std::task::Waker;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Reply;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_bridge::async_host::PluginFuture;
use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;

use super::COMMANDS;
use super::HELLO;
use super::Hello;
use super::MAX_COMMANDS;
use super::MAX_COMMANDS_BYTES;
use super::METHODS;
use super::NAME;
use super::REPORT;
use super::Report;
use super::WAIT;
use super::decode_wait;
use super::encode_commands;
use super::is_for_guest;

/// The most open calls an inbox wakes.
const MAX_WAKERS: usize = 8;

/// Where the plugin hands what a guest says: the host's `DevTools` reporter.
pub trait Link: Send + Sync + 'static {
	/// The guest said what it can feed.
	fn hello(&self, capabilities: BTreeSet<Capability>);

	/// The guest reported values.
	fn report(&self, report: Report);
}

/// The commands waiting for one guest. The reporter pushes them as the hub
/// sends them, and the guest's open [`COMMANDS`] call takes them.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Waiting>>);

#[derive(Default)]
struct Waiting {
	commands: VecDeque<Command>,
	/// The open calls, woken when a command arrives.
	wakers: Vec<Waker>,
	closed: bool,
}

impl Inbox {
	/// A new, empty inbox.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Queues `command` for the guest, if it is one the guest handles. Past
	/// [`MAX_COMMANDS`] waiting, the oldest is dropped.
	pub fn push(&self, command: Command) {
		if !is_for_guest(&command) {
			return;
		}
		let wakers = {
			let mut waiting = self.lock();
			if waiting.closed {
				return;
			}
			if waiting.commands.len() == MAX_COMMANDS {
				waiting.commands.pop_front();
			}
			waiting.commands.push_back(command);
			std::mem::take(&mut waiting.wakers)
		};
		wakers.into_iter().for_each(Waker::wake);
	}

	/// Stops taking commands, and ends the open calls.
	pub fn close(&self) {
		let wakers = {
			let mut waiting = self.lock();
			waiting.closed = true;
			waiting.commands.clear();
			std::mem::take(&mut waiting.wakers)
		};
		wakers.into_iter().for_each(Waker::wake);
	}

	/// The reply for the next commands, once there are any; `None` once the
	/// inbox closes.
	async fn next(&self) -> Option<Bytes> {
		std::future::poll_fn(|context| {
			let mut waiting = self.lock();
			if waiting.closed {
				return Poll::Ready(None);
			}
			if waiting.commands.is_empty() {
				if !waiting
					.wakers
					.iter()
					.any(|waker| waker.will_wake(context.waker()))
				{
					// A guest keeps one call open; a few stale wakers from
					// calls it dropped are all this ever holds.
					if waiting.wakers.len() == MAX_WAKERS {
						waiting.wakers.remove(0);
					}
					waiting.wakers.push(context.waker().clone());
				}
				return Poll::Pending;
			}
			Poll::Ready(Some(Bytes::from(encode_commands(&mut waiting.commands))))
		})
		.await
	}

	/// Each lock only moves values in or out, so a panic can't leave it half
	/// changed.
	fn lock(&self) -> MutexGuard<'_, Waiting> {
		self.0.lock().unwrap_or_else(PoisonError::into_inner)
	}
}

/// The plugin one guest talks to.
pub struct DevtoolsPlugin {
	link: Arc<dyn Link>,
	inbox: Inbox,
}

impl DevtoolsPlugin {
	/// A plugin handing what the guest says to `link`, and answering its
	/// [`COMMANDS`] calls from `inbox`.
	#[must_use]
	pub fn new(link: impl Link, inbox: Inbox) -> Self {
		Self {
			link: Arc::new(link),
			inbox,
		}
	}

	async fn commands(&self, cancellation: CancellationToken) -> Result<Reply, Error> {
		let mut cancelled = pin!(cancellation.cancelled());
		let mut next = pin!(self.inbox.next());
		std::future::poll_fn(|context| {
			if cancelled.as_mut().poll(context).is_ready() {
				return Poll::Ready(Err(Error::Cancelled));
			}
			next.as_mut()
				.poll(context)
				.map(|reply| reply.map(Reply::from).ok_or(Error::Cancelled))
		})
		.await
	}

	/// Resolves after `ms`, on the async host's clock, or with `Cancelled`.
	async fn wait(ms: u32, cancellation: CancellationToken) -> Result<Reply, Error> {
		let mut cancelled = pin!(cancellation.cancelled());
		let mut sleep = pin!(tokio::time::sleep(Duration::from_millis(u64::from(ms))));
		std::future::poll_fn(|context| {
			if cancelled.as_mut().poll(context).is_ready() {
				return Poll::Ready(Err(Error::Cancelled));
			}
			sleep
				.as_mut()
				.poll(context)
				.map(|()| Ok(Reply::from(Bytes::new())))
		})
		.await
	}
}

impl AsyncPlugin for DevtoolsPlugin {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn reply_limit(&self, method: MethodId) -> Result<usize, Error> {
		match method {
			HELLO | REPORT | WAIT => Ok(0),
			COMMANDS => Ok(MAX_COMMANDS_BYTES),
			_ => Err(Error::Unavailable),
		}
	}

	fn invoke(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> PluginFuture<'_> {
		Box::pin(async move {
			match method {
				HELLO => {
					let hello = Hello::decode(&payload)?;
					self.link.hello(hello.capabilities);
					Ok(Reply::from(Bytes::new()))
				}
				REPORT => {
					self.link.report(Report::decode(&payload)?);
					Ok(Reply::from(Bytes::new()))
				}
				COMMANDS => self.commands(cancellation).await,
				WAIT => Self::wait(decode_wait(&payload)?, cancellation).await,
				_ => Err(Error::Unavailable),
			}
		})
	}

	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(async move { self.inbox.close() })
	}
}
