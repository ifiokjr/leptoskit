//! Shared helpers for the integration tests.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptos::prelude::*;
use leptoskit_query::*;

/// Points the reactive runtime at tokio; a no-op after the first call.
pub fn init() {
	let _ = any_spawner::Executor::init_tokio();
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
