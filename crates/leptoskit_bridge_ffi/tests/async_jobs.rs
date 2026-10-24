#![cfg(feature = "async-host")]
#![allow(unsafe_code)]
use std::ffi::c_void;
use std::future::Future;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_bridge::async_host::PluginFuture;
use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::*;

const GUEST: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
)"#;
struct AwaitCancellation {
	entered: Arc<AtomicUsize>,
	cleaned: Arc<AtomicUsize>,
	limit: usize,
}
impl AsyncPlugin for AwaitCancellation {
	fn name(&self) -> &'static str {
		"async-test"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(0), MethodId(1)]
	}

	fn reply_limit(&self, _: MethodId) -> Result<usize, Error> {
		Ok(self.limit)
	}

	fn invoke(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> PluginFuture<'_> {
		Box::pin(async move {
			self.entered.fetch_add(1, Ordering::SeqCst);
			if method == MethodId(0) {
				return Ok(payload.into());
			}

			cancellation.cancelled().await;
			// Real asynchronous cleanup must finish before callback delivery.
			tokio::time::sleep(Duration::from_millis(10)).await;
			self.cleaned.fetch_add(1, Ordering::SeqCst);
			Err(Error::Cancelled)
		})
	}

	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(async {})
	}
}
struct Fixture {
	app: *mut App,
	plugin: u32,
	entered: Arc<AtomicUsize>,
	cleaned: Arc<AtomicUsize>,
}
impl Fixture {
	fn new(limit: usize) -> Self {
		let entered = Arc::new(AtomicUsize::new(0));
		let cleaned = Arc::new(AtomicUsize::new(0));
		let mut app = Box::new(App::new(GUEST).unwrap());
		let plugin = app
			.install_async(Arc::new(AwaitCancellation {
				entered: Arc::clone(&entered),
				cleaned: Arc::clone(&cleaned),
				limit,
			}))
			.unwrap()
			.0;
		Self {
			app: Box::into_raw(app),
			plugin,
			entered,
			cleaned,
		}
	}

	fn job(&self, method: u32) -> (u64, *mut AsyncJob) {
		// SAFETY: Fixture exclusively owns its app on this test thread.
		unsafe {
			let id = lk_bridge_submit(
				lk_app_plugins(self.app),
				self.plugin,
				method,
				b"echo".as_ptr(),
				4,
			);
			assert!(id > 0);
			let mut request = MaybeUninit::uninit();
			assert_eq!(lk_app_next_request(self.app, request.as_mut_ptr()), 1);
			let request = request.assume_init();
			let job = lk_app_async(self.app, &raw const request);
			assert!(!job.is_null());
			lk_buffer_release(request.payload);
			(id as u64, job)
		}
	}

