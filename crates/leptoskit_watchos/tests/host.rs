//! watchOS host stub coverage.

use leptoskit_native::components::Text;
use leptoskit_watch::WatchApp;
use leptoskit_watch::WatchHost;
use leptoskit_watch::WatchKind;
use leptoskit_watch::WatchPage;
use leptoskit_watch::WatchShape;
use leptoskit_watch::WatchSizeClass;
use leptoskit_watch::WatchViewport;
use leptoskit_watchos::DISPLAY_NAME;
use leptoskit_watchos::PLATFORM;
use leptoskit_watchos::WatchOsHost;

#[test]
fn constants_and_default_host() {
	assert_eq!(PLATFORM, "watchos");
	assert_eq!(DISPLAY_NAME, "watchOS");
	let host = WatchOsHost::default();
	assert_eq!(host.viewport().kind, WatchKind::WatchOs);
}

#[test]
fn custom_viewport_and_presentations() {
	let viewport = WatchViewport {
		kind: WatchKind::WatchOs,
		shape: WatchShape::Round,
		size_class: WatchSizeClass::Small,
		width_dp: 200.0,
		height_dp: 240.0,
		safe_inset_dp: 8.0,
	};
	let mut host = WatchOsHost::with_viewport(viewport);
	assert_eq!(host.viewport().height_dp, 240.0);
	let app = WatchApp::new("demo", WatchKind::WatchOs).page(WatchPage::new("home", Text("Hi")));
	host.present_app(&app).unwrap();
	host.present_page(&WatchPage::new("other", Text("x")))
		.unwrap();
	host.present_ambient(&leptoskit_watch::WatchAmbient::Glance(
		leptoskit_watch::WatchGlance::new("g", Text("p"), Text("c")),
	))
	.unwrap();
	assert_ne!(host.presentations(), []);
}

#[test]
#[should_panic(expected = "WatchOsHost requires a watchOS viewport")]
fn rejects_wearos_viewport() {
	let _ = WatchOsHost::with_viewport(WatchViewport::wear_os_medium());
}
