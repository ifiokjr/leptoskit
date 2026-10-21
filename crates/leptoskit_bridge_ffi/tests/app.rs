#![cfg(feature = "app")]
#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::ptr;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge_ffi::Buffer;
use leptoskit_bridge_ffi::NativeRequest;
use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::lk_buffer_release;

const GUEST: &str = r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "commit" (func $commit))
 (import "leptoskit_plugins_v1" "submit" (func $submit (param i32 i32 i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "take_result" (func $take (param i64 i32 i32) (result i64)))
 (memory (export "memory") 1)
 (global $call (mut i64) (i64.const 0))
 (func (export "leptoskit_main")
   (call $create (i64.const 1) (i32.const 0)) (call $commit)
   (global.set $call (call $submit (i32.const 0) (i32.const 7) (i32.const 0) (i32.const 0))))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
 (func (export "leptoskit_poll")
   (if (i64.ne (call $take (global.get $call) (i32.const 0) (i32.const 0)) (i64.const 0))
     (then unreachable)))
)"#;

struct Echo;
impl Plugin for Echo {
	fn name(&self) -> &'static str {
		"echo"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(7)]
	}

	fn invoke(&mut self, _: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		Ok(payload)
	}
}

fn take(buffer: Buffer) -> Vec<u8> {
	// SAFETY: The FFI returned this live owned buffer, released exactly once.
	unsafe {
		let bytes = std::slice::from_raw_parts(buffer.data, buffer.len).to_vec();
		lk_buffer_release(buffer);
		bytes
	}
}

#[test]
fn generated_runtime_starts_renders_and_polls_rust_plugin_completions() {
	let mut diagnostic = MaybeUninit::uninit();
	// SAFETY: All app accesses are exclusive and the input lives for the call.
	unsafe {
		let app = create(
			GUEST.as_ptr(),
			GUEST.len(),
			diagnostic.as_mut_ptr(),
			|app| app.install(Box::new(Echo)).map(|_| ()),
		);
		assert!(!app.is_null());
		assert_eq!(lk_app_resize(app, 100.0, 100.0, false), -1);
		assert!(!take(lk_app_error(app)).is_empty());
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);
		assert!(take(lk_app_error(app)).is_empty());
		let frame: serde_json::Value = serde_json::from_slice(&take(lk_app_render(app))).unwrap();
		assert!(frame["root"].is_number());
		assert!(!frame["mutations"].as_array().unwrap().is_empty());
		let create = frame["mutations"]
			.as_array()
			.unwrap()
			.iter()
			.find_map(|mutation| mutation.get("Create"))
			.unwrap();
		assert_eq!(
			create["kind"],
			serde_json::to_value(leptoskit_native::NativeKind::View).unwrap()
		);
		let empty: serde_json::Value = serde_json::from_slice(&take(lk_app_render(app))).unwrap();
		assert!(empty["mutations"].as_array().unwrap().is_empty());
		let mut request = MaybeUninit::<NativeRequest>::uninit();
		assert_eq!(
			lk_app_next_request(app, request.as_mut_ptr()),
			0,
			"{}",
			String::from_utf8_lossy(&take(lk_app_error(app)))
		);
		assert_eq!((*lk_app_plugins(app)).buffered_bytes(), 0);
		assert_eq!(lk_app_event(app, usize::MAX, 0, ptr::null(), 0), 0);
		assert_eq!(lk_app_start(app, 100.0, 100.0), -1);
		lk_app_destroy(app);
	}
}

#[test]
fn invalid_bundle_and_registration_failure_return_owned_diagnostics() {
	// SAFETY: Inputs are borrowed and diagnostics are released after each call.
	unsafe {
		let mut error = MaybeUninit::uninit();
		assert!(create(ptr::null(), 0, error.as_mut_ptr(), |_| Ok(())).is_null());
		assert_eq!(take(error.assume_init()), b"empty application bundle");
		let mut error = MaybeUninit::uninit();
		assert!(
			create(GUEST.as_ptr(), GUEST.len(), error.as_mut_ptr(), |_| {
				Err(Error::Unavailable)
			})
			.is_null()
		);
		assert!(
			String::from_utf8(take(error.assume_init()))
				.unwrap()
				.contains("register plugin")
		);
	}
}
