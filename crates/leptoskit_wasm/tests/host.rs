//! Host tests: a wat module implements the leptoskit ABI and drives the
//! headless renderer end to end.

use leptoskit_native::NativeKind;
use leptoskit_wasm::ABI_VERSION;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

/// A counter app implementing the leptoskit ABI: a view holding a label and
/// an increment button; pressing the button rewrites the label and commits.
const COUNTER_WAT: &str = r#"
(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $node_insert (param i64 i64 i32)))
  (import "leptoskit" "node_remove" (func $node_remove (param i64 i64)))
  (import "leptoskit" "node_delete" (func $node_delete (param i64)))
  (import "leptoskit" "commit" (func $commit))
  (global $count (mut i64) (i64.const 0))
  (memory (export "memory") 1)
  ;; `{"text":"Count: 0"}` — 19 bytes; the digit sits at offset 80.
  (data (i32.const 64) "{\"text\":\"Count: 0\"}")
  (func $build
    (call $node_create (i64.const 1) (i32.const 0))
    (call $node_create (i64.const 2) (i32.const 1))
    (call $node_create (i64.const 3) (i32.const 6))
    (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 19))
    (call $node_insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $node_insert (i64.const 1) (i64.const 3) (i32.const 1))
    (call $commit))
  (func $set_count
    (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 19)))
  (func (export "leptoskit_main") (call $build))
  (func (export "leptoskit_event")
    (param $id i64) (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    (if (i32.and
          (i64.eq (local.get $id) (i64.const 3))
          (i32.eq (local.get $kind) (i32.const 0)))
      (then
        (global.set $count (i64.add (global.get $count) (i64.const 1)))
        (i32.store8
          (i32.const 80)
          (i32.wrap_i64 (i64.add (i64.const 48) (global.get $count))))
        (call $set_count)
        (call $commit)
        (return (i32.const 1))))
    (i32.const 0))
)
"#;

#[test]
fn wasm_bundle_builds_and_lays_out_the_interface() {
	let mut host = WasmHost::headless(COUNTER_WAT.as_bytes()).expect("compile wat");
	host.start(320.0, 480.0).expect("start");

	assert_eq!(host.commits(), 1);
	let surface = host.surface();
	let root = surface.root().expect("root committed");
	assert_eq!(surface.kind(root).to_string(), "view");
	assert_eq!(surface.children(root).len(), 2);
	let (label, button) = (surface.children(root)[0], surface.children(root)[1]);
	assert_eq!(surface.kind(label).to_string(), "text");
	assert_eq!(surface.kind(button).to_string(), "button");
	assert_eq!(surface.props(label).text.as_deref(), Some("Count: 0"));
	assert_eq!(surface.props(button).text, None, "button label needs props");

	// The root has no style on the wire, so it wraps its content: the label
	// measures 8 characters wide ("Count: 0") and one line tall, and the
	// button, also textless in props, collapses.
	let frame = surface.frame(root).expect("root frame");
	assert_eq!((frame.width, frame.height), (64.0, 16.0));
}

#[test]
fn events_rebuild_the_tree_from_wasm() {
	let mut host = WasmHost::headless(COUNTER_WAT.as_bytes()).expect("compile wat");
	host.start(320.0, 480.0).expect("start");

	let surface = host.surface();
	let root = surface.root().expect("root");
	let button = surface.children(root)[1];
	let outcome = host
		.dispatch(button, &leptoskit_native::NativeEvent::Press)
		.expect("dispatch");
	assert_eq!(outcome, leptoskit_native::EventOutcome::Handled);
	assert_eq!(host.commits(), 2, "the module committed a rebuild");

	let surface = host.surface();
	let label = surface.children(root)[0];
	assert_eq!(surface.props(label).text.as_deref(), Some("Count: 1"));
}

#[test]
fn unhandled_events_return_zero_and_do_not_commit() {
	let mut host = WasmHost::headless(COUNTER_WAT.as_bytes()).expect("compile wat");
	host.start(320.0, 480.0).expect("start");

	let surface = host.surface();
	let root = surface.root().expect("root");
	let label = surface.children(root)[0];
	let outcome = host
		.dispatch(label, &leptoskit_native::NativeEvent::Focus)
		.expect("dispatch");
	assert_eq!(outcome, leptoskit_native::EventOutcome::Unhandled);
	assert_eq!(host.commits(), 1);
}

#[test]
fn wire_dispatch_reaches_the_module() {
	let mut host = WasmHost::headless(COUNTER_WAT.as_bytes()).expect("compile wat");
	host.start(320.0, 480.0).expect("start");
	let outcome = host
		.dispatch_wire(3, EventKind::Press, None)
		.expect("dispatch");
	assert_eq!(outcome, leptoskit_native::EventOutcome::Handled);

	let surface = host.surface();
	let root = surface.root().expect("root");
	let label = surface.children(root)[0];
	assert_eq!(surface.props(label).text.as_deref(), Some("Count: 1"));
}

