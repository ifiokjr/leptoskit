//! Reactive network state over the plugin's snapshot sources.
//!
//! [`use_network`] publishes the first snapshot, then follows changes until
//! the reactive owner is disposed or a source call fails. The host executor
//! owns the driving future; nothing is spawned internally.

use std::future::Future;
use std::pin::Pin;

use reactive_graph::prelude::*;
use reactive_graph::signal::ReadSignal;
use reactive_graph::signal::RwSignal;

use crate::Error;
use crate::Snapshot;

/// One source operation, polled by the caller's executor.
type SourceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// The future that keeps a network signal current.
pub type NetworkTask = Pin<Box<dyn Future<Output = Result<(), Error>>>>;

mod private {
	/// Seals [`Source`](super::Source) to this crate's implementations.
	pub trait Sealed {}
}

/// Snapshot sources usable with [`use_network`]: the interpreter's client and
/// the browser API. Sealed; test doubles live in this crate.
pub trait Source: private::Sealed + Send + Sync + 'static {
	/// Reads the current snapshot.
	fn snapshot(&self) -> SourceFuture<'_, Result<Snapshot, Error>>;
	/// Resolves once the platform differs from `previous`.
	fn wait_for_change(&self, previous: Snapshot) -> SourceFuture<'_, Result<Snapshot, Error>>;
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
impl private::Sealed for crate::guest::Network {}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
impl Source for crate::guest::Network {
	fn snapshot(&self) -> SourceFuture<'_, Result<Snapshot, Error>> {
		let client = *self;
		Box::pin(async move { client.snapshot().await })
	}

	fn wait_for_change(&self, previous: Snapshot) -> SourceFuture<'_, Result<Snapshot, Error>> {
		let client = *self;
		Box::pin(async move { client.wait_for_change(previous).await })
	}
}

/// Browser Network Information API source; unsupported browsers fail with
/// `Unavailable`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
#[derive(Debug, Clone, Copy)]
pub struct WebNetwork;

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
impl private::Sealed for WebNetwork {}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
impl Source for WebNetwork {
	fn snapshot(&self) -> SourceFuture<'_, Result<Snapshot, Error>> {
		Box::pin(std::future::ready(crate::web::snapshot()))
	}

	fn wait_for_change(&self, previous: Snapshot) -> SourceFuture<'_, Result<Snapshot, Error>> {
		Box::pin(crate::web::wait_for_change(previous))
	}
}

/// Reactive battery state.
pub struct UseNetworkReturn {
	/// `None` until the first snapshot resolves; later failures keep the last
	/// published value.
	pub value: ReadSignal<Option<Snapshot>>,
	/// Spawn once on the host executor. Completes with `Ok` when the owner is
	/// disposed, or with the first source error.
	pub task: NetworkTask,
}

/// Publishes network snapshots into a signal inside a Leptos owner.
///
/// The task performs one snapshot, then repeatedly waits for platform
/// changes; each result replaces the signal's value. Owner disposal ends the
/// task cleanly once the in-flight operation resolves, and dropping the task
/// cancels its outstanding wait. The plugin allows one waiter, so drive one
/// task per owner.
pub fn use_network(source: impl Source) -> UseNetworkReturn {
	let value = RwSignal::new(None);
	let task = Box::pin(async move {
		let mut previous = source.snapshot().await?;
		if value.try_set(Some(previous)).is_some() {
			return Ok(());
		}
		loop {
			if value.is_disposed() {
				return Ok(());
			}

			let next = source.wait_for_change(previous).await?;
			if value.try_set(Some(next)).is_some() {
				return Ok(());
			}

			previous = next;
		}
	});
	UseNetworkReturn {
		value: value.read_only(),
		task,
	}
}

#[cfg(all(not(target_arch = "wasm32"), test))]
mod tests {
	use std::collections::VecDeque;
	use std::pin::Pin;
	use std::sync::Mutex;
	use std::sync::atomic::AtomicBool;
	use std::sync::atomic::Ordering;
	use std::task::Context;
	use std::task::Poll;
	use std::task::Waker;

