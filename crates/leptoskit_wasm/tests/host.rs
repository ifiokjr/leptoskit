//! Host tests: a wat module implements the leptoskit ABI and drives the
//! headless renderer end to end.

use leptoskit_native::NativeKind;
use leptoskit_wasm::ABI_VERSION;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::MAX_TREE_DEPTH;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

const VIEWPORT_WIDTH: f32 = 320.0;
const VIEWPORT_HEIGHT: f32 = 480.0;
const COMPACT_VIEWPORT_HEIGHT: f32 = 240.0;
const WIDE_VIEWPORT_WIDTH: f32 = 900.0;
const WIDE_VIEWPORT_HEIGHT: f32 = 400.0;
const COUNTER_FRAME: (f32, f32, f32, f32) = (0.0, 0.0, VIEWPORT_WIDTH, 32.0);
const COUNTER_LABEL_FRAME: (f32, f32, f32, f32) = (0.0, 0.0, VIEWPORT_WIDTH, 16.0);
const COUNTER_BUTTON_FRAME: (f32, f32, f32, f32) = (0.0, 16.0, VIEWPORT_WIDTH, 16.0);

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
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT).expect("start");

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

	// The root has no style on the wire, so it fills the viewport's width as a
	// `<div>` does and wraps its content's height: the label is one line tall.
	// The default measurer gives the absent button title the same line height
	// as an empty title, and the column stretches both children to its width.
	let frame = surface.frame(root).expect("root frame");
	assert_eq!((frame.x, frame.y, frame.width, frame.height), COUNTER_FRAME);
	let frame = surface.frame(label).expect("label frame");
	assert_eq!(
		(frame.x, frame.y, frame.width, frame.height),
		COUNTER_LABEL_FRAME
	);
	let frame = surface.frame(button).expect("button frame");
	assert_eq!(
		(frame.x, frame.y, frame.width, frame.height),
		COUNTER_BUTTON_FRAME
	);
}

#[test]
fn events_rebuild_the_tree_from_wasm() {
	let mut host = WasmHost::headless(COUNTER_WAT.as_bytes()).expect("compile wat");
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT).expect("start");

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
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT).expect("start");

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
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT).expect("start");
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
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)?;
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
	const NEAR_MEMORY_END: i32 = 65_530;
	const PROPS_OFFSET: i32 = 64;

	for (ptr, len) in [(-1, 10), (NEAR_MEMORY_END, 10), (PROPS_OFFSET, -1)] {
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
	assert_eq!(surface.children(children[0]), []);
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
fn a_tree_at_the_depth_limit_renders() {
	const STACK_BYTES: usize = 16 * 1024 * 1024;
	// The recursive resolve, commit, and teardown passes each hold whole
	// elements on the stack, so the deepest legal tree needs more than a
	// default test thread offers on the tightest platform.
	std::thread::Builder::new()
		.stack_size(STACK_BYTES)
		.spawn(|| {
			let host = start(&format!(
				"(local $i i64) (local.set $i (i64.const 1)) (loop $next (call $node_insert \
				 (local.get $i) (i64.add (local.get $i) (i64.const 1)) (i32.const 0)) (local.set \
				 $i (i64.add (local.get $i) (i64.const 1))) (br_if $next (i64.lt_u (local.get $i) \
				 (i64.const {MAX_TREE_DEPTH})))) (call $commit)"
			))
			.expect("the documented maximum depth renders");
			let surface = host.surface();
			let mut node = surface.root().expect("root");

			for _ in 1..MAX_TREE_DEPTH {
				let children = surface.children(node);
				assert_eq!(children.len(), 1);
				node = children[0];
			}

			assert_eq!(surface.children(node), []);
			assert_eq!(host.commits(), 1);
		})
		.expect("spawn a deep-stack test thread")
		.join()
		.expect("the deep-tree test thread panicked");
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
	const FUEL_BUDGET: u64 = 10_000;
	let mut host =
		WasmHost::headless(module("(loop $spin (br $spin))", "").as_bytes()).expect("compile wat");
	host.set_fuel_budget(FUEL_BUDGET);
	let error = host
		.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
		.expect_err("the loop never ends");
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
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT).expect("start");
	let error = host
		.dispatch_wire(1, EventKind::Change, Some(r#"{"value":"x"}"#))
		.expect_err("no allocator");

	assert!(matches!(error, WasmError::MissingExport(ref name) if name == "leptoskit_alloc"));
}

#[test]
fn joining_chains_past_the_limit_is_refused_at_commit() {
	const FIRST_CHAIN_END: i64 = 200;
	const SECOND_CHAIN_START: i64 = 1001;
	const SECOND_CHAIN_END: i64 = 1200;
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
		chain(1, FIRST_CHAIN_END),
		chain(SECOND_CHAIN_START, SECOND_CHAIN_END)
	));
	assert!(
		error.contains("committed tree is deeper than 256"),
		"{error}"
	);
}

