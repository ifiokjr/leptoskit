//! Apps report to a `DevTools` hub only when the host has the `devtools`
//! feature and a hub to report to before its first app starts. This file is
//! its own process, so no other test has named a hub.

#![cfg(feature = "app")]
#![allow(unsafe_code)]

use std::io::ErrorKind;
use std::mem::MaybeUninit;
use std::net::TcpListener;
use std::time::Duration;

use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::devtools::DevtoolsFrame;
use leptoskit_bridge_ffi::devtools::lk_devtools_active;
use leptoskit_bridge_ffi::devtools::lk_devtools_highlight;
use leptoskit_bridge_ffi::devtools::lk_devtools_hub;
use leptoskit_bridge_ffi::devtools::lk_devtools_pick;
use leptoskit_bridge_ffi::devtools::lk_devtools_picking;
use leptoskit_bridge_ffi::lk_buffer_release;

const GUEST: &str = r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "commit" (func $commit))
 (memory (export "memory") 1)
 (func (export "leptoskit_main") (call $create (i64.const 1) (i32.const 0)) (call $commit))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
)"#;

/// Creates, starts, renders, and destroys an app.
fn run_app() {
	let mut diagnostic = MaybeUninit::uninit();
	// SAFETY: Every input lives for its call, and the app is used on this
	// thread and destroyed once.
	unsafe {
		let app = create(GUEST.as_ptr(), GUEST.len(), diagnostic.as_mut_ptr(), |_| {
			Ok(())
		});
		assert!(!app.is_null());
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);
		lk_buffer_release(lk_app_render(app));
		lk_app_destroy(app);
	}
}

#[test]
fn nothing_reports_without_a_hub_or_the_feature() {
	if std::env::var_os("LEPTOSKIT_DEVTOOLS").is_some() {
		eprintln!("skipped: LEPTOSKIT_DEVTOOLS names a hub for this process");

		return;
	}

	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	listener.set_nonblocking(true).unwrap();
	let url = format!("http://{}", listener.local_addr().unwrap());

	// No hub was named before the first app, so nothing starts, and a hub
	// named later is too late for this process. Without the feature, or in a
	// release build, the hub is refused outright.
	run_app();
	// SAFETY: Both inputs live for the call.
	let configured = unsafe { lk_devtools_hub(url.as_ptr(), url.len(), b"app".as_ptr(), 3) };
	let reports = cfg!(all(feature = "devtools", debug_assertions));

	assert_eq!(configured, if reports { 0 } else { -1 });
	run_app();

	std::thread::sleep(Duration::from_millis(300));
	assert!(
		matches!(listener.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock),
		"no app opened a session"
	);
	assert_eq!(lk_devtools_picking(), 0);
	assert_eq!(lk_devtools_active(), 0, "platforms draw no overlay");
	assert_eq!(lk_devtools_pick(1, 1.0, 1.0), 0, "taps reach the app");
	let mut frame = DevtoolsFrame::default();
	// SAFETY: The output is writable.
	assert_eq!(unsafe { lk_devtools_highlight(&raw mut frame) }, 0);
	// SAFETY: Invalid input is refused without being read.
	assert_eq!(
		unsafe { lk_devtools_hub(std::ptr::null(), 4, b"app".as_ptr(), 3) },
		-1
	);
}
