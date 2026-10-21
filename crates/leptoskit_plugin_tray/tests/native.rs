//! Real native trays, removed before the process exits: a temporary AppKit
//! status item on macOS, a hidden Shell notification icon on Windows, and a
//! refused session D-Bus connection on Linux.
#![cfg_attr(target_os = "linux", allow(unsafe_code))]

#[cfg(target_os = "macos")]
fn main() {
	use std::time::Duration;

	use dispatch2::DispatchQueue;
	use dispatch2::DispatchTime;
	use objc2::MainThreadMarker;
	use objc2_app_kit::NSApplication;
	use objc2_app_kit::NSApplicationActivationPolicy;
	let marker = MainThreadMarker::new().expect("a harness-free test runs on the main thread");
	let app = NSApplication::sharedApplication(marker);
	app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
	// The host refuses to install until AppKit's event loop runs.
	assert!(matches!(
		leptoskit_plugin_tray::TrayHost::install(|| {}),
		Err(leptoskit_plugin_tray::Error::EventLoopNotRunning)
	));
	let deadline = DispatchTime::try_from(Duration::from_millis(100)).expect("valid deadline");
	DispatchQueue::main()
		.after(deadline, || {
			let result = std::panic::catch_unwind(exercise);
			// Every tray and menu guard was dropped inside `exercise`. This
			// windowless AppKit process has no host loop to return to.
			std::process::exit(i32::from(result.is_err()));
		})
		.expect("schedule the native fixture");
	app.run();
}

#[cfg(target_os = "windows")]
fn main() {
	exercise();
}

/// Creates, updates and removes real icons, then tears down an owner that
/// still holds one.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn exercise() {
	use std::rc::Rc;

	use leptoskit_plugin_menu::Definition;
	use leptoskit_plugin_menu::DesktopMenu;
	use leptoskit_plugin_menu::EventChannel;
	use leptoskit_plugin_menu::Item;
	use leptoskit_plugin_menu::muda;
	use leptoskit_plugin_tray::Error;
	use leptoskit_plugin_tray::Options;
	use leptoskit_plugin_tray::RgbaIcon;
	use leptoskit_plugin_tray::TRAY_CAPACITY;
	use leptoskit_plugin_tray::TrayHost;
	let macos = cfg!(target_os = "macos");
	let channel = EventChannel::new(16).unwrap();
	let sender = channel.sender();
	muda::MenuEvent::set_event_handler(Some(move |event| sender.forward(event)));
	let menu = Rc::new(
		DesktopMenu::new(
			Definition::new(vec![Item::Command {
				id: "fixture".into(),
				label: "Temporary fixture".into(),
				enabled: true,
				accelerator: None,
			}])
			.unwrap(),
		)
		.unwrap(),
	);
	let mut host = TrayHost::install(|| {}).unwrap();
	assert!(matches!(TrayHost::install(|| {}), Err(Error::AlreadyOwned)));
	let icon = RgbaIcon::new(vec![255; 16 * 16 * 4], 16, 16).unwrap();
	let template = Options {
		template: true,
		..Options::default()
	};
	if !macos {
		assert!(matches!(
			host.create(icon.clone(), &template, None),
			Err(Error::Unavailable)
		));
	}
	let id = host
		.create(
			icon.clone(),
			&Options {
				tooltip: Some("Temporary LeptosKit fixture".into()),
				title: None,
				template: macos,
			},
			Some(Rc::clone(&menu)),
		)
		.unwrap();
	host.set_tooltip(id, Some("Updated temporary fixture"))
		.unwrap();
	host.set_tooltip(id, None).unwrap();
	assert!(matches!(
		host.set_tooltip(id, Some("bad\ntooltip")),
		Err(Error::InvalidText)
	));
	if macos {
		host.set_title(id, Some("Test")).unwrap();
		host.set_title(id, None).unwrap();
		host.set_icon(id, icon.clone(), true).unwrap();
	} else {
		assert!(matches!(
			host.set_title(id, Some("Test")),
			Err(Error::Unavailable)
		));
		assert!(matches!(
			host.set_icon(id, icon.clone(), true),
			Err(Error::Unavailable)
		));
	}
	host.set_icon(id, icon.clone(), false).unwrap();
	host.set_visible(id, false).unwrap();
	host.set_visible(id, true).unwrap();
	host.set_menu(id, None).unwrap();
	host.set_menu(id, Some(Rc::clone(&menu))).unwrap();
	host.refresh_menu(id).unwrap();
	host.bounds(id).unwrap();
	host.remove(id).unwrap();
	assert!(matches!(host.remove(id), Err(Error::UnknownTray)));
	assert!(matches!(
		host.set_visible(id, true),
		Err(Error::UnknownTray)
	));
	assert!(host.drain().events.is_empty());
	// The removed tray's menu wrapper remains usable by its owner.
	menu.set_enabled("fixture", false).unwrap();

	let mut ids = Vec::new();
	for _ in 0..TRAY_CAPACITY {
		ids.push(
			host.create(icon.clone(), &Options::default(), None)
				.unwrap(),
		);
	}
	assert!(!ids.contains(&id));
	assert!(matches!(
		host.create(icon.clone(), &Options::default(), None),
		Err(Error::Capacity)
	));
	for id in ids.drain(1..) {
		host.remove(id).unwrap();
	}
	// Owner teardown with a still-live native icon.
	drop(host);
	let mut replacement = TrayHost::install(|| {}).unwrap();
	let next = replacement.create(icon, &Options::default(), None).unwrap();
	assert!(!ids.contains(&next) && next != id);
	replacement.remove(next).unwrap();
	drop(replacement);
	drop(menu);
	drop(channel);
	println!("native tray fixture passed; all test icons removed");
}

#[cfg(target_os = "linux")]
fn main() {
	use leptoskit_plugin_tray::Error;
	use leptoskit_plugin_tray::Options;
	use leptoskit_plugin_tray::RgbaIcon;
	use leptoskit_plugin_tray::TrayHost;
	// SAFETY: this harness-free test process sets the variable before starting
	// any thread, and nothing else reads the environment concurrently.
	unsafe {
		std::env::set_var(
			"DBUS_SESSION_BUS_ADDRESS",
			"unix:path=/nonexistent/leptoskit-tray-test",
		);
	}
	let mut host = TrayHost::install(|| {}).unwrap();
	assert!(matches!(TrayHost::install(|| {}), Err(Error::AlreadyOwned)));
	let icon = RgbaIcon::new(vec![0; 4], 1, 1).unwrap();
	// A refused session bus never leaves a phantom tray behind, so a retry
	// fails the same way instead of reporting exhausted capacity.
	for _ in 0..2 {
		assert!(matches!(
			host.create(icon.clone(), &Options::default(), None),
			Err(Error::Native(_))
		));
	}
	assert!(host.drain().events.is_empty());
	drop(host);
	drop(TrayHost::install(|| {}).unwrap());
	println!("Linux tray refuses an unavailable session bus without retaining icons");
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn main() {}
