//! Exercises the exact generated native constructor and real service ownership.

extern crate self as service_crate;

mod platform;
mod service;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::task::Context;
use std::task::Wake;
use std::task::Waker;
use std::time::Duration;

use leptoskit_bridge_ffi::Buffer;
use leptoskit_bridge_ffi::app::App;
use leptoskit_bridge_ffi::app::NativeAppContext;
use leptoskit_bridge_ffi::app::ServiceWake;
pub use service::from_app_dirs;

include!(concat!(env!("SERVICE_REGISTRANT_DIR"), "/registrant.rs"));

const WASM: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
)"#;

struct Notify(mpsc::Sender<()>);

impl Wake for Notify {
	fn wake(self: Arc<Self>) {
		self.0.send(()).unwrap();
	}
}

fn empty_buffer() -> Buffer {
	Buffer {
		data: std::ptr::null(),
		len: 0,
		owner: std::ptr::null_mut(),
	}
}

fn create(error: &mut Buffer) -> *mut App {
	let directory = std::env::temp_dir();
	let path = directory.to_str().unwrap().as_bytes();
	let context = NativeAppContext {
		data_dir: path.as_ptr(),
		data_dir_len: path.len(),
		cache_dir: path.as_ptr(),
		cache_dir_len: path.len(),
	};

	// SAFETY: Module, path buffers and unique error output outlive this call.
	unsafe {
		leptoskit_app_create_with_context(WASM.as_ptr(), WASM.len(), &raw const context, error)
	}
}

fn constructor_failure() {
	let mut error = empty_buffer();
	service::FAIL_FACTORY.store(true, Ordering::SeqCst);
	assert!(create(&mut error).is_null());
	assert!(error.len > 0);
	assert_eq!(service::STARTED.load(Ordering::SeqCst), 0);

	// SAFETY: Failed construction transferred this unique error allocation.
	unsafe { leptoskit_bridge_ffi::lk_buffer_release(error) };
	service::FAIL_FACTORY.store(false, Ordering::SeqCst);
}

fn close(app: App) {
	let (notifications, receiver) = mpsc::channel();
	let waker = Waker::from(Arc::new(Notify(notifications)));
	let mut context = Context::from_waker(&waker);
	let mut closing = pin!(app.close());
	assert!(closing.as_mut().poll(&mut context).is_pending());
	assert_eq!(service::JOINED.load(Ordering::SeqCst), 0);
	service::release_close();

	loop {
		if let std::task::Poll::Ready(result) = closing.as_mut().poll(&mut context) {
			result.unwrap();
			break;
		}

		receiver.recv_timeout(Duration::from_secs(5)).unwrap();
	}

	assert_eq!(service::JOINED.load(Ordering::SeqCst), 1);
}

fn main() {
	constructor_failure();
	let mut error = empty_buffer();
	let pointer = create(&mut error);
	assert!(!pointer.is_null());
	assert_eq!(service::STARTED.load(Ordering::SeqCst), 0);

	// SAFETY: The generated constructor transferred its unique allocation.
	let mut app = unsafe { Box::from_raw(pointer) };
	platform::register(&mut app);
	let (notifications, receiver) = mpsc::channel();
	app.activate_service(
		ServiceWake::new(move || {
			notifications.send(()).unwrap();
		}),
		Rc::new(|_| panic!("close owner must be awaited")),
	)
	.unwrap();
	platform::assert_distinct_identities(&mut app);
	assert_eq!(service::COMPLETED.load(Ordering::SeqCst), 0);
	app.poll_service().unwrap();
	receiver.recv_timeout(Duration::from_secs(5)).unwrap();

	app.poll_service().unwrap();
	assert_eq!(service::COMPLETED.load(Ordering::SeqCst), 1);

	close(*app);
	println!(
		"constructor failure clean; registration inert; native task Pending/wake/completion; \
		 close joined"
	);
}
