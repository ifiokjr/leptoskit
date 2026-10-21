//! Hidden owned Win32 window; runs on Windows without real keyboard or mouse
//! input.
#![cfg_attr(target_os = "windows", allow(unsafe_code))]

fn main() {
	let mut arguments = libtest_mimic::Arguments::from_args();
	// Win32 menus belong to the thread that owns their window.
	arguments.test_threads = Some(1);
	#[cfg(target_os = "windows")]
	let trials = vec![libtest_mimic::Trial::test(
		"window_menu_commands_and_teardown",
		|| {
			window_menu_commands_and_teardown();
			Ok(())
		},
	)];
	#[cfg(not(target_os = "windows"))]
	let trials = Vec::new();
	libtest_mimic::run(&arguments, trials).exit();
}

#[cfg(target_os = "windows")]
fn window_menu_commands_and_teardown() {
	use std::rc::Rc;

	use leptoskit_plugin_menu::Definition;
	use leptoskit_plugin_menu::DesktopMenu;
	use leptoskit_plugin_menu::Error;
	use leptoskit_plugin_menu::EventChannel;
	use leptoskit_plugin_menu::Item;
	use leptoskit_plugin_menu::muda;
	use leptoskit_plugin_menu::windows::WindowMenu;
	use windows_sys::Win32::UI::WindowsAndMessaging::CreateMenu;
	use windows_sys::Win32::UI::WindowsAndMessaging::CreateWindowExW;
	use windows_sys::Win32::UI::WindowsAndMessaging::DestroyMenu;
	use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
	use windows_sys::Win32::UI::WindowsAndMessaging::GetMenu;
	use windows_sys::Win32::UI::WindowsAndMessaging::GetMenuItemID;
	use windows_sys::Win32::UI::WindowsAndMessaging::GetSubMenu;
	use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;
	use windows_sys::Win32::UI::WindowsAndMessaging::SetMenu;
	use windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND;
	use windows_sys::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW;

	let events = EventChannel::new(8).unwrap();
	let sender = events.sender();
	muda::MenuEvent::set_event_handler(Some(move |event| sender.forward(event)));
	let menu = Rc::new(
		DesktopMenu::new(
			Definition::new(vec![Item::Submenu {
				id: "file".into(),
				label: "File".into(),
				enabled: true,
				items: vec![Item::Command {
					id: "save".into(),
					label: "Save".into(),
					enabled: true,
					accelerator: Some("F6".into()),
				}],
			}])
			.unwrap(),
		)
		.unwrap(),
	);
	let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
	let title: Vec<u16> = "LeptosKit owned menu test\0".encode_utf16().collect();
	// SAFETY: the test owns its hidden top-level window and all HMENU
	// lifetimes. STATIC is a system class; no user window lookup or visible
	// UI is used.
	unsafe {
		let window = CreateWindowExW(
			0,
			class.as_ptr(),
			title.as_ptr(),
			WS_OVERLAPPEDWINDOW,
			0,
			0,
			240,
			160,
			std::ptr::null_mut(),
			std::ptr::null_mut(),
			std::ptr::null_mut(),
			std::ptr::null(),
		);
		assert!(!window.is_null());
		let previous = CreateMenu();
		assert!(!previous.is_null());
		assert_ne!(SetMenu(window, previous), 0);
		let lease = WindowMenu::attach(menu.clone(), window).unwrap();
		assert!(matches!(
			WindowMenu::attach(menu.clone(), window),
			Err(Error::Busy)
		));
		let installed = GetMenu(window);
		assert_ne!(installed, previous);
		let submenu = GetSubMenu(installed, 0);
		assert!(!submenu.is_null());
		let command_id = GetMenuItemID(submenu, 0);
		assert_ne!(command_id, u32::MAX);
		SendMessageW(window, WM_COMMAND, command_id as usize, 0);
		assert_eq!(
			menu.handle_event(&events.try_recv().expect("native WM_COMMAND activation"))
				.unwrap()
				.id,
			"save"
		);
		menu.set_enabled("save", false).unwrap();
		SendMessageW(window, WM_COMMAND, command_id as usize, 0);
		if let Some(event) = events.try_recv() {
			assert!(menu.handle_event(&event).is_none());
		}
		drop(lease);
		assert_eq!(GetMenu(window), previous);
		assert_ne!(SetMenu(window, std::ptr::null_mut()), 0);
		assert_ne!(DestroyMenu(previous), 0);
		assert_ne!(DestroyWindow(window), 0);
	}
}