/// A styled container and a pressable child with a distinct guest event id.
fn class_guest(class: &str) -> String {
	props_guest(&serde_json::json!({ "class": class }))
}

fn props_guest(props: &serde_json::Value) -> String {
	use std::fmt::Write;

	let json = props.to_string();
	let mut escaped = String::new();

	for byte in json.bytes() {
		write!(escaped, "\\{byte:02x}").expect("write to String");
	}

	format!(
		r#"(module
  (import "leptoskit" "node_create" (func $create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
  (data (i32.const 0) "{escaped}")
  (func (export "leptoskit_main")
    (call $create (i64.const 1) (i32.const 0))
    (call $create (i64.const 99) (i32.const 6))
    (call $props (i64.const 1) (i32.const 0) (i32.const {length}))
    (call $insert (i64.const 1) (i64.const 99) (i32.const 0))
    (call $commit))
  (func (export "leptoskit_event")
    (param $id i64) (param $kind i32) (param i32 i32) (result i32)
    (i32.and (i64.eq (local.get $id) (i64.const 99))
      (i32.eq (local.get $kind) (i32.const 0)))))"#,
		length = json.len()
	)
}

#[test]
fn wire_classes_drive_native_layout_and_paint() {
	let guest = class_guest("w-20 h-10 p-2 bg-white rounded-lg");
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("commit class tree");
	let surface = host.surface();
	let root = surface.root().expect("root");
	let frame = surface.frame(root).expect("frame");
	assert_eq!((frame.width, frame.height), (80.0, 40.0));
	assert_eq!(
		surface.props(root).background,
		Some(leptoskit_native::Color::rgba(255, 255, 255, 255))
	);

	assert_eq!(surface.props(root).corner_radius, Some(8.0));
}

#[test]
fn a_committed_node_names_the_wire_node_it_was_built_from() {
	let guest = props_guest(&serde_json::json!({
		"class": "p-2 gap-1",
		"source": "src/screens/home.rs:12:5",
	}));
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("start");
	let root = host.surface().root().expect("root");
	let wire = host.wire_node(root).expect("the root's wire node");
	assert_eq!(wire.class, "p-2 gap-1");
	assert_eq!(wire.source.as_deref(), Some("src/screens/home.rs:12:5"));
	let child = host.surface().children(root)[0];

	let wire = host.wire_node(child).expect("the child's wire node");
	assert_eq!((wire.class.as_str(), wire.source.as_deref()), ("", None));
}

/// A guest that sets the root's props twice before committing: `first`
/// as a full render, then `second` as a bound value changing in place.
fn twice_props_guest(first: &serde_json::Value, second: &serde_json::Value) -> String {
	use std::fmt::Write;

	let escape = |value: &serde_json::Value| {
		let mut escaped = String::new();

		for byte in value.to_string().bytes() {
			write!(escaped, "\\{byte:02x}").expect("write to String");
		}

		escaped
	};
	let (first_len, second_len) = (first.to_string().len(), second.to_string().len());
	format!(
		r#"(module
  (import "leptoskit" "node_create" (func $create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
  (data (i32.const 0) "{first}")
  (data (i32.const 4096) "{second}")
  (func (export "leptoskit_main")
    (call $create (i64.const 1) (i32.const 0))
    (call $props (i64.const 1) (i32.const 0) (i32.const {first_len}))
    (call $props (i64.const 1) (i32.const 4096) (i32.const {second_len}))
    (call $commit))
  (func (export "leptoskit_event")
    (param i64 i32 i32 i32) (result i32)
    (i32.const 0)))"#,
		first = escape(first),
		second = escape(second),
	)
}

#[test]
fn a_bound_value_changing_in_place_keeps_the_source_and_component() {
	let guest = twice_props_guest(
		&serde_json::json!({
			"class": "p-2",
			"source": "src/screens/home.rs:12:5",
			"component": "ClipCard",
		}),
		&serde_json::json!({ "class": "p-2", "props": { "text": "updated" } }),
	);
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("start");
	let root = host.surface().root().expect("root");
	let wire = host.wire_node(root).expect("the root's wire node");
	assert_eq!(wire.source.as_deref(), Some("src/screens/home.rs:12:5"));
	assert_eq!(wire.component.as_deref(), Some("ClipCard"));

	// A full render names its own source and component, which replace the
	// old ones, even when it has no component.
	let guest = twice_props_guest(
		&serde_json::json!({ "source": "a.rs:1:1", "component": "Old" }),
		&serde_json::json!({ "source": "b.rs:2:2" }),
	);
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("start");
	let root = host.surface().root().expect("root");
	let wire = host.wire_node(root).expect("the root's wire node");
	assert_eq!(
		(wire.source.as_deref(), wire.component.as_deref()),
		(Some("b.rs:2:2"), None)
	);
}

#[test]
fn invalid_wire_classes_trap_before_committing_native_widgets() {
	let guest = class_guest("md:unsupported-utility");
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	let error = host
		.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect_err("invalid classes trap");
	assert!(error.to_string().contains("unsupported-utility"));
	assert!(host.surface().root().is_none());
	assert_eq!(host.surface().backend().log(), []);
}

#[test]
fn environment_changes_reapply_wire_classes_without_a_guest_commit() {
	let guest = class_guest("w-[50vw] h-10 bg-white dark:bg-black md:h-20");
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	host.set_dark_mode(true);
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("start");
	let root = host.surface().root().expect("root");
	let frame = host.surface().frame(root).expect("frame");
	assert_eq!((frame.width, frame.height), (160.0, 40.0));
	assert_eq!(
		host.surface().props(root).background,
		Some(leptoskit_native::Color::rgba(0, 0, 0, 255))
	);

	host.set_viewport(WIDE_VIEWPORT_WIDTH, WIDE_VIEWPORT_HEIGHT);
	host.set_dark_mode(false);
	assert_eq!(host.surface().root(), Some(root));
	let frame = host.surface().frame(root).expect("resized frame");
	assert_eq!((frame.width, frame.height), (450.0, 80.0));
	assert_eq!(
		host.surface().props(root).background,
		Some(leptoskit_native::Color::rgba(255, 255, 255, 255))
	);
	assert_eq!(host.commits(), 1);
}

#[test]
fn responsive_widget_replacements_preserve_wire_event_targets() {
	let guest = class_guest("w-20 h-20 md:overflow-scroll");
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("start");
	let mut previous = host.surface().root().expect("root");
	let mut previous_button = host.surface().children(previous)[0];

	for (width, kind) in [
		(WIDE_VIEWPORT_WIDTH, NativeKind::ScrollView),
		(VIEWPORT_WIDTH, NativeKind::View),
		(WIDE_VIEWPORT_WIDTH, NativeKind::ScrollView),
	] {
		host.set_viewport(width, COMPACT_VIEWPORT_HEIGHT);
		let root = host.surface().root().expect("replacement root");
		assert_ne!(root, previous);
		assert_eq!(host.surface().kind(root), kind);
		let button = host.surface().children(root)[0];
		assert_ne!(button, previous_button);
		assert_eq!(
			host.dispatch(button, &leptoskit_native::NativeEvent::Press)
				.expect("dispatch to replacement"),
			leptoskit_native::EventOutcome::Handled
		);
		assert_eq!(
			host.dispatch(previous_button, &leptoskit_native::NativeEvent::Press)
				.expect("dispatch to old node"),
			leptoskit_native::EventOutcome::Unhandled
		);
		previous = root;
		previous_button = button;
	}

	assert_eq!(host.commits(), 1);
}

#[test]
fn dark_widget_replacements_preserve_wire_event_targets() {
	let guest = class_guest("w-20 h-20 dark:overflow-scroll");
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("start");
	let initial = host.surface().root().expect("root");
	host.set_dark_mode(true);
	let dark = host.surface().root().expect("dark root");
	assert_ne!(dark, initial);
	assert_eq!(host.surface().kind(dark), NativeKind::ScrollView);

	let dark_button = host.surface().children(dark)[0];
	assert_eq!(
		host.dispatch(dark_button, &leptoskit_native::NativeEvent::Press)
			.expect("dispatch in dark mode"),
		leptoskit_native::EventOutcome::Handled
	);

	host.set_dark_mode(false);
	let light = host.surface().root().expect("light root");
	assert_ne!(light, dark);
	assert_eq!(host.surface().kind(light), NativeKind::View);
	let light_button = host.surface().children(light)[0];
	assert_eq!(
		host.dispatch(light_button, &leptoskit_native::NativeEvent::Press)
			.expect("dispatch in light mode"),
		leptoskit_native::EventOutcome::Handled
	);
	assert_eq!(host.commits(), 1);
}

#[test]
fn hidden_and_disabled_ancestors_block_native_target_dispatch() {
	let guest = class_guest("w-20 h-20 md:hidden");
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile guest");
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("start");
	let root = host.surface().root().expect("root");
	let button = host.surface().children(root)[0];
	assert_eq!(
		host.dispatch(button, &leptoskit_native::NativeEvent::Press)
			.expect("visible button dispatch"),
		leptoskit_native::EventOutcome::Handled
	);

	host.set_viewport(WIDE_VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT);
	assert_eq!(
		host.dispatch(button, &leptoskit_native::NativeEvent::Press)
			.expect("hidden ancestor blocks dispatch"),
		leptoskit_native::EventOutcome::Unhandled
	);
	host.set_viewport(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT);
	assert_eq!(
		host.dispatch(button, &leptoskit_native::NativeEvent::Press)
			.expect("restored button dispatch"),
		leptoskit_native::EventOutcome::Handled
	);

	let guest = props_guest(&serde_json::json!({ "enabled": false }));
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile disabled guest");
	host.start(VIEWPORT_WIDTH, COMPACT_VIEWPORT_HEIGHT)
		.expect("start disabled guest");
	let root = host.surface().root().expect("disabled root");
	let button = host.surface().children(root)[0];
	assert_eq!(
		host.dispatch(button, &leptoskit_native::NativeEvent::Press)
			.expect("disabled ancestor blocks dispatch"),
		leptoskit_native::EventOutcome::Unhandled
	);
}

const HOST_IMPORTS: &str = r#"
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $node_insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))"#;

