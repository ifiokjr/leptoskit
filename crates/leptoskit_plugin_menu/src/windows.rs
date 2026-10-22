//! Win32 menu attachment and accelerator translation for an owned window.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::DrawMenuBar;
use windows_sys::Win32::UI::WindowsAndMessaging::GetMenu;
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::HMENU;
use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::MSG;
use windows_sys::Win32::UI::WindowsAndMessaging::SetMenu;
use windows_sys::Win32::UI::WindowsAndMessaging::TranslateAcceleratorW;

use crate::DesktopMenu;
use crate::Error;

thread_local! { static ATTACHED: RefCell<HashSet<isize>> = RefCell::new(HashSet::new()); }

/// Owns a menu slot on a live Win32 window. Drop before destroying that window.
/// The previous native menu is restored without being destroyed by this crate.
pub struct WindowMenu {
	menu: Rc<DesktopMenu>,
	hwnd: HWND,
	previous: HMENU,
	installed: HMENU,
}

impl WindowMenu {
	/// Installs a menu on an explicitly supplied, current-thread window.
	///
	/// # Safety
	/// `hwnd` must be owned by this host. The window and any previous HMENU
	/// must stay alive until this lease is dropped. Do not install another
	/// muda menu or destroy/replace this menu's native handles during the
	/// lease. The host must run the window's message loop on this thread.
	pub unsafe fn attach(menu: Rc<DesktopMenu>, hwnd: HWND) -> Result<Self, Error> {
		// SAFETY: Win32 validity/thread queries do not dereference caller
		// memory.
		if unsafe { IsWindow(hwnd) } == 0 {
			return Err(Error::Native);
		}
		if unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) != GetCurrentThreadId() } {
			return Err(Error::WrongThread);
		}
		if !ATTACHED.with(|attached| attached.borrow_mut().insert(hwnd as isize)) {
			return Err(Error::Busy);
		}
		// SAFETY: caller keeps the current-thread HWND and previous menu alive.
		let previous = unsafe { GetMenu(hwnd) };
		if unsafe { menu.native_menu().init_for_hwnd(hwnd as isize) }.is_err() {
			ATTACHED.with(|attached| attached.borrow_mut().remove(&(hwnd as isize)));
			return Err(Error::Native);
		}
		let installed = unsafe { GetMenu(hwnd) };
		if installed.is_null() || installed == previous {
			unsafe {
				let _ = menu.native_menu().remove_for_hwnd(hwnd as isize);
				SetMenu(hwnd, previous);
				DrawMenuBar(hwnd);
			}
			ATTACHED.with(|attached| attached.borrow_mut().remove(&(hwnd as isize)));
			return Err(Error::Native);
		}
		Ok(Self {
			menu,
			hwnd,
			previous,
			installed,
		})
	}

	/// Consumes keyboard accelerators before the host calls `TranslateMessage`
	/// and `DispatchMessage`. Pass the current `GetMessageW` message; return
	/// `true` means it has been handled and must not be dispatched a second
	/// time.
	pub fn translate_accelerator(&self, message: &MSG) -> bool {
		// SAFETY: the lease requires a live HWND on this thread and its menu
		// owns the accelerator table. Win32 takes a pointer to the
		// borrowed message.
		unsafe {
			TranslateAcceleratorW(self.hwnd, self.menu.native_menu().haccel() as _, message) != 0
		}
	}

	/// The attached menu's state-update and event-routing methods.
	pub fn menu(&self) -> &DesktopMenu {
		&self.menu
	}
}

impl Drop for WindowMenu {
	fn drop(&mut self) {
		// SAFETY: attach's contract keeps this window and its native handles
		// alive. Detach muda's subclass before releasing the menu. Preserve a
		// foreign menu if the host replaced the slot despite the lease
		// contract.
		unsafe {
			let current = GetMenu(self.hwnd);
			let restore = if current == self.installed {
				self.previous
			} else {
				current
			};
			let _ = self.menu.native_menu().remove_for_hwnd(self.hwnd as isize);
			SetMenu(self.hwnd, restore);
			DrawMenuBar(self.hwnd);
		}
		ATTACHED.with(|attached| attached.borrow_mut().remove(&(self.hwnd as isize)));
	}
}
