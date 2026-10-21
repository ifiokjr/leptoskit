//! Thread-confined application runtime for UIKit and Android hosts.
//!
//! Platform adapters drain rendering transactions and native requests after
//! each guest entry point. No platform callback runs while Rust borrows the
//! interpreter. Plugin payloads retain the binary bridge's ownership contract.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::PluginId;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeEvent;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::NodeId;

use crate::Buffer;
use crate::NativeRequest;

/// Native application directories supplied by the trusted platform host.
#[derive(Debug)]
pub struct AppContext {
	/// Existing, application-private persistent data directory.
	pub data_dir: PathBuf,
	/// Existing, application-private cache directory.
	pub cache_dir: PathBuf,
}

/// Borrowed UTF-8 directory paths used only during app creation.
#[repr(C)]
pub struct NativeAppContext {
	pub data_dir: *const u8,
	pub data_dir_len: usize,
	pub cache_dir: *const u8,
	pub cache_dir_len: usize,
}

impl NativeAppContext {
	unsafe fn decode(&self) -> Result<AppContext, Error> {
		unsafe fn path(bytes: *const u8, len: usize) -> Result<PathBuf, Error> {
			if bytes.is_null() || len == 0 || len > 32 * 1024 {
				return Err(Error::InvalidArgument);
			}
			// SAFETY: The caller provides readable path bytes for this call.
			let bytes = unsafe { std::slice::from_raw_parts(bytes, len) };
			if bytes.contains(&0) {
				return Err(Error::InvalidArgument);
			}
			let path =
				PathBuf::from(std::str::from_utf8(bytes).map_err(|_| Error::InvalidArgument)?);
			if !path.is_absolute() || !path.is_dir() {
				return Err(Error::InvalidArgument);
			}
			Ok(path)
		}
		// SAFETY: The host lends both path buffers for app creation.
		unsafe {
			Ok(AppContext {
				data_dir: path(self.data_dir, self.data_dir_len)?,
				cache_dir: path(self.cache_dir, self.cache_dir_len)?,
			})
		}
	}
}

/// Creates an application with OS-owned storage directories for plugin
/// factories.
///
/// # Safety
/// `wasm` and `error` obey [`create`]'s contract. `context` points to a
/// readable structure whose path pointers remain readable for their declared
/// lengths until this function returns. Paths are borrowed and copied before
/// installation.
pub unsafe fn create_with_context(
	wasm: *const u8,
	len: usize,
	context: *const NativeAppContext,
	error: *mut Buffer,
	install: impl FnOnce(&mut App, &AppContext) -> Result<(), Error>,
) -> *mut App {
	// SAFETY: The caller provides the context pointer and path storage.
	let context = unsafe {
		context
			.as_ref()
			.ok_or(Error::InvalidArgument)
			.and_then(|context| context.decode())
	};
	// SAFETY: Input/output buffers obey the same contract as create.
	unsafe { create(wasm, len, error, |app| install(app, &context?)) }
}

/// One loaded guest and its installed immediate and worker Rust plugins.
pub struct App {
	host: WasmHost<HeadlessBackend>,
	plugins: BTreeMap<u32, Box<dyn Plugin>>,
	workers: BTreeMap<u32, Arc<Mutex<Box<dyn Plugin + Send>>>>,
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
			workers: BTreeMap::new(),
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

	/// Installs blocking Rust work for execution by the platform's worker pool.
	///
	/// The host bounds outstanding jobs before taking requests. Operations for
	/// one plugin are serialized; running synchronous I/O cannot be interrupted
	/// by guest cancellation. Cancellation discards its reply while retaining
	/// the job's capacity until it actually finishes.
	pub fn install_worker(&mut self, plugin: Box<dyn Plugin + Send>) -> Result<PluginId, Error> {
		let bridge = self.host.plugins();
		let id = bridge.register(plugin.name(), plugin.methods().iter().copied())?;
		for method in plugin.methods() {
			bridge.grant(id, *method, true)?;
		}
		self.workers.insert(id.0, Arc::new(Mutex::new(plugin)));
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

/// Independently owned work; contains no app, bridge, or interpreter reference.
pub struct RustWorker {
	plugin: Arc<Mutex<Box<dyn Plugin + Send>>>,
	method: MethodId,
	payload: Bytes,
	cancelled: Arc<AtomicBool>,
}

/// Thread-safe cancellation flag retained independently of the worker handle.
pub struct WorkerCancellation(Arc<AtomicBool>);

/// An independently owned reply, completed or released once by the host.
pub struct WorkerReply(Result<Bytes, Error>);

/// Retains a worker implementation and payload for a drained native request.
///
/// Returns null for platform-native plugins. The original request is borrowed
/// and must still be released; its byte allocation is retained without copying.
/// A returned worker may outlive the app and be transferred to another thread.
///
/// # Safety
/// `app` is exclusively borrowed on its owning thread. `request` supplies the
/// plugin/method and live payload owner of a drained request; its other fields
/// are not read. Its owner must not have been released. The caller releases or
/// executes the returned worker once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_worker(
	app: *mut App,
	request: *const NativeRequest,
) -> *mut RustWorker {
	// SAFETY: Both inputs are valid borrows confined to this invocation.
	let (Some(app), Some(request)) = (unsafe { app.as_ref() }, unsafe { request.as_ref() }) else {
		return ptr::null_mut();
	};
	let Some(plugin) = app.workers.get(&request.plugin) else {
		return ptr::null_mut();
	};
	// SAFETY: The request contract lends its live Bytes owner.
	let Some(payload) = (unsafe { request.payload.owner.as_ref() }) else {
		return ptr::null_mut();
	};
	Box::into_raw(Box::new(RustWorker {
		plugin: Arc::clone(plugin),
		method: MethodId(request.method),
		payload: payload.clone(),
		cancelled: Arc::new(AtomicBool::new(false)),
	}))
}

/// Retains a cancellation flag before transferring a worker to its executor.
///
/// # Safety
/// `worker` is a live unexecuted worker borrowed for this call. The returned
/// token must be released once, after no caller can still borrow it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_worker_cancellation(
	worker: *const RustWorker,
) -> *mut WorkerCancellation {
	// SAFETY: The caller lends a live worker before execution takes ownership.
	unsafe { worker.as_ref() }.map_or(ptr::null_mut(), |worker| {
		Box::into_raw(Box::new(WorkerCancellation(Arc::clone(&worker.cancelled))))
	})
}

