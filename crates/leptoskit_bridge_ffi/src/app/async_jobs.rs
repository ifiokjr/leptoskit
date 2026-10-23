use std::collections::BTreeMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::OnceLock;

use futures::FutureExt;
use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::PluginId;
use leptoskit_bridge::Reply;
use leptoskit_bridge::ReplyGuard;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use tokio::runtime::Runtime;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;

use super::App;
use super::WorkerReply;
use crate::NativeRequest;

const MAX_JOBS: usize = 128;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_REPLY: usize = 8 * 1024 * 1024;

fn runtime() -> Result<&'static Runtime, Error> {
	static RUNTIME: OnceLock<Result<Runtime, std::io::Error>> = OnceLock::new();
	RUNTIME
		.get_or_init(|| {
			tokio::runtime::Builder::new_multi_thread()
				.worker_threads(2)
				.max_blocking_threads(4)
				.thread_name("leptoskit-async")
				.enable_all()
				.build()
		})
		.as_ref()
		.map_err(|_| Error::Native)
}

pub(super) struct AsyncHost {
	plugins: BTreeMap<u32, Arc<dyn AsyncPlugin>>,
	state: Arc<State>,
}

struct State {
	closed: CancellationToken,
	jobs: Arc<Semaphore>,
	bytes: Arc<Semaphore>,
}

impl Default for AsyncHost {
	fn default() -> Self {
		Self {
			plugins: BTreeMap::new(),
			state: Arc::new(State {
				closed: CancellationToken::new(),
				jobs: Arc::new(Semaphore::new(MAX_JOBS)),
				bytes: Arc::new(Semaphore::new(MAX_BYTES)),
			}),
		}
	}
}

impl AsyncHost {
	pub(super) fn install(
		&mut self,
		bridge: &mut Bridge,
		plugin: Arc<dyn AsyncPlugin>,
	) -> Result<PluginId, Error> {
		runtime()?;
		for method in plugin.methods() {
			if plugin.reply_limit(*method)? > MAX_REPLY {
				return Err(Error::TooLarge);
			}
		}
		let id = bridge.register(plugin.name(), plugin.methods().iter().copied())?;
		for method in plugin.methods() {
			bridge.grant(id, *method, true)?;
		}
		self.plugins.insert(id.0, plugin);
		Ok(id)
	}
}

impl Drop for AsyncHost {
	fn drop(&mut self) {
		self.state.closed.cancel();
		if self.plugins.is_empty() {
			return;
		}
		// A successful installation initialized the process-owned runtime. It
		// remains alive while app-independent calls and shutdown futures drain.
		if let Ok(runtime) = runtime() {
			for plugin in std::mem::take(&mut self.plugins).into_values() {
				runtime.spawn(async move {
					plugin.shutdown().await;
				});
			}
		}
	}
}

/// Independently retained async work. Starting or releasing consumes it once.
pub struct AsyncJob {
	plugin: Arc<dyn AsyncPlugin>,
	method: MethodId,
	payload: Bytes,
	cancellation: CancellationToken,
	permission: CancellationToken,
	app_closed: CancellationToken,
	admission: Result<Admission, Error>,
}

struct Admission {
	_job: OwnedSemaphorePermit,
	_request: OwnedSemaphorePermit,
	_scratch: OwnedSemaphorePermit,
	reply: ReplyReservation,
	limit: usize,
}

struct ReplyReservation {
	_permit: OwnedSemaphorePermit,
}
impl ReplyGuard for ReplyReservation {
	fn commit(&mut self) {}
}

/// A cancellation token retained independently of an async job and its reply.
pub struct AsyncCancellation(CancellationToken);

fn admit(state: &State, request: usize, reply: usize) -> Result<Admission, Error> {
	if request > MAX_REPLY || reply > MAX_REPLY {
		return Err(Error::TooLarge);
	}
	let job = Arc::clone(&state.jobs)
		.try_acquire_owned()
		.map_err(|_| Error::Busy)?;
	let request = Arc::clone(&state.bytes)
		.try_acquire_many_owned((request * 2) as u32)
		.map_err(|_| Error::Busy)?;
	let response = Arc::clone(&state.bytes)
		.try_acquire_many_owned(reply as u32)
		.map_err(|_| Error::Busy)?;
	let scratch = Arc::clone(&state.bytes)
		.try_acquire_many_owned(reply as u32)
		.map_err(|_| Error::Busy)?;
	Ok(Admission {
		_job: job,
		_request: request,
		_scratch: scratch,
		reply: ReplyReservation { _permit: response },
		limit: reply,
	})
}

