//! C ABI for native plugin hosts; see `include/leptoskit_bridge.h`.
//!
//! All bridge access must occur on its owning thread, without reentrancy.
//! Returned payloads have independent ownership and may be moved to workers.
//! The only unsafe code translates borrowed C pointers and transfers explicit
//! ownership handles. Plugin and protocol logic remains in `leptoskit_bridge`.
#![allow(unsafe_code)]

use std::ptr;
use std::slice;

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::Event;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::PluginId;

#[cfg(feature = "app")]
pub mod app;
#[cfg(feature = "app")]
pub mod updates;

/// A Rust-owned byte buffer, released exactly once with [`lk_buffer_release`].
#[derive(Debug)]
#[repr(C)]
pub struct Buffer {
	/// Immutable bytes, valid until the ownership handle is released.
	pub data: *const u8,
	/// Number of accessible bytes.
	pub len: usize,
	/// Opaque allocation owner. Never inspect or free with a foreign allocator.
	pub owner: *mut Bytes,
}

impl Buffer {
	fn new(bytes: Bytes) -> Self {
		let owner = Box::into_raw(Box::new(bytes));
		// SAFETY: owner is the live allocation just created above.
		let bytes = unsafe { &*owner };
		Self {
			data: bytes.as_ptr(),
			len: bytes.len(),
			owner,
		}
	}
}

/// A native request with independently owned payload storage.
#[derive(Debug)]
#[repr(C)]
pub struct NativeRequest {
	/// Call correlation identifier.
	pub id: u64,
	/// Host-assigned plugin identifier.
	pub plugin: u32,
	/// Plugin-defined method identifier.
	pub method: u32,
	/// Release after native work no longer needs the bytes.
	pub payload: Buffer,
}

fn status(result: Result<(), Error>) -> i32 {
	result.map_or_else(|error| -(error as i32), |()| 0)
}

/// Creates a standalone bridge. Rust runtime hosts may instead lend their
/// `WasmHost::plugins()` pointer for the duration of one native pump.
#[unsafe(no_mangle)]
pub extern "C" fn lk_bridge_create() -> *mut Bridge {
	Box::into_raw(Box::new(Bridge::default()))
}

/// Destroys a bridge created by [`lk_bridge_create`].
///
/// # Safety
/// `bridge` must be null or an unreleased pointer returned by
/// `lk_bridge_create`. Never destroy a borrowed runtime bridge. No operation
/// may still reference it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_destroy(bridge: *mut Bridge) {
	if !bridge.is_null() {
		// SAFETY: Ownership is transferred back by the caller contract.
		drop(unsafe { Box::from_raw(bridge) });
	}
}

/// Releases an owned buffer, including its immutable byte storage.
///
/// # Safety
/// `buffer` must be a live buffer returned by this library, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_buffer_release(buffer: Buffer) {
	if !buffer.owner.is_null() {
		// SAFETY: Buffer::new allocated this exact owner type.
		drop(unsafe { Box::from_raw(buffer.owner) });
	}
}

/// Registers a plugin; returns its nonnegative ID or a negative error code.
///
/// # Safety
/// `bridge` must be exclusively borrowed on its owning thread. `name` and
/// `methods` must address their declared lengths for this call (null is allowed
/// only for an empty slice). Nothing borrows those arrays after return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_register(
	bridge: *mut Bridge,
	name: *const u8,
	name_len: usize,
	methods: *const u32,
	method_count: usize,
) -> i64 {
	// SAFETY: All pointers are caller-provided borrows, bounded to this call.
	let result = unsafe {
		(|| {
			let bridge = bridge.as_mut().ok_or(Error::InvalidArgument)?;
			if name_len > 256 || method_count > 4096 {
				return Err(Error::TooLarge);
			}
			let name =
				std::str::from_utf8(borrow(name, name_len)?).map_err(|_| Error::InvalidArgument)?;
			bridge.register(
				name,
				borrow(methods, method_count)?.iter().copied().map(MethodId),
			)
		})()
	};
	result.map_or_else(|error| -i64::from(error as i32), |id| i64::from(id.0))
}

