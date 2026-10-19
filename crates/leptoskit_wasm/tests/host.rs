//! Host tests: a wat module implements the leptoskit ABI and drives the
//! headless renderer end to end.

use leptoskit_wasm::EventKind;
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
