use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;

use serde_json::Value;

use super::Host;
use super::Tree;
use super::decode_event;
use super::invoke;
use crate::Element;
use crate::EventOutcome;
use crate::NativeEvent;
use crate::NativeEventSlot;
use crate::NativeKind;
use crate::SliderState;

#[derive(Default)]
struct RecordedNode {
	kind: i32,
	props: Value,
	children: Vec<i64>,
}

#[derive(Default)]
struct RecordedHost {
	nodes: BTreeMap<i64, RecordedNode>,
	created: Vec<i64>,
	deleted: Vec<i64>,
	commits: usize,
}

impl Host for RecordedHost {
	fn create(&mut self, id: i64, kind: i32) {
		assert!(!self.nodes.contains_key(&id));
		self.nodes.insert(
			id,
			RecordedNode {
				kind,
				..RecordedNode::default()
			},
		);
		self.created.push(id);
	}

	fn props(&mut self, id: i64, bytes: &[u8]) {
		self.nodes.get_mut(&id).unwrap().props = serde_json::from_slice(bytes).unwrap();
	}

	fn insert(&mut self, parent: i64, child: i64, index: i32) {
		assert!(self.nodes.contains_key(&child));

		for node in self.nodes.values_mut() {
			node.children.retain(|&id| id != child);
		}

		self.nodes
			.get_mut(&parent)
			.unwrap()
			.children
			.insert(index as usize, child);
	}

	fn delete(&mut self, id: i64) {
		assert!(self.nodes.remove(&id).is_some());

		for node in self.nodes.values_mut() {
			node.children.retain(|&child| child != id);
		}

		self.deleted.push(id);
	}

	fn commit(&mut self) {
		let roots = self
			.nodes
			.keys()
			.filter(|id| !self.nodes.values().any(|node| node.children.contains(id)))
			.count();
		assert_eq!(roots, 1, "commits must contain exactly one connected tree");
		self.commits += 1;
	}
}

fn counter(count: &Arc<AtomicI32>) -> Element {
	let handler_count = Arc::clone(count);
	Element::view()
		.class("flex-col gap-4 p-6 dark:bg-slate-950")
		.child(Element::text(format!(
			"Count: {}",
			count.load(Ordering::Relaxed)
		)))
		.child(Element::button("Add one").on_press(move || {
			handler_count.fetch_add(1, Ordering::Relaxed);
		}))
}

#[test]
fn shared_counter_preserves_wire_ids_after_its_handler_changes_state() {
	let count = Arc::new(AtomicI32::new(0));
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(counter(&count), &mut host);
	let root = tree.root.unwrap();
	let children = host.nodes[&root].children.clone();
	let button = children[1];
	let (event, handlers) = tree.handlers(button, NativeEvent::Press).unwrap();
	assert_eq!(invoke(&event, &handlers), EventOutcome::Handled);
	tree.update(counter(&count), &mut host);

	assert_eq!(host.nodes[&children[0]].props["text"], "Count: 1");
	assert_eq!(host.nodes[&root].children, children);
	assert_eq!(host.nodes[&button].kind, 6);
	assert_eq!(host.created.len(), 3);
	assert!(host.deleted.is_empty());
	assert_eq!(host.commits, 2);
	assert_eq!(
		host.nodes[&root].props["class"],
		"dark:bg-slate-950 flex-col gap-4 p-6"
	);
}

#[test]
fn removing_and_replacing_subtrees_removes_stale_host_nodes() {
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(
		Element::view()
			.child(Element::view().child(Element::text("remove me")))
			.child(Element::button("keep me")),
		&mut host,
	);
	let root = tree.root.unwrap();
	let original = host.nodes[&root].children.clone();
	let removed_child = host.nodes[&original[0]].children[0];
	tree.update(
		Element::view()
			.child(Element::text("replacement"))
			.child(Element::button("keep me")),
		&mut host,
	);
	let children = &host.nodes[&root].children;

	assert_ne!(children[0], original[0]);
	assert_eq!(children[1], original[1]);
	assert_eq!(host.nodes.len(), 3);
	assert!(host.deleted.contains(&original[0]));
	assert!(host.deleted.contains(&removed_child));

	tree.update(Element::text("new root"), &mut host);
	assert_eq!(host.nodes.len(), 1);
	assert_eq!(host.nodes[&tree.root.unwrap()].props["text"], "new root");
}

