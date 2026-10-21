//! Main-thread AppKit smoke using only this test process's own invisible menu.
#![cfg_attr(target_os = "macos", allow(unsafe_code))]

#[cfg(target_os = "macos")]
fn main() {
	use std::rc::Rc;

	use leptoskit_plugin_menu::Definition;
	use leptoskit_plugin_menu::DesktopMenu;
	use leptoskit_plugin_menu::Error;
	use leptoskit_plugin_menu::EventChannel;
	use leptoskit_plugin_menu::Item;
	use leptoskit_plugin_menu::macos;
	use leptoskit_plugin_menu::muda;
	use muda::ContextMenu;
	use objc2::MainThreadMarker;
	use objc2::rc::Retained;
	use objc2_app_kit::NSMenu;

	let channel = EventChannel::new(4).unwrap();
	let sender = channel.sender();
	muda::MenuEvent::set_event_handler(Some(move |event| sender.forward(event)));
	let mtm = MainThreadMarker::new().unwrap();
	let application = macos::application(mtm);
	let previous = application.mainMenu();
	let definition = Definition::new(vec![Item::Submenu {
		id: "file".into(),
		label: "File".into(),
		enabled: true,
		items: vec![
			Item::Command {
				id: "save".into(),
				label: "Save".into(),
				enabled: true,
				accelerator: Some("CmdOrCtrl+S".into()),
			},
			Item::Separator,
			Item::Check {
				id: "toggle".into(),
				label: "Toggle".into(),
				enabled: true,
				checked: false,
				accelerator: None,
			},
		],
	}])
	.unwrap();
	let menu = Rc::new(DesktopMenu::new(definition.clone()).unwrap());
	let other = Rc::new(DesktopMenu::new(definition).unwrap());
	let lease = macos::ApplicationMenu::attach(menu.clone(), &application).unwrap();
	assert!(matches!(
		macos::ApplicationMenu::attach(other.clone(), &application),
		Err(Error::Busy)
	));
	let submenu = menu
		.native_menu()
		.items()
		.remove(0)
		.as_submenu()
		.unwrap()
		.clone();
	let command = submenu.items().remove(0).as_menuitem().unwrap().clone();
	let event = muda::MenuEvent {
		id: command.id().clone(),
	};
	assert_eq!(menu.handle_event(&event).unwrap().id, "save");
	assert!(other.handle_event(&event).is_none());
	menu.set_enabled("file", false).unwrap();
	assert!(menu.handle_event(&event).is_none());
	menu.set_enabled("file", true).unwrap();
	menu.set_enabled("save", false).unwrap();
	assert!(menu.handle_event(&event).is_none());
	menu.set_enabled("save", true).unwrap();
	menu.set_checked("toggle", true).unwrap();
	assert!(menu.is_checked("toggle").unwrap());
	assert_eq!(menu.set_checked("save", true), Err(Error::InvalidItem));
	assert_eq!(
		menu.set_label("save", "bad\nlabel"),
		Err(Error::InvalidDefinition)
	);
	menu.set_label("save", "Save Document").unwrap();
	assert_eq!(command.text(), "Save Document");
	// Toggles and submenus update in place too; separators have no identity.
	menu.set_label("toggle", "Toggle option").unwrap();
	menu.set_label("file", "Document").unwrap();
	assert_eq!(submenu.text(), "Document");
	menu.set_enabled("toggle", false).unwrap();
	menu.set_enabled("toggle", true).unwrap();
	assert_eq!(menu.is_checked("save"), Err(Error::InvalidItem));
	assert_eq!(menu.set_enabled("missing", true), Err(Error::InvalidItem));
	assert_eq!(
		menu.set_label("missing", "Missing"),
		Err(Error::InvalidItem)
	);
	assert!(leptoskit_plugin_menu::supports_menu_bar());
	// AppKit action dispatch is synthetic and does not open or focus any UI.
	// SAFETY: the submenu owns this native menu and all its items on main
	// thread.
	let native = unsafe { Retained::retain(submenu.ns_menu().cast::<NSMenu>()) }.unwrap();
	native.performActionForItemAtIndex(0);
	assert_eq!(
		menu.handle_event(&channel.try_recv().unwrap()).unwrap().id,
		"save"
	);
	native.performActionForItemAtIndex(2);
	let toggled = menu.handle_event(&channel.try_recv().unwrap()).unwrap();
	assert_eq!(toggled.id, "toggle");
	assert_eq!(toggled.checked, Some(false));
	assert!(!menu.is_checked("toggle").unwrap());
	assert!(matches!(
		DesktopMenu::new(
			Definition::new(vec![Item::Command {
				id: "invalid".into(),
				label: "Invalid".into(),
				enabled: true,
				accelerator: Some("Ctrl+NotARealKey".into())
			}])
			.unwrap()
		),
		Err(Error::InvalidAccelerator)
	));
	drop(lease);
	assert_eq!(
		application.mainMenu().as_deref().map(std::ptr::from_ref),
		previous.as_deref().map(std::ptr::from_ref)
	);
	let lease = macos::ApplicationMenu::attach(menu.clone(), &application).unwrap();
	// An unrelated replacement is preserved when the old lease is dropped.
	let replacement =
		unsafe { Retained::retain(other.native_menu().ns_menu().cast::<NSMenu>()) }.unwrap();
	application.setMainMenu(Some(&replacement));
	drop(lease);
	assert!(std::ptr::eq(
		std::ptr::from_ref(&*application.mainMenu().unwrap()),
		std::ptr::from_ref(&*replacement)
	));
	application.setMainMenu(previous.as_deref());
	assert!(
		std::thread::spawn(|| {
			DesktopMenu::new(Definition::new(vec![Item::Separator]).unwrap()).err()
		})
		.join()
		.unwrap()
			== Some(Error::WrongThread)
	);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
