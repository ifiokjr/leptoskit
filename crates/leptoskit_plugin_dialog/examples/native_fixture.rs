//! Native fixture for the dialog plugin's real-OS journey. A Win32 window
//! whose buttons present the plugin's file chooser and message box; the
//! `dialog_plugin_journey` example in `leptoskit_testing` drives it.
#![cfg_attr(target_os = "windows", allow(unsafe_code))]

#[cfg(target_os = "windows")]
fn main() {
	fixture::run();
}

#[cfg(not(target_os = "windows"))]
fn main() {
	eprintln!("the dialog plugin's native fixture currently targets Windows");
	std::process::exit(1);
}

#[cfg(target_os = "windows")]
mod fixture {
	use std::num::NonZeroIsize;
	use std::sync::Mutex;

	use leptoskit_plugin_dialog::desktop::FileDialog;
	use leptoskit_plugin_dialog::desktop::FileOptions;
	use leptoskit_plugin_dialog::desktop::MessageButtons;
	use leptoskit_plugin_dialog::desktop::MessageDialog;
	use leptoskit_plugin_dialog::desktop::MessageDialogResult;
	use leptoskit_plugin_dialog::desktop::MessageLevel;
	use raw_window_handle::DisplayHandle;
	use raw_window_handle::HandleError;
	use raw_window_handle::HasDisplayHandle;
	use raw_window_handle::HasWindowHandle;
	use raw_window_handle::Win32WindowHandle;
	use raw_window_handle::WindowHandle;
	use windows_sys::Win32::Foundation::HWND;
	use windows_sys::Win32::Foundation::LPARAM;
	use windows_sys::Win32::Foundation::LRESULT;
	use windows_sys::Win32::Foundation::WPARAM;
	use windows_sys::Win32::Graphics::Gdi::COLOR_WINDOW;
	use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
	use windows_sys::Win32::UI::WindowsAndMessaging::*;

	const CHOOSE: i32 = 1001;
	const CONFIRM: i32 = 1002;
	const STATUS: i32 = 1003;
	const OUTCOME: u32 = WM_APP + 1;

	/// The result text a worker thread hands back to the UI thread.
	static RESULT: Mutex<Option<String>> = Mutex::new(None);

	/// The fixture window as a dialog parent. It is created before any
	/// dialog and destroyed only after the message loop ends.
	#[derive(Clone, Copy)]
	struct Parent(isize);

