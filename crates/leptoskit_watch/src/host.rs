//! Host trait every watch backend implements.

use crate::WatchAmbient;
use crate::WatchApp;
use crate::WatchError;
use crate::WatchPage;
use crate::WatchViewport;

/// What a host presented, for tests and experimental stubs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchPresentation {
	/// An interactive page.
	Page(String),
	/// An ambient surface.
	Ambient(String),
	/// The whole app was accepted.
	App(String),
}

/// Platform host that presents watch surfaces.
///
/// Wear OS and watchOS backends implement this. Unsupported machines use
/// [`RecordingHost`], which validates and records without talking to a device
/// toolkit, so cross-platform crates still compile.
pub trait WatchHost {
	/// The constrained viewport this host draws into.
	fn viewport(&self) -> WatchViewport;

	/// Presents one interactive page.
	///
	/// # Errors
	///
	/// Returns a [`WatchError`] when validation fails or the host cannot
	/// present.
	fn present_page(&mut self, page: &WatchPage) -> Result<(), WatchError>;

	/// Presents one ambient surface.
	///
	/// # Errors
	///
	/// Returns a [`WatchError`] when validation fails or the host cannot
	/// present.
	fn present_ambient(&mut self, ambient: &WatchAmbient) -> Result<(), WatchError>;

	/// Validates and presents every surface in `app`.
	///
	/// # Errors
	///
	/// Returns the first validation or presentation error.
	fn present_app(&mut self, app: &WatchApp) -> Result<(), WatchError> {
		app.validate()?;
		if app.kind().as_str() != self.viewport().kind.as_str() {
			return Err(WatchError::Unavailable(format!(
				"host is {} but app targets {}",
				self.viewport().kind.display_name(),
				app.kind().display_name()
			)));
		}
		for page in app.pages() {
			self.present_page(page)?;
		}
		for ambient in app.ambients() {
			self.present_ambient(ambient)?;
		}
		Ok(())
	}
}

/// In-memory host used by stubs and unit tests.
#[derive(Debug, Clone)]
pub struct RecordingHost {
	viewport: WatchViewport,
	log: Vec<WatchPresentation>,
}

impl RecordingHost {
	/// Starts a recording host for `viewport`.
	#[must_use]
	pub fn new(viewport: WatchViewport) -> Self {
		Self {
			viewport,
			log: Vec::new(),
		}
	}

	/// Presentations recorded so far.
	#[must_use]
	pub fn presentations(&self) -> &[WatchPresentation] {
		&self.log
	}

	/// Clears the presentation log.
	pub fn clear(&mut self) {
		self.log.clear();
	}
}

impl WatchHost for RecordingHost {
	fn viewport(&self) -> WatchViewport {
		self.viewport
	}

	fn present_page(&mut self, page: &WatchPage) -> Result<(), WatchError> {
		page.validate()?;
		self.log.push(WatchPresentation::Page(page.id().to_owned()));
		Ok(())
	}

	fn present_ambient(&mut self, ambient: &WatchAmbient) -> Result<(), WatchError> {
		ambient.validate()?;
		self.log
			.push(WatchPresentation::Ambient(ambient.id().to_owned()));
		Ok(())
	}

	fn present_app(&mut self, app: &WatchApp) -> Result<(), WatchError> {
		app.validate()?;
		if app.kind().as_str() != self.viewport.kind.as_str() {
			return Err(WatchError::Unavailable(format!(
				"host is {} but app targets {}",
				self.viewport.kind.display_name(),
				app.kind().display_name()
			)));
		}
		for page in app.pages() {
			self.present_page(page)?;
		}
		for ambient in app.ambients() {
			self.present_ambient(ambient)?;
		}
		self.log.push(WatchPresentation::App(app.id().to_owned()));
		Ok(())
	}
}
