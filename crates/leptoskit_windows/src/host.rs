//! A minimal desktop host; applications retain their `NativeSurface`
//! separately.

use std::cell::Cell;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::rc::Weak;

use leptoskit_yoga::NodeId;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;
use windows::core::w;

use crate::backend::Widget;
use crate::backend::WindowsBackend;
use crate::backend::wide;

type ResizeListener = Rc<dyn Fn(f32, f32, u32)>;

#[derive(Default)]
struct HostState {
	resize: RefCell<Option<ResizeListener>>,
	root: RefCell<Weak<Widget>>,
	dpi: Cell<u32>,
	backend_host: RefCell<Option<Rc<Cell<HWND>>>>,
	parking: Cell<HWND>,
	closed: Cell<bool>,
}

/// An owned Win32 top-level window and message loop on the current UI thread.
pub struct WindowsWindow {
	hwnd: HWND,
	state: Rc<HostState>,
	_thread: PhantomData<Rc<()>>,
}

impl std::fmt::Debug for WindowsWindow {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WindowsWindow")
			.field("hwnd", &self.hwnd)
			.finish_non_exhaustive()
	}
}

impl WindowsWindow {
	/// Creates a hidden window whose client area has the requested logical
	/// size.
	pub fn new(title: &str, width: i32, height: i32) -> windows::core::Result<Self> {
		let state = Rc::new(HostState::default());
		// SAFETY: The class callback uses only Windows messages until userdata
		// is installed; the Rc ownership is released during
		// WM_NCDESTROY.
		unsafe {
			let instance = GetModuleHandleW(None)?;
			let class = WNDCLASSW {
				lpfnWndProc: Some(window_proc),
				hInstance: instance.into(),
				lpszClassName: w!("Leptoskit.NativeWindow"),
				hCursor: LoadCursorW(None, IDC_ARROW)?,
				hbrBackground: GetSysColorBrush(COLOR_WINDOW),
				..Default::default()
			};

			if RegisterClassW(&raw const class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS
			{
				return Err(windows::core::Error::from_win32());
			}

			let dpi = GetDpiForSystem();
			let scale = dpi as f32 / 96.0;
			let mut bounds = RECT {
				left: 0,
				top: 0,
				right: (width as f32 * scale).round() as i32,
				bottom: (height as f32 * scale).round() as i32,
			};
			AdjustWindowRectExForDpi(
				&raw mut bounds,
				WS_OVERLAPPEDWINDOW,
				false,
				WINDOW_EX_STYLE::default(),
				dpi,
			)?;
			let hwnd = CreateWindowExW(
				WINDOW_EX_STYLE::default(),
				w!("Leptoskit.NativeWindow"),
				PCWSTR(wide(title).as_ptr()),
				WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
				CW_USEDEFAULT,
				CW_USEDEFAULT,
				bounds.right - bounds.left,
				bounds.bottom - bounds.top,
				None,
				None,
				Some(instance.into()),
				None,
			)?;
			state.dpi.set(GetDpiForWindow(hwnd));
			SetWindowLongPtrW(
				hwnd,
				GWLP_USERDATA,
				Rc::into_raw(Rc::clone(&state)) as isize,
			);
			Ok(Self {
				hwnd,
				state,
				_thread: PhantomData,
			})
		}
	}

	/// The host handle for integrations with Windows APIs.
	pub fn handle(&self) -> HWND {
		self.hwnd
	}

	/// Whether this owned host is still open. A destroyed HWND cannot become
	/// open again if Windows later reuses its numeric value for another window.
	pub fn is_open(&self) -> bool {
		!self.state.closed.get()
	}

	/// Closes this host on its owning UI thread and ends its message loop.
	/// Final native focus and resize events can still arrive while it closes.
	pub fn close(&self) -> windows::core::Result<()> {
		if self.state.closed.replace(true) {
			return Ok(());
		}

		// SAFETY: This host owns the live HWND on its creating thread. Marking
		// it closed first keeps final native callbacks from remounting it.
		if let Err(error) = unsafe { DestroyWindow(self.hwnd) } {
			self.state.closed.set(false);

			return Err(error);
		}

		Ok(())
	}

	/// Parks the currently mounted tree without destroying the backend's
	/// widgets.
	pub fn unmount(&self) {
		let root = std::mem::take(&mut *self.state.root.borrow_mut()).upgrade();

		if let Some(root) = root {
			let _suppressed = root.sink.suppress();
			root.host.set(root.parking);
			root.place_tree();
		}
	}

	/// Waits without polling the UI. `None` waits for a native message; a
	/// timeout lets an application service workers before returning to the
	/// message pump.
	pub fn wait_for_messages(
		&self,
		timeout: Option<std::time::Duration>,
	) -> windows::core::Result<()> {
		if !self.is_open() {
			return Ok(());
		}

		// SAFETY: No native resources cross threads. The message queue belongs
		// to this host's creating UI thread and the handles slice is
		// empty.
		unsafe {
			if let Some(timeout) = timeout {
				let result = MsgWaitForMultipleObjectsEx(
					None,
					// Round up so a sub-millisecond frame deadline sleeps
					// instead of repeatedly polling with a zero timeout.
					timeout
						.as_nanos()
						.div_ceil(1_000_000)
						.min(u128::from(u32::MAX)) as u32,
					QS_ALLINPUT,
					MWMO_INPUTAVAILABLE,
				);

				if result == WAIT_FAILED {
					return Err(windows::core::Error::from_win32());
				}
			} else {
				WaitMessage()?;
			}
		}
		Ok(())
	}

	/// The host's current monitor DPI. Yoga sizes remain 96-DPI logical units.
	pub fn dpi(&self) -> u32 {
		self.state.dpi.get()
	}

	/// Whether this application window is foreground, visible, and restored.
	pub fn is_active(&self) -> bool {
		// SAFETY: The owned handle is queried only on its UI thread.
		self.is_open()
			&& unsafe {
				IsWindowVisible(self.hwnd).as_bool()
					&& !IsIconic(self.hwnd).as_bool()
					&& GetForegroundWindow() == self.hwnd
			}
	}

	/// The current Windows client-area animation preference.
	pub fn animations_enabled(&self) -> windows::core::Result<bool> {
		let mut enabled = windows::core::BOOL::default();
		// SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to this local
		// value.
		unsafe {
			SystemParametersInfoW(
				SPI_GETCLIENTAREAANIMATION,
				0,
				Some(std::ptr::from_mut(&mut enabled).cast()),
				SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
			)?;
		}
		Ok(enabled.as_bool())
	}

	/// The actual client area expressed in logical layout units.
	pub fn client_size(&self) -> windows::core::Result<(f32, f32)> {
		if !self.is_open() {
			return Err(E_HANDLE.into());
		}

		let mut rect = RECT::default();
		// SAFETY: The output rectangle belongs to this still-live host window.
		unsafe {
			GetClientRect(self.hwnd, &raw mut rect)?;
		}
		let scale = self.dpi() as f32 / 96.0;
		Ok((rect.right as f32 / scale, rect.bottom as f32 / scale))
	}

	/// Mounts a committed tree. The backend continues owning the widget HWNDs.
	pub fn mount(&self, backend: &WindowsBackend, root: NodeId) -> windows::core::Result<()> {
		if !self.is_open() || backend.view(root).is_none() {
			return Err(E_HANDLE.into());
		}
		*self.state.root.borrow_mut() = Rc::downgrade(
			backend
				.widget(root)
				.expect("mount requires a committed root"),
		);
		let widget = backend
			.widget(root)
			.expect("mount requires a committed root");
		*self.state.backend_host.borrow_mut() = Some(Rc::clone(&widget.host));
		self.state.parking.set(widget.parking);
		backend.mount(root, self.hwnd, self.dpi());
		Ok(())
	}

	/// Receives logical client width, height, and monitor DPI on resize. On DPI
	/// transitions refresh the surface text measurer before setting its
	/// viewport.
	pub fn set_resize_listener(&self, listener: ResizeListener) {
		*self.state.resize.borrow_mut() = Some(listener);
	}

	/// Shows the window without transferring ownership.
	pub fn show(&self) {
		if !self.is_open() {
			return;
		}

		// SAFETY: The handle remains owned by this host on its current thread.
		unsafe {
			let _ = ShowWindow(self.hwnd, SW_SHOW);
			let _ = UpdateWindow(self.hwnd);
		}
		// Translucent groups placed while this host was hidden paint into
		// their own surfaces only now that they are visible.
		let root = self.state.root.borrow().upgrade();

		if let Some(root) = root {
			root.repaint_layers();
		}
	}

	/// Drains messages currently queued for this UI thread, without blocking.
	pub fn pump_messages(&self) {
		if !self.is_open() {
			return;
		}

		// SAFETY: The local message structure remains alive during dispatch.
		unsafe {
			let mut message = MSG::default();

			while PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE).as_bool() {
				if message.message != WM_QUIT
					&& !IsDialogMessageW(self.hwnd, &raw const message).as_bool()
				{
					let _ = TranslateMessage(&raw const message);
					DispatchMessageW(&raw const message);
				}
			}
		}
	}

