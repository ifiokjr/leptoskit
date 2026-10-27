//! The device command surface: every action command builds the document it
//! stands for, without needing a device attached.

use leptoskit_cli::device::BrowserArg;
use leptoskit_cli::device::DeviceCommand;
use leptoskit_cli::device::DeviceKindArg;
use leptoskit_cli::device::LocatorArgs;
use leptoskit_cli::device::TargetArgs;
use leptoskit_cli::device::document;
use leptoskit_testing::control::Step;
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

/// A minimal W3C driver, one reply per request, for driving `run` end to end.
fn fake_driver(replies: Vec<(&'static str, serde_json::Value)>) -> String {
	use std::io::BufRead;
	use std::io::BufReader;
	use std::io::Read;
	use std::io::Write;
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	listener.set_nonblocking(true).unwrap();
	let endpoint = format!("http://{}", listener.local_addr().unwrap());
	std::thread::spawn(move || {
		for (expected, value) in replies {
			let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
			let mut stream = loop {
				match listener.accept() {
					Ok((stream, _)) => break stream,
					Err(error)
						if error.kind() == std::io::ErrorKind::WouldBlock
							&& std::time::Instant::now() < deadline =>
					{
						std::thread::sleep(std::time::Duration::from_millis(5));
					}
					Err(error) => panic!("expected {expected}: {error}"),
				}
			};
			stream.set_nonblocking(false).unwrap();
			let mut reader = BufReader::new(&stream);
			let mut first = String::new();
			reader.read_line(&mut first).unwrap();
			assert!(
				first.contains(expected),
				"{first} should contain {expected}"
			);
			let mut length = 0;
			loop {
				let mut line = String::new();
				reader.read_line(&mut line).unwrap();
				if line == "\r\n" {
					break;
				}
				if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
					length = value.trim().parse().unwrap();
				}
			}
			let mut bytes = vec![0; length];
			if length > 0 {
				reader.read_exact(&mut bytes).unwrap();
			}
			let body = serde_json::json!({"value": value}).to_string();
			write!(
				stream,
				"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
				 {}\r\nConnection: close\r\n\r\n{body}",
				body.len()
			)
			.unwrap();
		}
	});
	endpoint
}

fn png() -> Vec<u8> {
	let mut bytes = Vec::new();
	{
		let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
		encoder.set_color(png::ColorType::Rgb);
		encoder
			.write_header()
			.unwrap()
			.write_image_data(&[255, 0, 0, 0, 255, 0])
			.unwrap();
	}
	bytes
}

#[test]
fn run_executes_documents_and_prints_their_outcome() {
	use base64::Engine;
	let encoded = base64::engine::general_purpose::STANDARD.encode(png());
	let endpoint = fake_driver(vec![
		("GET /status ", serde_json::json!({"ready": true})),
		("POST /session ", serde_json::json!({"sessionId": "s"})),
		("POST /session/s/timeouts ", serde_json::Value::Null),
		("POST /session/s/url ", serde_json::Value::Null),
		("GET /session/s/screenshot ", serde_json::json!(encoded)),
		("DELETE /session/s ", serde_json::Value::Null),
	]);
	let built = document(DeviceCommand::Screenshot {
		target: TargetArgs {
			url: Some("http://127.0.0.1:1/".to_owned()),
			endpoint: Some(endpoint),
			..web()
		},
		label: "journey".to_owned(),
	})
	.unwrap();
	leptoskit_cli::device::execute(&built).unwrap();
}

#[test]
fn list_prints_both_formats() {
	leptoskit_cli::device::run(DeviceCommand::List { json: true }).unwrap();
	leptoskit_cli::device::run(DeviceCommand::List { json: false }).unwrap();
}

#[test]
fn documents_parse_from_bytes_with_their_source_named() {
	let parsed = leptoskit_cli::device::parse_document(
		br#"{"schema_version": 1, "target": {"kind": "web", "url": "https://example.com"}, "steps": []}"#,
		"the test",
	)
	.unwrap();
	assert_eq!(parsed.steps.len(), 0);
	let error = leptoskit_cli::device::parse_document(b"{", "the test").unwrap_err();
	assert!(error.to_string().contains("parsing the test"));
}

#[test]
fn boot_infers_kinds_from_the_inventory() {
	use leptoskit_cli::device::infer_kind;
	use leptoskit_testing::control::inventory;
	let simulator = inventory::Device {
		id: "278DD3BE".to_owned(),
		name: "iPhone 17".to_owned(),
		kind: inventory::DeviceKind::IosSimulator,
		state: "Shutdown".to_owned(),
	};
	let kind = infer_kind(
		"278DD3BE",
		&inventory::Inventory {
			schema_version: 1,
			devices: vec![simulator],
		},
	)
	.unwrap();
	assert_eq!(kind, TargetKind::IosSimulator);
	// An installed AVD boots as an android target.
	let avd = inventory::Device {
		id: "Pixel_7".to_owned(),
		name: "Pixel 7".to_owned(),
		kind: inventory::DeviceKind::AndroidAvd,
		state: "shutdown".to_owned(),
	};
	let kind = infer_kind(
		"Pixel_7",
		&inventory::Inventory {
			schema_version: 1,
			devices: vec![avd],
		},
	)
	.unwrap();
	assert_eq!(kind, TargetKind::Android);
}

