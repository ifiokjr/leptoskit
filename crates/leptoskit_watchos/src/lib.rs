//! <!-- {=watchosSummary|trim|linePrefix:"//! ":true} -->
//!
//! Experimental watchOS backend for `leptoskit_watch` surfaces. Builds on every
//! host as a recording stub so guests and tests compile; the Swift files under
//! `swift/` name the `WatchKit` / `WidgetKit` entry points a real device host
//! will bind later.
//!
//! <!-- {/watchosSummary} -->
//!
//! # Experimental
//!
//! watchOS packaging, simulator install, and live complication updates are
//! not implemented. Prefer [`WatchOsHost`] in tests and scaffolding; it
//! validates budgets and records presentations without Objective-C.

#![forbid(unsafe_code)]

use leptoskit_watch::RecordingHost;
use leptoskit_watch::WatchAmbient;
use leptoskit_watch::WatchError;
use leptoskit_watch::WatchHost;
use leptoskit_watch::WatchPage;
use leptoskit_watch::WatchPresentation;
use leptoskit_watch::WatchViewport;

/// watchOS host stub: validates surfaces and records presentations.
#[derive(Debug, Clone)]
pub struct WatchOsHost {
	inner: RecordingHost,
}

impl WatchOsHost {
	/// A medium Apple Watch viewport.
	#[must_use]
	pub fn new() -> Self {
		Self {
			inner: RecordingHost::new(WatchViewport::watch_os_medium()),
		}
	}

	/// Starts with an explicit viewport (for unusual faces).
	#[must_use]
	pub fn with_viewport(viewport: WatchViewport) -> Self {
		assert_eq!(
			viewport.kind,
			leptoskit_watch::WatchKind::WatchOs,
			"WatchOsHost requires a watchOS viewport"
		);
		Self {
			inner: RecordingHost::new(viewport),
		}
	}

	/// Presentations recorded so far.
	#[must_use]
	pub fn presentations(&self) -> &[WatchPresentation] {
		self.inner.presentations()
	}
}

impl Default for WatchOsHost {
	fn default() -> Self {
		Self::new()
	}
}

impl WatchHost for WatchOsHost {
	fn viewport(&self) -> WatchViewport {
		self.inner.viewport()
	}

	fn present_page(&mut self, page: &WatchPage) -> Result<(), WatchError> {
		self.inner.present_page(page)
	}

	fn present_ambient(&mut self, ambient: &WatchAmbient) -> Result<(), WatchError> {
		self.inner.present_ambient(ambient)
	}

	fn present_app(&mut self, app: &leptoskit_watch::WatchApp) -> Result<(), WatchError> {
		self.inner.present_app(app)
	}
}

/// Stable platform name used in config and the CLI.
pub const PLATFORM: &str = "watchos";

/// Human-readable product name.
pub const DISPLAY_NAME: &str = "watchOS";

#[cfg(test)]
mod tests {
	use leptoskit_native::components::Text;
	use leptoskit_watch::WatchApp;
	use leptoskit_watch::WatchKind;
	use leptoskit_watch::WatchPage;

	use super::*;

	#[test]
	fn presents_a_minimal_app() {
		let app =
			WatchApp::new("demo", WatchKind::WatchOs).page(WatchPage::new("home", Text("Hi")));
		let mut host = WatchOsHost::new();
		host.present_app(&app).unwrap();
		assert_ne!(host.presentations(), []);
	}
}
