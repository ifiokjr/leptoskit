use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::Barrier;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_bridge::Error;
use leptoskit_bridge_ffi::app::App;
use leptoskit_bridge_ffi::app::ClosingApp;
use leptoskit_bridge_ffi::app::NativeServiceCallbacks;
use leptoskit_bridge_ffi::app::lk_app_activate_service;
use leptoskit_bridge_ffi::app::lk_app_begin_close;
use leptoskit_bridge_ffi::app::lk_app_close_poll;
use leptoskit_bridge_ffi::app::lk_app_close_release;
use leptoskit_bridge_ffi::app::lk_app_destroy;
use leptoskit_bridge_ffi::app::lk_app_poll_service;

use super::support::GUEST;
use super::support::State;
use super::support::deferred;

#[derive(Default)]
struct Callbacks {
	notified: AtomicUsize,
	released: AtomicUsize,
	closing: Mutex<Vec<usize>>,
	barriers: Option<(Barrier, Barrier)>,
}

unsafe extern "C" fn wake(context: *mut c_void) {
	// SAFETY: The native service retains this boxed Arc through every wake.
	let state = unsafe { &*context.cast::<Arc<Callbacks>>() };
	state.notified.fetch_add(1, Ordering::AcqRel);

	if let Some((entered, finish)) = &state.barriers {
		entered.wait();
		finish.wait();
	}
}

unsafe extern "C" fn close(context: *mut c_void, owner: *mut ClosingApp) {
	// SAFETY: The context is retained and the callback takes this owner once.
	let state = unsafe { &*context.cast::<Arc<Callbacks>>() };
	state.closing.lock().unwrap().push(owner as usize);
}

unsafe extern "C" fn release(context: *mut c_void) {
	// SAFETY: The callback consumes its unique boxed Arc after all uses finish.
	let state = unsafe { Box::from_raw(context.cast::<Arc<Callbacks>>()) };
	state.released.fetch_add(1, Ordering::AcqRel);
}

fn callbacks(state: &Arc<Callbacks>) -> NativeServiceCallbacks {
	NativeServiceCallbacks {
		context: Box::into_raw(Box::new(Arc::clone(state))).cast(),
		wake: Some(wake),
		close: Some(close),
		release: Some(release),
	}
}

#[test]
fn invalid_tables_keep_caller_context_and_valid_null_activation_consumes_once() {
	let state = Arc::new(Callbacks::default());

	for missing in 0..3 {
		let mut table = callbacks(&state);
		let context = table.context;

		match missing {
			0 => table.wake = None,
			1 => table.close = None,
			_ => table.release = None,
		}

		// SAFETY: No App is lent. An invalid table leaves context with its
		// caller.
		unsafe {
			assert_eq!(
				lk_app_activate_service(ptr::null_mut(), table),
				-(Error::InvalidArgument as i32)
			);
			assert_eq!(state.released.load(Ordering::Acquire), missing);
			release(context);
		}
	}

	// SAFETY: A valid table consumes its context even with a null App.
	unsafe {
		assert_eq!(
			lk_app_activate_service(ptr::null_mut(), callbacks(&state)),
			-(Error::InvalidArgument as i32)
		);
	}

	assert_eq!(state.released.load(Ordering::Acquire), 4);
	assert_eq!(Arc::strong_count(&state), 1);
}

#[test]
fn rejected_second_activation_releases_only_its_context() {
	let state = Arc::new(State::default());
	let accepted = Arc::new(Callbacks::default());
	let rejected = Arc::new(Callbacks::default());
	let mut app = deferred(&state, false);

	// SAFETY: The App is exclusively lent. Both callback contexts transfer
	// once.
	unsafe {
		assert_eq!(
			lk_app_activate_service(&raw mut app, callbacks(&accepted)),
			0
		);
		assert_eq!(
			lk_app_activate_service(&raw mut app, callbacks(&rejected)),
			-(Error::Unavailable as i32)
		);
	}

	assert_eq!(rejected.released.load(Ordering::Acquire), 1);
	assert_eq!(accepted.released.load(Ordering::Acquire), 0);
	state.ready.store(true, Ordering::Release);
	drop(app);
	let owner = accepted.closing.lock().unwrap().pop().unwrap() as *mut ClosingApp;

	// SAFETY: The scheduler retained the sole closing handle on this thread.
	unsafe {
		assert_eq!(lk_app_close_poll(owner), 1);
		lk_app_close_release(owner);
	}

	state.waker.lock().unwrap().take();
	assert_eq!(accepted.released.load(Ordering::Acquire), 1);
}

