//! Guest declarations and growth must stay within the native runtime budget.

use leptoskit_wasm::EventKind;
use leptoskit_wasm::MAX_MEMORY_BYTES;
use leptoskit_wasm::MAX_TABLE_ELEMENTS;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

const PAGE_BYTES: usize = 65_536;

#[test]
fn guest_declared_growth_failures_still_return_minus_one() {
	for (resources, grow) in [
		("(memory 1 1)", "(memory.grow (i32.const 1))"),
		(
			"(table 1 1 funcref)",
			"(table.grow (ref.null func) (i32.const 1))",
		),
	] {
		let main = format!("(if (i32.ne {grow} (i32.const -1)) (then unreachable))");
		let wasm = guest(resources, &main, "", "");
		let mut host = WasmHost::headless(wasm.as_bytes()).expect("small guest loads");

		host.start(320.0, 480.0)
			.expect("guest handles its own declared growth failure");
	}
}

#[test]
fn growth_fuel_exhaustion_remains_distinct_from_resource_limits() {
	for (resources, grow) in [
		("(memory 0)", "(drop (memory.grow (i32.const 1)))"),
		(
			"(table 0 funcref)",
			"(drop (table.grow (ref.null func) (i32.const 65536)))",
		),
	] {
		let wasm = guest(resources, grow, "", "");
		let mut host = WasmHost::headless(wasm.as_bytes()).expect("small guest loads");
		host.set_fuel_budget(100);

		assert!(matches!(
			host.start(320.0, 480.0),
			Err(WasmError::OutOfFuel)
		));
	}
}

/// A minimal ABI guest whose event callback checks resource sizes after growth.
fn guest(resources: &str, main: &str, event: &str, extra: &str) -> String {
	format!(
		r#"(module
			{resources}
			(func (export "leptoskit_main") {main})
			(func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
				{event} i32.const 0)
			{extra}
		)"#
	)
}

#[test]
fn oversized_initial_table_is_rejected() {
	for element_type in ["funcref", "externref"] {
		let wasm = guest(
			&format!("(table {} {element_type})", MAX_TABLE_ELEMENTS + 1),
			"",
			"",
			"",
		);

		assert!(matches!(
			WasmHost::headless(wasm.as_bytes()),
			Err(WasmError::ResourceLimitExceeded)
		));
	}
}

#[test]
fn oversized_initial_memory_is_rejected_even_when_not_exported() {
	let pages = MAX_MEMORY_BYTES / PAGE_BYTES + 1;
	let wasm = guest(&format!("(memory {pages})"), "", "", "");

	assert!(matches!(
		WasmHost::headless(wasm.as_bytes()),
		Err(WasmError::ResourceLimitExceeded)
	));
}

#[test]
fn resource_counts_cannot_multiply_the_per_resource_budget() {
	for resources in [
		"(memory 0) (memory 0)",
		"(table 0 funcref) (table 0 funcref) (table 0 externref)",
	] {
		let wasm = guest(resources, "", "", "");

		assert!(matches!(
			WasmHost::headless(wasm.as_bytes()),
			Err(WasmError::ResourceLimitExceeded)
		));
	}
}

#[test]
fn memory_growth_reaches_the_ceiling_then_repeatedly_traps_without_growing() {
	let pages = MAX_MEMORY_BYTES / PAGE_BYTES;
	let wasm = guest(
		&format!("(memory {})", pages - 1),
		"(drop (memory.grow (i32.const 1)))",
		&format!("(if (i32.ne (memory.size) (i32.const {pages})) (then unreachable))"),
		"",
	);
	let mut host = WasmHost::headless(wasm.as_bytes()).expect("bounded memory loads");
	host.start(320.0, 480.0)
		.expect("growth to ceiling succeeds");

	for _ in 0..3 {
		assert!(matches!(
			host.start(320.0, 480.0),
			Err(WasmError::ResourceLimitExceeded)
		));
		host.dispatch_wire(1, EventKind::Press, None)
			.expect("failed growth leaves memory at the ceiling");
	}
}

#[test]
fn both_tables_can_grow_to_the_ceiling_but_not_beyond_it() {
	let elements = MAX_TABLE_ELEMENTS;
	let wasm = guest(
		&format!(
			"(table $functions {} funcref) (table $references {} externref)",
			elements - 1,
			elements - 1
		),
		"(drop (table.grow $functions (ref.null func) (i32.const 1)))",
		&format!(
			"(if (i32.ne (table.size $functions) (i32.const {elements})) (then unreachable))
			(drop (table.grow $references (ref.null extern) (i32.const 1)))"
		),
		"",
	);
	let mut host = WasmHost::headless(wasm.as_bytes()).expect("bounded tables load");
	host.start(320.0, 480.0).expect("function table grows");
	host.dispatch_wire(1, EventKind::Press, None)
		.expect("externref table grows");

	for _ in 0..3 {
		assert!(matches!(
			host.start(320.0, 480.0),
			Err(WasmError::ResourceLimitExceeded)
		));
		assert!(matches!(
			host.dispatch_wire(1, EventKind::Press, None),
			Err(WasmError::ResourceLimitExceeded)
		));
	}
}

#[test]
fn start_function_cannot_bypass_growth_limits() {
	for (resources, grow) in [
		("(memory 0)", "(drop (memory.grow (i32.const 1025)))"),
		(
			"(table 0 funcref)",
			"(drop (table.grow (ref.null func) (i32.const 65537)))",
		),
	] {
		let wasm = guest(
			resources,
			"",
			"",
			&format!("(func $init {grow}) (start $init)"),
		);

		assert!(matches!(
			WasmHost::headless(wasm.as_bytes()),
			Err(WasmError::ResourceLimitExceeded)
		));
	}
}

#[test]
fn allocator_and_plugin_callbacks_keep_the_store_limiter() {
	for callback in [
		"(func (export \"leptoskit_alloc\") (param i32) (result i32)
			(drop (memory.grow (i32.const 1025))) i32.const 1)",
		"(func (export \"leptoskit_plugins_ready\")
			(drop (memory.grow (i32.const 1025))))",
	] {
		let wasm = guest("(memory (export \"memory\") 1)", "", "", callback);
		let mut host = WasmHost::headless(wasm.as_bytes()).expect("small guest loads");
		let result = if callback.contains("leptoskit_alloc") {
			host.dispatch_wire(1, EventKind::Change, Some("{}"))
				.map(|_| ())
		} else {
			host.wake_plugins()
		};

		assert!(matches!(result, Err(WasmError::ResourceLimitExceeded)));
	}
}
