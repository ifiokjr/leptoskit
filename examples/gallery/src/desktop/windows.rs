//! The Win32 host: snapshots and live windows on Windows.
//!
//! Every function runs on the thread that owns the window. MSVC builds of
//! the `gallery` binary embed the backend's manifest (see `build.rs`).
#![allow(unsafe_code)]

use std::cell::Cell;
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use leptoskit::windows::WindowsBackend;
use leptoskit::windows::WindowsWindow;
use leptoskit_yoga::NodeId;
use windows::Win32::UI::WindowsAndMessaging::BM_CLICK;
use windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST;
use windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE;
use windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE;
use windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE;
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
use windows::Win32::UI::WindowsAndMessaging::SetWindowPos;

use crate::Assets;
use crate::SCREENS;
use crate::native::NativeGallery;
use crate::native::default_assets;
use crate::native::screen;

/// A live gallery session on the window's thread.
pub type Session = Rc<RefCell<NativeGallery<WindowsBackend>>>;

/// A screen shown in a Win32 window, with controls and resizes wired to it.
pub struct Host {
	// Declared first so the widgets are destroyed before their host window.
	session: Session,
	window: Rc<WindowsWindow>,
}

impl Host {
	/// The session the native controls update.
	pub fn session(&self) -> &Session {
		&self.session
	}

	/// Clicks the native `BUTTON` for `node` with `BM_CLICK`, as a pointer
	/// click does. Returns `false` when `node` is not a button or pressable.
	pub fn press(&self, node: NodeId) -> bool {
		// The click re-renders the session synchronously, so release the
		// borrow first.
		let control = {
			let gallery = self.session.borrow();
			if !gallery.is_pressable(node) {
				return false;
			}
			gallery.surface.backend().control(node)
		};
		let Some(button) = control else {
			return false;
		};
		// SAFETY: `button` is a live child control of this host's window, and
		// BM_CLICK takes no parameters.
		unsafe {
			SendMessageW(button, BM_CLICK, None, None);
		}
		true
	}
}

/// Mounts the session's root in `window` if a commit replaced it.
fn remount(window: &WindowsWindow, session: &Session, mounted: &Cell<Option<NodeId>>) {
	// Destroying the window can still deliver a final blur or resize.
	if !window.is_open() {
		return;
	}
	let gallery = session.borrow();
	let root = gallery
		.surface
		.root()
		.expect("a reconciled gallery screen has a root");
	if mounted.replace(Some(root)) != Some(root) {
		window
			.mount(gallery.surface.backend(), root)
			.expect("the live gallery window accepts its replacement root");
	}
}

/// Shows `slug` in a window sized to its viewport, with events and resizes
/// routed into its state.
///
/// # Errors
/// Returns a message when the window or its widgets cannot be created.
pub fn open(slug: &str, assets: Assets) -> Result<Host, String> {
	let screen = screen(slug)?;
	let (width, height) = screen.viewport;
	let window = Rc::new(
		WindowsWindow::new(
			&format!("leptoskit · {}", screen.title),
			width as i32,
			height as i32,
		)
		.map_err(|error| error.to_string())?,
	);
	let mut backend = WindowsBackend::new().map_err(|error| error.to_string())?;
	backend.set_dpi(window.dpi());
	let viewport = window.client_size().map_err(|error| error.to_string())?;
	let session = Rc::new(RefCell::new(NativeGallery::new(
		backend, screen, assets, viewport,
	)));
	let root = session
		.borrow()
		.surface
		.root()
		.ok_or("the screen committed no root")?;
	window
		.mount(session.borrow().surface.backend(), root)
		.map_err(|error| error.to_string())?;
	let mounted = Rc::new(Cell::new(Some(root)));

	// Both callbacks run from the window procedure, outside any borrow of the
	// session.
	let (weak, weak_window, events_mounted) = (
		Rc::downgrade(&session),
		Rc::downgrade(&window),
		Rc::clone(&mounted),
	);
	session
		.borrow_mut()
		.surface
		.backend_mut()
		.set_event_listener(Rc::new(move |node, event| {
			if let (Some(session), Some(window)) = (weak.upgrade(), weak_window.upgrade()) {
				session.borrow_mut().dispatch(node, &event);
				remount(&window, &session, &events_mounted);
			}
		}));
	let (weak, weak_window) = (Rc::downgrade(&session), Rc::downgrade(&window));
	let dpi = Cell::new(window.dpi());
	window.set_resize_listener(Rc::new(move |width, height, new_dpi| {
		if let (Some(session), Some(window)) = (weak.upgrade(), weak_window.upgrade()) {
			{
				let mut gallery = session.borrow_mut();
				// A monitor change alters text metrics: remeasure before
				// layout.
				if dpi.replace(new_dpi) != new_dpi {
					gallery.surface.backend_mut().set_dpi(new_dpi);
					gallery.surface.refresh_text_measurer();
				}
				gallery.surface.set_viewport(width, height);
			}
			remount(&window, &session, &mounted);
		}
	}));
	window.show();
	Ok(Host { session, window })
}

/// Shows `slug` in a window and runs its message loop until it closes.
///
/// # Errors
/// Returns a message when the screen cannot be shown or the loop fails.
pub fn window(slug: &str) -> Result<(), String> {
	let host = open(slug, default_assets()?)?;
	host.window.run().map_err(|error| error.to_string())
}

/// Captures every screen's window pixels to `<out_dir>/<screen>.png` and
/// returns the paths.
///
/// # Errors
/// Returns a message when a window cannot be captured or a snapshot cannot be
/// written.
pub fn snapshot(out_dir: &Path) -> Result<Vec<PathBuf>, String> {
	let mut written = Vec::with_capacity(SCREENS.len());
	let assets = default_assets()?;
	std::fs::create_dir_all(out_dir)
		.map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
	for screen in &SCREENS {
		let host = open(screen.slug, assets.clone())?;
		// The capture reads screen pixels, so keep the window above any
		// console or runner window.
		// SAFETY: The host owns this live top-level window.
		unsafe {
			SetWindowPos(
				host.window.handle(),
				Some(HWND_TOPMOST),
				0,
				0,
				0,
				0,
				SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
			)
		}
		.map_err(|error| error.to_string())?;
		let png = host
			.window
			.snapshot_png()
			.map_err(|error| format!("could not snapshot `{}`: {error}", screen.slug))?;
		let path = out_dir.join(format!("{}.png", screen.slug));
		std::fs::write(&path, png)
			.map_err(|error| format!("could not write {}: {error}", path.display()))?;
		written.push(path);
	}
	Ok(written)
}
