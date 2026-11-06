//! Incremental commits: a commit rebuilds only what the guest changed, and
//! widgets keep pairing with the wire ids they were built from.

use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeKind;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;

/// A root view (1) holding a label (2) and a box (3) with a label (4).
/// Each wire event id changes one thing and commits:
///
/// - `10` rewrites label 4;
/// - `11` creates label 5 at the front of box 3;
/// - `12` gives box 3 a text color, which label 4 inherits;
/// - `13` turns label 4 into a button;
/// - `14` gives label 2 an id, a relationship that resolves tree-wide.
const GUEST: &str = r#"
(module
  (import "leptoskit" "node_create" (func $create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
  (data (i32.const 64) "{\"text\":\"a\"}")
  (data (i32.const 100) "{\"text\":\"b\"}")
  (data (i32.const 120) "{\"text\":\"c\"}")
  (data (i32.const 140) "{\"text\":\"d\"}")
  (data (i32.const 160) "{\"class\":\"text-red-500\"}")
  (data (i32.const 200) "{\"text\":\"Go\"}")
  (data (i32.const 220) "{\"text\":\"a\",\"id\":\"x\"}")
  (func (export "leptoskit_main")
    (call $create (i64.const 1) (i32.const 0))
    (call $create (i64.const 2) (i32.const 1))
    (call $create (i64.const 3) (i32.const 0))
    (call $create (i64.const 4) (i32.const 1))
    (call $props (i64.const 2) (i32.const 64) (i32.const 12))
    (call $props (i64.const 4) (i32.const 100) (i32.const 12))
    (call $insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $insert (i64.const 1) (i64.const 3) (i32.const 1))
    (call $insert (i64.const 3) (i64.const 4) (i32.const 0))
    (call $commit))
  (func (export "leptoskit_event")
    (param $id i64) (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    (if (i64.eq (local.get $id) (i64.const 10))
      (then (call $props (i64.const 4) (i32.const 120) (i32.const 12))))
    (if (i64.eq (local.get $id) (i64.const 11))
      (then
        (call $create (i64.const 5) (i32.const 1))
        (call $props (i64.const 5) (i32.const 140) (i32.const 12))
        (call $insert (i64.const 3) (i64.const 5) (i32.const 0))))
    (if (i64.eq (local.get $id) (i64.const 12))
      (then (call $props (i64.const 3) (i32.const 160) (i32.const 24))))
    (if (i64.eq (local.get $id) (i64.const 13))
      (then
        (call $create (i64.const 4) (i32.const 6))
        (call $props (i64.const 4) (i32.const 200) (i32.const 13))))
    (if (i64.eq (local.get $id) (i64.const 14))
      (then (call $props (i64.const 2) (i32.const 220) (i32.const 21))))
    (call $commit)
    (i32.const 1))
)
"#;

fn started() -> WasmHost<HeadlessBackend> {
	let mut host = WasmHost::headless(GUEST.as_bytes()).expect("compile");
	host.start(320.0, 480.0).expect("start");
	host.take_mutations();
	host
}

fn send(host: &mut WasmHost<HeadlessBackend>, wire: i64) -> Vec<Mutation> {
	host.dispatch_wire(wire, EventKind::Press, None)
		.expect("dispatch");
	host.take_mutations()
}

/// The surface node of box 3's children, in order.
fn box_children(host: &WasmHost<HeadlessBackend>) -> Vec<leptoskit_yoga::NodeId> {
	let surface = host.surface();
	let root = surface.root().expect("root");
	surface.children(surface.children(root)[1]).to_vec()
}

fn text_of(host: &WasmHost<HeadlessBackend>, node: leptoskit_yoga::NodeId) -> Option<String> {
	host.surface().props(node).text.clone()
}

#[test]
fn changing_one_label_updates_only_that_label() {
	let mut host = started();
	let label = box_children(&host)[0];
	let mutations = send(&mut host, 10);

	assert_eq!(text_of(&host, label).as_deref(), Some("c"));
	assert!(
		mutations.iter().all(|mutation| {
			match mutation {
				Mutation::UpdateProps { id, .. } => *id == label,
				Mutation::UpdateLayout { .. } => true,
				_ => false,
			}
		}),
		"{mutations:?}"
	);
	assert!(
		mutations
			.iter()
			.any(|mutation| matches!(mutation, Mutation::UpdateProps { .. }))
	);
}

#[test]
fn an_inserted_child_pairs_with_its_wire_id() {
	let mut host = started();
	send(&mut host, 11);
	let children = box_children(&host);

	assert_eq!(children.len(), 2);
	assert_eq!(text_of(&host, children[0]).as_deref(), Some("d"));
	assert_eq!(text_of(&host, children[1]).as_deref(), Some("b"));

	// Both widgets still name the wire nodes the guest built them from, so
	// events reach the right handler.
	let wire = |node| {
		host.wire_node(node)
			.and_then(|wire| wire.props.text.clone())
	};
	assert_eq!(wire(children[0]).as_deref(), Some("d"));
	assert_eq!(wire(children[1]).as_deref(), Some("b"));

	// A later change to the moved label still finds its widget.
	send(&mut host, 10);
	assert_eq!(text_of(&host, box_children(&host)[1]).as_deref(), Some("c"));
}

#[test]
fn an_inherited_change_reaches_the_children() {
	let mut host = started();
	send(&mut host, 12);
	let label = box_children(&host)[0];

	assert!(host.surface().props(label).color.is_some());
}

#[test]
fn a_new_widget_kind_rebuilds_the_node_and_keeps_its_wire_id() {
	let mut host = started();
	let before = box_children(&host)[0];
	send(&mut host, 13);
	let after = box_children(&host)[0];

	assert_ne!(before, after);
	assert_eq!(host.surface().kind(after), NativeKind::Button);
	assert_eq!(
		host.wire_node(after)
			.and_then(|wire| wire.props.text.clone())
			.as_deref(),
		Some("Go")
	);
}

#[test]
fn a_tree_with_relationships_commits_in_full_and_still_renders() {
	let mut host = started();
	send(&mut host, 14);
	assert!(host.surface().resolves_relationships());

	// The next change commits the whole tree, and lands.
	send(&mut host, 10);
	assert_eq!(text_of(&host, box_children(&host)[0]).as_deref(), Some("c"));
}

#[test]
fn incremental_and_complete_commits_build_the_same_interface() {
	// The same final state, reached through patches and through a fresh
	// guest that commits it whole, renders identical props.
	let mut patched = started();
	for wire in [11, 12, 10] {
		send(&mut patched, wire);
	}

	let surface = patched.surface();
	let root = surface.root().expect("root");
	let mut seen = Vec::new();
	let mut stack = vec![root];

	while let Some(node) = stack.pop() {
		seen.push((surface.kind(node), surface.props(node).clone()));
		stack.extend(surface.children(node).iter().rev());
	}

	let complete = r#"
(module
  (import "leptoskit" "node_create" (func $create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
  (data (i32.const 64) "{\"text\":\"a\"}")
  (data (i32.const 120) "{\"text\":\"c\"}")
  (data (i32.const 140) "{\"text\":\"d\"}")
  (data (i32.const 160) "{\"class\":\"text-red-500\"}")
  (func (export "leptoskit_main")
    (call $create (i64.const 1) (i32.const 0))
    (call $create (i64.const 2) (i32.const 1))
    (call $create (i64.const 3) (i32.const 0))
    (call $create (i64.const 4) (i32.const 1))
    (call $create (i64.const 5) (i32.const 1))
    (call $props (i64.const 2) (i32.const 64) (i32.const 12))
    (call $props (i64.const 3) (i32.const 160) (i32.const 24))
    (call $props (i64.const 4) (i32.const 120) (i32.const 12))
    (call $props (i64.const 5) (i32.const 140) (i32.const 12))
    (call $insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $insert (i64.const 1) (i64.const 3) (i32.const 1))
    (call $insert (i64.const 3) (i64.const 5) (i32.const 0))
    (call $insert (i64.const 3) (i64.const 4) (i32.const 1))
    (call $commit))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0))
)
"#;
	let mut whole = WasmHost::headless(complete.as_bytes()).expect("compile");
	whole.start(320.0, 480.0).expect("start");
	let surface = whole.surface();
	let mut expected = Vec::new();
	let mut stack = vec![surface.root().expect("root")];

	while let Some(node) = stack.pop() {
		expected.push((surface.kind(node), surface.props(node).clone()));
		stack.extend(surface.children(node).iter().rev());
	}

	assert_eq!(seen, expected);
}
