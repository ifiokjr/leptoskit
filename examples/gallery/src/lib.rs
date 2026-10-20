//! A gallery of leptoskit screens, built purely from [`Element`] trees.
//!
//! The screens are platform-independent: they lay out under the headless
//! backend in tests, and the `gallery` binary renders them with the AppKit
//! backend on macOS, either to PNG or in a window.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;

pub mod assets;
pub mod screens;
pub mod ui;

pub use assets::Assets;

/// The viewport of a phone screen, in points.
pub const PHONE: (f32, f32) = (390.0, 844.0);

/// The viewport of a desktop screen, in points.
pub const DESKTOP: (f32, f32) = (1200.0, 800.0);

/// The state screens render from, shared with their event handlers.
#[derive(Debug, Clone)]
pub struct ScreenState {
	/// The counter screen's count.
	pub count: Arc<AtomicI32>,
	/// The names of the switches that are on.
	pub switches: Arc<RwLock<BTreeSet<&'static str>>>,
	/// The images screens show.
	pub assets: Assets,
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
		let switches = Arc::clone(&self.switches);
		move || {
			let mut switches = switches.write().unwrap_or_else(PoisonError::into_inner);
			if !switches.remove(switch) {
				switches.insert(switch);
			}
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
pub const SCREENS: [Screen; 6] = [
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
];

/// The screen with the given slug.
pub fn find_screen(slug: &str) -> Option<&'static Screen> {
	SCREENS.iter().find(|screen| screen.slug == slug)
}
