//! Thread-confined application runtime for UIKit and Android hosts.
//!
//! Platform adapters drain rendering transactions and native requests after
//! each guest entry point. No platform callback runs while Rust borrows the
//! interpreter. Plugin payloads retain the binary bridge's ownership contract.

use std::collections::BTreeMap;
use std::ptr;

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Error;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::PluginId;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeEvent;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::NodeId;

use crate::Buffer;
use crate::NativeRequest;

/// One loaded guest and its installed synchronous Rust plugins.
pub struct App {
	host: WasmHost<HeadlessBackend>,
	plugins: BTreeMap<u32, Box<dyn Plugin>>,
	nodes: BTreeMap<usize, NodeId>,
	error: String,
	started: bool,
}

impl App {
	/// Loads a guest without running it. Install plugins before starting.
	pub fn new(wasm: &[u8]) -> Result<Self, leptoskit_wasm::WasmError> {
		Ok(Self {
			host: WasmHost::headless(wasm)?,
			plugins: BTreeMap::new(),
			nodes: BTreeMap::new(),
			error: String::new(),
			started: false,
		})
	}

	/// Installs a selected Rust plugin and grants its declared methods.
	/// Native platform plugins are installed by their generated registrant.
	pub fn install(&mut self, plugin: Box<dyn Plugin>) -> Result<PluginId, Error> {
		let bridge = self.host.plugins();
		let id = bridge.register(plugin.name(), plugin.methods().iter().copied())?;
		for method in plugin.methods() {
			bridge.grant(id, *method, true)?;
		}
		self.plugins.insert(id.0, plugin);
		Ok(id)
	}

	fn record(&mut self, result: Result<(), leptoskit_wasm::WasmError>) -> i32 {
		match result {
			Ok(()) => {
				self.error.clear();
				0
			}
			Err(error) => {
				self.error = error.to_string();
				-1
			}
		}
	}
}

/// Entry-point helper used by generated application registrants.
///
/// # Safety
/// `wasm` addresses `len` bytes, and `error` is writable output storage.
/// On failure the caller must release the returned error buffer.
pub unsafe fn create(
	wasm: *const u8,
	len: usize,
	error: *mut Buffer,
	install: impl FnOnce(&mut App) -> Result<(), Error>,
) -> *mut App {
	if error.is_null() {
		return ptr::null_mut();
	}
	let result = if wasm.is_null() || len == 0 {
		Err("empty application bundle".to_owned())
	} else {
		// SAFETY: The host lends the input for this call only.
		App::new(unsafe { std::slice::from_raw_parts(wasm, len) })
			.map_err(|error| error.to_string())
			.and_then(|mut app| {
				install(&mut app).map_err(|error| format!("register plugin: {error:?}"))?;
				Ok(app)
			})
	};
	match result {
		Ok(app) => Box::into_raw(Box::new(app)),
		Err(message) => {
			// SAFETY: Caller provides writable output storage.
			unsafe {
				error.write(Buffer::new(message.into()));
			}
			ptr::null_mut()
		}
	}
}

/// Destroys an application after its native tasks have been cancelled.
///
/// # Safety
/// `app` is null or uniquely owns an unreleased `Box<App>`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_destroy(app: *mut App) {
	if !app.is_null() {
		// SAFETY: Ownership is returned by the embedding host.
		drop(unsafe { Box::from_raw(app) });
	}
}

/// Borrows the guest's plugin bridge, never a standalone bridge to destroy.
///
/// # Safety
/// The host exclusively borrows a live app on its owning thread. The returned
/// pointer may only be used until the next application operation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_plugins(app: *mut App) -> *mut Bridge {
	// SAFETY: App access is exclusive for this operation.
	unsafe { app.as_mut() }.map_or(ptr::null_mut(), |app| app.host.plugins())
}

/// Runs the initial guest entry point once.
///
/// # Safety
/// `app` is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_start(app: *mut App, width: f32, height: f32) -> i32 {
	// SAFETY: The embedding host owns the app.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return -1;
	};
	if app.started || !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
		app.error = "invalid viewport or application already started".into();
		return -1;
	}
	app.started = true;
	let result = app.host.start(width, height);
	app.record(result)
}

/// Updates layout after resizing or a native appearance change.
///
/// # Safety
/// `app` is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_resize(app: *mut App, width: f32, height: f32, dark: bool) -> i32 {
	// SAFETY: The embedding host owns the app.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return -1;
	};
	if !app.started || !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
		app.error = "cannot resize before start or with an invalid viewport".into();
		return -1;
	}
	app.host.set_viewport(width, height);
	app.host.set_dark_mode(dark);
	app.error.clear();
	0
}

