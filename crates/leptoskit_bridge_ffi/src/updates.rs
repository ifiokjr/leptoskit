//! Owned bundle activation for native hosts, independent of UI runtime handles.
//!
//! Call every operation on one serialized worker. Blocking network and disk
//! access never require borrowing an [`App`](crate::app::App).

use std::path::Path;
use std::ptr;
use std::slice;
use std::str;

use leptoskit_codepush::installed::InstalledBundles;
use leptoskit_codepush::installed::UpdateConfiguration;

use crate::Buffer;

const MAX_CONFIGURATION_BYTES: usize = 1024 * 1024;
const MAX_EMBEDDED_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIRECTORY_BYTES: usize = 32 * 1024;
const MAX_RECEIPT_BYTES: usize = 4096;

/// One installed-bundle owner, confined to the host's serialized update worker.
pub struct Updates {
	bundles: InstalledBundles,
	error: String,
}

#[derive(serde::Serialize)]
struct LaunchEnvelope<'a> {
	#[serde(flatten)]
	metadata: &'a leptoskit_codepush::installed::LaunchMetadata,
	#[serde(skip_serializing_if = "Option::is_none")]
	recovery: Option<&'a str>,
}

impl Updates {
	fn record(&mut self, result: Result<(), String>) -> i32 {
		match result {
			Ok(()) => {
				self.error.clear();
				0
			}
			Err(error) => {
				self.error = error;
				-1
			}
		}
	}
}

/// Borrows bounded foreign input without accepting null nonempty slices.
unsafe fn input<'a>(data: *const u8, len: usize, maximum: usize) -> Result<&'a [u8], String> {
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

/// Opens installed bundles from a trusted store-shipped container and JSON
/// config. The returned owner is independent of every application/interpreter
/// handle.
/// An overlapping open for the same directory fails with an owned diagnostic;
/// destroy the previous owner before retrying.
///
/// # Safety
/// Input pointers address their declared lengths during this call. `error`
/// points to writable aligned storage; it receives an owned buffer on failure.
/// Use and destroy the returned pointer only on one serialized worker.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_updates_open(
	configuration: *const u8,
	configuration_len: usize,
	embedded: *const u8,
	embedded_len: usize,
	directory: *const u8,
	directory_len: usize,
	error: *mut Buffer,
) -> *mut Updates {
	if error.is_null() {
		return ptr::null_mut();
	}

	// SAFETY: All foreign borrows are bounded to this call by the ABI contract.
	let result = unsafe {
		(|| -> Result<Updates, String> {
			let configuration: UpdateConfiguration = serde_json::from_slice(input(
				configuration,
				configuration_len,
				MAX_CONFIGURATION_BYTES,
			)?)
			.map_err(|error| format!("update configuration: {error}"))?;
			let embedded = input(embedded, embedded_len, MAX_EMBEDDED_BYTES)?;
			let directory = str::from_utf8(input(directory, directory_len, MAX_DIRECTORY_BYTES)?)
				.map_err(|error| format!("update directory must be UTF-8: {error}"))?;

			if directory.is_empty() {
				return Err("update directory is empty".into());
			}

			let bundles = InstalledBundles::open(Path::new(directory), embedded, configuration)
				.map_err(|error| error.to_string())?;
			Ok(Updates {
				bundles,
				error: String::new(),
			})
		})()
	};

	match result {
		Ok(updates) => Box::into_raw(Box::new(updates)),
		Err(message) => {
			// SAFETY: `error` is writable output storage supplied by the
			// caller.
			unsafe { error.write(Buffer::new(message.into())) };
			ptr::null_mut()
		}
	}
}

/// Destroys an update owner after its worker has completed all calls.
///
/// # Safety
/// `updates` is null or uniquely owns an unreleased `lk_updates_open` result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_updates_destroy(updates: *mut Updates) {
	if !updates.is_null() {
		// SAFETY: The caller transfers ownership of this exact allocation back.
		drop(unsafe { Box::from_raw(updates) });
	}
}

/// Selects Wasm and launch metadata; returns 0 on success and -1 on failure.
/// A candidate's trial marker is durable before success. Metadata contains
/// `receipt`, `version`, `build`, `candidate`, and an optional `recovery`
/// diagnostic. If application construction fails, reject this receipt and
/// launch again to recover the working bundle.
///
/// # Safety
/// `updates` is exclusive on its serialized worker. Outputs point to distinct
/// writable aligned storage and receive independently owned buffers on success.
/// Release each buffer once, even after destroying the update owner.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_updates_launch(
	updates: *mut Updates,
	wasm: *mut Buffer,
	metadata: *mut Buffer,
) -> i32 {
	// SAFETY: The caller exclusively lends the live owner for this call.
	let Some(updates) = (unsafe { updates.as_mut() }) else {
		return -1;
	};

	if wasm.is_null() || metadata.is_null() || wasm == metadata {
		return updates.record(Err("launch outputs must be nonnull and distinct".into()));
	}

	let result = updates
		.bundles
		.launch()
		.map_err(|error| error.to_string())
		.and_then(|launch| {
			let envelope = LaunchEnvelope {
				metadata: &launch.metadata,
				recovery: updates.bundles.recovery_diagnostic(),
			};
			let json = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;

			// SAFETY: Both outputs are valid distinct caller-owned storage.
			unsafe {
				wasm.write(Buffer::new(launch.wasm.into()));
				metadata.write(Buffer::new(json.into()));
			}

			Ok(())
		});
	updates.record(result)
}

