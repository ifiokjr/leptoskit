//! The device command surface: every action command builds the document it
//! stands for, without needing a device attached.

use leptoskit_cli::device::BrowserArg;
use leptoskit_cli::device::DeviceCommand;
use leptoskit_cli::device::DeviceKindArg;
use leptoskit_cli::device::LocatorArgs;
use leptoskit_cli::device::TargetArgs;
use leptoskit_cli::device::document;
use leptoskit_testing::control::TargetKind;

fn target(kind: DeviceKindArg, device: Option<&str>) -> TargetArgs {
	TargetArgs {
		kind: Some(kind),
		device: device.map(str::to_owned),
		app: None,
		url: None,
		browser: None,
		endpoint: None,
		headful: false,
	}
}

fn web() -> TargetArgs {
	TargetArgs {
		kind: None,
		device: None,
		app: None,
		url: Some("https://example.com".to_owned()),
		browser: Some(BrowserArg::Firefox),
		endpoint: None,
		headful: true,
	}
}

#[test]
fn every_action_command_builds_its_document() {
	let cases: Vec<(DeviceCommand, &str, TargetKind)> = vec![
		(
			DeviceCommand::Screenshot {
				target: target(DeviceKindArg::Android, Some("emulator-5554")),
				label: "home".to_owned(),
			},
			"device_screenshot",
			TargetKind::Android,
		),
		(
			DeviceCommand::Screenshot {
				target: web(),
				label: "page".to_owned(),
			},
			"screenshot",
			TargetKind::Web,
		),
		(
			DeviceCommand::Tap {
				target: web(),
				locator: LocatorArgs {
					locator_text: Some("Go".to_owned()),
					..Default::default()
				},
			},
			"tap",
			TargetKind::Web,
		),
		(
			DeviceCommand::EnterText {
				target: web(),
				locator: LocatorArgs {
					locator_css: Some("#name".to_owned()),
					..Default::default()
				},
				text: "hi".to_owned(),
			},
			"enter_text",
			TargetKind::Web,
		),
		(
			DeviceCommand::TypeText {
				target: web(),
				text: "hi".to_owned(),
			},
			"type_text",
			TargetKind::Web,
		),
		(
			DeviceCommand::ExpectVisible {
				target: web(),
				locator: LocatorArgs {
					locator_xpath: Some("//button".to_owned()),
					..Default::default()
				},
			},
			"expect_visible",
			TargetKind::Web,
		),
		(
			DeviceCommand::ExpectText {
				target: web(),
				locator: LocatorArgs {
					locator_id: Some("title".to_owned()),
					..Default::default()
				},
				text: "Welcome".to_owned(),
			},
			"expect_text",
			TargetKind::Web,
		),
		(
			DeviceCommand::Source { target: web() },
			"source",
			TargetKind::Web,
		),
		(
			DeviceCommand::StartApp {
				target: target(DeviceKindArg::IosSimulator, Some("udid")),
				identifier: "dev.app".to_owned(),
			},
			"start_app",
			TargetKind::IosSimulator,
		),
		(
			DeviceCommand::StopApp {
				target: target(DeviceKindArg::Android, Some("serial")),
				identifier: "dev.app".to_owned(),
			},
			"stop_app",
			TargetKind::Android,
		),
		(
			DeviceCommand::Logs {
				target: target(DeviceKindArg::Android, Some("serial")),
				tail: Some(10),
			},
			"logs",
			TargetKind::Android,
		),
	];
	for (command, action, kind) in cases {
		let built = document(command).unwrap_or_else(|error| panic!("{action}: {error}"));
		assert_eq!(built.target.kind, kind, "{action}");
		assert_eq!(built.steps.len(), 1, "{action}");
		assert_eq!(built.steps[0].action_name(), action);
	}
}

#[test]
fn web_targets_carry_their_browser_and_headful_choice() {
	let built = document(DeviceCommand::Source { target: web() }).unwrap();
	assert_eq!(
		built.target.browser,
		Some(leptoskit_testing::control::Browser::Firefox)
	);
	assert!(!built.target.headless);
	assert_eq!(built.target.url.as_deref(), Some("https://example.com"));
}

#[test]
fn run_reads_documents_from_stdin_shape() {
	// The parse itself is covered by the testing crate; here the reader must
	// reject a file that does not exist.
	let error = document(DeviceCommand::Run {
		from: "leptoskit-no-such-journey.json".to_owned(),
	})
	.unwrap_err();
	assert!(error.to_string().contains("reading"));
}

#[test]
fn boot_reports_unknown_devices() {
	let error = document(DeviceCommand::Boot {
		id: "leptoskit-no-such-device".to_owned(),
	})
	.unwrap_err();
	assert!(error.to_string().contains("device list"));
}