	fn close(&mut self) {
		// SAFETY: Destroy once, on the owner thread.
		unsafe {
			lk_app_destroy(std::mem::replace(&mut self.app, std::ptr::null_mut()));
		}
	}
}
impl Drop for Fixture {
	fn drop(&mut self) {
		self.close();
	}
}
unsafe extern "C" fn completed(context: *mut c_void, reply: *mut WorkerReply) {
	// SAFETY: start transfers the boxed sender to this exactly-once callback.
	let sender = unsafe { Box::from_raw(context.cast::<mpsc::Sender<Box<WorkerReply>>>()) };
	// SAFETY: callback receives sole reply ownership.
	sender.send(unsafe { Box::from_raw(reply) }).unwrap();
}
fn start(job: *mut AsyncJob) -> mpsc::Receiver<Box<WorkerReply>> {
	let (sender, receiver) = mpsc::channel();
	// SAFETY: Consumes the unstarted job and boxed callback sender.
	unsafe {
		lk_async_start(job, Box::into_raw(Box::new(sender)).cast(), completed);
	}
	receiver
}
fn finish(
	fixture: &Fixture,
	id: u64,
	receiver: &mpsc::Receiver<Box<WorkerReply>>,
) -> Result<Bytes, Error> {
	let reply = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
	// SAFETY: Completion runs on the owning test thread and consumes its reply.
	unsafe {
		assert_eq!(
			lk_worker_complete(lk_app_plugins(fixture.app), id, Box::into_raw(reply)),
			0
		);
		(*lk_app_plugins(fixture.app))
			.take_result(leptoskit_bridge::CallId(id))
			.unwrap()
			.unwrap()
	}
}
#[test]
fn null_handles_are_refused_and_unstarted_jobs_release_without_running() {
	// SAFETY: every entry point documents null as a refusal or a no-op.
	unsafe {
		assert!(lk_app_async(std::ptr::null_mut(), std::ptr::null()).is_null());
		assert!(lk_async_cancellation_retain(std::ptr::null()).is_null());
		lk_async_cancel(std::ptr::null());
		lk_async_cancellation_release(std::ptr::null_mut());
		lk_async_release(std::ptr::null_mut());
	}
	// Starting a null job still completes exactly once, with an error reply.
	let receiver = start(std::ptr::null_mut());
	let reply = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
	// SAFETY: the callback handed over sole ownership of the reply.
	unsafe { lk_worker_reply_release(Box::into_raw(reply)) };
	let fixture = Fixture::new(16);
	let entered = fixture.entered.load(Ordering::SeqCst);
	let (_, job) = fixture.job(0);
	// SAFETY: the job is unstarted and released exactly once.
	unsafe { lk_async_release(job) };

	assert_eq!(fixture.entered.load(Ordering::SeqCst), entered);
}
#[test]
fn async_echo_moves_owned_reply_and_callback_runs_once() {
	let fixture = Fixture::new(16);
	let (id, job) = fixture.job(0);
	let receiver = start(job);
	assert_eq!(finish(&fixture, id, &receiver).unwrap(), b"echo".as_slice());
	assert!(matches!(
		receiver.recv_timeout(Duration::from_secs(1)),
		Err(mpsc::RecvTimeoutError::Disconnected)
	));
}
#[test]
fn cancel_before_start_never_invokes_plugin() {
	let fixture = Fixture::new(16);
	let (id, job) = fixture.job(1);
	// SAFETY: Retain/cancel token before start; destruction follows all
	// borrows.
	unsafe {
		let token = lk_async_cancellation_retain(job);
		lk_async_cancel(token);
		lk_async_cancellation_release(token);
	}
	assert_eq!(finish(&fixture, id, &start(job)), Err(Error::Cancelled));
	assert_eq!(fixture.entered.load(Ordering::SeqCst), 0);
}
#[test]
fn destroying_app_before_start_prevents_execution_and_still_completes() {
	let mut fixture = Fixture::new(16);
	let (_, job) = fixture.job(0);
	fixture.close();
	let reply = start(job).recv_timeout(Duration::from_secs(5)).unwrap();
	assert_eq!(fixture.entered.load(Ordering::SeqCst), 0);
	let mut bridge = leptoskit_bridge::Bridge::default();
	let plugin = bridge.register("result", [MethodId(0)]).unwrap();
	bridge.grant(plugin, MethodId(0), true).unwrap();

	let id = bridge.submit(plugin, MethodId(0), Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	// SAFETY: This independent bridge is owned by the test thread. Consuming
	// the reply here verifies its status without touching the destroyed app.
	unsafe {
		assert_eq!(
			lk_worker_complete(&raw mut bridge, id.0, Box::into_raw(reply)),
			0
		);
	}
	assert_eq!(bridge.take_result(id).unwrap(), Some(Err(Error::Cancelled)));
}
#[test]
fn cancellation_waits_for_cleanup_and_app_can_die_before_callback() {
	let mut fixture = Fixture::new(16);
	let (_, job) = fixture.job(1);
	let receiver = start(job);

	for _ in 0..500 {
		if fixture.entered.load(Ordering::SeqCst) != 0 {
			break;
		}

		std::thread::sleep(Duration::from_millis(1));
	}

	assert_eq!(fixture.entered.load(Ordering::SeqCst), 1);

	fixture.close();
	let reply = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
	assert_eq!(fixture.cleaned.load(Ordering::SeqCst), 1);
	// SAFETY: No app exists; releasing the owned reply does not access one.
	unsafe {
		lk_worker_reply_release(Box::into_raw(reply));
	}
}
#[test]
fn revoke_then_regrant_does_not_execute_old_job() {
	let fixture = Fixture::new(16);
	let (id, job) = fixture.job(0);
	// SAFETY: Grant changes occur on the owning test thread.
	unsafe {
		assert_eq!(
			lk_bridge_grant(lk_app_plugins(fixture.app), fixture.plugin, 0, false),
			0
		);
		assert_eq!(
			lk_bridge_grant(lk_app_plugins(fixture.app), fixture.plugin, 0, true),
			0
		);
	}
	assert_eq!(
		finish(&fixture, id, &start(job)),
		Err(Error::PermissionDenied)
	);
	assert_eq!(fixture.entered.load(Ordering::SeqCst), 0);
}
#[test]
fn retained_reply_reservations_bound_admission_until_discarded() {
	let fixture = Fixture::new(7 * 1024 * 1024);
	let mut replies = Vec::new();

	for _ in 0..3 {
		let (_, job) = fixture.job(0);
		replies.push(start(job).recv_timeout(Duration::from_secs(5)).unwrap());
	}

	let (id, job) = fixture.job(0);
	assert_eq!(finish(&fixture, id, &start(job)), Err(Error::Busy));
	drop(replies);

	let (id, job) = fixture.job(0);
	assert_eq!(
		finish(&fixture, id, &start(job)).unwrap(),
		b"echo".as_slice()
	);
}