#[test]
fn commands_resolve_against_a_given_inventory() {
	use leptoskit_cli::device::document_with;
	use leptoskit_testing::control::inventory;
	let devices = |kind| {
		inventory::Inventory {
			schema_version: 1,
			devices: vec![inventory::Device {
				id: "KNOWN".to_owned(),
				name: "known".to_owned(),
				kind,
				state: "available".to_owned(),
			}],
		}
	};
	// --device alone resolves through the inventory.
	let resolved = TargetArgs {
		kind: None,
		device: Some("KNOWN".to_owned()),
		..target(DeviceKindArg::Android, None)
	}
	.resolve_with(&devices(inventory::DeviceKind::AndroidDevice))
	.unwrap();
	assert_eq!(resolved.kind, TargetKind::Android);
	// Boot and shutdown build their documents from the same inventory.
	let boot = document_with(
		DeviceCommand::Boot {
			id: "KNOWN".to_owned(),
		},
		&devices(inventory::DeviceKind::IosSimulator),
	)
	.unwrap();
	assert!(matches!(boot.steps[0], Step::Boot));
	let avd_boot = document_with(
		DeviceCommand::Boot {
			id: "KNOWN".to_owned(),
		},
		&devices(inventory::DeviceKind::AndroidAvd),
	)
	.unwrap();
	assert!(matches!(
		avd_boot.steps[0],
		Step::StartEmulator { port: None, .. }
	));
	let shutdown = document_with(
		DeviceCommand::Shutdown {
			id: "KNOWN".to_owned(),
		},
		&devices(inventory::DeviceKind::AndroidEmulator),
	)
	.unwrap();
	assert!(matches!(shutdown.steps[0], Step::Shutdown));
	// An already-running emulator refuses to boot; its shutdown still builds.
	let error = document_with(
		DeviceCommand::Boot {
			id: "KNOWN".to_owned(),
		},
		&devices(inventory::DeviceKind::AndroidEmulator),
	)
	.unwrap_err();
	assert!(error.to_string().contains("already running"));
	// An unknown device id cannot be inferred.
	let error = TargetArgs {
		kind: None,
		device: Some("UNKNOWN".to_owned()),
		..target(DeviceKindArg::Android, None)
	}
	.resolve_with(&devices(inventory::DeviceKind::AndroidDevice))
	.unwrap_err();
	assert!(error.to_string().contains("cannot tell"));
}

#[test]
fn documents_read_from_any_reader() {
	let document = leptoskit_cli::device::read_document_from(
		std::io::Cursor::new(
			br#"{"schema_version":1,"target":{"kind":"web","url":"https://example.com"},"steps":[]}"#,
		),
		"the pipe",
	)
	.unwrap();
	assert_eq!(document.target.url.as_deref(), Some("https://example.com"));
}

#[test]
fn every_kind_and_browser_argument_maps_to_its_target() {
	use leptoskit_cli::device::BrowserArg;
	use leptoskit_cli::device::DeviceKindArg;
	for (argument, kind) in [
		(DeviceKindArg::IosSimulator, TargetKind::IosSimulator),
		(DeviceKindArg::IosDevice, TargetKind::IosDevice),
		(DeviceKindArg::Android, TargetKind::Android),
		(DeviceKindArg::Macos, TargetKind::Macos),
		(DeviceKindArg::Windows, TargetKind::Windows),
		(DeviceKindArg::Linux, TargetKind::Linux),
		(DeviceKindArg::Web, TargetKind::Web),
	] {
		let resolved: TargetKind = argument.into();
		assert_eq!(resolved, kind);
	}
	for (argument, browser) in [
		(
			BrowserArg::Chrome,
			leptoskit_testing::control::Browser::Chrome,
		),
		(
			BrowserArg::Firefox,
			leptoskit_testing::control::Browser::Firefox,
		),
		(
			BrowserArg::Safari,
			leptoskit_testing::control::Browser::Safari,
		),
	] {
		let resolved: leptoskit_testing::control::Browser = argument.into();
		assert_eq!(resolved, browser);
	}
}

#[test]
fn resolving_without_any_target_names_the_mistake() {
	let error = TargetArgs {
		kind: None,
		device: None,
		app: None,
		url: None,
		browser: None,
		endpoint: None,
		headful: false,
	}
	.resolve()
	.unwrap_err();
	assert!(
		error
			.to_string()
			.contains("pass --device, --url, or --kind")
	);
}

#[test]
fn screenshots_of_desktop_targets_use_the_session_camera() {
	// A desktop target with no application drives through a session, so the
	// step is the session screenshot rather than the device camera.
	let built = document(DeviceCommand::Screenshot {
		target: target(DeviceKindArg::Macos, None),
		label: "window".to_owned(),
	})
	.unwrap();
	assert!(matches!(built.steps[0], Step::Screenshot { .. }));
}

#[test]
fn the_cli_dispatch_reaches_the_device_section() {
	use leptoskit_cli::cli::Cli;
	use leptoskit_cli::cli::Command;
	leptoskit_cli::cli::run(Cli {
		command: Command::Device {
			command: leptoskit_cli::device::DeviceCommand::List { json: true },
		},
	})
	.unwrap();
}
