//! The AppKit host: snapshots and live windows on macOS.
//!
//! Every function needs the main thread, which AppKit requires.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use leptoskit::appkit::AppKitBackend;
use leptoskit::appkit::MainThreadMarker;
use leptoskit::appkit::run_application;
use leptoskit_yoga::NodeId;
use objc2::Message;
use objc2_app_kit::NSButton;

use crate::Assets;
use crate::SCREENS;
use crate::native::NativeGallery;
use crate::native::default_assets;
use crate::native::screen;

/// A live gallery session on the main thread.
pub type Session = Rc<RefCell<NativeGallery<AppKitBackend>>>;

/// The pixels per point of every snapshot: retina quality.
const SNAPSHOT_SCALE: f64 = 2.0;

/// A screen mounted in native AppKit views, with controls wired to its state.
pub struct Host {
	session: Session,
}

impl Host {
	/// The session the native controls update.
	pub fn session(&self) -> &Session {
		&self.session
	}

	/// Clicks the native button for `node` through AppKit's own action path.
	/// Returns `false` when `node` is not a button or pressable.
	pub fn press(&self, node: NodeId) -> bool {
		// The click re-renders the session synchronously, so release the
		// borrow first. A button node's view is its `NSButton`.
		let view = {
			let gallery = self.session.borrow();
			if !gallery.is_pressable(node) {
				return false;
			}
			gallery.surface.backend().view(node).map(Message::retain)
		};
		let Some(button) = view.and_then(|view| view.downcast::<NSButton>().ok()) else {
			return false;
		};
		// SAFETY: `performClick:` accepts any sender, including none.
		unsafe { button.performClick(None) };
		true
	}
}

fn main_thread() -> Result<MainThreadMarker, String> {
	MainThreadMarker::new().ok_or_else(|| "AppKit must run on the main thread".to_owned())
}

/// Mounts `slug` at its viewport and routes native events into its state.
///
/// # Errors
/// Returns a message when not on the main thread or `slug` names no screen.
pub fn open(slug: &str, assets: Assets) -> Result<Host, String> {
	let screen = screen(slug)?;
	let mtm = main_thread()?;
	let session = Rc::new(RefCell::new(NativeGallery::new(
		AppKitBackend::new(mtm),
		screen,
		assets,
		screen.viewport,
	)));
	// Events arrive from AppKit actions, never while the session is
	// borrowed, so the listener can dispatch and re-render in place.
	let weak = Rc::downgrade(&session);
	session
		.borrow()
		.surface
		.backend()
		.set_event_listener(move |node, event| {
			if let Some(session) = weak.upgrade() {
				session.borrow_mut().dispatch(node, event);
			}
		});
	Ok(Host { session })
}

/// Renders every screen with AppKit to `<out_dir>/<screen>.png` and returns
/// the paths.
///
/// # Errors
/// Returns a message when rendering or writing a snapshot fails.
pub fn snapshot(out_dir: &Path) -> Result<Vec<PathBuf>, String> {
	let mut written = Vec::with_capacity(SCREENS.len());
	let assets = default_assets()?;
	std::fs::create_dir_all(out_dir)
		.map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
	for screen in &SCREENS {
		let host = open(screen.slug, assets.clone())?;
		let gallery = host.session.borrow();
		let root = gallery
			.surface
			.root()
			.ok_or("the screen committed no root")?;
		let png = gallery
			.surface
			.backend()
			.snapshot_png(root, SNAPSHOT_SCALE)
			.map_err(|error| format!("could not snapshot `{}`: {error}", screen.slug))?;
		let path = out_dir.join(format!("{}.png", screen.slug));
		std::fs::write(&path, png)
			.map_err(|error| format!("could not write {}: {error}", path.display()))?;
		written.push(path);
	}
	Ok(written)
}

/// Shows `slug` in a window and runs AppKit until it closes.
///
/// # Errors
/// Returns a message when the screen cannot be mounted or shown.
pub fn window(slug: &str) -> Result<(), String> {
	let host = open(slug, default_assets()?)?;
	let gallery = host.session.borrow();
	let root = gallery
		.surface
		.root()
		.ok_or("the screen committed no root")?;
	let _window = gallery
		.surface
		.backend()
		.open_window(root, &format!("leptoskit · {}", gallery.screen.title))
		.ok_or("the root view disappeared before the window opened")?;
	drop(gallery);
	run_application(main_thread()?);
	Ok(())
}
