//! Opt-in native fixture: creates temporary tray/menu objects and destroys them
//! before terminating its dedicated AppKit process. Never run automatically.
#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	use std::time::Duration;

	use dispatch2::DispatchQueue;
	use dispatch2::DispatchTime;
	use objc2::MainThreadMarker;
	use objc2_app_kit::NSApplication;
	use objc2_app_kit::NSApplicationActivationPolicy;
	let marker = MainThreadMarker::new().ok_or("fixture needs the main thread")?;
	let app = NSApplication::sharedApplication(marker);
	app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
	let deadline = DispatchTime::try_from(Duration::from_millis(100))
		.map_err(|()| "invalid fixture deadline")?;
	DispatchQueue::main()
		.after(deadline, || {
			let result = run_fixture();
			let exit_code = match result {
				Ok(()) => {
					println!("native tray fixture passed; all test icons removed");
					0
				}
				Err(error) => {
					eprintln!("native tray fixture failed: {error}");
					1
				}
			};
			// run_fixture has returned, so every native tray/menu guard is
			// already dropped. This isolated windowless AppKit
			// fixture has no host loop to return control to; exit
			// only after deterministic resource teardown.
			std::process::exit(exit_code);
		})
		.map_err(|_| "could not schedule native fixture")?;
	app.run();
	Ok(())
}
#[cfg(target_os = "macos")]
fn run_fixture() -> Result<(), Box<dyn std::error::Error>> {
	use std::rc::Rc;

	use leptoskit_plugin_menu::Definition;
	use leptoskit_plugin_menu::DesktopMenu;
	use leptoskit_plugin_menu::EventChannel;
	use leptoskit_plugin_menu::Item;
	use leptoskit_plugin_tray::Error;
	use leptoskit_plugin_tray::Options;
	use leptoskit_plugin_tray::RgbaIcon;
	use leptoskit_plugin_tray::TrayHost;
	let channel = EventChannel::new(16)?;
	let sender = channel.sender();
	tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| sender.forward(event)));
	let menu = Rc::new(DesktopMenu::new(Definition::new(vec![Item::Command {
		id: "fixture".into(),
		label: "Temporary fixture".into(),
		enabled: true,
		accelerator: None,
	}])?)?);
	let mut host = TrayHost::install(|| {})?;
	assert!(matches!(TrayHost::install(|| {}), Err(Error::AlreadyOwned)));
	let icon = RgbaIcon::new(vec![255; 16 * 16 * 4], 16, 16)?;
	let id = host.create(
		icon.clone(),
		&Options {
			tooltip: Some("Temporary LeptosKit fixture".into()),
			title: None,
			template: true,
		},
		Some(Rc::clone(&menu)),
	)?;
	host.set_tooltip(id, Some("Updated temporary fixture"))?;
	host.set_title(id, Some("Test"))?;
	host.set_icon(id, icon.clone(), false)?;
	host.set_visible(id, false)?;
	host.set_visible(id, true)?;
	host.set_menu(id, None)?;
	host.set_menu(id, Some(Rc::clone(&menu)))?;
	host.remove(id)?;
	assert!(matches!(host.remove(id), Err(Error::UnknownTray)));
	assert!(host.drain().events.is_empty());
	let second = host.create(icon.clone(), &Options::default(), None)?;
	assert_ne!(id, second);
	drop(host); // Exercises owner teardown with a still-live native icon.
	let mut replacement = TrayHost::install(|| {})?;
	let third = replacement.create(icon, &Options::default(), None)?;
	assert_ne!(second, third);
	replacement.remove(third)?;
	drop(replacement);
	drop(menu);
	drop(channel);
	Ok(())
}
#[cfg(not(target_os = "macos"))]
fn main() {
	eprintln!("this opt-in native fixture requires macOS");
}
