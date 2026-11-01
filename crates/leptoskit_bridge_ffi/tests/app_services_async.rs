#![cfg(feature = "async-host")]
#![allow(
	unsafe_code,
	reason = "exercises owned C ABI async completion alongside retained service close"
)]

#[path = "app_services_independent/support.rs"]
mod support;

use std::ffi::c_void;
use std::future::Future;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::task::Poll;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_bridge::async_host::PluginFuture;
use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::lk_buffer_release;
use support::State;
use support::deferred;
use support::poll;

const METHOD: MethodId = MethodId(42);
const TIMEOUT: Duration = Duration::from_secs(5);
const REPLY_LIMIT: usize = 16;

struct Cleanup {
	entered: mpsc::Sender<()>,
	cancelled: mpsc::Sender<()>,
	release: Arc<tokio::sync::Notify>,
}

impl AsyncPlugin for Cleanup {
	fn name(&self) -> &'static str {
		"combined-cleanup"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[METHOD]
	}

	fn reply_limit(&self, _: MethodId) -> Result<usize, Error> {
		Ok(REPLY_LIMIT)
	}

	fn invoke(&self, _: MethodId, _: Bytes, cancellation: CancellationToken) -> PluginFuture<'_> {
		Box::pin(async move {
			self.entered.send(()).unwrap();
			cancellation.cancelled().await;
			self.cancelled.send(()).unwrap();
			self.release.notified().await;
			Err(Error::Cancelled)
		})
	}

	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(async {})
	}
}

struct Payload(Arc<AtomicUsize>);

impl AsRef<[u8]> for Payload {
	fn as_ref(&self) -> &[u8] {
		b"retained interpreter"
	}
}

impl Drop for Payload {
	fn drop(&mut self) {
		self.0.fetch_add(1, Ordering::AcqRel);
	}
}

unsafe extern "C" fn completed(context: *mut c_void, reply: *mut WorkerReply) {
	// SAFETY: Start transfers the sender and unique reply to this callback
	// once.
	let sender = unsafe { Box::from_raw(context.cast::<mpsc::Sender<Box<WorkerReply>>>()) };
	// SAFETY: The callback receives sole ownership of the reply.
	sender.send(unsafe { Box::from_raw(reply) }).unwrap();
}

fn start(app: &mut App) -> mpsc::Receiver<Box<WorkerReply>> {
	let (sender, receiver) = mpsc::channel();
	let mut request = MaybeUninit::uninit();

	// SAFETY: The app and request are borrowed only on the owning test thread.
	unsafe {
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 1);
		let request = request.assume_init();
		let job = lk_app_async(app, &raw const request);
		assert!(!job.is_null());
		lk_buffer_release(request.payload);
		lk_async_start(job, Box::into_raw(Box::new(sender)).cast(), completed);
	}

	receiver
}

fn retain_payload(app: &mut App) -> Arc<AtomicUsize> {
	let dropped = Arc::new(AtomicUsize::new(0));
	app.with_host(|host| {
		let bridge = host.plugins();
		let retained = bridge.register("retained", [METHOD]).unwrap();
		bridge.grant(retained, METHOD, true).unwrap();
		bridge
			.submit(
				retained,
				METHOD,
				Bytes::from_owner(Payload(Arc::clone(&dropped))),
			)
			.unwrap();
	});

	dropped
}

fn finish(receiver: &mpsc::Receiver<Box<WorkerReply>>) {
	let reply = receiver.recv_timeout(TIMEOUT).unwrap();

	// SAFETY: The callback transferred the sole reply; no app borrow is used.
	unsafe {
		lk_worker_reply_release(Box::into_raw(reply));
	}

	assert!(matches!(
		receiver.recv_timeout(TIMEOUT),
		Err(mpsc::RecvTimeoutError::Disconnected)
	));
}

#[test]
fn retained_service_close_cancels_async_jobs_and_preserves_callback_cleanup() {
	let state = Arc::new(State::default());
	let release = Arc::new(tokio::sync::Notify::new());
	let (entered, entered_receiver) = mpsc::channel();
	let (cancelled, cancelled_receiver) = mpsc::channel();
	let mut app = deferred(&state, false);
	let plugin = app
		.install_async(Arc::new(Cleanup {
			entered,
			cancelled,
			release: Arc::clone(&release),
		}))
		.unwrap();
	app.activate_service(
		ServiceWake::new(|| {}),
		Rc::new(|_| panic!("close remains owned")),
	)
	.unwrap();
	app.with_host(|host| host.plugins().submit(plugin, METHOD, Bytes::new()))
		.unwrap();

	let receiver = start(&mut app);
	entered_receiver.recv_timeout(TIMEOUT).unwrap();

	let dropped = retain_payload(&mut app);
	let mut closing = app.close();
	cancelled_receiver.recv_timeout(TIMEOUT).unwrap();

	assert_eq!(poll(&mut closing), Poll::Pending);
	assert_eq!(dropped.load(Ordering::Acquire), 0);
	assert!(matches!(
		receiver.try_recv(),
		Err(mpsc::TryRecvError::Empty)
	));
	release.notify_one();
	finish(&receiver);
	assert_eq!(poll(&mut closing), Poll::Pending);
	assert_eq!(dropped.load(Ordering::Acquire), 0);

	state.ready.store(true, Ordering::Release);
	assert_eq!(poll(&mut closing), Poll::Ready(Ok(())));
	assert_eq!(state.closed.load(Ordering::Acquire), 1);
	assert_eq!(state.joined.load(Ordering::Acquire), 1);
	assert_eq!(dropped.load(Ordering::Acquire), 1);
	state.waker.lock().unwrap().take();
}
