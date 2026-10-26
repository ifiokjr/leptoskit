//! The terminal host: the shared screens in an interactive cell grid.
//!
//! The same `Screen` functions the widget backends render drive this host;
//! `gallery-tui <screen>` runs one live, with keyboard focus, carets, and
//! scrolling owned by the backend.

use leptoskit::tui::TuiBackend;
use leptoskit::tui::TuiOptions;
use leptoskit::tui::host::TuiApplication;
use leptoskit::tui::terminal;
use leptoskit_native::NativeEvent;
use leptoskit_yoga::NodeId;

use crate::Assets;
use crate::Screen;
use crate::native::NativeGallery;

/// Runs one gallery screen live in the process's terminal.
///
/// # Errors
///
/// Returns an error when the terminal cannot be prepared, read, or written.
pub fn run(screen: &'static Screen, assets: Assets) -> Result<(), String> {
	let session = NativeGallery::new(TuiBackend::new(), screen, assets, screen.viewport);
	let options = TuiOptions {
		title: format!("leptoskit — {}", screen.title),
		..TuiOptions::default()
	};

	terminal::run(options, move || session.screen.render(&session.state))
		.map(|_| ())
		.map_err(|error| error.to_string())
}

/// The gallery session as a host application, for tests that drive screens
/// headlessly through the terminal host.
pub struct GalleryApp {
	/// The screen, its state, and its surface.
	pub session: NativeGallery<TuiBackend>,
}

impl TuiApplication for GalleryApp {
	fn backend(&mut self) -> &mut TuiBackend {
		self.session.surface.backend_mut()
	}

	fn dispatch(&mut self, node: NodeId, event: &NativeEvent) {
		self.session.dispatch(node, event);
	}

	fn resize(&mut self, cols: u16, rows: u16) {
		let (width, height) = leptoskit::tui::viewport(cols, rows);
		self.session.surface.set_viewport(width, height);
	}

	fn mount(&mut self) {
		self.session
			.surface
			.set_root(&self.session.screen.render(&self.session.state));
	}
}

/// The cell-grid size for a screen's point viewport.
#[must_use]
pub fn cells(viewport: (f32, f32)) -> (u16, u16) {
	(
		(viewport.0 / leptoskit::tui::text::CELL_WIDTH).round() as u16,
		(viewport.1 / leptoskit::tui::text::CELL_HEIGHT).round() as u16,
	)
}