/// Grants or revokes one native operation.
///
/// # Safety
/// `bridge` must be exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_grant(
	bridge: *mut Bridge,
	plugin: u32,
	method: u32,
	allowed: bool,
) -> i32 {
	// SAFETY: The caller lends exclusive access for this call.
	status(
		unsafe { bridge.as_mut() }
			.ok_or(Error::InvalidArgument)
			.and_then(|bridge| bridge.grant(PluginId(plugin), MethodId(method), allowed)),
	)
}

/// Submits a request. Returns a positive call ID, or a negative error code.
///
/// # Safety
/// `bridge` is exclusively borrowed; `data` addresses `len` immutable bytes
/// during this call. The payload is copied once before returning.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_submit(
	bridge: *mut Bridge,
	plugin: u32,
	method: u32,
	data: *const u8,
	len: usize,
) -> i64 {
	// SAFETY: The caller supplies valid borrowed pointers for this call.
	let result = unsafe {
		(|| {
			let bridge = bridge.as_mut().ok_or(Error::InvalidArgument)?;
			bridge.check_call(PluginId(plugin), MethodId(method), len)?;
			bridge.submit(
				PluginId(plugin),
				MethodId(method),
				Bytes::copy_from_slice(borrow(data, len)?),
			)
		})()
	};
	result.map_or_else(|error| -i64::from(error as i32), |id| id.0 as i64)
}

/// Transfers the next request to `out`: 1 available, 0 empty, negative error.
///
/// # Safety
/// `bridge` is exclusively borrowed. `out` points to writable, aligned storage
/// for a `NativeRequest`. On success, release `out.payload` exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_next_request(
	bridge: *mut Bridge,
	out: *mut NativeRequest,
) -> i32 {
	// SAFETY: The bridge is borrowed for this call only.
	let Some(bridge) = (unsafe { bridge.as_mut() }) else {
		return -(Error::InvalidArgument as i32);
	};
	if out.is_null() {
		return -(Error::InvalidArgument as i32);
	}
	let Some(request) = bridge.next_request() else {
		return 0;
	};
	// SAFETY: out is caller-owned storage; writing initializes it.
	unsafe {
		ptr::write(
			out,
			NativeRequest {
				id: request.id.0,
				plugin: request.plugin.0,
				method: request.method.0,
				payload: Buffer::new(request.payload),
			},
		);
	};
	1
}

/// Completes native work. `error` is 0 for success or a positive error code.
///
/// # Safety
/// `bridge` is exclusively borrowed. On success `data` addresses `len` bytes
/// during this call. Error completions do not read `data`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_complete(
	bridge: *mut Bridge,
	id: u64,
	error: i32,
	data: *const u8,
	len: usize,
) -> i32 {
	// SAFETY: Input borrows are bounded by this invocation.
	status(unsafe {
		(|| {
			let bridge = bridge.as_mut().ok_or(Error::InvalidArgument)?;
			let result = if error == 0 {
				bridge
					.check_reply(CallId(id), len)
					.and_then(|()| borrow(data, len).map(Bytes::copy_from_slice))
			} else {
				Err(Error::from_status(-i64::from(error) - 1))
			};
			bridge.complete(CallId(id), result)
		})()
	})
}

