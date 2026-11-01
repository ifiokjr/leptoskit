//! Shared helpers for the integration tests.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptos::prelude::*;
use leptoskit_query::*;

/// Points the reactive runtime at tokio and gives this thread a root owner.
/// Idempotent.
///
/// The root owner matters once workspace feature unification turns on
/// `reactive_graph`'s `sandboxed-arenas`, as `cargo test --workspace` and
/// nextest do. Handles are then readable only while an arena is active, and
/// the tests read them outside any `owner.with`, just as event handlers and
/// views do inside a mounted app.
pub fn init() {
	let _ = any_spawner::Executor::init_tokio();
	thread_local! {
		static ROOT: Owner = Owner::new();
	}
	ROOT.with(Owner::set);
}

/// A client whose queries ignore window focus and network events, so tests
/// running in the same process cannot disturb each other's call counts.
pub fn quiet_client() -> QueryClient {
	init();
	QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never)
			.with_network_mode(NetworkMode::Always),
	)
}

/// Counts fetcher invocations across clones.
#[derive(Clone, Debug, Default)]
pub struct Counter(Arc<AtomicU32>);

impl Counter {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn hit(&self) -> u32 {
		self.0.fetch_add(1, Ordering::SeqCst) + 1
	}

	pub fn get(&self) -> u32 {
		self.0.load(Ordering::SeqCst)
	}
}

/// Lets spawned tasks and effects run.
pub async fn settle() {
	for _ in 0..3 {
		any_spawner::Executor::tick().await;
	}

	tokio::time::sleep(Duration::from_millis(10)).await;
}

/// Polls `condition` until it holds, failing after a second.
pub async fn wait_for(label: &str, condition: impl Fn() -> bool) {
	for _ in 0..200 {
		if condition() {
			return;
		}

		tokio::time::sleep(Duration::from_millis(5)).await;
	}

	panic!("timed out waiting for: {label}");
}

/// Runs `setup` inside a fresh reactive owner, returning the owner so the
/// test can clean it up (which unmounts every observer created inside).
pub fn owned<T>(setup: impl FnOnce() -> T) -> (Owner, T) {
	let owner = Owner::new();
	let value = owner.with(setup);
	(owner, value)
}

/// The error type the test fetchers fail with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boom(pub &'static str);

type Gate = futures::channel::oneshot::Sender<Result<u32, Boom>>;

/// A fetcher whose every call waits until the test answers it, so a test
/// decides exactly when, and in which order, responses arrive.
#[derive(Clone, Debug, Default)]
pub struct Gates {
	pending: Arc<parking_lot::Mutex<Vec<Option<Gate>>>>,
}

impl Gates {
	pub fn new() -> Self {
		Self::default()
	}

	/// A scope over the gated fetcher that never retries.
	pub fn scope(&self, name: &str) -> QueryScope<u32, u32, Boom> {
		let gates = self.clone();
		QueryScope::new(move |_key: u32| {
			let (sender, receiver) = futures::channel::oneshot::channel();
			gates.pending.lock().push(Some(sender));
			async move { receiver.await.unwrap_or(Err(Boom("gate dropped"))) }
		})
		.with_name(name)
		.with_retry(Retry::never())
	}

	/// How many times the fetcher has been called.
	pub fn calls(&self) -> usize {
		self.pending.lock().len()
	}

	/// Answers the `call`-th fetch, counting from zero. A fetch that was
	/// cancelled meanwhile no longer listens, which is fine.
	pub fn answer(&self, call: usize, result: Result<u32, Boom>) {
		let gate = self.pending.lock()[call]
			.take()
			.expect("each call is answered once");
		let _ = gate.send(result);
	}
}
