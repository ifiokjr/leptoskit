//! A gallery of screens built from leptoskit primitives and Tailwind classes.
//!
//! The screens are platform-independent: they lay out under the headless
//! backend in tests, with native widgets on macOS, iOS, Android, Windows, and
//! Linux, and in the browser through the optional `gallery-web` binary. Every
//! host renders the same screen functions.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;

pub mod assets;
pub mod native;
pub mod screens;
pub mod ui;

/// The terminal host: shared screens in an interactive cell grid. The loop
/// needs a real terminal, so it stays off the wasm builds.
#[cfg(not(target_arch = "wasm32"))]
pub mod tui;

/// The desktop host: AppKit on macOS, GTK 4 on Linux, and Win32 on Windows.
#[cfg(target_os = "macos")]
#[path = "desktop/appkit.rs"]
pub mod desktop;
/// The desktop host: AppKit on macOS, GTK 4 on Linux, and Win32 on Windows.
#[cfg(target_os = "linux")]
#[path = "desktop/gtk.rs"]
pub mod desktop;
/// The desktop host: AppKit on macOS, GTK 4 on Linux, and Win32 on Windows.
#[cfg(target_os = "windows")]
#[path = "desktop/windows.rs"]
pub mod desktop;

#[cfg(target_os = "android")]
mod android;

pub use assets::Assets;

/// The viewport of a phone screen, in points.
pub const PHONE: (f32, f32) = (390.0, 844.0);

/// The viewport of a desktop screen, in points.
pub const DESKTOP: (f32, f32) = (1200.0, 800.0);

/// The state screens render from, shared with their event handlers.
#[derive(Clone)]
pub struct ScreenState {
	/// The counter screen's count.
	pub count: Arc<AtomicI32>,
	/// The names of the switches that are on.
	pub switches: Arc<RwLock<BTreeSet<&'static str>>>,
	/// The images screens show.
	pub assets: Assets,
	volume: Arc<RwLock<f64>>,
	density: Arc<AtomicI32>,
	password: Arc<RwLock<String>>,
	on_change: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for ScreenState {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("ScreenState")
			.field("count", &self.count)
			.field("switches", &self.switches)
			.field("assets", &self.assets)
			.finish_non_exhaustive()
	}
}

impl ScreenState {
	/// The count the counter screen starts at.
	pub const INITIAL_COUNT: i32 = 12;

	/// Fresh state over a set of assets.
	pub fn new(assets: Assets) -> Self {
		Self {
			count: Arc::new(AtomicI32::new(Self::INITIAL_COUNT)),
			switches: Arc::new(RwLock::new(BTreeSet::from([
				"Haptics",
				"Wi-Fi",
				"Bluetooth",
			]))),
			assets,
			volume: Arc::new(RwLock::new(64.0)),
			density: Arc::new(AtomicI32::new(0)),
			password: Arc::new(RwLock::new(String::new())),
			on_change: None,
		}
	}

	/// Notifies a reactive host after a screen's state changes.
	#[must_use]
	pub fn on_change(mut self, notify: impl Fn() + Send + Sync + 'static) -> Self {
		self.on_change = Some(Arc::new(notify));
		self
	}

	fn notify(&self) {
		if let Some(notify) = &self.on_change {
			notify();
		}
	}

