//! Shared watch API: budgets, navigation, and recording hosts.

use leptoskit_native::components::Button;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_watch::ComplicationFamily;
use leptoskit_watch::RecordingHost;
use leptoskit_watch::WatchApp;
use leptoskit_watch::WatchComplication;
use leptoskit_watch::WatchError;
use leptoskit_watch::WatchGlance;
use leptoskit_watch::WatchHost;
use leptoskit_watch::WatchKind;
use leptoskit_watch::WatchNavigation;
use leptoskit_watch::WatchPage;
use leptoskit_watch::WatchPresentation;
use leptoskit_watch::WatchTile;
use leptoskit_watch::WatchViewport;

fn steps_page() -> WatchPage {
	WatchPage::new(
		"home",
		View()
			.class("items-center justify-center gap-2 p-2")
			.child(Text("Steps").class("text-sm"))
			.child(Text("8432").class("text-2xl font-bold"))
			.child(Button("Open")),
	)
	.title("Steps")
}

#[test]
fn wear_os_app_validates_and_records() {
	let app = WatchApp::new("steps", WatchKind::WearOs)
		.navigation(WatchNavigation::Stack)
		.page(steps_page())
		.tile(WatchTile::new("steps-tile", View().child(Text("8.4k"))))
		.complication(WatchComplication::new(
			"steps-face",
			ComplicationFamily::CircularSmall,
			Text("8k"),
		));

	app.validate().expect("valid wear app");

	let mut host = RecordingHost::new(WatchViewport::wear_os_medium());
	host.present_app(&app).expect("present");
	assert!(
		host.presentations()
			.contains(&WatchPresentation::App("steps".into()))
	);
	assert!(
		host.presentations()
			.contains(&WatchPresentation::Page("home".into()))
	);
	assert!(
		host.presentations()
			.contains(&WatchPresentation::Ambient("steps-tile".into()))
	);
}

#[test]
fn watch_os_glance_is_a_first_class_surface() {
	let glance = WatchGlance::new(
		"weather",
		Text("18°"),
		View().child(Text("18°C")).child(Text("Clear")),
	);
	let app = WatchApp::new("weather", WatchKind::WatchOs)
		.navigation(WatchNavigation::Single)
		.page(WatchPage::new("today", Text("Clear")))
		.glance(glance);

	let mut host = RecordingHost::new(WatchViewport::watch_os_medium());
	host.present_app(&app).expect("present watchOS app");
}

#[test]
fn wrong_host_kind_is_rejected() {
	let app = WatchApp::new("steps", WatchKind::WearOs).page(steps_page());
	let mut host = RecordingHost::new(WatchViewport::watch_os_medium());
	let error = host.present_app(&app).unwrap_err();
	assert!(matches!(error, WatchError::Unavailable(_)));
}

#[test]
fn stack_without_pages_fails_validation() {
	let app = WatchApp::new("empty", WatchKind::WearOs).navigation(WatchNavigation::Stack);
	assert_eq!(
		app.validate(),
		Err(WatchError::MissingPage(WatchNavigation::Stack))
	);
}

#[test]
fn phone_sized_text_is_rejected_on_a_complication() {
	let long = "a".repeat(40);
	let complication = WatchComplication::new("busy", ComplicationFamily::Inline, Text(long));
	assert!(matches!(
		complication.validate(),
		Err(WatchError::TooMuchText { .. })
	));
}

#[test]
fn kind_names_are_stable() {
	assert_eq!(WatchKind::WearOs.as_str(), "wearos");
	assert_eq!(WatchKind::WatchOs.as_str(), "watchos");
	assert_eq!("wearos".parse::<WatchKind>().unwrap(), WatchKind::WearOs);
	assert_eq!("watchos".parse::<WatchKind>().unwrap(), WatchKind::WatchOs);
}
