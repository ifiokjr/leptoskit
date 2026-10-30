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
 (import "leptoskit_plugins_v1" "resolve" (func $resolve (param i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "submit" (func $submit (param i32 i32 i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "take_result" (func $take (param i64 i32 i32) (result i64)))
 (memory (export "memory") 1)
 (data (i32.const 16) "echo")
 (global $call (mut i64) (i64.const 0))
 (func (export "leptoskit_main")
   (call $create (i64.const 1) (i32.const 0)) (call $commit)
   (global.set $call
     (call $submit (i32.wrap_i64 (call $resolve (i32.const 16) (i32.const 4))) (i32.const 7) (i32.const 0) (i32.const 0))))
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
		assert_ne!(take(lk_app_error(app)), [] as [u8; 0]);
		assert_eq!(
			lk_app_render_pending(app),
			0,
			"nothing to send before start"
		);
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);
		assert_eq!(take(lk_app_error(app)), [] as [u8; 0]);
		assert_eq!(
			lk_app_render_pending(app),
			1,
			"the first commit and its root"
		);
		let frame: serde_json::Value = serde_json::from_slice(&take(lk_app_render(app))).unwrap();
		assert!(frame["root"].is_number());
		assert_ne!(frame["mutations"], serde_json::json!([]));
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
		assert_eq!(
			lk_app_render_pending(app),
			0,
			"the root and its commit were sent"
		);
		let empty: serde_json::Value = serde_json::from_slice(&take(lk_app_render(app))).unwrap();
		assert_eq!(empty["mutations"], serde_json::json!([]));
		assert_eq!(lk_app_render_pending(ptr::null_mut()), -1);
		let mut request = MaybeUninit::<NativeRequest>::uninit();
		assert_eq!(
			lk_app_next_request(app, request.as_mut_ptr()),
			0,
			"{}",
			String::from_utf8_lossy(&take(lk_app_error(app)))
		);
		assert_eq!((*lk_app_plugins(app)).buffered_bytes(), 0);
		assert_eq!(lk_app_event(app, usize::MAX, 0, ptr::null(), 0), 0);
		let scroll = br#"{"x":0,"y":120.5}"#;
		assert_eq!(
			lk_app_event(app, usize::MAX, 5, scroll.as_ptr(), scroll.len()),
			0,
			"a well-formed scroll for an unknown node is ignored"
		);
		for malformed in [&b"{\"x\":0}"[..], b"{\"x\":0,\"y\":\"far\"}", b"120"] {
			assert_eq!(
				lk_app_event(app, usize::MAX, 5, malformed.as_ptr(), malformed.len()),
				-1
			);
			assert!(
				String::from_utf8(take(lk_app_error(app)))
					.unwrap()
					.contains("scroll event")
			);
		}
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

#[test]
fn a_mutation_budget_failure_never_serializes_a_partial_transaction() {
	let mut app = App::new(GUEST.as_bytes()).unwrap();
	app.with_host(|host| {
		host.update_backend(|backend| {
			*backend = leptoskit_native::HeadlessBackend::new().with_log_limit(100);
		});
	});
	// SAFETY: The app is exclusively borrowed and remains live throughout.
	unsafe {
		// Install the plugin the initial guest submits to.
		app.install(Box::new(Echo)).unwrap();
		let app = &raw mut app;
		assert_eq!(lk_app_start(app, 100.0, 100.0), -1);
		assert_eq!(
			lk_app_render_pending(app),
			1,
			"the failure must reach the platform"
		);
		assert_eq!(take(lk_app_render(app)), b"null");
		assert_eq!(
			lk_app_render_pending(app),
			1,
			"every later render reports it too"
		);
		assert!(
			String::from_utf8(take(lk_app_error(app)))
				.unwrap()
				.contains("render mutation limit")
		);
		assert_eq!(lk_app_wake(app), -1);
	}
}

/// Waits for a display frame on start, and again each time one arrives,
/// counting frames in memory at address 0 and committing a node per frame.
const FRAME_GUEST: &str = r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "commit" (func $commit))
 (import "leptoskit_plugins_v1" "resolve" (func $resolve (param i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "submit" (func $submit (param i32 i32 i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "take_result" (func $take (param i64 i32 i32) (result i64)))
 (memory (export "memory") 1)
 (data (i32.const 64) "leptoskit.frame")
 (global $call (mut i64) (i64.const 0))
 (func $request
   (global.set $call
     (call $submit (i32.wrap_i64 (call $resolve (i32.const 64) (i32.const 15))) (i32.const 0) (i32.const 0) (i32.const 0))))
 (func (export "leptoskit_main")
   (call $create (i64.const 1) (i32.const 0)) (call $commit)
   (call $request))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
 (func (export "leptoskit_poll")
   ;; A completed frame call returns its 16-byte payload length.
   (if (i64.eq (call $take (global.get $call) (i32.const 128) (i32.const 16)) (i64.const 16))
     (then
       (i32.store (i32.const 0) (i32.add (i32.load (i32.const 0)) (i32.const 1)))
       (call $request))))
)"#;

#[test]
fn display_frames_reach_waiting_guests_and_never_reach_the_platform() {
	let mut diagnostic = MaybeUninit::uninit();
	// SAFETY: All app accesses are exclusive and the input lives for the call.
	unsafe {
		let app = create(
			FRAME_GUEST.as_ptr(),
			FRAME_GUEST.len(),
			diagnostic.as_mut_ptr(),
			|_| Ok(()),
		);
		assert!(!app.is_null());
		assert_eq!(lk_app_enable_frames(app), 0);
		assert_eq!(
			lk_app_enable_frames(app),
			0,
			"enabling again keeps the clock"
		);
		assert_eq!(
			lk_app_frame_requested(app),
			0,
			"nothing waits before the guest runs"
		);
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);

		// The guest's frame request stays in the host: the platform sees no
		// request, only that a frame is wanted.
		let mut request = MaybeUninit::<NativeRequest>::uninit();
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 0);
		assert_eq!(lk_app_frame_requested(app), 1);

		// Each frame resumes the guest, which asks for the next one only after
		// reading a complete frame: still wanting one proves delivery.
		for frame in 1..=3_u32 {
			assert_eq!(
				lk_app_frame(app, u64::from(frame) * 16_666_667, 16_666_667),
				0,
				"{}",
				String::from_utf8_lossy(&take(lk_app_error(app)))
			);
			assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 0);
			assert_eq!(lk_app_frame_requested(app), 1);
		}

		// A frame request cancelled before its frame is dropped by the host:
		// the platform is never told about a call it never saw.
		let cancelled = (*app).with_host(|host| {
			let clock = host
				.plugins()
				.resolve(leptoskit_bridge::frame::FRAME_PLUGIN)
				.unwrap();
			host.plugins()
				.submit(clock, leptoskit_bridge::frame::NEXT_FRAME, Bytes::new())
				.unwrap()
		});
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 0);
		(*app)
			.with_host(|host| host.plugins().cancel(cancelled))
			.unwrap();
		assert_eq!((*app).next_cancelled(), None);
		assert_eq!(lk_app_frame(app, 4 * 16_666_667, 16_666_667), 0);
		lk_app_destroy(app);
	}
}

#[test]
fn a_host_without_a_display_link_never_leaves_a_guest_waiting() {
	let mut diagnostic = MaybeUninit::uninit();
	// SAFETY: All app accesses are exclusive and the input lives for the call.
	unsafe {
		let app = create(
			FRAME_GUEST.as_ptr(),
			FRAME_GUEST.len(),
			diagnostic.as_mut_ptr(),
			|_| Ok(()),
		);
		assert!(!app.is_null());
		// No `lk_app_enable_frames`: the reserved plugin does not resolve, so
		// the guest's request fails at once rather than waiting on a frame
		// this host will never deliver.
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);
		let mut request = MaybeUninit::<NativeRequest>::uninit();
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 0);
		assert_eq!(lk_app_frame_requested(app), 0);
		assert!(
			(*app)
				.with_host(|host| {
					host.plugins()
						.resolve(leptoskit_bridge::frame::FRAME_PLUGIN)
				})
				.is_none()
		);
		// A display link that ticks anyway delivers nothing and fails nothing.
		assert_eq!(lk_app_frame(app, 16_666_667, 16_666_667), 0);
		lk_app_destroy(app);
	}
}

#[test]
fn the_frame_clock_rejects_a_missing_app() {
	// SAFETY: Each function checks for a null app before using it.
	unsafe {
		assert_eq!(lk_app_enable_frames(ptr::null_mut()), -1);
		assert_eq!(lk_app_frame_requested(ptr::null_mut()), -1);
		assert_eq!(lk_app_frame(ptr::null_mut(), 0, 16_666_667), -1);
	}
}
