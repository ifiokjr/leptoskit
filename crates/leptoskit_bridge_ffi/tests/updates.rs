#![cfg(feature = "app")]
#![allow(unsafe_code)]

use std::fs;
use std::mem::MaybeUninit;
use std::ptr;
use std::slice;

use leptoskit_bridge_ffi::Buffer;
use leptoskit_bridge_ffi::app::App;
use leptoskit_bridge_ffi::lk_buffer_release;
use leptoskit_bridge_ffi::updates::*;
use leptoskit_codepush::bundle_directory;
use serde_json::Value;
use tempfile::TempDir;

fn take(buffer: Buffer) -> Vec<u8> {
	// SAFETY: Each owned output buffer is copied then released exactly once.
	unsafe {
		let bytes = slice::from_raw_parts(buffer.data, buffer.len).to_vec();
		lk_buffer_release(buffer);
		bytes
	}
}

fn fixture() -> (TempDir, Vec<u8>, Vec<u8>, String) {
	let temporary = TempDir::new().unwrap();
	let source = temporary.path().join("demo/release");
	let container = temporary.path().join("embedded.lkb");
	let directory = temporary
		.path()
		.join("installed")
		.to_str()
		.unwrap()
		.to_owned();
	let runtime = serde_json::json!({"abi": 1, "shell": "test-shell", "plugins": []});
	let configuration = serde_json::to_vec(&serde_json::json!({
		"enabled": false,
		"app": "demo",
		"channel": "production",
		"server": "https://updates.example.invalid",
		"public_keys": [],
		"runtime": runtime,
		"allow_http": false
	}))
	.unwrap();

	fs::create_dir_all(&source).unwrap();
	// A structurally valid module with no native exports: the updater must
	// preserve its receipt so the UI runtime can report construction failure.
	fs::write(source.join("app.wasm"), b"\0asm\x01\0\0\0").unwrap();
	fs::write(
		source.join("runtime.json"),
		serde_json::to_vec(&runtime).unwrap(),
	)
	.unwrap();
	bundle_directory(&source, &container).unwrap();
	let embedded = fs::read(container).unwrap();
	(temporary, configuration, embedded, directory)
}

#[test]
fn launch_transfers_independent_buffers_and_preserves_receipt_for_runtime_validation() {
	let (_temporary, configuration, embedded, directory) = fixture();
	let mut error = MaybeUninit::uninit();
	let mut wasm = MaybeUninit::uninit();
	let mut metadata = MaybeUninit::uninit();

	// SAFETY: All input/output storage remains live for calls; this test owns
	// the updater exclusively and releases each output allocation once.
	unsafe {
		let updates = lk_updates_open(
			configuration.as_ptr(),
			configuration.len(),
			embedded.as_ptr(),
			embedded.len(),
			directory.as_ptr(),
			directory.len(),
			error.as_mut_ptr(),
		);
		assert!(
			!updates.is_null(),
			"{}",
			String::from_utf8_lossy(&take(error.assume_init()))
		);
		assert_eq!(
			lk_updates_launch(updates, wasm.as_mut_ptr(), metadata.as_mut_ptr()),
			0
		);
		let metadata: Value = serde_json::from_slice(&take(metadata.assume_init())).unwrap();
		assert_eq!(metadata["version"], "0.1.0");
		assert_eq!(metadata["build"], 1);
		assert_eq!(metadata["candidate"], false);
		let receipt = metadata["receipt"].as_str().unwrap();
		assert!(!receipt.is_empty());

		let mut report = MaybeUninit::uninit();
		assert_eq!(lk_updates_sync(updates, report.as_mut_ptr()), 0);
		let report: Value = serde_json::from_slice(&take(report.assume_init())).unwrap();
		assert_eq!(report["status"], "current");
		assert_eq!(report["version"], "0.1.0");
		assert_eq!(lk_updates_confirm(updates, ptr::null(), 1), -1);
		assert!(
			String::from_utf8(take(lk_updates_error(updates)))
				.unwrap()
				.contains("null")
		);
		assert_eq!(
			lk_updates_confirm(updates, receipt.as_ptr(), receipt.len()),
			0
		);
		assert!(take(lk_updates_error(updates)).is_empty());
		lk_updates_destroy(updates);

		// Wasm remains owned after the updater is destroyed. The runtime,
		// rather than FFI launch, validates its import/export contract on the
		// UI thread.
		let wasm = take(wasm.assume_init());
		assert_eq!(wasm, b"\0asm\x01\0\0\0");
		assert!(App::new(&wasm).is_err());
	}
}