	/// Runs the native message loop until the user closes the window.
	pub fn run(&self) -> windows::core::Result<()> {
		if !self.is_open() {
			return Ok(());
		}

		self.show();
		// SAFETY: GetMessage fills one local MSG; it is dispatched
		// synchronously.
		unsafe {
			let mut message = MSG::default();

			loop {
				let result = GetMessageW(&raw mut message, None, 0, 0).0;

				if result == -1 {
					return Err(windows::core::Error::from_win32());
				}

				if result == 0 {
					return Ok(());
				}

				if !IsDialogMessageW(self.hwnd, &raw const message).as_bool() {
					let _ = TranslateMessage(&raw const message);
					DispatchMessageW(&raw const message);
				}
			}
		}
	}

	/// Captures the host's actual client pixels as PNG, including child
	/// controls. The window must be shown and unobscured on an interactive
	/// desktop.
	pub fn snapshot_png(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
		if !self.is_open() {
			return Err(windows::core::Error::from(E_HANDLE).into());
		}

		self.pump_messages();
		let mut rect = RECT::default();
		// SAFETY: Each temporary GDI object is released after its final use;
		// the selected bitmap is restored before deletion, including
		// failure paths.
		let (width, height, mut pixels) = unsafe {
			GetClientRect(self.hwnd, &raw mut rect)?;
			let width = (rect.right - rect.left).max(1);
			let height = (rect.bottom - rect.top).max(1);
			let mut origin = POINT::default();
			let _ = ClientToScreen(self.hwnd, &raw mut origin);
			let source = GetDC(None);
			let dc = CreateCompatibleDC(Some(source));
			let bitmap = CreateCompatibleBitmap(source, width, height);

			if dc.is_invalid() || bitmap.is_invalid() {
				if !bitmap.is_invalid() {
					let _ = DeleteObject(bitmap.into());
				}

				if !dc.is_invalid() {
					let _ = DeleteDC(dc);
				}

				ReleaseDC(None, source);

				return Err(windows::core::Error::from_win32().into());
			}

			let old = SelectObject(dc, bitmap.into());
			let copied = BitBlt(
				dc,
				0,
				0,
				width,
				height,
				Some(source),
				origin.x,
				origin.y,
				SRCCOPY | CAPTUREBLT,
			);
			SelectObject(dc, old);
			let mut info = BITMAPINFO {
				bmiHeader: BITMAPINFOHEADER {
					biSize: size_of::<BITMAPINFOHEADER>() as u32,
					biWidth: width,
					biHeight: -height,
					biPlanes: 1,
					biBitCount: 32,
					biCompression: BI_RGB.0,
					..Default::default()
				},
				..Default::default()
			};
			let mut pixels = vec![0; width as usize * height as usize * 4];
			let read = GetDIBits(
				dc,
				bitmap,
				0,
				height as u32,
				Some(pixels.as_mut_ptr().cast()),
				&raw mut info,
				DIB_RGB_COLORS,
			);
			let _ = DeleteObject(bitmap.into());
			let _ = DeleteDC(dc);
			ReleaseDC(None, source);
			copied?;

			if read == 0 {
				return Err(windows::core::Error::from_win32().into());
			}

			(width as u32, height as u32, pixels)
		};

		for pixel in pixels.as_chunks_mut::<4>().0 {
			pixel.swap(0, 2);
			pixel[3] = 255;
		}

		let image = image::RgbaImage::from_raw(width, height, pixels)
			.expect("capture has exactly four bytes per pixel");
		let mut output = std::io::Cursor::new(Vec::new());
		image::DynamicImage::ImageRgba8(image).write_to(&mut output, image::ImageFormat::Png)?;
		Ok(output.into_inner())
	}
}

