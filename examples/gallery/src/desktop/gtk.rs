//! The GTK 4 host: snapshots and live windows on Linux.
//!
//! Every function needs a display and runs on the thread that initialized
//! GTK; [`open`] initializes it on first use.

use std::cell::Cell;
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use leptoskit::gtk::GtkBackend;
use leptoskit::gtk::gtk;
use leptoskit::gtk::gtk::prelude::*;
use leptoskit_yoga::NodeId;

use crate::Assets;
use crate::SCREENS;
use crate::native::NativeGallery;
use crate::native::default_assets;
use crate::native::screen;

/// A live gallery session on the GTK thread.
pub type Session = Rc<RefCell<NativeGallery<GtkBackend>>>;

/// How long a mapped window may take to paint its first frame.
const PAINT_TIMEOUT: Duration = Duration::from_secs(5);

/// A screen mounted in GTK widgets, with controls and resizes wired to it.
pub struct Host {
	session: Session,
}

impl Host {
	/// The session the native controls update.
	pub fn session(&self) -> &Session {
		&self.session
	}

	/// Clicks the native `GtkButton` for `node`, emitting the same signal a
	/// pointer click does. Returns `false` when `node` is not a button or
	/// pressable.
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
		let Some(button) = control.and_then(|control| control.downcast::<gtk::Button>().ok())
		else {
			return false;
		};
		button.emit_clicked();
		true
	}

	/// Shows the screen in a window sized to its viewport.
	fn show(&self) -> Result<gtk::Window, String> {
		let gallery = self.session.borrow();
		let root = gallery
			.surface
			.root()
			.ok_or("the screen committed no root")?;
		let (width, height) = gallery.screen.viewport;
		let window = gallery
			.surface
			.backend()
			.open_window(
				root,
				&format!("leptoskit · {}", gallery.screen.title),
				width as i32,
				height as i32,
			)
			.ok_or("the root view disappeared before the window opened")?;
		// Monitor scale changes alter text metrics, so remeasure.
		let weak = Rc::downgrade(&self.session);
		window.connect_scale_factor_notify(move |_| {
			if let Some(session) = weak.upgrade() {
				session.borrow_mut().surface.refresh_text_measurer();
			}
		});
		Ok(window)
	}
}

/// Mounts `slug` at its viewport and routes widget events and window
/// resizes into its state.
///
/// # Errors
/// Returns a message when GTK cannot start or `slug` names no screen.
pub fn open(slug: &str, assets: Assets) -> Result<Host, String> {
	let screen = screen(slug)?;
	gtk::init().map_err(|error| format!("GTK could not start: {error}"))?;
	let session = Rc::new(RefCell::new(NativeGallery::new(
		GtkBackend::new(),
		screen,
		assets,
		screen.viewport,
	)));
	{
		let gallery = session.borrow();
		let backend = gallery.surface.backend();
		// GTK delivers both callbacks outside any borrow of the session.
		let weak = Rc::downgrade(&session);
		backend.set_event_listener(move |node, event| {
			if let Some(session) = weak.upgrade() {
				session.borrow_mut().dispatch(node, &event);
			}
		});
		let weak = Rc::downgrade(&session);
		backend.set_resize_listener(move |width, height| {
			if let Some(session) = weak.upgrade() {
				session.borrow_mut().surface.set_viewport(width, height);
			}
		});
	}
	Ok(Host { session })
}

/// Shows `slug` in a window and runs GTK until it closes.
///
/// # Errors
/// Returns a message when the screen cannot be mounted or shown.
pub fn window(slug: &str) -> Result<(), String> {
	let host = open(slug, default_assets()?)?;
	let _window = host.show()?;
	leptoskit::gtk::run_application();
	Ok(())
}

/// Renders the window's root with GTK's own renderer after its first paint.
fn painted_png(session: &Session, window: &gtk::Window) -> Result<Vec<u8>, String> {
	let clock = window
		.frame_clock()
		.ok_or("the mapped window has no frame clock")?;
	let result = Rc::new(RefCell::new(None));
	let painted = clock.connect_after_paint({
		let result = Rc::clone(&result);
		let session = Rc::clone(session);
		move |_| {
			if result.borrow().is_some() {
				return;
			}
			let gallery = session.borrow();
			let png = gallery
				.surface
				.root()
				.ok_or_else(|| "the painted screen has no root".to_owned())
				.and_then(|root| {
					gallery
						.surface
						.backend()
						.snapshot_png(root)
						.map_err(|error| error.to_string())
				});
			result.replace(Some(png));
		}
	});
	clock.request_phase(gtk::gdk::FrameClockPhase::AFTER_PAINT);

	let timed_out = Rc::new(Cell::new(false));
	let timeout = gtk::glib::timeout_add_local_once(PAINT_TIMEOUT, {
		let timed_out = Rc::clone(&timed_out);
		move || timed_out.set(true)
	});
	let context = gtk::glib::MainContext::default();
	while result.borrow().is_none() && !timed_out.get() {
		context.iteration(true);
	}
	clock.disconnect(painted);
	if !timed_out.get() {
		timeout.remove();
	}
	result.take().unwrap_or_else(|| {
		Err(format!(
			"the window did not paint within {} seconds",
			PAINT_TIMEOUT.as_secs()
		))
	})
}

/// Renders every screen with GTK's renderer to `<out_dir>/<screen>.png` and
/// returns the paths.
///
/// # Errors
/// Returns a message when a window does not paint or a snapshot cannot be
/// written.
pub fn snapshot(out_dir: &Path) -> Result<Vec<PathBuf>, String> {
	let mut written = Vec::with_capacity(SCREENS.len());
	let assets = default_assets()?;
	std::fs::create_dir_all(out_dir)
		.map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
	for screen in &SCREENS {
		let host = open(screen.slug, assets.clone())?;
		let window = host.show()?;
		let png = painted_png(&host.session, &window);
		window.close();
		let png = png.map_err(|error| format!("could not snapshot `{}`: {error}", screen.slug))?;
		let path = out_dir.join(format!("{}.png", screen.slug));
		std::fs::write(&path, png)
			.map_err(|error| format!("could not write {}: {error}", path.display()))?;
		written.push(path);
	}
	Ok(written)
}
