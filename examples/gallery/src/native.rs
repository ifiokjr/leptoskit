//! The session every native gallery host retains on its UI thread.

use std::path::Path;

use leptoskit_native::EventOutcome;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_yoga::NodeId;

use crate::Assets;
use crate::SCREENS;
use crate::Screen;
use crate::ScreenState;
use crate::find_screen;

/// A screen, its state, and the persistent native surface showing it.
pub struct NativeGallery<B: NativeBackend> {
	/// Native widgets and the committed shared element tree.
	pub surface: NativeSurface<B>,
	/// The selected shared screen.
	pub screen: &'static Screen,
	/// Application-owned values updated by the shared event handlers.
	pub state: ScreenState,
}

impl<B: NativeBackend> NativeGallery<B> {
	/// Mounts a shared screen at `viewport` with the gallery's fixed light
	/// palette.
	pub fn new(backend: B, screen: &'static Screen, assets: Assets, viewport: (f32, f32)) -> Self {
		let state = ScreenState::new(assets);
		let mut surface = NativeSurface::new(backend);
		surface.set_dark_mode(false);
		surface.set_viewport(viewport.0, viewport.1);
		surface.set_root(&screen.render(&state));
		Self {
			surface,
			screen,
			state,
		}
	}

	/// Runs the handler for a platform event, then commits the re-rendered
	/// screen. Controls keep their native identity across the commit.
	pub fn dispatch(&mut self, node: NodeId, event: &NativeEvent) -> EventOutcome {
		let outcome = self.surface.dispatch_event(node, event);
		self.surface.set_root(&self.screen.render(&self.state));
		outcome
	}

	/// Whether `node` is a button or pressable that a click activates.
	pub fn is_pressable(&self, node: NodeId) -> bool {
		matches!(
			self.surface.kind(node),
			NativeKind::Button | NativeKind::Pressable
		)
	}
}

/// The screen named `slug`.
///
/// # Errors
/// Returns a message listing every screen when `slug` names none of them.
pub fn screen(slug: &str) -> Result<&'static Screen, String> {
	find_screen(slug).ok_or_else(|| {
		let known: Vec<_> = SCREENS.iter().map(|screen| screen.slug).collect();
		format!("no screen named `{slug}`; try one of: {}", known.join(", "))
	})
}

/// The gallery's images, generated into `dir`.
///
/// # Errors
/// Returns a message naming `dir` when the images cannot be written there.
pub fn generate_assets(dir: &Path) -> Result<Assets, String> {
	Assets::generate(dir).map_err(|error| {
		format!(
			"could not write the gallery images to {}: {error}",
			dir.display()
		)
	})
}

/// The gallery's images, generated into [`Assets::default_dir`].
///
/// # Errors
/// Returns a message naming the directory when the images cannot be written.
pub fn default_assets() -> Result<Assets, String> {
	generate_assets(&Assets::default_dir())
}