	impl HasWindowHandle for Parent {
		fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
			let raw =
				Win32WindowHandle::new(NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?);
			// SAFETY: the window outlives every dialog it presents.
			Ok(unsafe { WindowHandle::borrow_raw(raw.into()) })
		}
	}

	impl HasDisplayHandle for Parent {
		fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
			Ok(DisplayHandle::windows())
		}
	}

	fn wide(text: &str) -> Vec<u16> {
		text.encode_utf16().chain(std::iter::once(0)).collect()
	}

	pub fn run() {
		// SAFETY: plain Win32 window creation and a message loop on this
		// thread, which owns every window it creates.
		unsafe {
			let instance = GetModuleHandleW(std::ptr::null());
			let class = wide("LeptoskitDialogFixture");
			let registration = WNDCLASSW {
				lpfnWndProc: Some(procedure),
				hInstance: instance,
				lpszClassName: class.as_ptr(),
				hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
				hbrBackground: (COLOR_WINDOW + 1) as _,
				..std::mem::zeroed()
			};
			assert_ne!(
				RegisterClassW(&raw const registration),
				0,
				"register the window class"
			);
			let title = wide("Leptoskit dialog fixture");
			let window = CreateWindowExW(
				0,
				class.as_ptr(),
				title.as_ptr(),
				WS_OVERLAPPEDWINDOW | WS_VISIBLE,
				CW_USEDEFAULT,
				CW_USEDEFAULT,
				560,
				320,
				std::ptr::null_mut(),
				std::ptr::null_mut(),
				instance,
				std::ptr::null(),
			);
			assert!(!window.is_null(), "create the fixture window");
			let mut message = std::mem::zeroed::<MSG>();
			while GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) > 0 {
				TranslateMessage(&raw const message);
				DispatchMessageW(&raw const message);
			}
		}
	}

	unsafe fn child(parent: HWND, class: &str, text: &str, id: i32, y: i32, style: u32) {
		let class = wide(class);
		let text = wide(text);
		// SAFETY: the parent is a live window owned by this thread.
		unsafe {
			CreateWindowExW(
				0,
				class.as_ptr(),
				text.as_ptr(),
				WS_CHILD | WS_VISIBLE | style,
				24,
				y,
				480,
				40,
				parent,
				id as isize as HMENU,
				GetModuleHandleW(std::ptr::null()),
				std::ptr::null(),
			);
		}
	}

	/// Presents a dialog on a worker thread and posts its result back. The UI
	/// thread keeps pumping messages: Windows enables and disables the owner
	/// synchronously, so awaiting the dialog here would deadlock.
	fn present(window: HWND, choose: bool) {
		let parent = Parent(window as isize);
		std::thread::spawn(move || {
			let text = if choose {
				choose_evidence(parent)
			} else {
				confirm(parent)
			};
			*RESULT.lock().unwrap() = Some(text);
			// SAFETY: posting to a window handle is valid even after it closes.
			unsafe {
				PostMessageW(parent.0 as HWND, OUTCOME, 0, 0);
			}
		});
	}

	fn choose_evidence(parent: Parent) -> String {
		let directory = std::env::current_exe()
			.ok()
			.and_then(|path| path.parent().map(std::path::Path::to_path_buf));
		let options = FileOptions {
			title: "Choose evidence".into(),
			directory,
			file_name: Some("evidence.txt".into()),
			..FileOptions::default()
		};
		match FileDialog::new(&parent, options) {
			Ok(dialog) => {
				match futures::executor::block_on(dialog.open_file()) {
					Some(path) => {
						format!(
							"Selected: {}",
							path.file_name().map_or_else(String::new, |name| {
								name.to_string_lossy().into_owned()
							})
						)
					}
					None => "Cancelled".into(),
				}
			}
			Err(error) => format!("Dialog failed: {error}"),
		}
	}

	fn confirm(parent: Parent) -> String {
		match MessageDialog::new(
			&parent,
			"Confirm evidence",
			"Keep the selected evidence?",
			MessageLevel::Info,
			MessageButtons::OkCancel,
		) {
			Ok(dialog) => {
				match futures::executor::block_on(dialog.show()) {
					MessageDialogResult::Ok => "Confirmed".into(),
					_ => "Declined".into(),
				}
			}
			Err(error) => format!("Dialog failed: {error}"),
		}
	}

	unsafe extern "system" fn procedure(
		window: HWND,
		message: u32,
		wparam: WPARAM,
		lparam: LPARAM,
	) -> LRESULT {
		// SAFETY: Windows calls this procedure on the thread that owns
		// `window`, with parameters valid for `message`.
		unsafe {
			match message {
				WM_CREATE => {
					child(
						window,
						"BUTTON",
						"Choose evidence",
						CHOOSE,
						24,
						BS_PUSHBUTTON as u32,
					);
					child(
						window,
						"BUTTON",
						"Confirm",
						CONFIRM,
						80,
						BS_PUSHBUTTON as u32,
					);
					child(window, "STATIC", "Ready", STATUS, 136, 0);
					0
				}
				WM_COMMAND => {
					match (wparam & 0xffff) as i32 {
						CHOOSE => present(window, true),
						CONFIRM => present(window, false),
						_ => {}
					}
					0
				}
				OUTCOME => {
					if let Some(text) = RESULT.lock().unwrap().take() {
						let text = wide(&text);
						SetWindowTextW(GetDlgItem(window, STATUS), text.as_ptr());
					}
					0
				}
				WM_DESTROY => {
					PostQuitMessage(0);
					0
				}
				_ => DefWindowProcW(window, message, wparam, lparam),
			}
		}
	}
}
