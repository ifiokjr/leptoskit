#![cfg(feature = "app")]
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_bridge::Error;
use leptoskit_bridge_ffi::Buffer;
#[cfg(target_vendor = "apple")]
use leptoskit_bridge_ffi::app::AppleOwnerKind;
use leptoskit_bridge_ffi::app::NativeAppContext;
use leptoskit_bridge_ffi::app::PlatformContext;
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
		data: ptr::null(),
		len: 0,
		owner: ptr::null_mut(),
	}
}

#[test]
fn validates_host_directories_before_installing_plugins() {
	let directory = std::env::temp_dir();
	let path = directory.to_str().unwrap().as_bytes();
	let context = NativeAppContext::from_dirs(path.as_ptr(), path.len(), path.as_ptr(), path.len());
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
				assert!(context.platform().is_none());
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
		let context =
			NativeAppContext::from_dirs(path.as_ptr(), path.len(), path.as_ptr(), path.len());
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
	let context = NativeAppContext::from_dirs(path.as_ptr(), path.len(), path.as_ptr(), path.len());
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
			ptr::null(),
			&raw mut error,
			|_, _| panic!("null context reached registration"),
		)
	};
	assert!(app.is_null());
	// SAFETY: Error allocation is uniquely owned.
	unsafe { leptoskit_bridge_ffi::lk_buffer_release(error) };
}

static RELEASES: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn record_release(vm: *mut c_void, context: *mut c_void) {
	assert_eq!(vm.addr(), 0x1111);
	assert_eq!(context.addr(), 0x2222);
	RELEASES.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn platform_context_is_retained_across_plugin_clones_and_released_on_failure() {
	RELEASES.store(0, Ordering::SeqCst);
	let directory = std::env::temp_dir();
	let path = directory.to_str().unwrap().as_bytes();
	let vm = ptr::without_provenance_mut(0x1111);
	let host_ctx = ptr::without_provenance_mut(0x2222);

	// 1. Invalid path still releases the transferred platform handle.
	let bad = NativeAppContext {
		data_dir: b"relative".as_ptr(),
		data_dir_len: b"relative".len(),
		cache_dir: path.as_ptr(),
		cache_dir_len: path.len(),
		platform_vm: vm,
		platform_context: host_ctx,
		release_platform_context: Some(record_release),
	};
	let mut error = empty_buffer();
	// SAFETY: Pointers are valid for this call and release records cleanup.
	let app = unsafe {
		create_with_context(
			WASM.as_ptr(),
			WASM.len(),
			&raw const bad,
			&raw mut error,
			|_, _| unreachable!(),
		)
	};
	assert!(app.is_null());
	assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
	// SAFETY: Error allocation is uniquely owned.
	unsafe { leptoskit_bridge_ffi::lk_buffer_release(error) };

	// 2. Factory failure still releases the transferred platform handle.
	let valid = NativeAppContext {
		data_dir: path.as_ptr(),
		data_dir_len: path.len(),
		cache_dir: path.as_ptr(),
		cache_dir_len: path.len(),
		platform_vm: vm,
		platform_context: host_ctx,
		release_platform_context: Some(record_release),
	};
	let mut error = empty_buffer();
	// SAFETY: Pointers are valid for this call and release records cleanup.
	let app = unsafe {
		create_with_context(
			WASM.as_ptr(),
			WASM.len(),
			&raw const valid,
			&raw mut error,
			|_, _| Err(Error::Native),
		)
	};
	assert!(app.is_null());
	assert_eq!(RELEASES.load(Ordering::SeqCst), 2);
	// SAFETY: Error allocation is uniquely owned.
	unsafe { leptoskit_bridge_ffi::lk_buffer_release(error) };

	// 3. A plugin clone keeps the handle alive after the App drops.
	let mut retained_clone = None;
	let mut error = empty_buffer();
	// SAFETY: Pointers are valid for this call.
	let app = unsafe {
		create_with_context(
			WASM.as_ptr(),
			WASM.len(),
			&raw const valid,
			&raw mut error,
			|_, context| {
				assert_eq!(
					context
						.platform()
						.and_then(PlatformContext::android_handles),
					Some((vm, host_ctx))
				);
				retained_clone = context.platform().cloned();
				Ok(())
			},
		)
	};
	assert!(!app.is_null());
	assert_eq!(RELEASES.load(Ordering::SeqCst), 2);

	// Dropping the App while the plugin clone is still alive does not release.
	// SAFETY: The create call returned the unique app handle.
	unsafe { leptoskit_bridge_ffi::app::lk_app_destroy(app) };
	assert_eq!(RELEASES.load(Ordering::SeqCst), 2);

	drop(retained_clone.take());
	assert_eq!(RELEASES.load(Ordering::SeqCst), 3);
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" fn fake_apple_resolver(context: *mut c_void, kind: i32) -> *mut c_void {
	match kind {
		0 => context,
		1 => ptr::without_provenance_mut(context.addr() + 1),
		_ => ptr::null_mut(),
	}
}

#[cfg(target_vendor = "apple")]
#[test]
fn apple_owner_resolver_dispatches_attached_and_presentation_queries() {
	let directory = std::env::temp_dir();
	let path = directory.to_str().unwrap().as_bytes();
	let owner = ptr::without_provenance_mut(0x4000);
	let resolver: leptoskit_bridge_ffi::app::AppleOwnerResolver = fake_apple_resolver;
	let context = NativeAppContext {
		data_dir: path.as_ptr(),
		data_dir_len: path.len(),
		cache_dir: path.as_ptr(),
		cache_dir_len: path.len(),
		platform_vm: resolver as *mut c_void,
		platform_context: owner,
		release_platform_context: None,
	};
	let mut error = empty_buffer();
	// SAFETY: Pointers and resolver function are valid for this call.
	let app = unsafe {
		create_with_context(
			WASM.as_ptr(),
			WASM.len(),
			&raw const context,
			&raw mut error,
			|_, context| {
				let platform = context.platform().unwrap();
				assert_eq!(platform.apple_attached_owner(), Some(owner));
				assert_eq!(
					platform.apple_presentation_owner(),
					Some(ptr::without_provenance_mut(0x4001))
				);
				assert_eq!(platform.apple_owner(AppleOwnerKind::Attached), Some(owner));
				Ok(())
			},
		)
	};
	assert!(!app.is_null());
	// SAFETY: The create call returned the unique app handle.
	unsafe { leptoskit_bridge_ffi::app::lk_app_destroy(app) };
}
