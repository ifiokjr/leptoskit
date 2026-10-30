#![cfg(feature = "app")]
#![allow(unsafe_code)]

//! The guest thread runs a real guest: it starts it, answers its frames,
//! events, and plugin calls, posts transactions and requests for the UI
//! thread, and gives the app back when stopped.

use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge_ffi::Buffer;
use leptoskit_bridge_ffi::NativeRequest;
use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::lk_buffer_release;

/// Renders a view at start and asks for display frames. Adds a label for
/// each frame, for each of its 70 calls to the platform's `native` plugin
/// when it is answered, and for each event. Labels always get a native view;
/// an empty `View` would be flattened away.
const GUEST: &str = r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
 (import "leptoskit" "commit" (func $commit))
 (import "leptoskit_plugins_v1" "resolve" (func $resolve (param i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "submit" (func $submit (param i32 i32 i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "take_result" (func $take (param i64 i32 i32) (result i64)))
 (memory (export "memory") 1)
 (data (i32.const 16) "leptoskit.frame")
 (data (i32.const 48) "native")
 (global $frame (mut i64) (i64.const -1))
 (global $next (mut i64) (i64.const 2))
 (global $first (mut i64) (i64.const 0))
 (global $calls (mut i32) (i32.const 0))
 (func $node
   (call $create (global.get $next) (i32.const 1))
   (call $insert (i64.const 1) (global.get $next) (i32.const 0))
   (global.set $next (i64.add (global.get $next) (i64.const 1)))
   (call $commit))
 (func $request_frame
   (global.set $frame
     (call $submit (i32.wrap_i64 (call $resolve (i32.const 16) (i32.const 15))) (i32.const 0) (i32.const 0) (i32.const 0))))
 (func (export "leptoskit_main")
   (local $i i32)
   (call $create (i64.const 1) (i32.const 0))
   (call $commit)
   (call $request_frame)
   (loop $submit
     (global.set $first
       (call $submit (i32.wrap_i64 (call $resolve (i32.const 48) (i32.const 6))) (i32.const 1) (i32.const 0) (i32.const 0)))
     (local.set $i (i32.add (local.get $i) (i32.const 1)))
     (br_if $submit (i32.lt_u (local.get $i) (i32.const 70)))))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
   (call $node)
   (i32.const 1))
 (func (export "leptoskit_plugins_ready"))
 (func (export "leptoskit_poll")
   (local $id i64)
   (if (i64.ge_s (call $take (global.get $frame) (i32.const 1024) (i32.const 64)) (i64.const 0))
     (then (call $node) (call $request_frame)))
   ;; Calls were numbered from the last one down; take every reply.
   (local.set $id (i64.sub (global.get $first) (i64.const 69)))
   (loop $replies
     (if (i64.ge_s (call $take (local.get $id) (i32.const 1024) (i32.const 64)) (i64.const 0))
       (then (call $node)))
     (local.set $id (i64.add (local.get $id) (i64.const 1)))
     (br_if $replies (i64.le_s (local.get $id) (global.get $first)))))
)"#;

/// Counts notifications; the test owns it until `release`.
struct Notifications(AtomicUsize);

unsafe extern "C" fn notify(context: *mut c_void) {
	// SAFETY: The context is the live `Notifications` the test passed.
	unsafe { &*context.cast::<Notifications>() }
		.0
		.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn release(context: *mut c_void) {
	// SAFETY: The guest returns the allocation the test leaked, once.
	drop(unsafe { Box::from_raw(context.cast::<Notifications>()) });
}

fn take(buffer: Buffer) -> Vec<u8> {
	// SAFETY: The FFI returned this live owned buffer, released exactly once.
	unsafe {
		let bytes = std::slice::from_raw_parts(buffer.data, buffer.len).to_vec();
		lk_buffer_release(buffer);
		bytes
	}
}

/// Polls `check` until it returns `Some`, as the UI thread would on each
/// notification, failing after five seconds.
fn wait<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
	let deadline = Instant::now() + Duration::from_secs(5);

	loop {
		if let Some(value) = check() {
			return value;
		}

		assert!(Instant::now() < deadline, "timed out waiting for {what}");
		std::thread::sleep(Duration::from_millis(1));
	}
}

/// The next transaction: its root and its mutation count.
unsafe fn transaction(guest: *const Guest) -> Option<(i64, u32)> {
	// SAFETY: The test lends its live guest.
	let bytes = take(unsafe { lk_guest_take_render(guest) });

	(!bytes.is_empty()).then(|| {
		assert_eq!(&bytes[..4], b"LKB1");
		(
			i64::from_le_bytes(bytes[4..12].try_into().unwrap()),
			u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
		)
	})
}

/// Every platform request waiting now.
unsafe fn requests(guest: *const Guest) -> Vec<u64> {
	let mut ids = Vec::new();

	loop {
		let mut request = MaybeUninit::<NativeRequest>::uninit();
		let mut worker = ptr::null_mut();
		let mut job = ptr::null_mut();
		// SAFETY: The test lends its guest and writable outputs.
		let status = unsafe {
			lk_guest_next_request(guest, request.as_mut_ptr(), &raw mut worker, &raw mut job)
		};

		if status == 0 {
			return ids;
		}

		assert_eq!(status, 1);
		assert!(worker.is_null() && job.is_null(), "a platform plugin call");
		// SAFETY: A taken request was written; its payload is released once.
		let request = unsafe { request.assume_init() };
		assert_eq!(request.method, 1);
		ids.push(request.id);
		// SAFETY: As above.
		unsafe { lk_buffer_release(request.payload) };
	}
}

fn app() -> *mut App {
	let mut diagnostic = MaybeUninit::uninit();
	// SAFETY: The input outlives the call and the output is writable.
	let app = unsafe {
		create(
			GUEST.as_ptr(),
			GUEST.len(),
			diagnostic.as_mut_ptr(),
			|app| {
				app.with_host(|host| {
					let bridge = host.plugins();
					let id = bridge.register("native", [MethodId(1)])?;
					bridge.grant(id, MethodId(1), true)
				})
			},
		)
	};
	assert!(!app.is_null());
	// SAFETY: The app is live and owned here.
	assert_eq!(unsafe { lk_app_enable_frames(app) }, 0);
	app
}

fn spawn(app: *mut App) -> (*mut Guest, &'static Notifications) {
	let notifications = Box::into_raw(Box::new(Notifications(AtomicUsize::new(0))));
	// SAFETY: The app is lent until the guest stops; the context lives until
	// `release`.
	let guest = unsafe {
		lk_guest_spawn(
			app,
			GuestCallbacks {
				context: notifications.cast(),
				notify: Some(notify),
				release: Some(release),
			},
		)
	};
	assert!(!guest.is_null());
	// SAFETY: The context lives until the guest is stopped, after the test's
	// last read.
	(guest, unsafe { &*notifications })
}

#[test]
fn the_guest_thread_runs_the_guest_and_posts_for_the_ui_thread() {
	let app = app();
	let (guest, notifications) = spawn(app);

	// SAFETY: The guest is live until stopped at the end.
	unsafe {
		assert_eq!(lk_guest_start(guest, 390.0, 844.0), 0);
		let (root, _) = wait("the first transaction", || transaction(guest));
		assert!(root >= 0, "the first transaction names its root");
		assert!(notifications.0.load(Ordering::SeqCst) > 0);
		assert_eq!(lk_guest_frame_requested(guest), 1);

		// The guest moves at most 64 calls ahead of the UI thread, and more as
		// the UI thread takes them.
		let mut calls = Vec::new();
		wait("the platform calls", || {
			lk_guest_ack(guest);
			calls.extend(requests(guest));
			(calls.len() == 70).then_some(())
		});

		for id in calls {
			assert_eq!(lk_guest_complete(guest, id, 0, b"ok".as_ptr(), 2), 0);
		}

		let mut rendered = 0;
		wait("a view per reply", || {
			while let Some((_, count)) = transaction(guest) {
				rendered += count;
			}
			(rendered >= 70).then_some(())
		});

		// A display frame renders a view and asks for the next frame.
		assert_eq!(lk_guest_frame(guest, 1_000_000, 16_666_667), 0);
		wait("the frame's view", || transaction(guest));
		assert_eq!(lk_guest_frame_requested(guest), 1);

		// An event reaches the guest through the node it was rendered as.
		let press = lk_guest_event(guest, usize::try_from(root).unwrap(), 0, ptr::null(), 0);
		assert_eq!(press, 0);
		wait("the event's view", || transaction(guest));

		assert_eq!(lk_guest_failed(guest), 0);
		assert_eq!(take(lk_guest_error(guest)), b"");
		assert_eq!(lk_guest_stop(guest), 0);

		// The app is the caller's again.
		assert_eq!(lk_app_render_pending(app), 0);
		lk_app_destroy(app);
	}
}

#[test]
fn a_malformed_event_fails_the_guest_and_it_still_stops() {
	let app = app();
	let (guest, _) = spawn(app);

	// SAFETY: The guest is live until stopped.
	unsafe {
		assert_eq!(lk_guest_start(guest, 390.0, 844.0), 0);
		wait("the first transaction", || transaction(guest));
		assert_eq!(lk_guest_event(guest, 0, 77, ptr::null(), 0), -1);
		assert_eq!(lk_guest_failed(guest), 1);
		assert_eq!(take(lk_guest_error(guest)), b"unknown event kind");

		// A failed guest drops what it is sent.
		assert_eq!(lk_guest_frame(guest, 2_000_000, 16_666_667), 0);
		std::thread::sleep(Duration::from_millis(20));
		assert!(transaction(guest).is_none());
		assert_eq!(lk_guest_stop(guest), 0);
		lk_app_destroy(app);
	}
}

#[test]
fn a_guest_failure_is_reported_with_its_reason() {
	let app = app();
	let (guest, notifications) = spawn(app);

	// SAFETY: The guest is live until stopped.
	unsafe {
		// The viewport is checked on the guest thread, as `lk_app_start` does.
		assert_eq!(lk_guest_start(guest, -1.0, 844.0), 0);
		wait("the failure", || {
			(lk_guest_failed(guest) == 1).then_some(())
		});
		assert!(notifications.0.load(Ordering::SeqCst) > 0);
		assert_eq!(
			String::from_utf8(take(lk_guest_error(guest))).unwrap(),
			"invalid viewport or application already started"
		);
		assert_eq!(lk_guest_stop(guest), 0);
		lk_app_destroy(app);
	}
}

#[test]
fn rive_ticks_check_their_input_and_report_no_scenes() {
	let app = app();
	let (guest, _) = spawn(app);

	// SAFETY: The guest is live until stopped.
	unsafe {
		assert_eq!(
			lk_guest_rive_tick(guest, f64::NAN, 2.0, 1, ptr::null(), 0),
			-1
		);
		assert_eq!(lk_guest_rive_tick(guest, 0.016, 2.0, 4, ptr::null(), 0), -1);
		assert_eq!(lk_guest_rive_tick(guest, 0.016, 2.0, 1, ptr::null(), 3), -1);
		assert_eq!(lk_guest_rive_tick(guest, 0.016, 2.0, 1, ptr::null(), 0), 0);
		assert_eq!(lk_guest_has_rive(guest), 0);
		let (mut node, mut width, mut height) = (0, 0, 0);
		assert_eq!(
			take(lk_guest_take_rive_frame(
				guest,
				&raw mut node,
				&raw mut width,
				&raw mut height
			)),
			b""
		);
		assert_eq!(lk_guest_stop(guest), 0);
		lk_app_destroy(app);
	}
}

#[test]
fn spawning_needs_an_app_and_both_callbacks() {
	// SAFETY: Null inputs are rejected before any use.
	unsafe {
		assert!(
			lk_guest_spawn(
				ptr::null_mut(),
				GuestCallbacks {
					context: ptr::null_mut(),
					notify: Some(notify),
					release: Some(release),
				}
			)
			.is_null()
		);
		let app = app();
		assert!(
			lk_guest_spawn(
				app,
				GuestCallbacks {
					context: ptr::null_mut(),
					notify: None,
					release: Some(release),
				}
			)
			.is_null()
		);
		assert_eq!(lk_guest_stop(ptr::null_mut()), -1);
		assert_eq!(lk_guest_start(ptr::null(), 1.0, 1.0), -1);
		assert_eq!(lk_guest_frame_requested(ptr::null()), -1);
		assert_eq!(lk_guest_next_cancelled(ptr::null()), 0);
		assert_eq!(
			lk_guest_complete_worker(ptr::null(), 1, ptr::null_mut()),
			-1
		);
		assert_eq!(
			lk_guest_complete(ptr::null(), 1, Error::Cancelled as i32, ptr::null(), 0),
			-1
		);
		lk_app_destroy(app);
	}
}