/// Completes native work by transferring a Rust buffer back without copying.
///
/// The buffer is consumed on every return path, including an invalid call or
/// null bridge. Use [`lk_bridge_complete`] for bytes owned by a foreign
/// allocator.
///
/// # Safety
/// `bridge` is exclusively borrowed or null. `buffer` must be an unmodified,
/// unreleased buffer returned by this library. The caller relinquishes it and
/// every view into it, even when completion fails.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_complete_owned(
	bridge: *mut Bridge,
	id: u64,
	buffer: Buffer,
) -> i32 {
	if buffer.owner.is_null() {
		return -(Error::InvalidArgument as i32);
	}
	// SAFETY: The caller transfers the allocation originally made by
	// Buffer::new. Moving Bytes out releases the small owner allocation,
	// retaining its payload.
	let bytes = *unsafe { Box::from_raw(buffer.owner) };
	// SAFETY: The bridge is exclusively borrowed for this invocation.
	status(
		unsafe { bridge.as_mut() }
			.ok_or(Error::InvalidArgument)
			.and_then(|bridge| bridge.complete(CallId(id), Ok(bytes))),
	)
}

/// Takes a reply: 1 available, 0 pending, negative plugin/transport error.
/// Error replies are consumed as well.
///
/// # Safety
/// `bridge` is exclusively borrowed. `out` points to writable aligned Buffer
/// storage. Release that buffer exactly once when this returns 1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_take_result(
	bridge: *mut Bridge,
	id: u64,
	out: *mut Buffer,
) -> i32 {
	// SAFETY: The caller provides a unique borrow.
	let Some(bridge) = (unsafe { bridge.as_mut() }) else {
		return -(Error::InvalidArgument as i32);
	};
	if out.is_null() {
		return -(Error::InvalidArgument as i32);
	}
	match bridge.take_result(CallId(id)) {
		Ok(Some(Ok(bytes))) => {
			// SAFETY: out points to caller-owned output storage.
			unsafe { ptr::write(out, Buffer::new(bytes)) };
			1
		}
		Ok(None) => 0,
		Ok(Some(Err(error))) | Err(error) => -(error as i32),
	}
}

/// Cancels a call. Running calls retain their slot until the host takes the
/// notification with [`lk_bridge_next_cancelled`] and cancels native work.
///
/// # Safety
/// `bridge` is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_cancel(bridge: *mut Bridge, id: u64) -> i32 {
	// SAFETY: The caller provides a unique borrow.
	status(
		unsafe { bridge.as_mut() }
			.ok_or(Error::InvalidArgument)
			.and_then(|bridge| bridge.cancel(CallId(id))),
	)
}

/// Transfers the next cancellation to the host: positive ID, 0 empty, or a
/// negative error. Stop the native job matching the returned ID.
///
/// # Safety
/// `bridge` is exclusively borrowed on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_next_cancelled(bridge: *mut Bridge) -> i64 {
	// SAFETY: The caller lends exclusive access for this call.
	match unsafe { bridge.as_mut() } {
		Some(bridge) => bridge.next_cancelled().map_or(0, |id| id.0 as i64),
		None => -i64::from(Error::InvalidArgument as i32),
	}
}

/// Queues a native event, copying its bytes once.
///
/// # Safety
/// `bridge` is exclusively borrowed; `data` addresses `len` immutable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bridge_emit(
	bridge: *mut Bridge,
	plugin: u32,
	kind: u32,
	data: *const u8,
	len: usize,
) -> i32 {
	// SAFETY: The caller supplies borrowed storage valid for the call.
	status(unsafe {
		(|| {
			let bridge = bridge.as_mut().ok_or(Error::InvalidArgument)?;
			bridge.check_event(PluginId(plugin), len)?;
			bridge.emit(Event {
				plugin: PluginId(plugin),
				kind,
				payload: Bytes::copy_from_slice(borrow(data, len)?),
			})
		})()
	})
}

unsafe fn borrow<'a, T>(data: *const T, len: usize) -> Result<&'a [T], Error> {
	if len == 0 {
		return Ok(&[]);
	}
	if data.is_null() || len > isize::MAX as usize / size_of::<T>() {
		return Err(Error::InvalidArgument);
	}
	// SAFETY: Validity/alignment and lifetime are required by every public
	// caller.
	Ok(unsafe { slice::from_raw_parts(data, len) })
}