#[test]
fn handlers_bubble_after_unhandled_children_and_stop_after_consumption() {
	let calls = Arc::new(Mutex::new(Vec::new()));
	let handler = |label, outcome| {
		let calls = Arc::clone(&calls);
		move |_: &NativeEvent| {
			calls.lock().unwrap().push(label);
			outcome
		}
	};
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(
		Element::view()
			.on(
				NativeEventSlot::Press,
				handler("root", EventOutcome::Handled),
			)
			.child(
				Element::view()
					.on(
						NativeEventSlot::Press,
						handler("parent", EventOutcome::Handled),
					)
					.child(Element::text("child").on(
						NativeEventSlot::Press,
						handler("child", EventOutcome::Unhandled),
					)),
			),
		&mut host,
	);
	let parent = host.nodes[&tree.root.unwrap()].children[0];
	let child = host.nodes[&parent].children[0];
	let (event, handlers) = tree.handlers(child, NativeEvent::Press).unwrap();

	assert_eq!(invoke(&event, &handlers), EventOutcome::Handled);
	assert_eq!(*calls.lock().unwrap(), ["child", "parent"]);
}

#[test]
fn disabled_ancestor_rejects_events_and_missing_targets_are_unhandled() {
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(
		Element::view()
			.enabled(false)
			.child(Element::button("inactive").on_press(|| {
				panic!("a disabled descendant must not dispatch");
			})),
		&mut host,
	);
	let child = host.nodes[&tree.root.unwrap()].children[0];
	assert!(tree.handlers(child, NativeEvent::Press).is_none());
	assert!(tree.handlers(999, NativeEvent::Press).is_none());
}

#[test]
fn controlled_values_accept_only_valid_widget_proposals() {
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(
		Element::view()
			.child(Element::new(NativeKind::Switch))
			.child(
				Element::new(NativeKind::Slider)
					.slider(SliderState::new(0.0, 10.0, 0.0, Some(2.0)).unwrap()),
			)
			.child(Element::text("label")),
		&mut host,
	);
	let nodes = &host.nodes[&tree.root.unwrap()].children;
	assert!(
		tree.handlers(nodes[0], NativeEvent::Toggle { checked: true })
			.is_some()
	);
	assert!(
		tree.handlers(nodes[2], NativeEvent::Toggle { checked: true })
			.is_none()
	);
	assert!(
		tree.handlers(nodes[0], NativeEvent::ValueChange { value: 4.0 })
			.is_none()
	);
	let (normalized, _) = tree
		.handlers(nodes[1], NativeEvent::ValueChange { value: 5.0 })
		.unwrap();
	assert_eq!(normalized, NativeEvent::ValueChange { value: 6.0 });
}

#[test]
fn event_payloads_decode_unicode_controls_and_reject_malformed_values() {
	assert_eq!(decode_event(0, b""), Some(NativeEvent::Press));
	assert_eq!(
		decode_event(1, br#"{"value":"a\u0000\u2713"}"#),
		Some(NativeEvent::Change {
			value: "a\0✓".into()
		})
	);
	assert_eq!(
		decode_event(2, br#"{"value":"done"}"#),
		Some(NativeEvent::Submit {
			value: "done".into()
		})
	);
	assert_eq!(decode_event(3, b""), Some(NativeEvent::Focus));
	assert_eq!(decode_event(4, b""), Some(NativeEvent::Blur));
	assert_eq!(
		decode_event(5, br#"{"x":1.5,"y":-2.0}"#),
		Some(NativeEvent::Scroll {
			offset_x: 1.5,
			offset_y: -2.0
		})
	);
	assert_eq!(
		decode_event(6, br#"{"checked":false}"#),
		Some(NativeEvent::Toggle { checked: false })
	);
	assert_eq!(
		decode_event(7, br#"{"value":3.5}"#),
		Some(NativeEvent::ValueChange { value: 3.5 })
	);

	for (kind, payload) in [
		(1, b"{}".as_slice()),
		(6, br#"{"checked":"false"}"#),
		(7, br#"{"value":1e999}"#),
		(9, b""),
	] {
		assert!(decode_event(kind, payload).is_none());
	}
}

#[test]
fn invalid_control_trees_fail_before_host_mutation() {
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		tree.update(
			Element::button("outer").child(Element::button("inner")),
			&mut host,
		);
	}));

	assert!(failed.is_err());
	assert!(host.nodes.is_empty());
	assert_eq!(host.commits, 0);
}

#[test]
fn unsupported_custom_widgets_are_rejected_instead_of_becoming_containers() {
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(Element::text("previous screen"), &mut host);
	let root = tree.root;
	let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		tree.update(Element::new(NativeKind::Custom("special")), &mut host);
	}));

	assert!(failed.is_err());
	assert_eq!(tree.root, root);
	assert_eq!(host.nodes.len(), 1);
	assert_eq!(host.nodes[&root.unwrap()].props["text"], "previous screen");
	assert_eq!(host.commits, 1);
}
