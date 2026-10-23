//! Foreign callbacks own notification storage independently of application
//! state.

use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_bridge::Error;

use super::ClosingApp;
use super::ServiceWake;
use crate::app::App;

/// Owned callback context. Wake/release permit concurrent arbitrary threads.
/// Close runs on the UI owner and retains its closing handle until Ready.
/// Every callback must enqueue work rather than reenter the current operation.
#[repr(C)]
pub struct NativeServiceCallbacks {
	pub context: *mut c_void,
	pub wake: Option<unsafe extern "C" fn(*mut c_void)>,
	pub close: Option<unsafe extern "C" fn(*mut c_void, *mut ClosingApp)>,
	pub release: Option<unsafe extern "C" fn(*mut c_void)>,
}

struct Callbacks {
	context: usize,
	wake: unsafe extern "C" fn(*mut c_void),
	close: unsafe extern "C" fn(*mut c_void, *mut ClosingApp),
	release: unsafe extern "C" fn(*mut c_void),
}

impl Callbacks {
	fn notify(&self) {
		// SAFETY: The callback contract owns thread-safe storage until release.
		unsafe { (self.wake)(self.context as *mut c_void) };
	}
}

impl Drop for Callbacks {
	fn drop(&mut self) {
		// SAFETY: This is the last retained callback owner, including late
		// wakes.
		unsafe { (self.release)(self.context as *mut c_void) };
	}
}

/// Activates an inert installer after native wake and close owners exist.
/// A no-service app succeeds and releases the unused callbacks immediately.
///
/// # Safety
/// App is exclusively borrowed on its UI thread. Context is owned until
/// release. All callbacks are nonnull. Wake/release are thread-safe, and close
/// retains and polls the supplied owner on this thread until Ready, then
/// releases it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_activate_service(
	app: *mut App,
	callbacks: NativeServiceCallbacks,
) -> i32 {
	let (Some(wake), Some(close), Some(release)) =
		(callbacks.wake, callbacks.close, callbacks.release)
	else {
		return -(Error::InvalidArgument as i32);
	};
	let callbacks = Arc::new(Callbacks {
		context: callbacks.context as usize,
		wake,
		close,
		release,
	});
	// SAFETY: The embedding host lends the live app for this operation.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return -(Error::InvalidArgument as i32);
	};

	if app.service.is_none() {
		return 0;
	}

	let notification = Arc::clone(&callbacks);
	let wake = ServiceWake::new(move || notification.notify());
	let scheduler = Rc::new(move |owner| {
		let handle = Box::into_raw(Box::new(owner));
		// SAFETY: Close receives ownership on the app's current UI thread.
		unsafe { (callbacks.close)(callbacks.context as *mut c_void, handle) };
	});
	crate::status(app.activate_service(wake, scheduler))
}

/// Polls admission and completion delivery on the UI owner after a real wake.
///
/// # Safety
/// App is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_poll_service(app: *mut App) -> i32 {
	// SAFETY: The caller lends a live app for this operation.
	crate::status(
		unsafe { app.as_mut() }
			.ok_or(Error::InvalidArgument)
			.and_then(App::poll_service),
	)
}

/// Takes cancellation after polling services, preserving unrelated plugin work.
///
/// # Safety
/// App is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_next_cancelled(app: *mut App) -> i64 {
	// SAFETY: The caller lends the live app for this operation.
	unsafe { app.as_mut() }.map_or(-(Error::InvalidArgument as i64), |app| {
		app.next_cancelled().map_or(0, |id| id.0 as i64)
	})
}

/// Transfers the App into an explicitly awaitable retained closing owner.
///
/// # Safety
/// App is null or uniquely owns an unreleased App on its UI thread. After this
/// call, no app/bridge operation or destruction may use the transferred
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_begin_close(app: *mut App) -> *mut ClosingApp {
	if app.is_null() {
		return ptr::null_mut();
	}

	// SAFETY: The caller transfers the exact uniquely owned allocation.
	let app = unsafe { Box::from_raw(app) };
	Box::into_raw(Box::new(app.close()))
}

/// Returns zero while joining, one when closed, or a negative joined failure.
/// Real service wake notifications schedule subsequent owner-thread polls.
///
/// # Safety
/// Owner is exclusively borrowed on its UI thread and has not been released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_close_poll(owner: *mut ClosingApp) -> i32 {
	// SAFETY: The embedding host lends the unique closing owner.
	let Some(owner) = (unsafe { owner.as_mut() }) else {
		return -(Error::InvalidArgument as i32);
	};
	let waker = owner
		.wake
		.as_ref()
		.map_or_else(|| Waker::noop().clone(), ServiceWake::waker);

	match Pin::new(owner).poll(&mut Context::from_waker(&waker)) {
		Poll::Pending => 0,
		Poll::Ready(Ok(())) => 1,
		Poll::Ready(Err(error)) => -(error as i32),
	}
}

/// Releases a completed owner. A cancelled pending await returns its retained
/// remainder to the configured close scheduler instead of destroying the guest.
///
/// # Safety
/// Owner is null or uniquely owns an unreleased closing handle on its UI
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_close_release(owner: *mut ClosingApp) {
	if !owner.is_null() {
		// SAFETY: The caller returns ownership exactly once.
		drop(unsafe { Box::from_raw(owner) });
	}
}