#[test]
fn missing_abi_exports_fail_clearly() {
	let broken = r#"(module (memory (export "memory") 1))"#;
	let error = WasmHost::headless(broken.as_bytes())
		.err()
		.expect("missing exports must fail");
	assert!(error.to_string().contains("leptoskit_main"), "got {error}");
}

/// A module whose `leptoskit_main` runs `body`, with `{"text":"Count: 0"}`
/// (19 bytes) at offset 64 and `extra` spliced in at module level.
fn module(body: &str, extra: &str) -> String {
	format!(
		r#"(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $node_insert (param i64 i64 i32)))
  (import "leptoskit" "node_remove" (func $node_remove (param i64 i64)))
  (import "leptoskit" "node_delete" (func $node_delete (param i64)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
  (data (i32.const 64) "{{\"text\":\"Count: 0\"}}")
  (func (export "leptoskit_main") {body})
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0))
  {extra})"#
	)
}

fn start(body: &str) -> Result<WasmHost<leptoskit_native::HeadlessBackend>, WasmError> {
	let mut host = WasmHost::headless(module(body, "").as_bytes())?;
	host.start(320.0, 480.0)?;
	Ok(host)
}

fn start_err(body: &str) -> String {
	match start(body) {
		Ok(_) => panic!("expected `{body}` to fail"),
		Err(error) => error.to_string(),
	}
}

#[test]
fn insert_index_past_the_end_appends() {
	let host = start(
		"(call $node_create (i64.const 1) (i32.const 0)) (call $node_create (i64.const 2) \
		 (i32.const 1)) (call $node_create (i64.const 3) (i32.const 6)) (call $node_insert \
		 (i64.const 1) (i64.const 2) (i32.const 5)) (call $node_insert (i64.const 1) (i64.const \
		 3) (i32.const -1)) (call $commit)",
	)
	.expect("start");
	let surface = host.surface();
	let root = surface.root().expect("root");
	let kinds: Vec<NativeKind> = surface
		.children(root)
		.iter()
		.map(|&id| surface.kind(id))
		.collect();
	assert_eq!(kinds, [NativeKind::Text, NativeKind::Button]);
}

#[test]
fn out_of_bounds_props_pointers_trap() {
	for (ptr, len) in [(-1, 10), (65_530, 10), (64, -1)] {
		let error = start_err(&format!(
			"(call $node_set_props (i64.const 1) (i32.const {ptr}) (i32.const {len}))"
		));
		assert!(
			error.contains("out of bounds"),
			"ptr {ptr} len {len}: {error}"
		);
	}
}

#[test]
fn inserting_a_node_under_its_descendant_traps() {
	let error = start_err(
		"(call $node_insert (i64.const 1) (i64.const 2) (i32.const 0)) (call $node_insert \
		 (i64.const 2) (i64.const 1) (i32.const 0)) (call $commit)",
	);
	assert!(error.contains("own ancestor"), "{error}");
	let error = start_err("(call $node_insert (i64.const 1) (i64.const 1) (i32.const 0))");
	assert!(error.contains("own ancestor"), "{error}");
}

#[test]
fn inserting_an_attached_node_moves_it() {
	// 4 goes under 2, then under 3: it must end up under 3 only.
	let host = start(
		"(call $node_insert (i64.const 1) (i64.const 2) (i32.const 0)) (call $node_insert \
		 (i64.const 1) (i64.const 3) (i32.const 1)) (call $node_insert (i64.const 2) (i64.const \
		 4) (i32.const 0)) (call $node_insert (i64.const 3) (i64.const 4) (i32.const 0)) (call \
		 $commit)",
	)
	.expect("start");
	let surface = host.surface();
	let root = surface.root().expect("root");
	let children = surface.children(root);
	assert_eq!(children.len(), 2);
	assert!(surface.children(children[0]).is_empty());
	assert_eq!(surface.children(children[1]).len(), 1);
}

#[test]
fn trees_deeper_than_the_limit_are_refused() {
	// A chain of 400 nodes: 1 -> 2 -> ... -> 400.
	let error = start_err(
		"(local $i i64) (local.set $i (i64.const 1)) (loop $next (call $node_insert (local.get \
		 $i) (i64.add (local.get $i) (i64.const 1)) (i32.const 0)) (local.set $i (i64.add \
		 (local.get $i) (i64.const 1))) (br_if $next (i64.lt_u (local.get $i) (i64.const 400)))) \
		 (call $commit)",
	);
	assert!(error.contains("deeper than 256"), "{error}");
}

