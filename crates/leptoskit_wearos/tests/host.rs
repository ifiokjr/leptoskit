//! Wear OS host stub coverage.

use leptoskit_native::components::Text;
use leptoskit_watch::WatchApp;
use leptoskit_watch::WatchHost;
use leptoskit_watch::WatchKind;
use leptoskit_watch::WatchPage;
use leptoskit_watch::WatchShape;
use leptoskit_watch::WatchSizeClass;
use leptoskit_watch::WatchViewport;
use leptoskit_wearos::DISPLAY_NAME;
use leptoskit_wearos::PLATFORM;
use leptoskit_wearos::WearOsHost;

#[test]
fn constants_and_default_host() {
	assert_eq!(PLATFORM, "wearos");
	assert_eq!(DISPLAY_NAME, "Wear OS");
	let host = WearOsHost::default();
	assert_eq!(host.viewport().kind, WatchKind::WearOs);
}

#[test]
fn custom_viewport_and_presentations() {
	let viewport = WatchViewport {
		kind: WatchKind::WearOs,
		shape: WatchShape::Square,
		size_class: WatchSizeClass::Large,
		width_dp: 400.0,
		height_dp: 400.0,
		safe_inset_dp: 10.0,
	};
	let mut host = WearOsHost::with_viewport(viewport);
	assert_eq!(host.viewport().width_dp, 400.0);
	let app = WatchApp::new("demo", WatchKind::WearOs).page(WatchPage::new("home", Text("Hi")));
	host.present_app(&app).unwrap();
	host.present_page(&WatchPage::new("other", Text("x")))
		.unwrap();
	host.present_ambient(&leptoskit_watch::WatchAmbient::Tile(
		leptoskit_watch::WatchTile::new("tile", Text("t")),
	))
	.unwrap();
	assert_ne!(host.presentations(), []);
}

#[test]
#[should_panic(expected = "WearOsHost requires a Wear OS viewport")]
fn rejects_watchos_viewport() {
	let _ = WearOsHost::with_viewport(WatchViewport::watch_os_medium());
}
