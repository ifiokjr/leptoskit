//! The Win32 host: snapshots and live windows on Windows.

use std::cell::Cell;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gallery::SCREENS;
use gallery::native::NativeGallery;
use gallery::native::screen;
use leptoskit::windows::WindowsBackend;
use leptoskit::windows::WindowsWindow;
use leptoskit_yoga::NodeId;

use crate::assets;

type Session = Rc<RefCell<NativeGallery<WindowsBackend>>>;

/// Mounts the session's root in `window` if a commit replaced it.
fn remount(window: &WindowsWindow, session: &Session, mounted: &Cell<Option<NodeId>>) {
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

/// Opens a hidden window showing `slug`, with events and resizes routed into
/// its session.
fn open(slug: &str) -> Result<(Rc<WindowsWindow>, Session), String> {
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
		backend,
		screen,
		assets()?,
		viewport,
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
	Ok((window, session))
}

pub fn window(slug: &str) -> Result<(), String> {
	let (window, _session) = open(slug)?;
	window.run().map_err(|error| error.to_string())
}

pub fn snapshot(out_dir: &Path) -> Result<(), String> {
	std::fs::create_dir_all(out_dir)
		.map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
	for screen in &SCREENS {
		let (window, _session) = open(screen.slug)?;
		window.show();
		let png = window
			.snapshot_png()
			.map_err(|error| format!("could not snapshot `{}`: {error}", screen.slug))?;
		let path = out_dir.join(format!("{}.png", screen.slug));
		std::fs::write(&path, png)
			.map_err(|error| format!("could not write {}: {error}", path.display()))?;
		println!("{}", path.display());
	}
	Ok(())
}