#[test]
fn null_owner_and_invalid_open_inputs_fail_with_owned_diagnostics() {
	let mut error = MaybeUninit::uninit();
	let mut output = MaybeUninit::uninit();

	// SAFETY: Null handles are accepted; invalid foreign slices fail before
	// dereference. Every diagnostic allocation is released once.
	unsafe {
		assert!(
			lk_updates_open(
				ptr::null(),
				1,
				ptr::null(),
				0,
				ptr::null(),
				0,
				error.as_mut_ptr()
			)
			.is_null()
		);
		assert!(
			String::from_utf8(take(error.assume_init()))
				.unwrap()
				.contains("null")
		);
		let mut error = MaybeUninit::uninit();
		assert!(
			lk_updates_open(
				ptr::null(),
				usize::MAX,
				ptr::null(),
				0,
				ptr::null(),
				0,
				error.as_mut_ptr()
			)
			.is_null()
		);
		assert!(
			String::from_utf8(take(error.assume_init()))
				.unwrap()
				.contains("exceeds")
		);
		assert!(
			lk_updates_open(
				ptr::null(),
				0,
				ptr::null(),
				0,
				ptr::null(),
				0,
				ptr::null_mut()
			)
			.is_null()
		);
		assert_eq!(
			lk_updates_launch(ptr::null_mut(), output.as_mut_ptr(), ptr::null_mut()),
			-1
		);
		assert_eq!(lk_updates_sync(ptr::null_mut(), output.as_mut_ptr()), -1);
		assert_eq!(lk_updates_confirm(ptr::null_mut(), ptr::null(), 0), -1);
		assert_eq!(lk_updates_reject(ptr::null_mut(), ptr::null(), 0), -1);
		assert_eq!(
			take(lk_updates_error(ptr::null_mut())),
			b"invalid update owner"
		);
		lk_updates_destroy(ptr::null_mut());
	}
}

#[test]
fn invalid_output_and_receipt_arguments_do_not_consume_the_owner() {
	let (_temporary, configuration, embedded, directory) = fixture();
	let mut error = MaybeUninit::uninit();
	let mut output = MaybeUninit::uninit();

	// SAFETY: The owner and output storage are exclusively borrowed, while
	// malformed/null input slices are rejected without dereference.
	unsafe {
		let updates = lk_updates_open(
			configuration.as_ptr(),
			configuration.len(),
			embedded.as_ptr(),
			embedded.len(),
			directory.as_ptr(),
			directory.len(),
			error.as_mut_ptr(),
		);
		assert!(
			!updates.is_null(),
			"{}",
			String::from_utf8_lossy(&take(error.assume_init()))
		);
		assert_eq!(
			lk_updates_launch(updates, output.as_mut_ptr(), output.as_mut_ptr()),
			-1
		);
		assert!(
			String::from_utf8(take(lk_updates_error(updates)))
				.unwrap()
				.contains("distinct")
		);
		assert_eq!(lk_updates_sync(updates, ptr::null_mut()), -1);
		assert_eq!(lk_updates_reject(updates, ptr::null(), usize::MAX), -1);
		assert!(
			String::from_utf8(take(lk_updates_error(updates)))
				.unwrap()
				.contains("exceeds")
		);
		assert_eq!(lk_updates_reject(updates, [255].as_ptr(), 1), -1);
		assert!(
			String::from_utf8(take(lk_updates_error(updates)))
				.unwrap()
				.contains("UTF-8")
		);
		assert_eq!(lk_updates_sync(updates, output.as_mut_ptr()), 0);
		take(output.assume_init());
		lk_updates_destroy(updates);
	}
}

#[test]
fn recovered_selector_diagnostic_is_visible_to_native_hosts() {
	let (_temporary, configuration, embedded, directory) = fixture();
	fs::create_dir_all(&directory).unwrap();
	fs::write(
		std::path::Path::new(&directory).join("state.json"),
		b"broken selector",
	)
	.unwrap();
	let mut error = MaybeUninit::uninit();
	let mut wasm = MaybeUninit::uninit();
	let mut metadata = MaybeUninit::uninit();

	// SAFETY: The updater and outputs are exclusively borrowed until destroy;
	// all output allocations are transferred back exactly once.
	unsafe {
		let updates = lk_updates_open(
			configuration.as_ptr(),
			configuration.len(),
			embedded.as_ptr(),
			embedded.len(),
			directory.as_ptr(),
			directory.len(),
			error.as_mut_ptr(),
		);
		assert!(
			!updates.is_null(),
			"{}",
			String::from_utf8_lossy(&take(error.assume_init()))
		);
		assert_eq!(
			lk_updates_launch(updates, wasm.as_mut_ptr(), metadata.as_mut_ptr()),
			0
		);
		let metadata: Value = serde_json::from_slice(&take(metadata.assume_init())).unwrap();
		assert!(metadata["recovery"].as_str().unwrap().contains("selector"));
		take(wasm.assume_init());
		lk_updates_destroy(updates);
	}
}