/// Retains an async plugin and request without copying its payload. Returns
/// null only when the request does not select an async Rust plugin.
///
/// # Safety
/// `app` is exclusively borrowed on its owner thread. `request` lends a live
/// native request and payload owner. Its ID, plugin and method must identify
/// the same drained call. Release the original request separately.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_async(
	app: *mut App,
	request: *const NativeRequest,
) -> *mut AsyncJob {
	// SAFETY: The host lends both structures for this call.
	let (Some(app), Some(request)) = (unsafe { app.as_mut() }, unsafe { request.as_ref() }) else {
		return ptr::null_mut();
	};
	let Some(plugin) = app.async_host.plugins.get(&request.plugin) else {
		return ptr::null_mut();
	};
	// SAFETY: The request contract lends its live Bytes owner.
	let Some(payload) = (unsafe { request.payload.owner.as_ref() }) else {
		return ptr::null_mut();
	};
	let method = MethodId(request.method);
	let permission = app.host.plugins().permission(CallId(request.id));
	let admission = permission
		.as_ref()
		.map_err(|e| *e)
		.and_then(|_| plugin.reply_limit(method))
		.and_then(|limit| admit(&app.async_host.state, payload.len(), limit));
	let permission = permission.unwrap_or_else(|_| {
		let token = CancellationToken::new();
		token.cancel();
		token
	});
	let cancellation = permission.child_token();
	Box::into_raw(Box::new(AsyncJob {
		plugin: Arc::clone(plugin),
		method,
		payload: payload.clone(),
		cancellation,
		permission,
		app_closed: app.async_host.state.closed.clone(),
		admission,
	}))
}

/// Retains cancellation before starting the job.
///
/// # Safety
/// `job` is a live unstarted job. Release the returned token once, after all
/// concurrent borrows of that token have finished.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_cancellation_retain(
	job: *const AsyncJob,
) -> *mut AsyncCancellation {
	// SAFETY: The host lends an unstarted job.
	unsafe { job.as_ref() }.map_or(ptr::null_mut(), |job| {
		Box::into_raw(Box::new(AsyncCancellation(job.cancellation.clone())))
	})
}

/// Requests cancellation; completion is still delivered after actual cleanup.
///
/// # Safety
/// `token` is null or a live borrowed token, including during concurrent calls.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_cancel(token: *const AsyncCancellation) {
	// SAFETY: Token destruction is synchronized by its owner.
	if let Some(token) = unsafe { token.as_ref() } {
		token.0.cancel();
	}
}

/// Releases a token independently of job completion.
///
/// # Safety
/// `token` is null or uniquely owned and has no outstanding concurrent borrows.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_cancellation_release(token: *mut AsyncCancellation) {
	if !token.is_null() {
		// SAFETY: Ownership is returned exactly once.
		drop(unsafe { Box::from_raw(token) });
	}
}

/// Releases an unstarted job without invoking its plugin.
///
/// # Safety
/// `job` is null or uniquely owned, unstarted, and not previously released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_release(job: *mut AsyncJob) {
	if !job.is_null() {
		// SAFETY: Ownership is returned exactly once.
		drop(unsafe { Box::from_raw(job) });
	}
}

struct Completion {
	context: usize,
	callback: unsafe extern "C" fn(*mut c_void, *mut WorkerReply),
}
impl Completion {
	fn finish(self, result: Result<Reply, Error>) {
		let reply = Box::into_raw(Box::new(WorkerReply(result)));
		// SAFETY: start transfers the context to this exactly-once callback.
		unsafe { (self.callback)(self.context as *mut c_void, reply) };
	}
}

/// Consumes a job and invokes `completed` exactly once with an owned reply.
/// The callback can run before this function returns or on a runtime thread.
/// It must only resume the host job, never enter the app/bridge directly.
///
/// # Safety
/// `job` is null or uniquely owned and unstarted. `context` is transferred to
/// `completed`, which must release it and complete/release its reply once.
/// The callback must be thread-safe, must not unwind, and must remain valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_start(
	job: *mut AsyncJob,
	context: *mut c_void,
	completed: unsafe extern "C" fn(*mut c_void, *mut WorkerReply),
) {
	let completion = Completion {
		context: context as usize,
		callback: completed,
	};
	if job.is_null() {
		completion.finish(Err(Error::InvalidArgument));
		return;
	}
	// SAFETY: start consumes the unique job allocation.
	let job = unsafe { Box::from_raw(job) };
	let Ok(runtime) = runtime() else {
		drop(job);
		completion.finish(Err(Error::Native));
		return;
	};
	runtime.spawn(async move {
		let result = std::panic::AssertUnwindSafe(execute(*job))
			.catch_unwind()
			.await
			.unwrap_or(Err(Error::Native));
		completion.finish(result);
	});
}

async fn execute(job: AsyncJob) -> Result<Reply, Error> {
	let admission = job.admission?;
	if job.permission.is_cancelled() {
		return Err(Error::PermissionDenied);
	}
	if job.cancellation.is_cancelled() || job.app_closed.is_cancelled() {
		return Err(Error::Cancelled);
	}
	let result = {
		let future = job
			.plugin
			.invoke(job.method, job.payload, job.cancellation.clone());
		tokio::pin!(future);
		tokio::select! {
			biased;
			() = job.permission.cancelled() => { job.cancellation.cancel(); future.await }
			() = job.app_closed.cancelled() => { job.cancellation.cancel(); future.await }
			result = &mut future => result,
		}
	};
	if job.permission.is_cancelled() {
		return Err(Error::PermissionDenied);
	}
	if job.cancellation.is_cancelled() || job.app_closed.is_cancelled() {
		return Err(Error::Cancelled);
	}
	let reply = result?;
	if reply.bytes().len() > admission.limit {
		return Err(Error::TooLarge);
	}
	Ok(reply.with_guard(admission.reply))
}
