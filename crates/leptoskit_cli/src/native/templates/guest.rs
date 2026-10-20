//! Native UI and plugin example. This module runs inside the installed host.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::guest::Client;

#[link(wasm_import_module = "leptoskit")]
unsafe extern "C" {
	fn node_create(id: i64, kind: i32);
	fn node_set_props(id: i64, pointer: *const u8, length: usize);
	fn node_insert(parent: i64, child: i64, index: i32);
	fn commit();
}

thread_local! {
	static EXECUTOR: RefCell<LocalPool> = RefCell::new(LocalPool::new());
}

fn props(id: i64, value: serde_json::Value) {
	let bytes = serde_json::to_vec(&value).expect("valid UI properties");
	// The host copies these properties synchronously.
	unsafe {
		node_set_props(id, bytes.as_ptr(), bytes.len());
	}
}

fn status(text: &str) {
	props(
		2,
		serde_json::json!({
			"text": text,
			"style": { "width": {"Points": 320.0}, "height": {"Points": 100.0} }
		}),
	);
	unsafe {
		commit();
	}
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	unsafe {
		node_create(1, 0);
		node_create(2, 1);
		node_create(3, 6);
		node_create(4, 6);
		node_insert(1, 2, 0);
		node_insert(1, 3, 1);
		node_insert(1, 4, 2);
	}
	props(
		1,
		serde_json::json!({"style": {"width": {"Points": 350.0}, "height": {"Points": 400.0},
        "padding": {"left": {"Points": 20.0}, "right": {"Points": 20.0}, "top": {"Points": 60.0}, "bottom": {"Points": 20.0}}}}),
	);
	props(
		3,
		serde_json::json!({"text": "Hash with Rust", "accessibility_label": "Hash with Rust",
        "style": {"width": {"Points": 300.0}, "height": {"Points": 60.0}}}),
	);
	props(
		4,
		serde_json::json!({"text": "Copy to clipboard", "accessibility_label": "Copy to clipboard",
        "style": {"width": {"Points": 300.0}, "height": {"Points": 60.0}}}),
	);
	status("Ready");
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, _ptr: i32, _len: i32) -> i32 {
	if kind != 0 || !matches!(node, 3 | 4) {
		return 0;
	}
	let task = async move {
		let result = async {
			if node == 3 {
				let reply = Client::new("crypto")?
					.call(MethodId(0), b"LeptosKit")?
					.await?;
				Ok(format!("SHA-256: {} bytes", reply.len()))
			} else {
				Client::new("clipboard")?
					.call(MethodId(1), b"Hello from LeptosKit")?
					.await?;
				Ok::<_, leptoskit_bridge::Error>("Copied to clipboard".to_owned())
			}
		}
		.await;
		match result {
			Ok(message) => status(&message),
			Err(error) => status(&format!("Plugin failed: {error:?}")),
		}
	};
	EXECUTOR.with_borrow_mut(|executor| {
		executor
			.spawner()
			.spawn_local(task)
			.expect("executor is live")
	});
	leptoskit_poll();
	1
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	EXECUTOR.with_borrow_mut(LocalPool::run_until_stalled);
}
