//! The wasm-bindgen shims: linked-but-unused browser imports must not stop
//! instantiation, calling one traps loudly, and imports from outside
//! wasm-bindgen still stop it.

use leptoskit_native::NativeKind;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

const VIEWPORT_WIDTH: f32 = 320.0;
const VIEWPORT_HEIGHT: f32 = 480.0;

/// A guest in the shape rustc emits for crates that link wasm-bindgen without
/// calling into a browser: the console-warn helper, its runtime shims, and
/// the externref bookkeeping, around a working tree.
fn shimmed_module(extra_imports: &str, body: &str) -> String {
	format!(
		r#"(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $node_insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))
  (import "__wbindgen_placeholder__" "__wbg_warn_13abc63e0d4b3527" (func $warn (param i32)))
  (import "__wbindgen_placeholder__" "__wbindgen_describe" (func $describe (param i32)))
  (import "__wbindgen_placeholder__" "__wbindgen_describe_generic_import"
    (func $describe_import (param i32 i32 i32)))
  (import "__wbindgen_placeholder__" "__wbindgen_object_drop_ref" (func $drop_ref (param i32)))
  (import "__wbindgen_externref_xform__" "__wbindgen_externref_table_grow"
    (func $table_grow (param i32) (result i32)))
  (import "__wbindgen_externref_xform__" "__wbindgen_externref_table_set_null"
    (func $set_null (param i32)))
  {extra_imports}
  (memory (export "memory") 1)
  ;; `{{"text":"Shimmed"}}` — 18 bytes.
  (data (i32.const 64) "{{\"text\":\"Shimmed\"}}")
  (func (export "leptoskit_main") {body})
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))"#
	)
}

/// Calls every tolerated shim and then mounts a label.
const CALLS_THE_SHIMS: &str = r"
    (call $warn (i32.const 7))
    (call $describe (i32.const 0))
    (call $describe_import (i32.const 0) (i32.const 0) (i32.const 0))
    (drop (call $table_grow (i32.const 1)))
    (call $set_null (i32.const 0))
    (call $drop_ref (i32.const 0))
    (call $node_create (i64.const 1) (i32.const 0))
    (call $node_create (i64.const 2) (i32.const 1))
    (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 18))
    (call $node_insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $commit)";

#[test]
fn linked_but_unused_wasm_bindgen_shims_do_not_stop_instantiation() {
	let mut host = WasmHost::headless(shimmed_module("", CALLS_THE_SHIMS).as_bytes())
		.expect("the shimmed guest links");
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
		.expect("the shimmed guest mounts");
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	assert_eq!(surface.kind(root), NativeKind::View);
	let label = surface
		.children(root)
		.first()
		.expect("the root holds the label");
	assert_eq!(surface.props(*label).text.as_deref(), Some("Shimmed"));
}

#[test]
fn other_browser_imports_link_but_trap_when_called() {
	let import = r#"(import "__wbindgen_placeholder__" "__wbg_performance_now_0d0da1bcca1a1d64"
    (func $now (param i32 i32) (result f64)))"#;

	// Linked but never called, as leptos links js-sys's futures and streams:
	// the guest runs.
	let mut host = WasmHost::headless(shimmed_module(import, CALLS_THE_SHIMS).as_bytes())
		.expect("unused browser glue links");
	host.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
		.expect("the guest mounts");

	// Called, it traps and says which import it was.
	let mut host = WasmHost::headless(
		shimmed_module(import, "(drop (call $now (i32.const 0) (i32.const 0)))").as_bytes(),
	)
	.expect("the browser import links");
	let error = host
		.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
		.expect_err("calling it is browser work");
	assert!(
		matches!(error, WasmError::Trap(ref message) if message.contains("__wbg_performance_now")),
		"{error}"
	);
}

#[test]
fn imports_from_other_modules_still_fail_to_link() {
	let error = WasmHost::headless(
		shimmed_module(
			r#"(import "env" "performance_now" (func $now (result f64)))"#,
			"(drop (call $now))",
		)
		.as_bytes(),
	)
	.err()
	.expect("an import from outside wasm-bindgen must not resolve");
	assert!(
		matches!(error, WasmError::Compile(ref message) if message.contains("performance_now")),
		"{error}"
	);
}

#[test]
fn shims_that_mean_browser_work_trap_loudly() {
	let mut host = WasmHost::headless(
		shimmed_module(
			r#"(import "__wbindgen_placeholder__" "__wbg___wbindgen_throw_41e9ee4f547fc59a"
    (func $throw (param i32 i32)))"#,
			"(call $throw (i32.const 0) (i32.const 0))",
		)
		.as_bytes(),
	)
	.expect("the throw shim links");
	let error = host
		.start(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
		.expect_err("throwing is browser work");
	assert!(
		matches!(error, WasmError::Trap(ref message) if message.contains("wasm-bindgen")),
		"{error}"
	);
}

#[test]
fn non_function_imports_from_the_placeholder_module_are_ignored() {
	// A memory import from the shim module resolves to nothing, so
	// instantiation still fails loudly instead of binding a shim to it.
	let error = WasmHost::headless(
		shimmed_module(
			r#"(import "__wbindgen_placeholder__" "memory" (memory 1))"#,
			"",
		)
		.as_bytes(),
	)
	.err()
	.expect("a memory import must not resolve");
	assert!(
		matches!(error, WasmError::Compile(ref message) if message.contains("memory")),
		"{error}"
	);
}
