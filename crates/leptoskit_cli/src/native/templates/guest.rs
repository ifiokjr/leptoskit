//! Native UI and plugin example. This module runs inside the installed host.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::guest::Client;
use leptoskit_plugin_battery::guest::Battery;
use leptoskit_plugin_device::guest::Device;
use leptoskit_plugin_filesystem::Kind;
use leptoskit_plugin_filesystem::guest::Filesystem;
use leptoskit_plugin_haptics::guest::Haptics;
use leptoskit_plugin_secure_store::Key;
use leptoskit_plugin_secure_store::guest::SecureStore;

#[link(wasm_import_module = "leptoskit")]
unsafe extern "C" {
	fn node_create(id: i64, kind: i32);
	fn node_set_props(id: i64, pointer: *const u8, length: usize);
	fn node_insert(parent: i64, child: i64, index: i32);
	fn commit();
}

thread_local! {
	static EXECUTOR: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

async fn secure_storage() -> Result<String, leptoskit_bridge::Error> {
	let store = SecureStore::new()?;
	let key = Key::new("leptoskit-example", "roundtrip")?;
	let expected = b"\0LeptosKit nonsecret fixture\xff";
	store.set(key, expected).await?;
	let stored = store.get(key).await;
	// Delete the fixture even if reading it fails.
	store.delete(key).await?;
	let stored = stored?;

	if stored.as_deref() != Some(expected.as_slice()) || store.get(key).await?.is_some() {
		return Err(leptoskit_bridge::Error::Native);
	}

	Ok("Secure storage roundtrip passed".to_owned())
}

async fn clipboard() -> Result<String, leptoskit_bridge::Error> {
	let clipboard = Client::new("clipboard")?;
	let expected = "Hello from LeptosKit \u{2713}";
	clipboard.call(MethodId(1), expected.as_bytes())?.await?;
	let copied = clipboard.call(MethodId(0), &[])?.await?;
	clipboard.call(MethodId(2), &[])?.await?;
	let cleared = clipboard.call(MethodId(0), &[])?.await?;

	if copied.as_ref() != expected.as_bytes() || !cleared.is_empty() {
		return Err(leptoskit_bridge::Error::Native);
	}

	Ok("Clipboard roundtrip passed".to_owned())
}

async fn app_files() -> Result<String, leptoskit_bridge::Error> {
	let files = Filesystem::new()?;
	let expected = b"\0LeptosKit file fixture\xff";
	files.mkdir("leptoskit-example").await?;
	files.write("leptoskit-example/original", expected).await?;
	let contents = files.read("leptoskit-example/original").await?;
	let metadata = files.stat("leptoskit-example/original").await?;
	files
		.rename("leptoskit-example/original", "leptoskit-example/moved")
		.await?;
	let listing = files.list("leptoskit-example").await?;

	files.remove("leptoskit-example/moved").await?;
	files.remove("leptoskit-example").await?;

	if contents.as_ref() != expected
		|| metadata.kind != Kind::File
		|| metadata.len != expected.len() as u64
		|| listing != ["moved"]
	{
		return Err(leptoskit_bridge::Error::Native);
	}
	Ok("File roundtrip passed".to_owned())
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
			"style": { "width": {"Percent": 1.0}, "height": {"Points": 100.0}, "flex_shrink": 0.0 }
		}),
	);
	unsafe {
		commit();
	}
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	unsafe {
		node_create(1, 4);
		node_create(10, 0);
		node_create(2, 1);
		node_insert(1, 10, 0);
		node_insert(10, 2, 0);
	}
	props(
		1,
		serde_json::json!({"style": {"width": {"Percent": 1.0}, "height": {"Percent": 1.0}}}),
	);
	props(
		10,
		serde_json::json!({"style": {"width": {"Percent": 1.0}, "flex_shrink": 0.0,
        "padding": {"left": {"Points": 20.0}, "right": {"Points": 20.0}, "top": {"Points": 60.0}, "bottom": {"Points": 20.0}}}}),
	);
	for (id, index, label) in [
		(3, 1, "Hash with Rust"),
		(4, 2, "Check clipboard"),
		(5, 3, "Play haptic feedback"),
		(6, 4, "Read battery"),
		(7, 5, "Read device"),
		(8, 6, "Check secure storage"),
		(9, 7, "Check app files"),
	] {
		unsafe {
			node_create(id, 6);
			node_insert(10, id, index);
		}
		props(
			id,
			serde_json::json!({"text": label, "accessibility_label": label,
        "style": {"width": {"Percent": 1.0}, "height": {"Points": 48.0}, "flex_shrink": 0.0}}),
		);
	}
	status("Ready");
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, _ptr: i32, _len: i32) -> i32 {
	if kind != 0 || !(3..=9).contains(&node) {
		return 0;
	}

	if BUSY.with(|busy| busy.replace(true)) {
		return 1;
	}

	status("Working");
	let task = async move {
		let result = async {
			match node {
				3 => {
					let reply = Client::new("crypto")?
						.call(MethodId(0), b"LeptosKit")?
						.await?;
					Ok(format!("SHA-256: {} bytes", reply.len()))
				}
				4 => clipboard().await,
				5 => {
					Haptics::new()?.selection().await?;
					Ok("Haptic request accepted".to_owned())
				}
				6 => {
					let snapshot = Battery::new()?.snapshot().await?;
					Ok(match snapshot.charge {
						Some(charge) => format!("Battery: {}%", charge.thousandths() / 10),
						None => "Battery: unknown".to_owned(),
					})
				}
				7 => {
					let snapshot = Device::new()?.snapshot().await?;
					let os = snapshot.os_name.as_deref().unwrap_or("unknown");
					let memory = snapshot
						.total_memory
						.map(|bytes| bytes.get() / (1024 * 1024));
					Ok(match memory {
						Some(memory) => format!("Device: {os}; memory: {memory} MiB"),
						None => format!("Device: {os}; memory: unknown"),
					})
				}
				8 => secure_storage().await,
				9 => app_files().await,
				_ => Err(leptoskit_bridge::Error::InvalidArgument),
			}
		}
		.await;
		BUSY.with(|busy| busy.set(false));

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
