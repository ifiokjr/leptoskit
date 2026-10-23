//! Stable optional async ABI for hosts built without async plugins.
use std::ffi::c_void;
use std::ptr;

use leptoskit_bridge::Error;

use super::App;
use super::WorkerReply;
use crate::NativeRequest;

/// Opaque uninhabited job when async support is disabled.
pub enum AsyncJob {}
/// Opaque uninhabited cancellation when async support is disabled.
pub enum AsyncCancellation {}

/// No async plugin is installed in this build.
/// # Safety
/// Arguments follow the enabled async ABI's borrowed-pointer contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_async(_: *mut App, _: *const NativeRequest) -> *mut AsyncJob {
	ptr::null_mut()
}
/// No token is created without async support.
/// # Safety
/// The pointer must be null in this build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_cancellation_retain(
	_: *const AsyncJob,
) -> *mut AsyncCancellation {
	ptr::null_mut()
}
/// Cancels a null token.
/// # Safety
/// The pointer must be null in this build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_cancel(_: *const AsyncCancellation) {}
/// Releases a null token.
/// # Safety
/// The pointer must be null in this build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_cancellation_release(_: *mut AsyncCancellation) {}
/// Releases a null job.
/// # Safety
/// The pointer must be null in this build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_release(_: *mut AsyncJob) {}
/// Reports unavailable through the same exactly-once callback contract.
/// # Safety
/// The job is null; the callback consumes context and its owned reply.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_async_start(
	_: *mut AsyncJob,
	context: *mut c_void,
	completed: unsafe extern "C" fn(*mut c_void, *mut WorkerReply),
) {
	// SAFETY: Caller transfers context ownership to its valid callback.
	unsafe {
		completed(
			context,
			Box::into_raw(Box::new(WorkerReply(Err(Error::Unavailable)))),
		)
	};
}
