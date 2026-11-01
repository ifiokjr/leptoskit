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
use crate::reactive::Get;
use crate::reactive::Set;

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
	/// Every node whose properties were sent, in order.
	updated: Vec<i64>,
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
		self.updated.push(id);
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

#[cfg(feature = "rive")]
#[test]
fn rive_input_updates_reuse_only_the_asset_already_installed_on_the_same_node() {
	use leptoskit_rive::Asset;
	use leptoskit_rive::Input;
	use leptoskit_rive::Name;
	use leptoskit_rive::Playback;
	let mut bytes = vec![0; 1024 * 1024];
	bytes[..8].copy_from_slice(b"RIVE\x07\x00\x00\x00");
	let mut playback = Playback::new(Asset::new(&bytes).unwrap());
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(Element::rive(playback.clone()).aria_hidden(true), &mut host);
	let root = tree.root.unwrap();
	assert_eq!(host.nodes[&root].kind, 16);

	assert!(
		host.nodes[&root].props["rive"]["asset"]
			.as_str()
			.unwrap()
			.len()
			> 1024 * 1024
	);
	playback
		.set_input(Name::new("amount").unwrap(), Input::Number(0.5), 0)
		.unwrap();
	tree.update(Element::rive(playback.clone()).aria_hidden(true), &mut host);
	let update = &host.nodes[&root].props;
	assert!(update.get("rive").is_none());
	assert!(update["rive_inputs"].get("amount").is_some());
	assert!(
		serde_json::to_vec(update).unwrap().len() < 8192,
		"an input update must not re-encode the asset"
	);
	// A different-kind node cannot inherit the previous node's scene.
	tree.update(
		Element::view().child(Element::rive(playback).aria_hidden(true)),
		&mut host,
	);

	let child = host.nodes[&tree.root.unwrap()].children[0];
	assert!(host.nodes[&child].props["rive"]["asset"].is_string());
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
	assert_eq!(host.deleted, [] as [i64; 0]);
	assert_eq!(host.commits, 2);
	assert_eq!(
		host.nodes[&root].props["class"],
		"dark:bg-slate-950 flex-col gap-4 p-6"
	);
}