/// A guest that shows `narrow` or `wide` for the width the host reports,
/// once it has mounted, and traps on a report while `$fragile` is set.
fn viewport_guest() -> String {
	format!(
		r#"(module {HOST_IMPORTS}
  (memory (export "memory") 1)
  (data (i32.const 64) "{{\"text\":\"narrow\"}}")
  (data (i32.const 96) "{{\"text\":\"wide\"}}")
  (global $mounted (mut i32) (i32.const 0))
  (global $wide (mut i32) (i32.const 0))
  (func $show
    (if (global.get $wide)
      (then (call $node_set_props (i64.const 2) (i32.const 96) (i32.const 15)))
      (else (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 17))))
    (call $commit))
  (func (export "leptoskit_main")
    (global.set $mounted (i32.const 1))
    (call $node_create (i64.const 1) (i32.const 0))
    (call $node_create (i64.const 2) (i32.const 1))
    (call $node_insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $show))
  (func (export "leptoskit_viewport") (param $width f32) (param $height f32)
    (if (f32.gt (local.get $height) (f32.const 1000)) (then unreachable))
    (global.set $wide (f32.gt (local.get $width) (f32.const 500)))
    (if (global.get $mounted) (then (call $show))))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))"#
	)
}

fn label(host: &WasmHost<leptoskit_native::HeadlessBackend>) -> Option<String> {
	let surface = host.surface();
	let root = surface.root()?;
	surface.props(surface.children(root)[0]).text.clone()
}