/// Dispatches an event using the rendered node ID. Text is borrowed UTF-8.
/// Kinds: press 0, change 1, submit 2, focus 3, blur 4.
///
/// # Safety
/// `app` is exclusive; `text` addresses `len` bytes (null only when empty).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_event(
	app: *mut App,
	node: usize,
	kind: u32,
	text: *const u8,
	len: usize,
) -> i32 {
	// SAFETY: All inputs are borrows for this call.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return -1;
	};
	let value = if len == 0 {
		""
	} else {
		if text.is_null() {
			app.error = "null event text with nonzero length".into();
			return -1;
		}
		// SAFETY: Caller supplies the declared readable bytes.
		let Ok(value) = std::str::from_utf8(unsafe { std::slice::from_raw_parts(text, len) })
		else {
			app.error = "event text must be UTF-8".into();
			return -1;
		};
		value
	};
	let event = match kind {
		0 => NativeEvent::Press,
		1 => {
			NativeEvent::Change {
				value: value.into(),
			}
		}
		2 => {
			NativeEvent::Submit {
				value: value.into(),
			}
		}
		3 => NativeEvent::Focus,
		4 => NativeEvent::Blur,
		_ => {
			app.error = "unknown event kind".into();
			return -1;
		}
	};
	let Some(node) = app.nodes.get(&node).copied() else {
		return 0;
	};
	let result = app.host.dispatch(node, &event).map(|_| ());
	app.record(result)
}

/// Wakes the guest after completing native work or emitting an event.
///
/// # Safety
/// `app` is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_wake(app: *mut App) -> i32 {
	// SAFETY: The embedding host owns the app.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return -1;
	};
	let result = app.host.wake_plugins();
	app.record(result)
}

/// Drains render transactions. Release the returned buffer once.
/// The v1 JSON rendering protocol is separate from binary plugin payloads.
///
/// # Safety
/// `app` is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_render(app: *mut App) -> Buffer {
	// SAFETY: The embedding host owns the app.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return Buffer::new("null".into());
	};
	let root = app.host.surface().root().map(NodeId::index);
	let mutations = app.host.take_mutations();
	for mutation in &mutations {
		match mutation {
			Mutation::Create { id, .. } => {
				app.nodes.insert(id.index(), *id);
			}
			Mutation::Delete { id } => {
				app.nodes.remove(&id.index());
			}
			_ => {}
		}
	}
	// Only finite renderer frames, strings, integers and optional fields are
	// serialized; no fallible custom serializers participate.
	let bytes = serde_json::to_vec(
		&serde_json::json!({ "version": 1, "root": root, "mutations": mutations }),
	)
	.expect("render transaction is JSON representable");
	Buffer::new(bytes.into())
}

/// Copies the last runtime diagnostic into an owned buffer.
///
/// # Safety
/// `app` is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_error(app: *mut App) -> Buffer {
	// SAFETY: The embedding host owns the app.
	Buffer::new(
		unsafe { app.as_ref() }
			.map_or("invalid app", |app| app.error.as_str())
			.to_owned()
			.into(),
	)
}

/// Services at most 64 Rust calls, then returns a native request (1), idle (0),
/// or a request to yield and pump on the next UI turn (2). Negative is failure.
///
/// # Safety
/// `app` is exclusively borrowed; `out` is writable. An available request
/// transfers payload ownership to the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_next_request(app: *mut App, out: *mut NativeRequest) -> i32 {
	// SAFETY: The embedding host lends both outputs exclusively.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return -1;
	};
	if out.is_null() {
		return -1;
	}
	for _ in 0..64 {
		let Some(request) = app.host.plugins().next_request() else {
			return 0;
		};
		if let Some(plugin) = app.plugins.get_mut(&request.plugin.0) {
			let reply = plugin.invoke(request.method, request.payload);
			if let Err(error) = app.host.plugins().complete(request.id, reply) {
				app.error = format!("complete Rust plugin: {error:?}");
				return -1;
			}
			let result = app.host.wake_plugins();
			if app.record(result) != 0 {
				return -1;
			}
		} else {
			// SAFETY: `out` points to caller-owned output storage.
			unsafe {
				out.write(NativeRequest {
					id: request.id.0,
					plugin: request.plugin.0,
					method: request.method.0,
					payload: Buffer::new(request.payload),
				});
			}
			return 1;
		}
	}
	2
}
