//! AppKit menu ownership on the host's main thread.
#![allow(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;

use muda::ContextMenu;
use objc2::MainThreadMarker;
use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::NSApplication;
use objc2_app_kit::NSMenu;

use crate::DesktopMenu;
use crate::Error;
use crate::Item;

thread_local! { static ATTACHED: Cell<bool> = const { Cell::new(false) }; }

/// Owns the process's application-menu slot until drop.
///
/// Saves the old menu and restores it only if this lease's menu is still
/// installed. The host must deliberately call this API for its own application.
/// It does not discover or modify another application's menu and never
/// activates the app.
pub struct ApplicationMenu {
	menu: Rc<DesktopMenu>,
	application: Retained<NSApplication>,
	previous: Option<Retained<NSMenu>>,
	installed: Retained<NSMenu>,
}

impl ApplicationMenu {
	/// Attaches a menu bar. AppKit menu bars require only submenus at the root;
	/// flat menus remain usable in trays. A second active lease returns `Busy`.
	pub fn attach(menu: Rc<DesktopMenu>, application: &NSApplication) -> Result<Self, Error> {
		if !menu
			.definition
			.items()
			.iter()
			.all(|item| matches!(item, Item::Submenu { .. }))
		{
			return Err(Error::Unavailable);
		}

		if ATTACHED.with(|attached| attached.replace(true)) {
			return Err(Error::Busy);
		}

		let previous = application.mainMenu();
		// SAFETY: muda owns a live NSMenu on this main thread. Retain it so the
		// lease can compare identities independently of the application slot.
		let installed = unsafe { Retained::retain(menu.native_menu().ns_menu().cast::<NSMenu>()) };
		let Some(installed) = installed else {
			ATTACHED.with(|attached| attached.set(false));

			return Err(Error::Native);
		};
		application.setMainMenu(Some(&installed));
		Ok(Self {
			menu,
			application: application.retain(),
			previous,
			installed,
		})
	}

	/// The owned menu; updating its items preserves the installed native tree.
	pub fn menu(&self) -> &DesktopMenu {
		&self.menu
	}
}

impl Drop for ApplicationMenu {
	fn drop(&mut self) {
		if self.application.mainMenu().is_some_and(|menu| {
			std::ptr::eq(
				std::ptr::from_ref(&*menu),
				std::ptr::from_ref(&*self.installed),
			)
		}) {
			self.application.setMainMenu(self.previous.as_deref());
		}

		ATTACHED.with(|attached| attached.set(false));
	}
}

/// Acquires this process's AppKit application explicitly on the UI thread.
/// Does not activate the application or create a visible window.
pub fn application(main_thread: MainThreadMarker) -> Retained<NSApplication> {
	NSApplication::sharedApplication(main_thread)
}
