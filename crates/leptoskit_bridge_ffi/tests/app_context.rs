#![cfg(feature = "app")]
#![allow(unsafe_code)]

use leptoskit_bridge::Error;
use leptoskit_bridge_ffi::Buffer;
use leptoskit_bridge_ffi::app::NativeAppContext;
use leptoskit_bridge_ffi::app::create_with_context;

// The minimal module is enough to load a host; these tests do not start a
// guest.
const WASM: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
)"#;

fn empty_buffer() -> Buffer {
	Buffer {
		data: std::ptr::null(),
		len: 0,
		owner: std::ptr::null_mut(),
	}
}

#[test]
fn validates_host_directories_before_installing_plugins() {
	let directory = std::env::temp_dir();
	let path = directory.to_str().unwrap().as_bytes();
	let context = NativeAppContext {
		data_dir: path.as_ptr(),
		data_dir_len: path.len(),
		cache_dir: path.as_ptr(),
		cache_dir_len: path.len(),
	};
	let mut error = empty_buffer();
	let mut called = false;
	// SAFETY: Paths/module/output remain alive and exclusively borrowed.
	let app = unsafe {
		create_with_context(
			WASM.as_ptr(),
			WASM.len(),
			&raw const context,
			&raw mut error,
			|_, context| {
				called = true;
				assert_eq!(context.data_dir, directory);
				assert_eq!(context.cache_dir, directory);
				Ok(())
			},
		)
	};
	assert!(called);
	assert!(!app.is_null());

	// SAFETY: The create call returned the unique app handle.
	unsafe { leptoskit_bridge_ffi::app::lk_app_destroy(app) };
}

#[test]
fn rejects_invalid_context_without_invoking_registration() {
	for path in [
		"relative",
		"\0",
		"",
		"/leptoskit-nonexistent-test-directory",
	] {
		let context = NativeAppContext {
			data_dir: path.as_ptr(),
			data_dir_len: path.len(),
			cache_dir: path.as_ptr(),
			cache_dir_len: path.len(),
		};
		let mut error = empty_buffer();
		// SAFETY: Paths/module/output remain alive for this call.
		let app = unsafe {
			create_with_context(
				WASM.as_ptr(),
				WASM.len(),
				&raw const context,
				&raw mut error,
				|_, _| panic!("invalid directory reached registration"),
			)
		};
		assert!(app.is_null());
		assert!(error.len > 0);
		// SAFETY: Error allocation is uniquely owned.
		unsafe { leptoskit_bridge_ffi::lk_buffer_release(error) };
	}
}

#[test]
fn factory_failure_is_reported_and_null_context_rejected() {
	let directory = std::env::temp_dir();
	let path = directory.to_str().unwrap().as_bytes();
	let context = NativeAppContext {
		data_dir: path.as_ptr(),
		data_dir_len: path.len(),
		cache_dir: path.as_ptr(),
		cache_dir_len: path.len(),
	};
	let mut error = empty_buffer();
	// SAFETY: Context/module/output remain readable/writable for this call.
	let app = unsafe {
		create_with_context(
			WASM.as_ptr(),
			WASM.len(),
			&raw const context,
			&raw mut error,
			|_, _| Err(Error::PermissionDenied),
		)
	};
	assert!(app.is_null());
	// SAFETY: Error allocation is uniquely owned.
	unsafe { leptoskit_bridge_ffi::lk_buffer_release(error) };
	let mut error = empty_buffer();

	// SAFETY: Null context is explicitly checked; module/output are valid.
	let app = unsafe {
		create_with_context(
			WASM.as_ptr(),
			WASM.len(),
			std::ptr::null(),
			&raw mut error,
			|_, _| panic!("null context reached registration"),
		)
	};
	assert!(app.is_null());
	// SAFETY: Error allocation is uniquely owned.
	unsafe { leptoskit_bridge_ffi::lk_buffer_release(error) };
}