	/// A handler that adjusts the count, keeping it nonnegative.
	pub fn adjust_count(&self, delta: i32) -> impl Fn() + Send + Sync + 'static {
		let state = self.clone();
		move || {
			let _ = state
				.count
				.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
					Some(value.saturating_add(delta).max(0))
				});
			state.notify();
		}
	}

	/// A handler that resets the count.
	pub fn reset_count(&self) -> impl Fn() + Send + Sync + 'static {
		let state = self.clone();
		move || {
			state.count.store(0, Ordering::SeqCst);
			state.notify();
		}
	}

	/// The current count.
	pub fn count(&self) -> i32 {
		self.count.load(Ordering::SeqCst)
	}

	/// Whether the named switch is on.
	pub fn is_on(&self, switch: &str) -> bool {
		// Writers only insert or remove whole names, so a poisoned set is
		// still a valid set.
		self.switches
			.read()
			.unwrap_or_else(PoisonError::into_inner)
			.contains(switch)
	}

	/// A handler that flips the named switch.
	pub fn toggle(&self, switch: &'static str) -> impl Fn() + Send + Sync + 'static {
		let state = self.clone();
		move || {
			let mut switches = state
				.switches
				.write()
				.unwrap_or_else(PoisonError::into_inner);
			if !switches.remove(switch) {
				switches.insert(switch);
			}
			drop(switches);
			state.notify();
		}
	}

	/// A controlled handler for a named boolean setting.
	pub fn set_switch(&self, switch: &'static str) -> impl Fn(bool) + Send + Sync + 'static {
		let state = self.clone();
		move |checked| {
			let mut switches = state
				.switches
				.write()
				.unwrap_or_else(PoisonError::into_inner);
			if checked {
				switches.insert(switch);
			} else {
				switches.remove(switch);
			}
			drop(switches);
			state.notify();
		}
	}

	/// The component demo's current volume.
	pub fn volume(&self) -> f64 {
		*self.volume.read().unwrap_or_else(PoisonError::into_inner)
	}

	/// Accepts a validated slider proposal and notifies the host.
	pub fn set_volume(&self) -> impl Fn(f64) + Send + Sync + 'static {
		let state = self.clone();
		move |value| {
			*state.volume.write().unwrap_or_else(PoisonError::into_inner) = value;
			state.notify();
		}
	}

	/// The selected density option in the component demo.
	pub fn density(&self) -> i32 {
		self.density.load(Ordering::SeqCst)
	}

	/// Selects an option when its radio control proposes activation.
	pub fn select_density(&self, option: i32) -> impl Fn(bool) + Send + Sync + 'static {
		let state = self.clone();
		move |checked| {
			if checked {
				state.density.store(option, Ordering::SeqCst);
				state.notify();
			}
		}
	}

	/// The component demo's application-owned password value.
	pub fn password(&self) -> String {
		self.password
			.read()
			.unwrap_or_else(PoisonError::into_inner)
			.clone()
	}

	/// Accepts a password edit without including its content in Debug output.
	pub fn set_password(&self) -> impl Fn(&str) + Send + Sync + 'static {
		let state = self.clone();
		move |value| {
			value.clone_into(
				&mut state
					.password
					.write()
					.unwrap_or_else(PoisonError::into_inner),
			);
			state.notify();
		}
	}
}

/// One screen of the gallery.
#[derive(Debug, Clone, Copy)]
pub struct Screen {
	/// The command-line name and file stem.
	pub slug: &'static str,
	/// The human-readable title, used as the window title.
	pub title: &'static str,
	/// The viewport, in points.
	pub viewport: (f32, f32),
	/// Builds the element tree from the current state.
	pub build: fn(&ScreenState) -> Element,
}

impl Screen {
	/// The element tree for the current state.
	pub fn render(&self, state: &ScreenState) -> Element {
		(self.build)(state)
	}
}

/// Every screen, in gallery order.
pub const SCREENS: [Screen; 7] = [
	Screen {
		slug: "counter",
		title: "Counter",
		viewport: PHONE,
		build: screens::counter::screen,
	},
	Screen {
		slug: "profile",
		title: "Profile",
		viewport: PHONE,
		build: screens::profile::screen,
	},
	Screen {
		slug: "settings",
		title: "Settings",
		viewport: PHONE,
		build: screens::settings::screen,
	},
	Screen {
		slug: "photos",
		title: "Photos",
		viewport: PHONE,
		build: screens::photos::screen,
	},
	Screen {
		slug: "chat",
		title: "Chat",
		viewport: PHONE,
		build: screens::chat::screen,
	},
	Screen {
		slug: "dashboard",
		title: "Dashboard",
		viewport: DESKTOP,
		build: screens::dashboard::screen,
	},
	Screen {
		slug: "components",
		title: "Components",
		viewport: DESKTOP,
		build: screens::components::screen,
	},
];

/// The screen with the given slug.
pub fn find_screen(slug: &str) -> Option<&'static Screen> {
	SCREENS.iter().find(|screen| screen.slug == slug)
}
