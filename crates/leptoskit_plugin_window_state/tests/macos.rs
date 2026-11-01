//! Hidden native-window journey; AppKit requires the process main thread.
#![cfg_attr(target_os = "macos", allow(unsafe_code))]

fn main() {
	let mut arguments = libtest_mimic::Arguments::from_args();
	// AppKit windows require the process main thread.
	arguments.test_threads = Some(1);
	#[cfg(target_os = "macos")]
	let trials = vec![libtest_mimic::Trial::test(
		"hidden_window_captures_and_restores_onto_a_visible_display",
		|| {
			hidden_window_captures_and_restores_onto_a_visible_display();
			Ok(())
		},
	)];
	#[cfg(not(target_os = "macos"))]
	let trials = Vec::new();
	libtest_mimic::run(&arguments, trials).exit();
}

#[cfg(target_os = "macos")]
fn hidden_window_captures_and_restores_onto_a_visible_display() {
	use leptoskit_plugin_window_state::Bounds;
	use leptoskit_plugin_window_state::WindowMode;
	use leptoskit_plugin_window_state::WindowState;
	use leptoskit_plugin_window_state::macos;
	use objc2::MainThreadMarker;
	use objc2::MainThreadOnly;
	use objc2_app_kit::NSApplication;
	use objc2_app_kit::NSBackingStoreType;
	use objc2_app_kit::NSWindow;
	use objc2_app_kit::NSWindowStyleMask;
	use objc2_foundation::NSPoint;
	use objc2_foundation::NSRect;
	use objc2_foundation::NSSize;

	let main_thread = MainThreadMarker::new().expect("test runs on main thread");
	let _application = NSApplication::sharedApplication(main_thread);
	let displays = macos::displays(main_thread).unwrap();
	let area = displays.first().expect("macOS has a display").work_area();
	// SAFETY: this test owns an ordinary buffered window on the main thread.
	// The window is never shown or made key, so it cannot take user focus.
	let window = unsafe {
		NSWindow::initWithContentRect_styleMask_backing_defer(
			NSWindow::alloc(main_thread),
			NSRect::new(
				NSPoint::new(area.x() + 20.0, area.y() + 30.0),
				NSSize::new(320.0, 240.0),
			),
			NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable,
			NSBackingStoreType::Buffered,
			false,
		)
	};
	// SAFETY: Retained owns the window, so AppKit must not release on close.
	unsafe {
		window.setReleasedWhenClosed(false);
	}
	let original = macos::capture(&window, None).unwrap();
	assert_eq!(original.mode, WindowMode::Normal);

	assert!(!window.isVisible());
	let saved = WindowState {
		normal_bounds: Bounds::new(area.x() - 5000.0, area.y() - 5000.0, 320.0, 260.0).unwrap(),
		display_id: Some("disconnected-monitor".into()),
		mode: WindowMode::Normal,
		minimized: false,
	};
	macos::restore(&window, &saved, main_thread).unwrap();
	let restored = macos::capture(&window, None).unwrap();
	assert!(restored.normal_bounds.x() >= area.x());
	assert!(restored.normal_bounds.y() >= area.y());
	assert!(restored.normal_bounds.x() + restored.normal_bounds.width() <= area.x() + area.width());
	assert!(
		restored.normal_bounds.y() + restored.normal_bounds.height() <= area.y() + area.height()
	);
	assert!(
		!window.isVisible(),
		"restoring a hidden window must not activate it"
	);
}
