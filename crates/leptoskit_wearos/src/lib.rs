//! <!-- {=wearosSummary|trim|linePrefix:"//! ":true} -->
//!
//! Experimental Wear OS backend for `leptoskit_watch` surfaces. Builds on every
//! host as a recording stub so guests and tests compile; the Kotlin files under
//! `android/` name the Wear OS services a real device host will bind later.
//!
//! <!-- {/wearosSummary} -->
//!
//! # Experimental
//!
//! Wear OS packaging, device install, and live tile/complication updates are
//! not implemented. Prefer [`WearOsHost`] in tests and scaffolding; it
//! validates budgets and records presentations without JNI.

#![forbid(unsafe_code)]

use leptoskit_watch::RecordingHost;
use leptoskit_watch::WatchAmbient;
use leptoskit_watch::WatchError;
use leptoskit_watch::WatchHost;
use leptoskit_watch::WatchPage;
use leptoskit_watch::WatchPresentation;
use leptoskit_watch::WatchViewport;

/// Wear OS host stub: validates surfaces and records presentations.
#[derive(Debug, Clone)]
pub struct WearOsHost {
	inner: RecordingHost,
}

impl WearOsHost {
	/// A medium round Wear OS viewport.
	#[must_use]
	pub fn new() -> Self {
		Self {
			inner: RecordingHost::new(WatchViewport::wear_os_medium()),
		}
	}

	/// Starts with an explicit viewport (for unusual faces).
	#[must_use]
	pub fn with_viewport(viewport: WatchViewport) -> Self {
		assert_eq!(
			viewport.kind,
			leptoskit_watch::WatchKind::WearOs,
			"WearOsHost requires a Wear OS viewport"
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

impl Default for WearOsHost {
	fn default() -> Self {
		Self::new()
	}
}

impl WatchHost for WearOsHost {
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
pub const PLATFORM: &str = "wearos";

/// Human-readable product name.
pub const DISPLAY_NAME: &str = "Wear OS";

#[cfg(test)]
mod tests {
	use leptoskit_native::components::Text;
	use leptoskit_watch::WatchApp;
	use leptoskit_watch::WatchKind;
	use leptoskit_watch::WatchPage;

	use super::*;

	#[test]
	fn presents_a_minimal_app() {
		let app = WatchApp::new("demo", WatchKind::WearOs).page(WatchPage::new("home", Text("Hi")));
		let mut host = WearOsHost::new();
		host.present_app(&app).unwrap();
		assert_ne!(host.presentations(), []);
	}
}