/// Cancels work that has not entered its plugin, including work waiting for
/// another call's mutex. Does not interrupt I/O already executing in a plugin.
///
/// # Safety
/// `token` is null or a borrowed live cancellation token. Concurrent calls are
/// permitted; token release must wait until they finish.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_worker_cancel(token: *const WorkerCancellation) {
	// SAFETY: The token is lent for this call; its flag uses atomic access.
	if let Some(token) = unsafe { token.as_ref() } {
		token.0.store(true, Ordering::Release);
	}
}

/// Releases a cancellation token independently of worker/reply ownership.
///
/// # Safety
/// `token` is null or uniquely owns an unreleased token with no active borrows.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_worker_cancellation_release(token: *mut WorkerCancellation) {
	if !token.is_null() {
		// SAFETY: Ownership is transferred back exactly once.
		drop(unsafe { Box::from_raw(token) });
	}
}

/// Consumes a worker on a background thread and returns its owned reply.
///
/// This can block on plugin I/O or another call to the same plugin. It never
/// accesses the interpreter. A plugin panic becomes a native error, and poisons
/// that plugin's mutex so subsequent calls fail rather than use damaged state.
///
/// # Safety
/// `worker` is null or uniquely owns an unreleased worker from
/// [`lk_app_worker`]. Do not execute/release it again, even after cancellation.
/// Release or complete the returned reply exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_worker_execute(worker: *mut RustWorker) -> *mut WorkerReply {
	if worker.is_null() {
		return Box::into_raw(Box::new(WorkerReply(Err(Error::InvalidArgument))));
	}
	// SAFETY: The caller transfers the exact allocation returned above.
	let worker = unsafe { Box::from_raw(worker) };
	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		if worker.cancelled.load(Ordering::Acquire) {
			return Err(Error::Cancelled);
		}
		let mut plugin = worker.plugin.lock().map_err(|_| Error::Native)?;
		// A worker can spend time waiting for another invocation. Cancellation
		// during that wait must not begin a new filesystem/database operation.
		if worker.cancelled.load(Ordering::Acquire) {
			return Err(Error::Cancelled);
		}
		plugin.invoke(worker.method, worker.payload)
	}))
	.unwrap_or(Err(Error::Native));
	Box::into_raw(Box::new(WorkerReply(result)))
}

/// Releases work that has not started, without invoking the plugin.
///
/// # Safety
/// `worker` is null or uniquely owns an unexecuted, unreleased worker.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_worker_release(worker: *mut RustWorker) {
	if !worker.is_null() {
		// SAFETY: Ownership is returned exactly once.
		drop(unsafe { Box::from_raw(worker) });
	}
}

/// Completes a call by moving its reply bytes without copying.
///
/// The reply is consumed on every return path, including after app destruction
/// (pass a null bridge) or guest cancellation. Only the caller accesses the
/// bridge; worker execution never holds it.
///
/// # Safety
/// `bridge` is null or exclusively borrowed on its owning thread. `reply`
/// uniquely owns an unreleased reply returned by [`lk_worker_execute`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_worker_complete(
	bridge: *mut Bridge,
	id: u64,
	reply: *mut WorkerReply,
) -> i32 {
	if reply.is_null() {
		return -(Error::InvalidArgument as i32);
	}
	// SAFETY: The caller relinquishes this allocation on all paths.
	let reply = unsafe { Box::from_raw(reply) };
	// SAFETY: The host only lends its bridge on the owning thread.
	crate::status(
		unsafe { bridge.as_mut() }
			.ok_or(Error::InvalidArgument)
			.and_then(|bridge| bridge.complete(CallId(id), reply.0)),
	)
}

/// Releases a reply whose app closed or whose call was cancelled.
///
/// # Safety
/// `reply` is null or uniquely owns an unreleased, uncompleted worker reply.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_worker_reply_release(reply: *mut WorkerReply) {
	if !reply.is_null() {
		// SAFETY: Ownership is returned exactly once.
		drop(unsafe { Box::from_raw(reply) });
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
/// Independently retained Rust workers may finish afterward without this app.
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
