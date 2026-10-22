//! Native fixture for the dialog plugin's real-OS journey. A Win32 or AppKit
//! window whose buttons present the plugin's file chooser and message box;
//! the `dialog_plugin_journey` example in `leptoskit_testing` drives it.
#![cfg_attr(any(target_os = "windows", target_os = "macos"), allow(unsafe_code))]

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn main() {
	fixture::run();
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn main() {
	eprintln!("the dialog plugin's native fixture currently targets Windows and macOS");
	std::process::exit(1);
}

/// The dialogs each fixture presents and the status text it shows for their
/// results. Both block their thread until the dialog closes, so call them off
/// the UI thread.
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod dialogs {
	use std::path::PathBuf;

	use leptoskit_plugin_dialog::desktop::FileDialog;
	use leptoskit_plugin_dialog::desktop::FileOptions;
	use leptoskit_plugin_dialog::desktop::HasDisplayHandle;
	use leptoskit_plugin_dialog::desktop::HasWindowHandle;
	use leptoskit_plugin_dialog::desktop::MessageButtons;
	use leptoskit_plugin_dialog::desktop::MessageDialog;
	use leptoskit_plugin_dialog::desktop::MessageDialogResult;
	use leptoskit_plugin_dialog::desktop::MessageLevel;

	/// Opens the file chooser in `directory`, suggesting `evidence.txt`.
	pub fn choose_evidence(
		parent: &(impl HasWindowHandle + HasDisplayHandle),
		directory: Option<PathBuf>,
	) -> String {
		let options = FileOptions {
			title: "Choose evidence".into(),
			directory,
			file_name: Some("evidence.txt".into()),
			..FileOptions::default()
		};
		match FileDialog::new(parent, options) {
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

	/// Asks to keep the evidence with an OK/Cancel message box.
	pub fn confirm(parent: &(impl HasWindowHandle + HasDisplayHandle)) -> String {
		match MessageDialog::new(
			parent,
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
}

#[cfg(target_os = "windows")]
mod fixture {
	use std::num::NonZeroIsize;
	use std::sync::Mutex;

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
				let directory = std::env::current_exe()
					.ok()
					.and_then(|path| path.parent().map(std::path::Path::to_path_buf));
				crate::dialogs::choose_evidence(&parent, directory)
			} else {
				crate::dialogs::confirm(&parent)
			};
			*RESULT.lock().unwrap() = Some(text);
			// SAFETY: posting to a window handle is valid even after it closes.
			unsafe {
				PostMessageW(parent.0 as HWND, OUTCOME, 0, 0);
			}
		});
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

#[cfg(target_os = "macos")]
mod fixture {
	use std::cell::OnceCell;
	use std::ffi::c_void;
	use std::path::PathBuf;
	use std::ptr::NonNull;

	use objc2::DefinedClass;
	use objc2::MainThreadMarker;
	use objc2::MainThreadOnly;
	use objc2::define_class;
	use objc2::msg_send;
	use objc2::rc::Retained;
	use objc2::runtime::AnyObject;
	use objc2::runtime::NSObject;
	use objc2::runtime::NSObjectProtocol;
	use objc2::sel;
	use objc2_app_kit::NSApplication;
	use objc2_app_kit::NSApplicationActivationPolicy;
	use objc2_app_kit::NSBackingStoreType;
	use objc2_app_kit::NSButton;
	use objc2_app_kit::NSTextField;
	use objc2_app_kit::NSView;
	use objc2_app_kit::NSWindow;
	use objc2_app_kit::NSWindowStyleMask;
	use objc2_foundation::NSPoint;
	use objc2_foundation::NSRect;
	use objc2_foundation::NSSize;
	use objc2_foundation::NSString;
	use raw_window_handle::AppKitWindowHandle;
	use raw_window_handle::DisplayHandle;
	use raw_window_handle::HandleError;
	use raw_window_handle::HasDisplayHandle;
	use raw_window_handle::HasWindowHandle;
	use raw_window_handle::WindowHandle;

	thread_local! {
		/// The status label, owned by the main thread for the whole run.
		static STATUS: OnceCell<Retained<NSTextField>> = const { OnceCell::new() };
	}

	/// The fixture window's content view as a dialog parent. The window is
	/// created before any dialog and lives until the process exits.
	#[derive(Clone, Copy)]
	struct Parent(NonNull<c_void>);

	// SAFETY: the pointer is only an identity here. rfd turns it back into
	// the view's window and presents every sheet on the main thread.
	unsafe impl Send for Parent {}

	impl HasWindowHandle for Parent {
		fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
			let raw = AppKitWindowHandle::new(self.0);
			// SAFETY: the view outlives every dialog it presents.
			Ok(unsafe { WindowHandle::borrow_raw(raw.into()) })
		}
	}

	impl HasDisplayHandle for Parent {
		fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
			Ok(DisplayHandle::appkit())
		}
	}

	define_class!(
		// SAFETY:
		// - `NSObject` has no subclassing requirements.
		// - The class does not implement `Drop`.
		#[unsafe(super(NSObject))]
		#[thread_kind = MainThreadOnly]
		#[ivars = Parent]
		/// The target of both buttons' actions.
		struct Actions;

		unsafe impl NSObjectProtocol for Actions {}

		impl Actions {
			#[unsafe(method(choose:))]
			fn choose(&self, _sender: Option<&AnyObject>) {
				present(*self.ivars(), true);
			}

			#[unsafe(method(confirm:))]
			fn confirm(&self, _sender: Option<&AnyObject>) {
				present(*self.ivars(), false);
			}
		}
	);

	impl Actions {
		fn new(mtm: MainThreadMarker, parent: Parent) -> Retained<Self> {
			let this = Self::alloc(mtm).set_ivars(parent);
			// SAFETY: `init` is `NSObject`'s designated initializer.
			unsafe { msg_send![super(this), init] }
		}
	}

	/// The directory beside the application bundle, where the runner puts
	/// the evidence file. An open panel does not browse inside a bundle.
	fn evidence_directory() -> Option<PathBuf> {
		let executable = std::env::current_exe().ok()?;
		let bundle = executable
			.ancestors()
			.find(|path| path.extension().is_some_and(|extension| extension == "app"))?;
		bundle.parent().map(std::path::Path::to_path_buf)
	}

	/// Presents a dialog from a worker thread and shows its result. rfd
	/// begins the sheet on the main thread, whose run loop must keep running
	/// until the sheet completes, so the main thread cannot wait for it.
	fn present(parent: Parent, choose: bool) {
		std::thread::spawn(move || {
			let text = if choose {
				crate::dialogs::choose_evidence(&parent, evidence_directory())
			} else {
				crate::dialogs::confirm(&parent)
			};
			dispatch2::run_on_main(move |_| {
				STATUS.with(|status| {
					if let Some(status) = status.get() {
						status.setStringValue(&NSString::from_str(&text));
					}
				});
			});
		});
	}

	fn button(
		mtm: MainThreadMarker,
		title: &str,
		target: &Actions,
		action: objc2::runtime::Sel,
		y: f64,
	) -> Retained<NSButton> {
		// SAFETY: the target is retained by the caller for the whole run,
		// and both actions take one optional sender.
		let button = unsafe {
			NSButton::buttonWithTitle_target_action(
				&NSString::from_str(title),
				Some(target),
				Some(action),
				mtm,
			)
		};
		button.setFrame(NSRect::new(NSPoint::new(24.0, y), NSSize::new(240.0, 32.0)));
		button
	}

	pub fn run() {
		let mtm = MainThreadMarker::new().expect("AppKit runs on the main thread");
		let app = NSApplication::sharedApplication(mtm);
		app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
		// SAFETY: a buffered, non-deferred window with a valid content
		// rectangle and style mask is the standard configuration; it is
		// never closed before the process exits.
		let window = unsafe {
			NSWindow::initWithContentRect_styleMask_backing_defer(
				NSWindow::alloc(mtm),
				NSRect::new(NSPoint::ZERO, NSSize::new(560.0, 240.0)),
				NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
				NSBackingStoreType::Buffered,
				false,
			)
		};
		// SAFETY: `Retained` owns the window, so AppKit must not release it
		// when it closes.
		unsafe { window.setReleasedWhenClosed(false) };
		window.setTitle(&NSString::from_str("Leptoskit dialog fixture"));
		let content = NSView::new(mtm);
		window.setContentView(Some(&content));
		let parent = Parent(NonNull::from(&*content).cast());
		let actions = Actions::new(mtm, parent);
		content.addSubview(&button(
			mtm,
			"Choose evidence",
			&actions,
			sel!(choose:),
			160.0,
		));
		content.addSubview(&button(mtm, "Confirm", &actions, sel!(confirm:), 112.0));
		let status = NSTextField::labelWithString(&NSString::from_str("Ready"), mtm);
		status.setFrame(NSRect::new(
			NSPoint::new(24.0, 56.0),
			NSSize::new(480.0, 32.0),
		));
		content.addSubview(&status);
		STATUS.with(|cell| cell.set(status).expect("the status label is set once"));
		window.center();
		window.makeKeyAndOrderFront(None);
		#[allow(deprecated)]
		app.activateIgnoringOtherApps(true);
		app.run();
	}
}
