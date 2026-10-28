//! Host allocations remain bounded across separately refueled guest calls.

use leptoskit_wasm::EventKind;
use leptoskit_wasm::MAX_RENDER_ALLOCATED_NODES;
use leptoskit_wasm::WasmHost;

fn guest(main: &str, event: &str, extra: &str) -> String {
	format!(
		r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
 (import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
 (import "leptoskit" "node_delete" (func $delete (param i64)))
 (import "leptoskit" "commit" (func $commit))
 (memory (export "memory") 2)
 (global $id (mut i64) (i64.const 1))
 (func (export "leptoskit_main") {main})
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
   {event} i32.const 0)
 (func (export "leptoskit_plugins_ready") {event})
 {extra})"#
	)
}

#[test]
fn node_growth_is_bounded_across_refueled_events_and_plugin_wakes() {
	let source = guest(
		"",
		"(call $create (global.get $id) (i32.const 0)) (global.set $id (i64.add (global.get $id) \
		 (i64.const 1)))",
		"",
	);
	let mut host = WasmHost::headless(source.as_bytes()).unwrap();
	host.start(320.0, 480.0).unwrap();

	for _ in 0..2048 {
		host.dispatch_wire(1, EventKind::Press, None).unwrap();
		host.wake_plugins().unwrap();
	}

	let error = host.wake_plugins().unwrap_err().to_string();
	assert!(error.contains("render node limit"), "{error}");
	assert!(host.dispatch_wire(1, EventKind::Press, None).is_err());
}

#[test]
fn oversized_valid_memory_ranges_are_rejected_before_json_parsing() {
	let source = guest(
		"(call $props (i64.const 1) (i32.const 0) (i32.const 65537))",
		"",
		"",
	);
	let mut host = WasmHost::headless(source.as_bytes()).unwrap();
	let error = host.start(320.0, 480.0).unwrap_err().to_string();
	assert!(error.contains("render property limit"), "{error}");
}

#[test]
fn wide_shallow_trees_obey_the_same_limit_as_detached_nodes() {
	let source = guest(
		"(loop $build
 (call $insert (i64.const 0) (global.get $id) (i32.const -1))
 (global.set $id (i64.add (global.get $id) (i64.const 1)))
 (br_if $build (i64.lt_u (global.get $id) (i64.const 4096))))
 (call $commit)",
		"(call $insert (i64.const 0) (i64.const 4096) (i32.const -1))",
		"",
	);
	let mut host = WasmHost::headless(source.as_bytes()).unwrap();
	host.start(320.0, 480.0).unwrap();
	let root = host.surface().root().unwrap();
	assert_eq!(host.surface().children(root).len(), 4095);
	let error = host
		.dispatch_wire(0, EventKind::Press, None)
		.unwrap_err()
		.to_string();
	assert!(error.contains("render node limit"), "{error}");
	assert_eq!(host.surface().children(root).len(), 4095);
}

fn property_guest(event: &str) -> String {
	let json = format!(r#"{{"text":"{}"}}"#, "x".repeat(65525));
	assert_eq!(json.len(), 65536);
	guest(
		"(loop $build
 (call $props (global.get $id) (i32.const 0) (i32.const 65536))
 (global.set $id (i64.add (global.get $id) (i64.const 1)))
 (br_if $build (i64.le_u (global.get $id) (i64.const 64))))",
		event,
		&format!(
			r#"(data (i32.const 0) "{}") (data (i32.const 65536) "{{}}")"#,
			json.replace('"', "\\22")
		),
	)
}

#[test]
fn retained_property_bytes_do_not_reset_with_fuel() {
	let source = property_guest("(call $props (i64.const 65) (i32.const 0) (i32.const 65536))");
	let mut host = WasmHost::headless(source.as_bytes()).unwrap();
	host.start(320.0, 480.0).unwrap();
	let error = host
		.dispatch_wire(1, EventKind::Press, None)
		.unwrap_err()
		.to_string();
	assert!(error.contains("render retained property limit"), "{error}");
}