#[test]
fn a_single_node_interface_renders() {
	let host = start(
		"(call $node_create (i64.const 1) (i32.const 1)) (call $node_set_props (i64.const 1) \
		 (i32.const 64) (i32.const 19)) (call $commit)",
	)
	.expect("start");
	let surface = host.surface();
	let root = surface.root().expect("root committed");
	assert_eq!(surface.kind(root), NativeKind::Text);
	assert_eq!(surface.props(root).text.as_deref(), Some("Count: 0"));
}

#[test]
fn a_replacement_tree_becomes_the_root() {
	let host = start(
		"(call $node_insert (i64.const 1) (i64.const 2) (i32.const 0)) (call $commit) (call \
		 $node_delete (i64.const 1)) (call $node_delete (i64.const 2)) (call $node_create \
		 (i64.const 3) (i32.const 6)) (call $node_create (i64.const 4) (i32.const 1)) (call \
		 $node_set_props (i64.const 4) (i32.const 64) (i32.const 19)) (call $node_insert \
		 (i64.const 3) (i64.const 4) (i32.const 0)) (call $commit)",
	)
	.expect("start");
	let surface = host.surface();
	let root = surface.root().expect("root committed");
	assert_eq!(surface.kind(root), NativeKind::Button);
	let label = surface.children(root)[0];
	assert_eq!(surface.props(label).text.as_deref(), Some("Count: 0"));
}

#[test]
fn committing_several_parentless_nodes_traps() {
	let error = start_err(
		"(call $node_create (i64.const 1) (i32.const 0)) (call $node_create (i64.const 2) \
		 (i32.const 0)) (call $commit)",
	);
	assert!(error.contains("2 parentless nodes (1, 2)"), "{error}");
}

#[test]
fn deleting_a_parent_orphans_its_children() {
	// Deleting 1 leaves 2 as the only parentless node, so it becomes the root.
	let host = start(
		"(call $node_insert (i64.const 1) (i64.const 2) (i32.const 0)) (call $node_delete \
		 (i64.const 1)) (call $node_create (i64.const 2) (i32.const 1)) (call $commit)",
	)
	.expect("start");
	let surface = host.surface();
	assert_eq!(
		surface.kind(surface.root().expect("root")),
		NativeKind::Text
	);
}

#[test]
fn running_out_of_fuel_is_reported() {
	let mut host =
		WasmHost::headless(module("(loop $spin (br $spin))", "").as_bytes()).expect("compile wat");
	host.set_fuel_budget(10_000);
	let error = host.start(320.0, 480.0).expect_err("the loop never ends");
	assert!(matches!(error, WasmError::OutOfFuel), "{error}");
}

#[test]
fn a_mismatched_abi_version_is_refused() {
	let version = |found: i32| {
		module(
			"",
			&format!(r#"(func (export "leptoskit_abi_version") (result i32) (i32.const {found}))"#),
		)
	};
	let error = WasmHost::headless(version(2).as_bytes())
		.err()
		.expect("version 2 must be refused");
	assert!(
		matches!(
			error,
			WasmError::AbiMismatch {
				expected: ABI_VERSION,
				found: 2
			}
		),
		"{error}"
	);
	WasmHost::headless(version(ABI_VERSION as i32).as_bytes()).expect("the current version loads");
}

#[test]
fn payload_events_need_the_allocator() {
	let mut host = WasmHost::headless(module("", "").as_bytes()).expect("compile wat");
	host.start(320.0, 480.0).expect("start");
	let error = host
		.dispatch_wire(1, EventKind::Change, Some(r#"{"value":"x"}"#))
		.expect_err("no allocator");
	assert!(matches!(error, WasmError::MissingExport(ref name) if name == "leptoskit_alloc"));
}

#[test]
fn joining_chains_past_the_limit_is_refused_at_commit() {
	// Two 200-node chains, 1..=200 and 1001..=1200, each fine alone; hanging
	// the second under the tail of the first makes the committed tree 400 deep.
	let chain = |from: i64, to: i64| {
		format!(
			"(local.set $i (i64.const {from})) (loop $next{from} (call $node_insert (local.get \
			 $i) (i64.add (local.get $i) (i64.const 1)) (i32.const 0)) (local.set $i (i64.add \
			 (local.get $i) (i64.const 1))) (br_if $next{from} (i64.lt_u (local.get $i) \
			 (i64.const {to}))))"
		)
	};
	let error = start_err(&format!(
		"(local $i i64) {} {} (call $node_insert (i64.const 200) (i64.const 1001) (i32.const 0)) \
		 (call $commit)",
		chain(1, 200),
		chain(1001, 1200)
	));
	assert!(
		error.contains("committed tree is deeper than 256"),
		"{error}"
	);
}
