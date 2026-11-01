//! One [`For`](crate::reactive::For) or [`Show`](crate::reactive::Show)
//! subscribed to the signals its children read.
//!
//! The slot is the observer while those children are built, so a change queues
//! this container and nothing above it. The tree applies the queue when it
//! settles, after the turn that wrote the signal, rather than inside the
//! write: a handler can update several signals and the list rebuilds once.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Weak;

use reactive_graph::graph::AnySource;
use reactive_graph::graph::AnySubscriber;
use reactive_graph::graph::ReactiveNode;
use reactive_graph::graph::Source;
use reactive_graph::graph::Subscriber;
use reactive_graph::graph::ToAnySubscriber;
use reactive_graph::graph::WithObserver;

use crate::Element;
use crate::reactive::Region;
use crate::reactive::guest_render_scope;

thread_local! {
	static DIRTY: std::cell::RefCell<Vec<i64>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Node ids whose regions changed since the last settle, in the order they
/// changed. A container is listed once per turn.
pub(super) fn drain_dirty() -> Vec<i64> {
	DIRTY.with(|dirty| std::mem::take(&mut *dirty.borrow_mut()))
}

fn queue(id: i64) {
	DIRTY.with(|dirty| {
		let mut dirty = dirty.borrow_mut();

		if !dirty.contains(&id) {
			dirty.push(id);
		}
	});
}

/// The subscription for one region node.
pub(super) struct RegionSlot {
	id: i64,
	render: Arc<dyn Fn() -> Vec<(u64, Element)> + Send + Sync>,
	sources: Mutex<Vec<AnySource>>,
	this: AnySubscriber,
}

impl RegionSlot {
	/// Subscribes `region` as the children of node `id`.
	pub(super) fn new(id: i64, region: Region) -> Arc<Self> {
		Arc::new_cyclic(|weak: &Weak<Self>| {
			let subscriber = AnySubscriber(
				weak.as_ptr() as usize,
				Weak::clone(weak) as Weak<dyn Subscriber + Send + Sync>,
			);
			Self {
				id,
				render: region.into_render(),
				sources: Mutex::new(Vec::new()),
				this: subscriber,
			}
		})
	}

	/// The children now, subscribed to whatever the closure reads.
	///
	/// Binding reads inside the closure stay untracked: a label that changes
	/// is a refresh, not a new row.
	pub(super) fn evaluate(&self) -> Vec<(u64, Element)> {
		self.clear();
		let render = Arc::clone(&self.render);
		self.this.with_observer(|| guest_render_scope(|| render()))
	}

	fn clear(&self) {
		let sources = std::mem::take(
			&mut *self
				.sources
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner),
		);

		for source in sources {
			source.remove_subscriber(&self.this);
		}
	}
}

impl Drop for RegionSlot {
	fn drop(&mut self) {
		self.clear();
	}
}

impl ToAnySubscriber for RegionSlot {
	fn to_any_subscriber(&self) -> AnySubscriber {
		self.this.clone()
	}
}

impl ReactiveNode for RegionSlot {
	fn mark_dirty(&self) {
		queue(self.id);
	}

	fn mark_check(&self) {
		queue(self.id);
	}

	fn mark_subscribers_check(&self) {}

	fn update_if_necessary(&self) -> bool {
		false
	}
}

impl Subscriber for RegionSlot {
	fn add_source(&self, source: AnySource) {
		let mut sources = self
			.sources
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);

		if !sources.iter().any(|known| known == &source) {
			sources.push(source);
		}
	}

	fn clear_sources(&self, _subscriber: &AnySubscriber) {
		self.clear();
	}
}