#[test]
fn replacing_and_deleting_properties_return_their_budget() {
	let source = property_guest(
		"(call $props (i64.const 1) (i32.const 0) (i32.const 65536))
 (call $props (i64.const 1) (i32.const 65536) (i32.const 2))
 (call $delete (i64.const 2))
 (call $props (i64.const 65) (i32.const 0) (i32.const 65536))",
	);
	let mut host = WasmHost::headless(source.as_bytes()).unwrap();
	host.start(320.0, 480.0).unwrap();
	host.dispatch_wire(1, EventKind::Press, None).unwrap();
}

#[test]
fn repeated_commits_pay_for_host_work_even_with_fuel_remaining() {
	let source = guest(
		"(call $create (i64.const 1) (i32.const 0)) (loop $forever (call $commit) (br $forever))",
		"",
		"",
	);
	let mut host = WasmHost::headless(source.as_bytes()).unwrap();
	let error = host.start(320.0, 480.0).unwrap_err().to_string();
	assert!(error.contains("render work limit"), "{error}");
	assert!(
		host.surface().root().is_none(),
		"a trapped entry must not publish its pending commit"
	);
	assert!(host.dispatch_wire(1, EventKind::Press, None).is_err());
}

#[test]
fn repeated_widget_replacements_release_properties_and_bound_inert_slots() {
	let source = guest(
		"(call $create (i64.const 1) (i32.const 0))
 (call $props (i64.const 1) (i32.const 0) (i32.const 18)) (call $commit)",
		"(call $create (i64.const 1) (i32.wrap_i64 (i64.and (global.get $id) (i64.const 1))))
 (global.set $id (i64.add (global.get $id) (i64.const 1))) (call $commit)",
		r#"(data (i32.const 0) "{\22text\22:\22retired\22}")"#,
	);
	let mut host = WasmHost::headless(source.as_bytes()).unwrap();
	host.start(320.0, 480.0).unwrap();
	let retired = host.surface().root().unwrap();
	host.take_mutations();

	for _ in 1..MAX_RENDER_ALLOCATED_NODES - 1 {
		host.dispatch_wire(1, EventKind::Press, None).unwrap();
		host.take_mutations();
	}

	assert!(host.surface().props(retired).text.is_none());
	assert_eq!(
		host.surface().allocated_nodes(),
		MAX_RENDER_ALLOCATED_NODES - 1
	);
	let previous = host.surface().root();
	let error = host
		.dispatch_wire(1, EventKind::Press, None)
		.unwrap_err()
		.to_string();
	assert!(error.contains("render allocation limit"), "{error}");
	assert_eq!(host.surface().root(), previous);
}

#[test]
fn repeated_accessibility_references_cannot_amplify_a_small_tree_without_limit() {
	let label = serde_json::json!({ "id": "label", "text": "x".repeat(60000) }).to_string();
	let references =
		serde_json::json!({ "aria": { "aria_labelled_by": vec!["label"; 512] } }).to_string();
	let source = guest(
		&format!(
			"(call $props (i64.const 1) (i32.const 0) (i32.const {}))
 (call $props (i64.const 2) (i32.const 65536) (i32.const {}))
 (call $insert (i64.const 1) (i64.const 2) (i32.const 0)) (call $commit)",
			label.len(),
			references.len()
		),
		"",
		&format!(
			r#"(data (i32.const 0) "{}") (data (i32.const 65536) "{}")"#,
			label.replace('"', "\\22"),
			references.replace('"', "\\22")
		),
	);
	let mut host = WasmHost::headless(source.as_bytes()).unwrap();
	let error = host.start(320.0, 480.0).unwrap_err().to_string();
	assert!(error.contains("render commit byte limit"), "{error}");
	assert!(host.surface().root().is_none());
}