impl Drop for WindowsWindow {
	fn drop(&mut self) {
		// SAFETY: The host only destroys its own still-live HWND.
		unsafe {
			if self.is_open() {
				let _ = DestroyWindow(self.hwnd);
			}
		}
	}
}

unsafe extern "system" fn window_proc(
	hwnd: HWND,
	message: u32,
	wparam: WPARAM,
	lparam: LPARAM,
) -> LRESULT {
	// SAFETY: Only new installs this userdata pointer, holding one Rc
	// reference.
	let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const HostState;

	if message == WM_DPICHANGED && !pointer.is_null() {
		// SAFETY: Windows supplies a suggested RECT and this retained context
		// survives callbacks that may synchronously close the window.
		let context = unsafe {
			Rc::increment_strong_count(pointer);
			Rc::from_raw(pointer)
		};
		let dpi = (wparam.0 & 0xffff) as u32;
		context.dpi.set(dpi);
		let root = context.root.borrow().upgrade();

		if let Some(root) = root {
			root.refresh_dpi(dpi);
		}

		// SAFETY: WM_DPICHANGED's lParam points to a valid suggested RECT.
		unsafe {
			let rect = &*(lparam.0 as *const RECT);
			SetWindowPos(
				hwnd,
				None,
				rect.left,
				rect.top,
				rect.right - rect.left,
				rect.bottom - rect.top,
				SWP_NOZORDER | SWP_NOACTIVATE,
			)
			.expect("host accepts the system's DPI rectangle");
		}

		return LRESULT(0);
	}

	if message == WM_SIZE && !pointer.is_null() {
		// SAFETY: Retain the context while a resize callback may destroy the
		// host.
		let context = unsafe {
			Rc::increment_strong_count(pointer);
			Rc::from_raw(pointer)
		};
		let listener = context.resize.borrow().clone();

		if let Some(listener) = listener {
			let dpi = context.dpi.get();
			let scale = dpi as f32 / 96.0;
			listener(
				(lparam.0 as u32 & 0xffff) as f32 / scale,
				((lparam.0 as u32 >> 16) & 0xffff) as f32 / scale,
				dpi,
			);
		}

		return LRESULT(0);
	}

	if message == WM_DESTROY {
		if !pointer.is_null() {
			// SAFETY: The context remains installed until WM_NCDESTROY below.
			let context = unsafe { &*pointer };

			if let Some(host) = context.backend_host.borrow().as_ref() {
				host.set(context.parking.get());
			}

			let root = context.root.borrow().upgrade();

			if let Some(root) = root {
				root.host.set(root.parking);
				root.destroy();
			}
		}

		// SAFETY: This host's close operation terminates its own thread's loop.
		unsafe {
			PostQuitMessage(0);
		}

		return LRESULT(0);
	}

	if message == WM_NCDESTROY && !pointer.is_null() {
		// SAFETY: Clear and release the installed userdata exactly once.
		unsafe {
			(*pointer).closed.set(true);
			SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
			drop(Rc::from_raw(pointer));
		}
	}

	// SAFETY: Unhandled messages pass unchanged to the original procedure.
	unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