#[test]
fn refreshing_bindings_sends_only_the_labels_that_changed() {
	use crate::reactive::Get as _;
	use crate::reactive::RwSignal;
	use crate::reactive::Set as _;

	let owner = reactive_graph::owner::Owner::new();
	owner.set();
	let marked = RwSignal::new(false);
	let rows = |marked: RwSignal<bool>| {
		Element::view()
			.child(Element::text("fixed"))
			.child(Element::text_from(move || {
				if marked.get() {
					"Row !".to_owned()
				} else {
					"Row".to_owned()
				}
			}))
	};
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(rows(marked), &mut host);
	let root = tree.root.unwrap();
	let children = host.nodes[&root].children.clone();
	assert_eq!(host.commits, 1);

	host.updated.clear();

	marked.set(true);
	assert!(tree.refresh_reactive(&mut host));
	assert_eq!(host.updated, vec![children[1]]);
	assert_eq!(host.commits, 2);
	assert_eq!(host.nodes[&children[1]].props["text"], "Row !");
	assert_eq!(host.nodes[&children[0]].props["text"], "fixed");

	// A refresh with nothing to send costs no commit, so a frame that changes
	// no bound value does not reach the host at all.
	assert!(!tree.refresh_reactive(&mut host));
	assert_eq!(host.commits, 2);
	assert_eq!(host.created.len(), 3);
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

#[cfg(feature = "devtools")]
#[test]
fn the_wire_names_where_each_node_was_built() {
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	let line = line!() + 1;
	tree.update(Element::view().child(Element::text("hello")), &mut host);
	let root = tree.root.unwrap();
	let source = host.nodes[&root].props["source"].as_str().unwrap();
	assert!(
		source.starts_with(&format!("{}:{line}:", file!())),
		"{source}"
	);
	let text = host.nodes[&root].children[0];

	assert!(host.nodes[&text].props["source"].is_string());
}

#[cfg(feature = "devtools")]
#[test]
fn a_node_built_somewhere_else_is_sent_again() {
	let first = || Element::view().child(Element::text("same"));
	let second = || Element::view().child(Element::text("same"));
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(first(), &mut host);
	host.updated.clear();
	tree.update(first(), &mut host);
	assert!(host.updated.is_empty(), "the same source sends nothing");

	tree.update(second(), &mut host);
	assert_eq!(
		host.updated.len(),
		2,
		"both nodes now come from other lines"
	);
}

#[test]
fn a_rerender_sends_properties_only_for_the_nodes_that_changed() {
	let list = |marked: usize| {
		Element::new(NativeKind::View).children((0..4).map(|row| {
			let suffix = if row == marked { " !" } else { "" };
			Element::text(format!("Row {row}{suffix}"))
		}))
	};
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(list(usize::MAX), &mut host);
	assert_eq!(host.updated.len(), 5, "the first render sends every node");

	host.updated.clear();
	tree.update(list(2), &mut host);
	let changed = host
		.nodes
		.iter()
		.find(|(_, node)| node.props["text"] == "Row 2 !")
		.map(|(id, _)| *id);
	assert_eq!(
		host.updated,
		vec![changed.unwrap()],
		"only the edited row is sent again"
	);

	host.updated.clear();
	tree.update(list(2), &mut host);
	assert!(host.updated.is_empty(), "an identical render sends nothing");
}

/// Where a render's time goes, natively; the interpreter runs the same code
/// many times slower but in the same proportions. Run with
/// `cargo test -p leptoskit_native --features guest --release -- --ignored
/// --nocapture render_phases`.
#[test]
#[ignore = "a timing probe, not a check"]
fn render_phases() {
	use std::time::Instant;
	let cards = |marked: bool| {
		Element::new(NativeKind::View).children((0..1000).map(|number| {
			let title = if marked && number % 10 == 0 {
				format!("Card {number} !")
			} else {
				format!("Card {number}")
			};

			Element::new(NativeKind::View).class("px-4 py-2").children([
				Element::text(title),
				Element::text(format!("Subtitle {number}")).class("text-sm"),
			])
		}))
	};
	let time = |label: &str, run: &mut dyn FnMut()| {
		let start = Instant::now();

		for _ in 0..20 {
			run();
		}

		println!(
			"{label:>28}: {:>8.3} ms",
			start.elapsed().as_secs_f64() * 1000.0 / 20.0
		);
	};
	struct NullHost;
	impl Host for NullHost {
		fn create(&mut self, _: i64, _: i32) {}

		fn props(&mut self, _: i64, _: &[u8]) {}

		fn insert(&mut self, _: i64, _: i64, _: i32) {}

		fn delete(&mut self, _: i64) {}

		fn commit(&mut self) {}
	}
	time("build 1,000 cards", &mut || drop(cards(false)));
	let tree = cards(false);
	time("validate", &mut || tree.validate().unwrap());
	time("create (empty tree)", &mut || {
		Tree::default().update(cards(false), &mut NullHost);
	});
	let mut warm = Tree::default();
	warm.update(cards(false), &mut NullHost);
	time("update every tenth", &mut || {
		warm.update(cards(false), &mut NullHost);
		warm.update(cards(true), &mut NullHost);
	});
	time("identical render", &mut || {
		warm.update(cards(true), &mut NullHost);
	});
}

#[test]
#[ignore = "a size probe, not a check"]
fn wire_size() {
	struct Capture(Vec<usize>, Vec<u8>);
	impl Host for Capture {
		fn create(&mut self, _: i64, _: i32) {}

		fn props(&mut self, _: i64, bytes: &[u8]) {
			self.0.push(bytes.len());
			self.1 = bytes.to_vec();
		}

		fn insert(&mut self, _: i64, _: i64, _: i32) {}

		fn delete(&mut self, _: i64) {}

		fn commit(&mut self) {}
	}
	let mut host = Capture(Vec::new(), Vec::new());
	Tree::default().update(
		Element::new(NativeKind::View)
			.class("px-4 py-2")
			.children([Element::text("Card 1").class("text-sm")]),
		&mut host,
	);
	println!(
		"bytes per node: {:?}\n{}",
		host.0,
		String::from_utf8_lossy(&host.1)
	);
}

/// The host hears about nodes in build order, parents before children, and
/// the same renders always produce the same calls, whatever order the tree's
/// map keeps them in.
#[test]
fn mutations_follow_the_build_order() {
	#[derive(Default)]
	struct Log(Vec<String>);
	impl Host for Log {
		fn create(&mut self, id: i64, kind: i32) {
			self.0.push(format!("create {id} {kind}"));
		}

		fn props(&mut self, id: i64, _: &[u8]) {
			self.0.push(format!("props {id}"));
		}

		fn insert(&mut self, parent: i64, child: i64, index: i32) {
			self.0.push(format!("insert {parent} {child} {index}"));
		}

		fn delete(&mut self, id: i64) {
			self.0.push(format!("delete {id}"));
		}

		fn commit(&mut self) {
			self.0.push("commit".into());
		}
	}
	let screen = |rows: u32| {
		Element::new(NativeKind::View).children((0..rows).map(|row| {
			Element::new(NativeKind::View)
				.child(Element::text(format!("row {row}")))
				.child(Element::text("label"))
		}))
	};
	let run = || {
		let mut tree = Tree::default();
		let mut log = Log::default();
		tree.update(screen(40), &mut log);
		tree.update(screen(25), &mut log);
		tree.update(screen(60), &mut log);
		log.0
	};
	let log = run();

	assert_eq!(log, run());
	let created: Vec<i64> = log
		.iter()
		.take_while(|line| *line != "commit")
		.filter_map(|line| line.strip_prefix("create "))
		.map(|rest| rest.split(' ').next().unwrap().parse().unwrap())
		.collect();
	assert_eq!(created.len(), 1 + 40 * 3);
	assert!(
		created.windows(2).all(|pair| pair[0] < pair[1]),
		"a fresh tree creates parents before children: {created:?}"
	);
	let deleted: Vec<&String> = log
		.iter()
		.filter(|line| line.starts_with("delete "))
		.collect();
	assert_eq!(
		deleted.len(),
		15 * 3,
		"shrinking deletes each removed row and its labels"
	);
}

fn texts(host: &RecordedHost) -> Vec<String> {
	host.nodes
		.values()
		.filter_map(|node| node.props.get("text").and_then(Value::as_str))
		.map(str::to_owned)
		.collect()
}

#[test]
fn a_binding_settles_without_recreating_its_siblings() {
	let owner = reactive_graph::owner::Owner::new();
	owner.set();
	let count = crate::reactive::RwSignal::new(0);
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(
		Element::view().children([
			Element::text_from(move || format!("Count: {}", count.get())),
			Element::text("static"),
		]),
		&mut host,
	);
	let static_id = *host
		.nodes
		.iter()
		.find(|(_, node)| node.props["text"] == "static")
		.map(|(id, _)| id)
		.unwrap();
	host.created.clear();

	host.updated.clear();
	host.deleted.clear();
	count.set(1);
	tree.settle(&mut host);

	assert_eq!(host.created, Vec::<i64>::new());
	assert_eq!(host.deleted, Vec::<i64>::new());
	assert!(!host.updated.contains(&static_id));
	assert!(texts(&host).iter().any(|text| text == "Count: 1"));
}

#[test]
fn a_list_region_replaces_its_rows_and_leaves_the_rest_of_the_screen() {
	let owner = reactive_graph::owner::Owner::new();
	owner.set();
	let items = crate::reactive::RwSignal::new(vec!["a".to_owned(), "b".to_owned()]);
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(
		Element::view().children([
			Element::text("static"),
			crate::reactive::For(
				move || items.get(),
				Clone::clone,
				|item| Element::text(item),
			),
		]),
		&mut host,
	);
	let static_id = *host
		.nodes
		.iter()
		.find(|(_, node)| node.props["text"] == "static")
		.map(|(id, _)| id)
		.unwrap();
	host.created.clear();

	host.updated.clear();
	host.deleted.clear();
	items.set(vec!["a".to_owned(), "c".to_owned()]);
	tree.settle(&mut host);

	assert!(!host.created.contains(&static_id));
	assert!(!host.deleted.contains(&static_id));
	assert!(!host.updated.contains(&static_id));
	let rendered = texts(&host);
	assert!(rendered.iter().any(|text| text == "a"));
	assert!(rendered.iter().any(|text| text == "c"));
	assert!(!rendered.iter().any(|text| text == "b"));
}

#[test]
fn show_drops_its_child_without_touching_the_sibling() {
	let owner = reactive_graph::owner::Owner::new();
	owner.set();
	let open = crate::reactive::RwSignal::new(true);
	let mut tree = Tree::default();
	let mut host = RecordedHost::default();
	tree.update(
		Element::view().children([
			Element::text("static"),
			crate::reactive::Show(move || open.get(), || Element::text("open")),
		]),
		&mut host,
	);
	assert!(texts(&host).iter().any(|text| text == "open"));
	let static_id = *host
		.nodes
		.iter()
		.find(|(_, node)| node.props["text"] == "static")
		.map(|(id, _)| id)
		.unwrap();

	host.created.clear();
	host.deleted.clear();
	host.updated.clear();
	open.set(false);
	tree.settle(&mut host);

	assert!(!host.deleted.contains(&static_id));
	assert!(!host.updated.contains(&static_id));
	assert!(!texts(&host).iter().any(|text| text == "open"));
	assert!(texts(&host).iter().any(|text| text == "static"));
}