#[test]
fn wake_in_flight_survives_invalidation_join_and_last_foreign_owner_release() {
	let state = Arc::new(State::default());
	let callbacks_owner = Arc::new(Callbacks {
		barriers: Some((Barrier::new(2), Barrier::new(2))),
		..Callbacks::default()
	});
	let mut app = deferred(&state, false);

	// SAFETY: Activation and polling exclusively borrow this owner-thread App.
	unsafe {
		assert_eq!(
			lk_app_activate_service(&raw mut app, callbacks(&callbacks_owner)),
			0
		);
		assert_eq!(lk_app_poll_service(&raw mut app), 0);
	}

	let waker = state.waker.lock().unwrap().take().unwrap();
	let late = waker.clone();
	let thread = std::thread::spawn(move || waker.wake());
	let (entered, finish) = callbacks_owner.barriers.as_ref().unwrap();
	entered.wait();
	state.ready.store(true, Ordering::Release);
	drop(app);
	let owner = callbacks_owner.closing.lock().unwrap().pop().unwrap() as *mut ClosingApp;

	// SAFETY: The sole retained close handle is polled and released on its
	// owner.
	unsafe {
		assert_eq!(lk_app_close_poll(owner), 1);
		lk_app_close_release(owner);
	}

	state.waker.lock().unwrap().take();
	assert_eq!(state.joined.load(Ordering::Acquire), 1);
	assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 0);
	finish.wait();
	thread.join().unwrap();
	late.wake();
	assert_eq!(callbacks_owner.notified.load(Ordering::Acquire), 1);
	assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 1);

	assert_eq!(Arc::strong_count(&callbacks_owner), 1);
}

#[test]
fn pending_foreign_close_release_reschedules_until_joined_error() {
	let state = Arc::new(State::default());
	let callbacks_owner = Arc::new(Callbacks::default());
	let app = Box::into_raw(Box::new(deferred(&state, true)));

	// SAFETY: App ownership transfers once. Each pending close handle transfers
	// back to its scheduler before a replacement handle is used.
	unsafe {
		assert_eq!(lk_app_activate_service(app, callbacks(&callbacks_owner)), 0);
		let mut owner = lk_app_begin_close(app);

		for _ in 0..3 {
			assert_eq!(lk_app_close_poll(owner), 0);
			lk_app_close_release(owner);
			assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 0);
			owner = callbacks_owner.closing.lock().unwrap().pop().unwrap() as *mut ClosingApp;
		}

		state.ready.store(true, Ordering::Release);
		state.waker.lock().unwrap().as_ref().unwrap().wake_by_ref();
		assert_eq!(callbacks_owner.notified.load(Ordering::Acquire), 1);
		assert_eq!(lk_app_close_poll(owner), -(Error::Native as i32));
		assert_eq!(lk_app_close_poll(owner), -(Error::Native as i32));
		lk_app_close_release(owner);
	}

	state.waker.lock().unwrap().take();
	assert_eq!(state.closed.load(Ordering::Acquire), 1);
	assert_eq!(state.joined.load(Ordering::Acquire), 1);
	assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 1);
}

#[test]
fn no_service_activation_releases_context_without_scheduling_cleanup() {
	let callbacks_owner = Arc::new(Callbacks::default());
	let app = Box::into_raw(Box::new(App::new(GUEST).unwrap()));

	// SAFETY: App is uniquely owned and callbacks transfer to activation once.
	unsafe {
		assert_eq!(lk_app_activate_service(app, callbacks(&callbacks_owner)), 0);
		assert_eq!(callbacks_owner.released.load(Ordering::Acquire), 1);
		lk_app_destroy(app);
	}

	assert!(callbacks_owner.closing.lock().unwrap().is_empty());
}
