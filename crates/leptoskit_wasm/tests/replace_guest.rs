//! Replacing a running guest: the reload `leptoskit dev` performs on every
//! native host.

use leptoskit_native::EventOutcome;
use leptoskit_native::Mutation;
use leptoskit_native::NativeEvent;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

const VIEWPORT_WIDTH: f32 = 320.0;
const VIEWPORT_HEIGHT: f32 = 480.0;

/// A counter whose label reads `<prefix>: <count>`: a view holding the label
/// and an increment button. `prefix` is five bytes, so the count digit sits
/// at offset 80 for every prefix.
fn counter(prefix: &str) -> String {
	assert_eq!(
		prefix.len(),
		5,
		"the digit offset assumes a five-byte prefix"
	);
	r#"
(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $node_insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))
  (global $count (mut i64) (i64.const 0))
  (memory (export "memory") 1)
  (data (i32.const 64) "{\"text\":\"PREFIX: 0\"}")
  (func (export "leptoskit_main")
    (call $node_create (i64.const 1) (i32.const 0))
    (call $node_create (i64.const 2) (i32.const 1))
    (call $node_create (i64.const 3) (i32.const 6))
    (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 19))
    (call $node_insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $node_insert (i64.const 1) (i64.const 3) (i32.const 1))
    (call $commit))
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
        (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 19))
        (call $commit)
        (return (i32.const 1))))
    (i32.const 0))
)
"#
	.replace("PREFIX", prefix)
}

fn started(prefix: &str) -> WasmHost<leptoskit_native::HeadlessBackend> {
	let mut host = WasmHost::headless(counter(prefix).as_bytes()).expect("compile the counter");
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
		.expect("start the counter");
	host
}

fn label(host: &WasmHost<leptoskit_native::HeadlessBackend>) -> String {
	let surface = host.surface();
	let root = surface.root().expect("a committed root");
	surface
		.props(surface.children(root)[0])
		.text
		.clone()
		.expect("the label has text")
}

fn button(host: &WasmHost<leptoskit_native::HeadlessBackend>) -> leptoskit_yoga::NodeId {
	let surface = host.surface();
	surface.children(surface.root().expect("a committed root"))[1]
}

#[test]
fn a_replacement_guest_reuses_the_committed_widgets() {
	let mut host = started("Count");
	let pressed = host
		.dispatch(button(&host), &NativeEvent::Press)
		.expect("press the first guest's button");
	assert_eq!(pressed, EventOutcome::Handled);
	assert_eq!(label(&host), "Count: 1");
	let root = host.surface().root();
	let first = host.identity();
	host.take_mutations();

	host.replace_guest(counter("Total").as_bytes())
		.expect("replace the guest");
	assert_ne!(
		first,
		host.identity(),
		"a new interpreter has a new identity"
	);
	let stale = host
		.dispatch(button(&host), &NativeEvent::Press)
		.expect("press before the replacement commits");
	assert_eq!(
		stale,
		EventOutcome::Unhandled,
		"inherited widgets reach no guest until the replacement commits"
	);

	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
		.expect("start the replacement");
	assert_eq!(host.surface().root(), root, "the root widget stays mounted");
	assert_eq!(label(&host), "Total: 0", "the replacement starts fresh");
	assert_eq!(host.commits(), 1, "commits count the replacement's own");
	let mutations = host.take_mutations();
	assert!(
		mutations.iter().all(|mutation| {
			!matches!(mutation, Mutation::Create { .. } | Mutation::Delete { .. })
		}),
		"an unchanged tree shape keeps every widget: {mutations:?}"
	);

	let pressed = host
		.dispatch(button(&host), &NativeEvent::Press)
		.expect("press the replacement's button");
	assert_eq!(pressed, EventOutcome::Handled);
	assert_eq!(label(&host), "Total: 1");
}

#[test]
fn a_module_that_fails_to_compile_leaves_the_running_guest() {
	let mut host = started("Count");
	let identity = host.identity();

	let error = host
		.replace_guest(b"not a wasm module")
		.expect_err("garbage does not compile");
	assert!(matches!(error, WasmError::Compile(_)), "{error:?}");
	assert_eq!(identity, host.identity());

	let pressed = host
		.dispatch(button(&host), &NativeEvent::Press)
		.expect("press the running guest's button");
	assert_eq!(pressed, EventOutcome::Handled);
	assert_eq!(label(&host), "Count: 1");
}
