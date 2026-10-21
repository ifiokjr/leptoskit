//! The GTK 4 host: snapshots and live windows on Linux.

use std::cell::Cell;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gallery::SCREENS;
use gallery::native::NativeGallery;
use gallery::native::screen;
use leptoskit::gtk::GtkBackend;
use leptoskit::gtk::gtk;
use leptoskit::gtk::gtk::prelude::*;

use crate::assets;

type Session = Rc<RefCell<NativeGallery<GtkBackend>>>;

/// How long a mapped window may take to paint its first frame.
const PAINT_TIMEOUT: Duration = Duration::from_secs(5);

/// Mounts `slug` and routes widget events and window resizes into it.
fn session(slug: &str) -> Result<Session, String> {
	let screen = screen(slug)?;
	let session = Rc::new(RefCell::new(NativeGallery::new(
		GtkBackend::new(),
		screen,
		assets()?,
		screen.viewport,
	)));
	{
		let gallery = session.borrow();
		let backend = gallery.surface.backend();
		// GTK delivers both callbacks from the GLib loop, outside any borrow
		// of the session.
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
	Ok(session)
}

fn open(session: &Session) -> Result<gtk::Window, String> {
	let gallery = session.borrow();
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
	let weak = Rc::downgrade(session);
	window.connect_scale_factor_notify(move |_| {
		if let Some(session) = weak.upgrade() {
			session.borrow_mut().surface.refresh_text_measurer();
		}
	});
	Ok(window)
}

pub fn window(slug: &str) -> Result<(), String> {
	gtk::init().map_err(|error| error.to_string())?;
	let session = session(slug)?;
	let _window = open(&session)?;
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

pub fn snapshot(out_dir: &Path) -> Result<(), String> {
	gtk::init().map_err(|error| error.to_string())?;
	std::fs::create_dir_all(out_dir)
		.map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
	for screen in &SCREENS {
		let session = session(screen.slug)?;
		let window = open(&session)?;
		let png = painted_png(&session, &window);
		window.close();
		let png = png.map_err(|error| format!("could not snapshot `{}`: {error}", screen.slug))?;
		let path = out_dir.join(format!("{}.png", screen.slug));
		std::fs::write(&path, png)
			.map_err(|error| format!("could not write {}: {error}", path.display()))?;
		println!("{}", path.display());
	}
	Ok(())
}