	use futures::executor::block_on;
	use futures::task::noop_waker;
	use reactive_graph::owner::Owner;
	use reactive_graph::prelude::Get;

	use super::Source;
	use super::use_network;
	use crate::Connection;
	use crate::Error;
	use crate::Snapshot;

	fn online(connected: bool) -> Snapshot {
		Snapshot {
			connection: Connection::Wifi,
			connected: Some(connected),
			internet_validated: None,
			expensive: None,
			data_saver: None,
			downlink_kbps: None,
			round_trip_ms: None,
			effective_type: crate::EffectiveType::Unknown,
		}
	}

	/// Yields queued results in order; an empty queue reports `Unavailable`.
	struct Fake {
		calls: Mutex<VecDeque<Result<Snapshot, Error>>>,
		delay_first_wait: AtomicBool,
	}

	impl Fake {
		fn new(calls: Vec<Result<Snapshot, Error>>) -> Self {
			Self {
				calls: Mutex::new(calls.into()),
				delay_first_wait: AtomicBool::new(false),
			}
		}

		fn pop(&self) -> Result<Snapshot, Error> {
			self.calls
				.lock()
				.unwrap()
				.pop_front()
				.unwrap_or(Err(Error::Unavailable))
		}
	}

	impl super::private::Sealed for Fake {}

	impl Source for Fake {
		fn snapshot(&self) -> super::SourceFuture<'_, Result<Snapshot, Error>> {
			Box::pin(std::future::ready(self.pop()))
		}

		fn wait_for_change(
			&self,
			_previous: Snapshot,
		) -> super::SourceFuture<'_, Result<Snapshot, Error>> {
			let result = self.pop();
			let delay = self.delay_first_wait.swap(false, Ordering::Relaxed);
			Box::pin(PendingOnce { result, delay })
		}
	}

	/// Pends once when asked, then yields. Lets tests act between polls.
	struct PendingOnce {
		result: Result<Snapshot, Error>,
		delay: bool,
	}

	impl Future for PendingOnce {
		type Output = Result<Snapshot, Error>;

		fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
			if self.delay {
				self.delay = false;
				context.waker().wake_by_ref();

				return Poll::Pending;
			}

			Poll::Ready(self.result)
		}
	}

	#[test]
	fn publishes_snapshots_until_the_source_fails() {
		let owner = Owner::new();
		let source = Fake::new(vec![Ok(online(true)), Ok(online(false)), Err(Error::Busy)]);
		let hook = owner.with(|| use_network(source));
		assert_eq!(hook.value.get(), None);
		assert_eq!(block_on(hook.task), Err(Error::Busy));
		assert_eq!(
			hook.value.get().map(|snapshot| snapshot.connected),
			Some(Some(false))
		);
		owner.cleanup();
	}

	#[test]
	fn owner_disposal_during_a_wait_ends_the_task_cleanly() {
		let owner = Owner::new();
		let source = Fake::new(vec![Ok(online(true)), Ok(online(false))]);
		source.delay_first_wait.store(true, Ordering::Relaxed);
		let hook = owner.with(|| use_network(source));
		let mut task = hook.task;
		let waker: Waker = noop_waker();
		let mut context = Context::from_waker(&waker);
		assert!(task.as_mut().poll(&mut context).is_pending());

		owner.cleanup();
		assert_eq!(task.as_mut().poll(&mut context), Poll::Ready(Ok(())));
	}

	#[test]
	fn initial_snapshot_failure_leaves_the_signal_empty() {
		let owner = Owner::new();
		let source = Fake::new(vec![Err(Error::PermissionDenied)]);
		let hook = owner.with(|| use_network(source));
		assert_eq!(block_on(hook.task), Err(Error::PermissionDenied));
		assert_eq!(hook.value.get(), None);
		owner.cleanup();
	}
}
