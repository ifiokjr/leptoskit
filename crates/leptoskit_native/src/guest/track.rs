//! Whether the mounted view itself reads application state.
//!
//! A binding read does not count: those update by refresh. A read in the view
//! body does, because the tree that read built is how that value reaches the
//! screen, and the next write has to build it again.
//! [`For`](crate::reactive::For) and [`Show`](crate::reactive::Show) read
//! inside their own subscription, so a list changing does not make the screen
//! one of these reads.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use reactive_graph::graph::AnySource;
use reactive_graph::graph::AnySubscriber;
use reactive_graph::graph::ReactiveNode;
use reactive_graph::graph::Source;
use reactive_graph::graph::Subscriber;
use reactive_graph::graph::ToAnySubscriber;
use reactive_graph::graph::WithObserver;

use crate::reactive::guest_render_scope;

thread_local! {
	static PROBE: std::cell::RefCell<Option<Arc<RootProbe>>> = const { std::cell::RefCell::new(None) };
}

fn probe() -> Arc<RootProbe> {
	PROBE.with(|slot| {
		let mut slot = slot.borrow_mut();
		slot.get_or_insert_with(RootProbe::new).clone()
	})
}

/// Builds the view once, tracking signals it reads outside a binding.
///
/// The flag is whether anything was read. A later write to one of those
/// signals sets [`take_root_dirty`].
pub(super) fn render_tracking<T>(build: impl FnOnce() -> T) -> (T, bool) {
	let probe = probe();
	probe.clear();
	probe.tracked.store(false, Ordering::Relaxed);
	let value = probe.this.with_observer(|| guest_render_scope(build));
	(value, probe.tracked.load(Ordering::Relaxed))
}

/// Whether a signal the mounted view reads has changed since the last take.
pub(super) fn take_root_dirty() -> bool {
	probe().dirty.swap(false, Ordering::Relaxed)
}

/// How the mounted view hears about state.
///
/// A tracked view rebuilds when a signal it read changes. A dynamic view only
/// binds values or uses [`For`](crate::reactive::For) and
/// [`Show`](crate::reactive::Show), so it settles those and does not rebuild.
/// A plain view stores its state in ordinary values: an event rebuilds it,
/// because that is how the new value reaches the screen, and a flush does not.
#[derive(Clone, Copy)]
pub(super) enum ViewKind {
	/// The view read a signal. `dirty` is whether that signal has changed.
	Tracked { dirty: bool },
	/// Bindings or regions, and no read in the view body.
	Dynamic,
	/// No signals at all.
	Plain,
}

/// Whether this turn should build the view again before settling.
///
/// `on_event` is the plain view's rebuild: a press has to show the new value,
/// and a flush has nothing that asked for one.
pub(super) fn needs_render(kind: ViewKind, on_event: bool) -> bool {
	match kind {
		ViewKind::Tracked { dirty } => dirty,
		ViewKind::Dynamic => false,
		ViewKind::Plain => on_event,
	}
}

struct RootProbe {
	tracked: AtomicBool,
	dirty: AtomicBool,
	sources: Mutex<Vec<AnySource>>,
	this: AnySubscriber,
}

impl RootProbe {
	fn new() -> Arc<Self> {
		Arc::new_cyclic(|weak: &Weak<Self>| {
			let subscriber = AnySubscriber(
				weak.as_ptr() as usize,
				Weak::clone(weak) as Weak<dyn Subscriber + Send + Sync>,
			);
			Self {
				tracked: AtomicBool::new(false),
				dirty: AtomicBool::new(false),
				sources: Mutex::new(Vec::new()),
				this: subscriber,
			}
		})
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

impl ToAnySubscriber for RootProbe {
	fn to_any_subscriber(&self) -> AnySubscriber {
		self.this.clone()
	}
}

impl ReactiveNode for RootProbe {
	fn mark_dirty(&self) {
		self.dirty.store(true, Ordering::Relaxed);
	}

	fn mark_check(&self) {
		self.dirty.store(true, Ordering::Relaxed);
	}

	fn mark_subscribers_check(&self) {}

	fn update_if_necessary(&self) -> bool {
		false
	}
}

impl Subscriber for RootProbe {
	fn add_source(&self, source: AnySource) {
		self.tracked.store(true, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
	use super::ViewKind;
	use super::needs_render;
	use super::render_tracking;
	use super::take_root_dirty;
	use crate::Element;
	use crate::reactive::Get;
	use crate::reactive::RwSignal;
	use crate::reactive::Set;

	fn owner() -> reactive_graph::owner::Owner {
		let owner = reactive_graph::owner::Owner::new();
		owner.set();
		owner
	}

	#[test]
	fn a_binding_does_not_subscribe_the_view_that_builds_it() {
		let _owner = owner();
		let count = RwSignal::new(0);
		let (_element, tracked) =
			render_tracking(|| Element::text_from(move || count.get().to_string()));
		assert!(!tracked, "the label subscribes itself, not the screen");
		count.set(1);
		assert!(
			!take_root_dirty(),
			"writing the label does not ask the screen to rebuild"
		);
	}

	#[test]
	fn a_read_in_the_view_body_subscribes_the_view() {
		let _owner = owner();
		let count = RwSignal::new(0);
		let (_element, tracked) = render_tracking(|| {
			let _value = count.get();
			Element::view()
		});
		assert!(tracked);
		count.set(1);
		assert!(take_root_dirty());
	}

	#[test]
	fn only_a_dirty_tracked_view_or_a_plain_event_rebuilds() {
		assert!(needs_render(ViewKind::Tracked { dirty: true }, true));
		assert!(!needs_render(ViewKind::Tracked { dirty: false }, true));
		assert!(!needs_render(ViewKind::Dynamic, true));
		assert!(needs_render(ViewKind::Plain, true));
		assert!(!needs_render(ViewKind::Plain, false));
	}
}
