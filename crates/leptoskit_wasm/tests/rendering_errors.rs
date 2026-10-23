//! Rendering cleanup and malformed guest inputs retain observable contracts.

use leptoskit_native::NativeKind;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 480.0;

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
	host.start(WIDTH, HEIGHT)?;
	Ok(host)
}

fn start_err(body: &str) -> String {
	match start(body) {
		Ok(_) => panic!("expected `{body}` to fail"),
		Err(error) => error.to_string(),
	}
}

#[test]
fn removing_a_child_preserves_it_as_a_parentless_node() {
	let error = start_err(
		"(call $node_insert (i64.const 1) (i64.const 2) (i32.const 0)) (call $node_remove \
		 (i64.const 1) (i64.const 2)) (call $commit)",
	);
	assert!(error.contains("2 parentless nodes (1, 2)"), "{error}");
}

#[test]
fn removing_from_a_different_parent_preserves_the_committed_child() {
	let host = start(
		"(call $node_create (i64.const 2) (i32.const 1)) (call $node_insert (i64.const 1) \
		 (i64.const 2) (i32.const 0)) (call $node_remove (i64.const 99) (i64.const 2)) (call \
		 $commit)",
	)
	.unwrap();
	let surface = host.surface();
	let root = surface.root().unwrap();
	assert_eq!(surface.children(root).len(), 1);
	assert_eq!(surface.kind(surface.children(root)[0]), NativeKind::Text);
	assert_eq!(host.commits(), 1);
}

#[test]
fn excess_root_diagnostics_bound_the_list_without_hiding_the_count() {
	let error = start_err(
		"(local $id i64) (local.set $id (i64.const 1)) (loop $create (call $node_create \
		 (local.get $id) (i32.const 0)) (local.set $id (i64.add (local.get $id) (i64.const 1))) \
		 (br_if $create (i64.le_u (local.get $id) (i64.const 9)))) (call $commit)",
	);
	assert!(
		error.contains("9 parentless nodes (1, 2, 3, 4, 5, 6, 7, 8, …)"),
		"{error}"
	);
}

#[test]
fn empty_props_report_invalid_json_without_reading_the_pointer() {
	let error = start_err("(call $node_set_props (i64.const 1) (i32.const -1) (i32.const 0))");
	assert!(error.contains("EOF while parsing a value"), "{error}");
}

#[test]
fn an_abi_version_with_the_wrong_result_type_is_refused() {
	let guest = module(
		"",
		r#"(func (export "leptoskit_abi_version") (result f64) (f64.const 1))"#,
	);
	let error = WasmHost::headless(guest.as_bytes())
		.err()
		.expect("wrong ABI result type");
	assert!(
		error
			.to_string()
			.contains("leptoskit_abi_version must return an i32"),
		"{error}"
	);
}