/// Downloads, verifies, and stages an update without activating it.
/// Returns 0 and owned JSON `{status,version,build}` on success; -1 on failure.
/// This performs blocking HTTP and filesystem work on the serialized worker.
///
/// # Safety
/// `updates` is exclusively borrowed. `metadata` points to writable aligned
/// output storage; release its resulting buffer once after success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_updates_sync(updates: *mut Updates, metadata: *mut Buffer) -> i32 {
	// SAFETY: The owner is exclusively lent for this operation.
	let Some(updates) = (unsafe { updates.as_mut() }) else {
		return -1;
	};

	if metadata.is_null() {
		return updates.record(Err("update report output is null".into()));
	}

	let result = updates
		.bundles
		.sync()
		.map_err(|error| error.to_string())
		.and_then(|report| {
			let json = serde_json::to_vec(&report).map_err(|error| error.to_string())?;
			// SAFETY: The caller provides writable output storage.
			unsafe { metadata.write(Buffer::new(json.into())) };
			Ok(())
		});
	updates.record(result)
}

/// Confirms the launch whose native controls completed their first frame.
/// Returns 0 on success, -1 on failure; stale receipts fail explicitly.
///
/// # Safety
/// `updates` is exclusively borrowed on its worker. `receipt` addresses `len`
/// bytes of UTF-8 returned in launch metadata, borrowed only during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_updates_confirm(
	updates: *mut Updates,
	receipt: *const u8,
	len: usize,
) -> i32 {
	// SAFETY: Forwarded borrows obey this entry point's identical contract.
	unsafe { finish(updates, receipt, len, true) }
}

/// Rejects a failed launch; call launch again to obtain the working fallback.
/// Returns 0 on success and -1 on failure.
///
/// # Safety
/// `updates` is exclusively borrowed on its worker. `receipt` addresses `len`
/// bytes of UTF-8 returned in launch metadata, borrowed only during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_updates_reject(
	updates: *mut Updates,
	receipt: *const u8,
	len: usize,
) -> i32 {
	// SAFETY: Forwarded borrows obey this entry point's identical contract.
	unsafe { finish(updates, receipt, len, false) }
}

unsafe fn finish(updates: *mut Updates, receipt: *const u8, len: usize, confirm: bool) -> i32 {
	// SAFETY: The entry point caller lends exclusive owner access.
	let Some(updates) = (unsafe { updates.as_mut() }) else {
		return -1;
	};
	let result = (|| {
		// SAFETY: The entry point caller lends readable receipt bytes.
		let receipt = str::from_utf8(unsafe { input(receipt, len, MAX_RECEIPT_BYTES) }?)
			.map_err(|error| format!("launch receipt must be UTF-8: {error}"))?;
		let result = if confirm {
			updates.bundles.confirm(receipt)
		} else {
			updates.bundles.reject(receipt)
		};
		result.map_err(|error| error.to_string())
	})();
	updates.record(result)
}

/// Copies the last failure diagnostic into an independently owned buffer.
///
/// # Safety
/// `updates` is null or exclusively borrowed on its serialized worker. Release
/// the returned buffer once with `lk_buffer_release`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_updates_error(updates: *mut Updates) -> Buffer {
	// SAFETY: The caller lends this owner only for this call.
	let message =
		unsafe { updates.as_ref() }.map_or("invalid update owner", |updates| &updates.error);
	Buffer::new(message.to_owned().into())
}

/// Copies the verified entry module out of a trusted store-shipped container.
/// Hosts call it when updates are disabled or the update store cannot open,
/// so an app ships its guest only once, inside the container. Returns 0 and
/// an owned Wasm buffer, or -1 and an owned diagnostic. Any thread may call it.
///
/// # Safety
/// `embedded` addresses `len` readable bytes during this call. `wasm` and
/// `error` point to distinct writable aligned storage; exactly one of them
/// receives a buffer, which the caller releases once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_bundle_entry(
	embedded: *const u8,
	len: usize,
	wasm: *mut Buffer,
	error: *mut Buffer,
) -> i32 {
	if wasm.is_null() || error.is_null() || wasm == error {
		return -1;
	}

	// SAFETY: The caller lends readable container bytes for this call.
	let result = unsafe { input(embedded, len, MAX_EMBEDDED_BYTES) }.and_then(|container| {
		leptoskit_codepush::installed::entry_module(container).map_err(|error| error.to_string())
	});

	match result {
		Ok(module) => {
			// SAFETY: `wasm` is writable output storage supplied by the caller.
			unsafe { wasm.write(Buffer::new(module.into())) };
			0
		}
		Err(message) => {
			// SAFETY: `error` is writable output storage supplied by the
			// caller.
			unsafe { error.write(Buffer::new(message.into())) };
			-1
		}
	}
}
