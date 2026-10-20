#![cfg(not(target_arch = "wasm32"))]

//! Keeping several caches in sync over a [`LocalBroadcast`] bus.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::Boom;
use common::quiet_client;
use common::wait_for;
use leptos::prelude::*;
use leptoskit_query::*;

/// A scope whose data counts how many times it was produced.
fn counter(name: &'static str, codec: bool, calls: Arc<AtomicU32>) -> QueryScope<(), u32, Boom> {
	let scope = QueryScope::new(move |()| {
		let calls = Arc::clone(&calls);
		async move {
			calls.fetch_add(1, Ordering::SeqCst);
			Ok(42)
		}
	})
	.with_name(name)
	.with_stale_time(Duration::from_secs(60));
	if codec { scope.with_ssr() } else { scope }
}

#[tokio::test]
async fn data_written_in_one_cache_appears_in_the_other() {
	let left = quiet_client();
	let right = quiet_client();
	let bus = LocalBroadcast::new();
	let _left_sync = left.broadcast_with(bus.peer());
	let _right_sync = right.broadcast_with(bus.peer());
	assert_eq!(bus.len(), 2);

	let calls = Arc::new(AtomicU32::new(0));
	let scope = counter("shared", true, Arc::clone(&calls));
	assert_eq!(left.fetch_query(&scope, ()).await, Ok(42));

	wait_for("data lands in the peer", || {
		right.get_query_data(&scope, &()) == Some(42)
	})
	.await;
	assert_eq!(
		calls.load(Ordering::SeqCst),
		1,
		"the peer hydrated instead of fetching"
	);
}

#[tokio::test]
async fn data_for_a_query_that_does_not_exist_yet_arrives_when_it_does() {
	let left = quiet_client();
	let right = quiet_client();
	let bus = LocalBroadcast::new();
	let _left_sync = left.broadcast_with(bus.peer());
	let _right_sync = right.broadcast_with(bus.peer());

	let calls = Arc::new(AtomicU32::new(0));
	let scope = counter("lazy", true, Arc::clone(&calls));
	left.set_query_data(&scope, (), 7);
	assert!(!right.query_exists(&scope, &()));

	// Mounting the query in the peer creates it — and the broadcast data is
	// waiting. Disabled, so nothing fetches.
	let owner = Owner::new();
	owner.with(|| {
		use_query_in(
			right.clone(),
			scope.clone(),
			|| (),
			QueryObserverOptions::new().with_enabled(|| false),
		)
	});
	assert_eq!(
		right.get_query_data(&scope, &()),
		Some(7),
		"hydration was waiting for the query to exist"
	);
	assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn scopes_without_a_codec_stay_local() {
	let left = quiet_client();
	let right = quiet_client();
	let bus = LocalBroadcast::new();
	let _left_sync = left.broadcast_with(bus.peer());
	let _right_sync = right.broadcast_with(bus.peer());

	let calls = Arc::new(AtomicU32::new(0));
	let scope = counter("opaque", false, Arc::clone(&calls));
	assert_eq!(left.fetch_query(&scope, ()).await, Ok(42));
	tokio::time::sleep(Duration::from_millis(30)).await;
	assert!(
		!right.query_exists(&scope, &()),
		"nothing serializable to send"
	);
}

#[tokio::test]
async fn updates_do_not_echo_between_peers() {
	let left = quiet_client();
	let right = quiet_client();
	let bus = LocalBroadcast::new();
	let sent = Arc::new(AtomicU32::new(0));
	let _left_sync = left.broadcast_with(CountingPeer::new(&bus, &sent));
	let _right_sync = right.broadcast_with(CountingPeer::new(&bus, &sent));

	let calls = Arc::new(AtomicU32::new(0));
	let scope = counter("echo", true, Arc::clone(&calls));
	assert_eq!(left.fetch_query(&scope, ()).await, Ok(42));
	wait_for("data lands in the peer", || {
		right.get_query_data(&scope, &()) == Some(42)
	})
	.await;
	assert_eq!(
		sent.load(Ordering::SeqCst),
		1,
		"the peer's hydration is not broadcast back"
	);
	assert_eq!(
		left.get_query_state(&scope, &())
			.map(|state| state.meta.data_update_count),
		Some(1)
	);
}

#[tokio::test]
async fn fresher_local_data_wins_over_an_incoming_broadcast() {
	let left = quiet_client();
	let right = quiet_client();
	let bus = LocalBroadcast::new();
	let _left_sync = left.broadcast_with(bus.peer());
	let _right_sync = right.broadcast_with(bus.peer());

	let calls = Arc::new(AtomicU32::new(0));
	let scope = counter("fresh", true, Arc::clone(&calls));
	// The peer has newer data of its own; the broadcast carries older data.
	right.set_query_data_at(&scope, (), 9, now_ms() + 60_000);
	left.set_query_data(&scope, (), 1);

	tokio::time::sleep(Duration::from_millis(30)).await;
	assert_eq!(
		right.get_query_data(&scope, &()),
		Some(9),
		"older data from a peer does not clobber fresher local data"
	);
}

#[tokio::test]
async fn dropping_the_broadcast_stops_syncing() {
	let left = quiet_client();
	let right = quiet_client();
	let bus = LocalBroadcast::new();
	let left_sync = left.broadcast_with(bus.peer());
	let _right_sync = right.broadcast_with(bus.peer());

	let calls = Arc::new(AtomicU32::new(0));
	let scope = counter("gone", true, Arc::clone(&calls));
	left.set_query_data(&scope, (), 5);
	wait_for("synced", || right.get_query_data(&scope, &()) == Some(5)).await;

	assert!(left_sync.is_connected());
	drop(left_sync);
	left.set_query_data(&scope, (), 6);
	tokio::time::sleep(Duration::from_millis(30)).await;
	assert_eq!(
		right.get_query_data(&scope, &()),
		Some(5),
		"syncing stopped"
	);
}

/// A peer that counts the messages crossing it.
struct CountingPeer {
	peer: LocalPeer,
	sent: Arc<AtomicU32>,
}

impl CountingPeer {
	fn new(bus: &LocalBroadcast, sent: &Arc<AtomicU32>) -> Self {
		Self {
			peer: bus.peer(),
			sent: Arc::clone(sent),
		}
	}
}

impl BroadcastTransport for CountingPeer {
	fn send(&self, message: &str) {
		self.sent.fetch_add(1, Ordering::SeqCst);
		self.peer.send(message);
	}

	fn receive(&self, handler: Arc<dyn Fn(&str) + Send + Sync>) {
		self.peer.receive(handler);
	}
}
