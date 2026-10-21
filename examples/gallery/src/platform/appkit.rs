//! The AppKit host: snapshots and live windows on macOS.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gallery::SCREENS;
use gallery::native::NativeGallery;
use gallery::native::screen;
use leptoskit::appkit::AppKitBackend;
use leptoskit::appkit::MainThreadMarker;
use leptoskit::appkit::run_application;

use crate::assets;

/// The pixels per point of every snapshot: retina quality.
const SNAPSHOT_SCALE: f64 = 2.0;

fn main_thread() -> Result<MainThreadMarker, String> {
	MainThreadMarker::new().ok_or_else(|| "AppKit must run on the main thread".to_owned())
}

pub fn snapshot(out_dir: &Path) -> Result<(), String> {
	let mtm = main_thread()?;
	let assets = assets()?;
	std::fs::create_dir_all(out_dir)
		.map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
	for screen in &SCREENS {
		let gallery = NativeGallery::new(
			AppKitBackend::new(mtm),
			screen,
			assets.clone(),
			screen.viewport,
		);
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
		println!("{}", path.display());
	}
	Ok(())
}

pub fn window(slug: &str) -> Result<(), String> {
	let mtm = main_thread()?;
	let screen = screen(slug)?;
	let gallery = Rc::new(RefCell::new(NativeGallery::new(
		AppKitBackend::new(mtm),
		screen,
		assets()?,
		screen.viewport,
	)));
	let root = gallery
		.borrow()
		.surface
		.root()
		.ok_or("the screen committed no root")?;

	// Events arrive from the AppKit run loop, never while the session is
	// borrowed, so the listener can dispatch and re-render in place.
	let weak = Rc::downgrade(&gallery);
	gallery
		.borrow()
		.surface
		.backend()
		.set_event_listener(move |node, event| {
			if let Some(gallery) = weak.upgrade() {
				gallery.borrow_mut().dispatch(node, event);
			}
		});

	let _window = gallery
		.borrow()
		.surface
		.backend()
		.open_window(root, &format!("leptoskit · {}", screen.title))
		.ok_or("the root view disappeared before the window opened")?;
	run_application(mtm);
	Ok(())
}
