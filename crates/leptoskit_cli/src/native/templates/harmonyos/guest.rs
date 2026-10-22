//! A counter shared with the native Wasm runtime, rendered by ArkUI.
#![cfg(target_arch = "wasm32")]

use std::cell::Cell;

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
	let bytes = serde_json::to_vec(&value).expect("JSON-representable properties");
	// SAFETY: The host borrows the property bytes only for this call.
	unsafe {
		node_set_props(id, bytes.as_ptr(), bytes.len());
	}
}

fn render() {
	props(
		2,
		serde_json::json!({
			"text": format!("Count: {}", COUNT.get()),
			"accessibility_label": "Counter value",
			"style": {"width": {"Percent": 1.0}, "height": {"Points": 80.0}},
			"font_size": 28.0
		}),
	);
	// SAFETY: The tree and properties are valid native ABI input.
	unsafe {
		commit();
	}
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	// SAFETY: Stable application-owned node IDs and ABI widget kinds.
	unsafe {
		node_create(1, 0);
		node_create(2, 1);
		node_create(3, 6);
		node_insert(1, 2, 0);
		node_insert(1, 3, 1);
	}
	props(
		1,
		serde_json::json!({"style": {
			"width": {"Percent": 1.0}, "height": {"Percent": 1.0},
			"padding": {"left": {"Points": 24.0}, "top": {"Points": 48.0}, "right": {"Points": 24.0}, "bottom": {"Points": 24.0}}
		}}),
	);
	props(
		3,
		serde_json::json!({
			"text": "Increment", "accessibility_label": "Increment counter",
			"style": {"width": {"Percent": 1.0}, "height": {"Points": 56.0}}
		}),
	);
	render();
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, _pointer: i32, _length: i32) -> i32 {
	if node == 3 && kind == 0 {
		COUNT.set(COUNT.get().saturating_add(1));
		render();
		return 1;
	}
	0
}