#[test]
fn the_viewport_reaches_a_guest_that_asks_for_it() {
	let mut host = WasmHost::headless(viewport_guest().as_bytes()).unwrap();
	// Reported before `leptoskit_main`, so the first frame already knows it.
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT).unwrap();
	assert_eq!(label(&host).as_deref(), Some("narrow"));

	host.set_viewport(WIDE_VIEWPORT_WIDTH, WIDE_VIEWPORT_HEIGHT);
	assert_eq!(label(&host).as_deref(), Some("wide"));
}

#[test]
fn a_viewport_trap_surfaces_at_the_next_dispatch() {
	let mut host = WasmHost::headless(viewport_guest().as_bytes()).unwrap();
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT).unwrap();

	host.set_viewport(VIEWPORT_WIDTH, 2000.0);
	assert_eq!(
		label(&host).as_deref(),
		Some("narrow"),
		"the interface stays"
	);
	let root = host.surface().root().unwrap();
	let error = host
		.dispatch(root, &leptoskit_native::NativeEvent::Press)
		.unwrap_err();
	assert!(matches!(error, WasmError::Trap(_)), "{error}");
	assert_eq!(
		host.dispatch(root, &leptoskit_native::NativeEvent::Press)
			.unwrap(),
		leptoskit_native::EventOutcome::Unhandled,
		"the error is reported once"
	);
}
