//! What the web agent sends on each flush, kept apart from the DOM so the
//! decision can be tested on the host.
//!
//! A session starts with a snapshot: the tree and every value the hooks
//! report. After that a flush sends only the values that changed, as
//! `RouteChanged`, `QueryChanged`, and the rest. It sends a whole snapshot
//! again when the tree changed, when a value went away (the protocol has no
//! event for that), and after anything that may have lost a batch: a new
//! session, a `RequestSnapshot`, or a post that failed.

use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Tree;

use crate::registry;

/// What the hub has, as far as the agent knows.
#[derive(Debug, Default)]
pub(crate) struct Outbox {
	/// The JSON of the last tree the hub accepted, or `None` when the next
	/// batch must be a snapshot.
	sent_tree: Option<String>,
}

/// The events for one flush.
#[derive(Debug)]
pub(crate) struct Batch {
	pub(crate) events: Vec<Event>,
	/// The tree a snapshot carries, to remember once the hub accepts it.
	tree: Option<String>,
}

impl Outbox {
	/// Forgets what the hub has, so the next batch is a snapshot.
	pub(crate) fn reset(&mut self) {
		self.sent_tree = None;
	}

	/// The batch for one flush, or `None` when nothing changed.
	///
	/// `read_tree` reads the tree, which costs a layout on the web, so it
	/// runs only when the DOM changed or a snapshot is due.
	pub(crate) fn batch(
		&self,
		dom_changed: bool,
		read_tree: impl FnOnce() -> Tree,
	) -> Option<Batch> {
		let changes = registry::collect();
		// Server calls ride after whatever else the batch carries, so a
		// snapshot never replaces them.
		let calls = calls();
		let deltas = |mut events: Vec<Event>| {
			events.extend(calls.iter().cloned());
			(!events.is_empty()).then_some(Batch { events, tree: None })
		};

		if self.sent_tree.is_some() && !dom_changed && !changes.removed {
			return deltas(changes.events);
		}

		let tree = read_tree();
		// A tree that can't be serialized can't be posted either; the failed
		// post resets the outbox.
		let json = serde_json::to_string(&tree).unwrap_or_default();

		if !changes.removed && self.sent_tree.as_deref() == Some(json.as_str()) {
			// The DOM changed, but not the tree the hub has.
			return deltas(changes.events);
		}

		// A snapshot carries every value, so the changes ride along in it.
		let mut events = vec![Event::Snapshot {
			snapshot: registry::snapshot(tree),
		}];
		events.extend(calls);
		Some(Batch {
			events,
			tree: Some(json),
		})
	}

	/// The hub accepted `batch`.
	pub(crate) fn sent(&mut self, batch: Batch) {
		if let Some(tree) = batch.tree {
			self.sent_tree = Some(tree);
		}
	}
}

#[cfg(all(test, feature = "rpc"))]
std::thread_local! {
	/// The rpc timeline is process-wide and tests run in parallel, so only a
	/// test that opts in drains it; the rest never see its attempts.
	pub(crate) static DRAINS_CALLS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The server call attempts recorded since the last flush.
fn calls() -> Vec<Event> {
	#[cfg(feature = "rpc")]
	{
		#[cfg(test)]
		if !DRAINS_CALLS.get() {
			return Vec::new();
		}

		leptoskit_rpc::devtools::drain()
			.into_iter()
			.map(|attempt| Event::RpcAttempt { attempt })
			.collect()
	}
	#[cfg(not(feature = "rpc"))]
	Vec::new()
}
