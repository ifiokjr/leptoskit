//! Development guests for apps `leptoskit dev` launched, independent of UI
//! runtime handles.
//!
//! Each call may block for the server's whole wait, so call them on one
//! dedicated background thread. Only debug builds accept development guests;
//! in a release build [`lk_dev_open`] fails.

use std::ptr;
use std::slice;
use std::str;

use leptoskit_codepush::dev::DevGuests;

use crate::Buffer;

const MAX_URL_BYTES: usize = 4096;
const MAX_RUNNING_BYTES: usize = 32 * 1024 * 1024;

/// One app's subscription to its platform's development guests.
pub struct Development {
	guests: DevGuests,
	error: String,
}

/// Borrows bounded foreign input without accepting null nonempty slices.
pub(crate) unsafe fn input<'a>(
	data: *const u8,
	len: usize,
	maximum: usize,
) -> Result<&'a [u8], String> {
	if len > maximum {
		return Err(format!("input exceeds {maximum} bytes"));
	}

	if len == 0 {
		return Ok(&[]);
	}

	if data.is_null() {
		return Err("null input with nonzero length".into());
	}

	// SAFETY: The caller supplies readable storage for the bounded input.
	Ok(unsafe { slice::from_raw_parts(data, len) })
}

/// Follows the development guest at `url`, starting from the `running`
/// guest, so the first [`lk_dev_next`] waits for a change.
///
/// # Safety
/// Input pointers address their declared lengths during this call. `error`
/// points to writable aligned storage; it receives an owned buffer on
/// failure. Use and destroy the returned pointer only on one background
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_dev_open(
	url: *const u8,
	url_len: usize,
	running: *const u8,
	running_len: usize,
	error: *mut Buffer,
) -> *mut Development {
	if error.is_null() {
		return ptr::null_mut();
	}

	// SAFETY: Both foreign borrows are bounded to this call by the ABI
	// contract.
	let result = unsafe {
		(|| -> Result<Development, String> {
			let url = str::from_utf8(input(url, url_len, MAX_URL_BYTES)?)
				.map_err(|error| format!("development guest URL must be UTF-8: {error}"))?;
			let running = input(running, running_len, MAX_RUNNING_BYTES)?;
			let guests = DevGuests::new(url, running).map_err(|error| error.to_string())?;
			Ok(Development {
				guests,
				error: String::new(),
			})
		})()
	};

	match result {
		Ok(development) => Box::into_raw(Box::new(development)),
		Err(message) => {
			// SAFETY: `error` is writable output storage supplied by the
			// caller.
			unsafe { error.write(Buffer::new(message.into())) };
			ptr::null_mut()
		}
	}
}

/// Waits for a guest other than the running one. Returns 1 with an owned
/// Wasm buffer in `wasm`, 0 when nothing changed within the server's wait,
/// and -1 on failure, with the diagnostic in [`lk_dev_error`]. A returned
/// guest counts as running from then on.
///
/// # Safety
/// `development` is exclusively borrowed on its thread. `wasm` points to
/// writable aligned storage; release its buffer once after a return of 1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_dev_next(development: *mut Development, wasm: *mut Buffer) -> i32 {
	// SAFETY: The caller exclusively lends the live owner for this call.
	let Some(development) = (unsafe { development.as_mut() }) else {
		return -1;
	};

	if wasm.is_null() {
		development.error = "development guest output is null".into();
		return -1;
	}

	match development.guests.next_guest() {
		Ok(Some(module)) => {
			development.error.clear();
			// SAFETY: The caller provides writable output storage.
			unsafe { wasm.write(Buffer::new(module.into())) };
			1
		}
		Ok(None) => {
			development.error.clear();
			0
		}
		Err(error) => {
			development.error = error.to_string();
			-1
		}
	}
}

/// Copies the last failure diagnostic into an independently owned buffer.
///
/// # Safety
/// `development` is null or exclusively borrowed on its thread. Release the
/// returned buffer once with `lk_buffer_release`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_dev_error(development: *mut Development) -> Buffer {
	// SAFETY: The caller lends this owner only for this call.
	let message = unsafe { development.as_ref() }
		.map_or("invalid development guest owner", |development| {
			&development.error
		});
	Buffer::new(message.to_owned().into())
}

/// Destroys a development guest owner after its thread's last call.
///
/// # Safety
/// `development` is null or uniquely owns an unreleased `lk_dev_open`
/// result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_dev_destroy(development: *mut Development) {
	if !development.is_null() {
		// SAFETY: The caller transfers ownership of this exact allocation back.
		drop(unsafe { Box::from_raw(development) });
	}
}
