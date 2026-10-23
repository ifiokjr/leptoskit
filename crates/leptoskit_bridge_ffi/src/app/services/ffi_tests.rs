//! Foreign activation, destructor and joined-close use owned native handles.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_bridge::Error;

use super::tests::GUEST;
use super::tests::Service;
use super::tests::State;
use crate::app::*;

#[derive(Default)]
struct Callbacks {
	wake: AtomicUsize,
	released: AtomicUsize,
	closing: Mutex<Vec<usize>>,
}

unsafe extern "C" fn wake(context: *mut c_void) {
	// SAFETY: Activation retains the boxed Arc until the release callback.
	unsafe { &*context.cast::<Arc<Callbacks>>() }
		.wake
		.fetch_add(1, Ordering::AcqRel);
}

unsafe extern "C" fn close(context: *mut c_void, owner: *mut ClosingApp) {
	// SAFETY: Close runs on the owner thread and transfers the closing handle.
	unsafe { &*context.cast::<Arc<Callbacks>>() }
		.closing
		.lock()
		.unwrap()
		.push(owner as usize);
}

unsafe extern "C" fn release(context: *mut c_void) {
	// SAFETY: This callback receives its unique allocation exactly once.
	let owner = unsafe { Box::from_raw(context.cast::<Arc<Callbacks>>()) };
	owner.released.fetch_add(1, Ordering::AcqRel);
}

fn callbacks(owner: &Arc<Callbacks>) -> NativeServiceCallbacks {
	NativeServiceCallbacks {
		context: Box::into_raw(Box::new(Arc::clone(owner))).cast(),
		wake: Some(wake),
		close: Some(close),
		release: Some(release),
	}
}

#[test]
fn foreign_activation_and_error_drop_retain_close_and_callback_storage() {
	let state = Arc::new(State::default());
	let service_state = Arc::clone(&state);
	let callbacks_owner = Arc::new(Callbacks::default());
	let mut app = App::new(GUEST).unwrap();
	app.defer_service(Box::new(move |_| {
		Ok(Box::new(Service {
			state: service_state,
			call: None,
		}))
	}))
	.unwrap();
	let app = Box::into_raw(Box::new(app));

	// SAFETY: Every handle is uniquely owned and accessed on this test thread.
	unsafe {
		assert_eq!(lk_app_start(app, 100.0, 100.0), -1);
		assert_eq!(lk_app_activate_service(app, callbacks(&callbacks_owner)), 0);
		assert_eq!(lk_app_poll_service(app), 0);
		lk_app_destroy(app);
		assert!(state.closed.load(Ordering::Acquire));
		assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 0);
		let owner = callbacks_owner.closing.lock().unwrap().pop().unwrap() as *mut ClosingApp;
		assert_eq!(lk_app_close_poll(owner), 0);
		state.ready.store(true, Ordering::Release);
		state.waker.lock().unwrap().as_ref().unwrap().wake_by_ref();
		assert_eq!(callbacks_owner.wake.load(Ordering::Acquire), 1);
		assert_eq!(lk_app_close_poll(owner), 1);
		assert!(state.joined.load(Ordering::Acquire));
		lk_app_close_release(owner);
	}
	state.waker.lock().unwrap().take();
	assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 1);
}

#[test]
fn failed_activation_never_starts_guest_and_no_service_releases_callbacks() {
	let callbacks_owner = Arc::new(Callbacks::default());
	let mut app = App::new(GUEST).unwrap();
	app.defer_service(Box::new(|_| Err(Error::Native))).unwrap();
	let app = Box::into_raw(Box::new(app));

	// SAFETY: Every handle is transferred or borrowed once on this owner
	// thread.
	unsafe {
		assert_eq!(
			lk_app_activate_service(app, callbacks(&callbacks_owner)),
			-(Error::Native as i32)
		);
		assert_eq!(lk_app_start(app, 100.0, 100.0), -1);
		lk_app_destroy(app);
		assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 1);
		let app = Box::into_raw(Box::new(App::new(GUEST).unwrap()));
		assert_eq!(lk_app_activate_service(app, callbacks(&callbacks_owner)), 0);
		assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 2);
		let owner = lk_app_begin_close(app);
		assert_eq!(lk_app_close_poll(owner), 1);
		lk_app_close_release(owner);
	}
	assert_eq!(callbacks_owner.closing.lock().unwrap().len(), 0);
}
