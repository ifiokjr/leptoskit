//! Experimental watchOS guest: a watch surface, not a phone screen.
//!
//! Replace this with your own `WatchApp` once packaging lands. Until then the
//! host stubs only exist so the project shape is reviewable.

use leptoskit_watch::WatchApp;
use leptoskit_watch::WatchKind;
use leptoskit_watch::WatchNavigation;
use leptoskit_watch::WatchPage;

/// Builds the watchOS counter surface.
pub fn app() -> WatchApp {
	WatchApp::new("counter", WatchKind::WatchOs)
		.navigation(WatchNavigation::Single)
		.page(WatchPage::new(
			"home",
			leptoskit_native::components::View()
				.class("items-center justify-center gap-2 p-2")
				.child(leptoskit_native::components::Text("Count").class("text-sm"))
				.child(leptoskit_native::components::Text("0").class("text-2xl font-bold"))
				.child(leptoskit_native::components::Button("Add one")),
		))
}
