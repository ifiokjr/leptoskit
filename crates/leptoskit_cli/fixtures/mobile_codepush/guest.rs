//! The same host renders both releases; only this interpreted guest changes.
#![cfg(target_arch = "wasm32")]
use std::cell::Cell;

const TITLE: &str = "__TITLE__";
const STEP: u32 = 1;
const TRAP: bool = false;

#[link(wasm_import_module = "leptoskit")]
unsafe extern "C" {
	fn node_create(id: i64, kind: i32);
	fn node_set_props(id: i64, pointer: *const u8, length: usize);
	fn node_insert(parent: i64, child: i64, index: i32);
	fn commit();
}

thread_local! {
	static COUNT: Cell<u32> = const { Cell::new(0) };
}

fn props(id: i64, value: serde_json::Value) {
	let bytes = serde_json::to_vec(&value).expect("valid fixture properties");
	// The host copies the guest memory before returning.
	unsafe {
		node_set_props(id, bytes.as_ptr(), bytes.len());
	}
}

fn counter(value: u32) {
	props(
		3,
		serde_json::json!({"text": format!("Count: {value}"),
        "style": {"width": {"Points": 300.0}, "height": {"Points": 60.0}}}),
	);
	unsafe {
		commit();
	}
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	if TRAP {
		panic!("intentional startup trap");
	}

	unsafe {
		node_create(1, 0);
		node_create(2, 1);
		node_create(3, 1);
		node_create(4, 6);
		node_insert(1, 2, 0);
		node_insert(1, 3, 1);
		node_insert(1, 4, 2);
	}
	props(
		1,
		serde_json::json!({"style": {"width": {"Points": 350.0},
        "height": {"Points": 350.0}, "padding": {"left": {"Points": 20.0},
        "right": {"Points": 20.0}, "top": {"Points": 60.0}, "bottom": {"Points": 20.0}}}}),
	);
	props(
		2,
		serde_json::json!({"text": TITLE,
        "style": {"width": {"Points": 300.0}, "height": {"Points": 80.0}}}),
	);
	props(
		4,
		serde_json::json!({"text": "Increment", "accessibility_label": "Increment",
        "style": {"width": {"Points": 300.0}, "height": {"Points": 60.0}}}),
	);
	counter(0);
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, _ptr: i32, _len: i32) -> i32 {
	if node != 4 || kind != 0 {
		return 0;
	}

	COUNT.with(|count| {
		count.set(count.get() + STEP);
		counter(count.get());
	});
	1
}
