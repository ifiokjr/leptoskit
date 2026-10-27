//! The `leptoskit device` section: what is attached, and everything an agent
//! can do to it.
//!
//! Every action command builds one control document, runs it, prints the
//! outcome as JSON, and exits non-zero when a step failed — so the same
//! command works for a human glancing at output and a machine parsing it.
//! `device run` is the escape hatch that takes a whole document, from a file
//! or standard input.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use clap::ValueEnum;
use leptoskit_testing::control::Browser;
use leptoskit_testing::control::Document;
use leptoskit_testing::control::Locator;
use leptoskit_testing::control::SCHEMA_VERSION;
use leptoskit_testing::control::Step;
use leptoskit_testing::control::Target;
use leptoskit_testing::control::TargetKind;
use leptoskit_testing::control::inventory;
use leptoskit_testing::native::Application;

/// The device commands.
#[derive(Debug, Subcommand)]
pub enum DeviceCommand {
	/// Lists everything attached right now: simulators, phones, emulators,
	/// AVDs, the desktop, and browsers with installed drivers.
	List {
		/// Print the machine-readable inventory instead of a table.
		#[arg(long)]
		json: bool,
	},
	/// Boots a simulator or AVD and waits for it to finish.
	Boot {
		/// The simulator UDID or AVD name, as `device list` prints it.
		id: String,
	},
	/// Shuts a simulator down, or stops the emulator behind a serial.
	Shutdown {
		/// The simulator UDID or `emulator-PORT` serial.
		id: String,
	},
	/// Runs one control document from a file or standard input.
	Run {
		/// The document's path, or `-` for standard input.
		#[arg(long, default_value = "-")]
		from: String,
	},
	/// Saves a screenshot: the device's whole screen, or the app's window
	/// when `--app` or `--url` names a session.
	Screenshot {
		#[command(flatten)]
		target: TargetArgs,
		/// Names the saved file.
		#[arg(long, default_value = "screenshot")]
		label: String,
	},
	/// Clicks a visible control.
	Tap {
		#[command(flatten)]
		target: TargetArgs,
		#[command(flatten)]
		locator: LocatorArgs,
	},
	/// Replaces the text of a visible input.
	EnterText {
		#[command(flatten)]
		target: TargetArgs,
		#[command(flatten)]
		locator: LocatorArgs,
		/// The full replacement text.
		#[arg(long)]
		text: String,
	},
	/// Types into whatever has focus.
	TypeText {
		#[command(flatten)]
		target: TargetArgs,
		/// The text to send as keystrokes.
		#[arg(long)]
		text: String,
	},
	/// Waits for exactly one visible control.
	ExpectVisible {
		#[command(flatten)]
		target: TargetArgs,
		#[command(flatten)]
		locator: LocatorArgs,
	},
	/// Waits for a control's exact visible text.
	ExpectText {
		#[command(flatten)]
		target: TargetArgs,
		#[command(flatten)]
		locator: LocatorArgs,
		/// The text it must show.
		#[arg(long)]
		text: String,
	},
	/// Reads the accessibility tree or page source.
	Source {
		#[command(flatten)]
		target: TargetArgs,
	},
	/// Launches an installed app by bundle id or package.
	StartApp {
		#[command(flatten)]
		target: TargetArgs,
		/// The bundle identifier or package name.
		#[arg(long)]
		identifier: String,
	},
	/// Terminates an installed app.
	StopApp {
		#[command(flatten)]
		target: TargetArgs,
		/// The bundle identifier or package name.
		#[arg(long)]
		identifier: String,
	},
	/// Reads recent system logs.
	Logs {
		#[command(flatten)]
		target: TargetArgs,
		/// The most recent lines to keep; 500 when unset.
		#[arg(long)]
		tail: Option<u64>,
	},
}

/// How one locator is selected: exactly one flag per command.
#[derive(Debug, Default, Args)]
#[group(required = true, multiple = false)]
pub struct LocatorArgs {
	/// Accessibility identifier, `AutomationId`, content description, or on
	/// the web `aria-label`/`data-testid`.
	#[arg(long)]
	pub locator_accessibility_id: Option<String>,
	/// Visible text or accessible name.
	#[arg(long)]
	pub locator_text: Option<String>,
	/// Native resource or HTML element id.
	#[arg(long)]
	pub locator_id: Option<String>,
	/// A CSS selector; web targets only.
	#[arg(long)]
	pub locator_css: Option<String>,
	/// An explicit `XPath` into the native tree or the DOM.
	#[arg(long)]
	pub locator_xpath: Option<String>,
}

impl LocatorArgs {
	fn locator(&self) -> Locator {
		match self {
			Self {
				locator_accessibility_id: Some(value),
				..
			} => Locator::AccessibilityId(value.clone()),
			Self {
				locator_text: Some(value),
				..
			} => Locator::Text(value.clone()),
			Self {
				locator_id: Some(value),
				..
			} => Locator::Id(value.clone()),
			Self {
				locator_css: Some(value),
				..
			} => Locator::Css(value.clone()),
			Self {
				locator_xpath: Some(value),
				..
			} => Locator::Xpath(value.clone()),
			_ => unreachable!("clap requires exactly one locator flag"),
		}
	}
}

/// The target every action command shares.
#[derive(Debug, Args)]
pub struct TargetArgs {
	/// What to drive; inferred from `--device` or `--url` when omitted.
	#[arg(long, value_enum)]
	pub kind: Option<DeviceKindArg>,
	/// The simulator UDID, device UDID, or adb serial.
	#[arg(long)]
	pub device: Option<String>,
	/// The app a session launches or attaches to: an installed identity, a
	/// path containing a separator, or `desktop` for the whole Windows or
	/// Linux screen.
	#[arg(long)]
	pub app: Option<String>,
	/// The page a web target opens; implies `--kind web`.
	#[arg(long)]
	pub url: Option<String>,
	/// The browser a web target uses.
	#[arg(long, value_enum)]
	pub browser: Option<BrowserArg>,
	/// A W3C endpoint instead of the default: the Appium server, or a remote
	/// browser driver that is then not spawned locally.
	#[arg(long)]
	pub endpoint: Option<String>,
	/// Show the browser window; headless is the default.
	#[arg(long)]
	pub headful: bool,
}

/// The CLI mirror of [`TargetKind`], named for humans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DeviceKindArg {
	/// An Apple simulator, selected by UDID.
	IosSimulator,
	/// A physically connected iPhone or iPad, selected by UDID.
	IosDevice,
	/// An Android device or emulator, selected by adb serial.
	Android,
	/// A native app on this Mac, through an Appium Mac2 driver.
	Macos,
	/// A native app on this Windows desktop, through an Appium driver.
	Windows,
	/// A native app on this Linux desktop, through an Appium driver.
	Linux,
	/// A website in a real browser.
	Web,
}

impl From<DeviceKindArg> for TargetKind {
	fn from(kind: DeviceKindArg) -> Self {
		match kind {
			DeviceKindArg::IosSimulator => Self::IosSimulator,
			DeviceKindArg::IosDevice => Self::IosDevice,
			DeviceKindArg::Android => Self::Android,
			DeviceKindArg::Macos => Self::Macos,
			DeviceKindArg::Windows => Self::Windows,
			DeviceKindArg::Linux => Self::Linux,
			DeviceKindArg::Web => Self::Web,
		}
	}
}

/// The CLI mirror of [`Browser`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrowserArg {
	/// Chrome through chromedriver.
	Chrome,
	/// Firefox through geckodriver.
	Firefox,
	/// Safari through the macOS safaridriver.
	Safari,
}

impl From<BrowserArg> for Browser {
	fn from(browser: BrowserArg) -> Self {
		match browser {
			BrowserArg::Chrome => Self::Chrome,
			BrowserArg::Firefox => Self::Firefox,
			BrowserArg::Safari => Self::Safari,
		}
	}
}

/// Whether `--app` means a path, an installed identity, or the desktop.
/// Separators decide, because identifiers like `dev.example.app` end in the
/// same suffixes artifact paths do.
fn application(value: &str) -> Application {
	if value == "desktop" {
		Application::Desktop
	} else if value.contains('/') || value.contains('\\') {
		Application::Path(value.to_owned())
	} else {
		Application::Identifier(value.to_owned())
	}
}

impl TargetArgs {
	/// Builds the document target, inferring the kind from the inventory when
	/// only a device id is given, so `--device` alone never picks the wrong
	/// kind of device.
	fn resolve(&self) -> Result<Target> {
		let kind = if self.url.is_some() {
			TargetKind::Web
		} else if let Some(kind) = self.kind {
			kind.into()
		} else if let Some(device) = &self.device {
			let inventory = inventory::list();
			inventory
				.devices
				.iter()
				.find(|entry| &entry.id == device)
				.and_then(|entry| entry.kind.target())
				.with_context(|| {
					format!(
						"cannot tell what {device:?} is; pass --kind or pick from `{} device list`",
						crate::cli::invoked_name()
					)
				})?
		} else {
			bail!(
				"pass --device, --url, or --kind; `{} device list` shows every target",
				crate::cli::invoked_name()
			);
		};
		Ok(Target {
			kind,
			device: self.device.clone(),
			application: self.app.as_deref().map(application),
			url: self.url.clone(),
			browser: self.browser.map(Into::into),
			endpoint: self.endpoint.clone(),
			capabilities: BTreeMap::default(),
			headless: !self.headful,
			startup_seconds: 300,
			command_seconds: 30,
			wait_seconds: 15,
			artifacts: PathBuf::from("target/leptoskit-control"),
		})
	}
}

/// Dispatches the device commands.
pub fn run(command: DeviceCommand) -> Result<()> {
	match command {
		DeviceCommand::List { json } => {
			let inventory = inventory::list();
			if json {
				println!("{}", serde_json::to_string_pretty(&inventory)?);
			} else {
				println!("{:<38} {:<16} {:<10} NAME", "ID", "KIND", "STATE");
				for device in &inventory.devices {
					println!(
						"{:<38} {:<16} {:<10} {}",
						device.id, device.kind, device.state, device.name
					);
				}
			}
			Ok(())
		}
		other => execute(&document(other)?),
	}
}

/// The document one action command stands for, built without running it, so
/// the command surface itself stays testable without devices attached.
pub fn document(command: DeviceCommand) -> Result<Document> {
	match command {
		DeviceCommand::List { .. } => unreachable!("list takes no document"),
		DeviceCommand::Boot { id } => {
			let kind = infer_kind(&id)?;
			let (step, device) = if kind == TargetKind::IosSimulator {
				(Step::Boot, Some(id))
			} else {
				// The step picks the emulator's own free port and serial.
				(
					Step::StartEmulator {
						avd: id,
						port: None,
					},
					None,
				)
			};
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: device_only_target(kind, device),
				steps: vec![step],
			})
		}
		DeviceCommand::Shutdown { id } => {
			let kind = infer_kind(&id)?;
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: device_only_target(kind, Some(id)),
				steps: vec![Step::Shutdown],
			})
		}
		DeviceCommand::Run { from } => read_document(&from),
		DeviceCommand::Screenshot { target, label } => {
			let resolved = target.resolve()?;
			let step = if resolved.application.is_none()
				&& resolved.kind != TargetKind::Web
				&& matches!(
					resolved.kind,
					TargetKind::IosSimulator | TargetKind::Android
				) {
				Step::DeviceScreenshot { label }
			} else {
				Step::Screenshot { label }
			};
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: resolved,
				steps: vec![step],
			})
		}
		DeviceCommand::Tap { target, locator } => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::Tap {
					locator: locator.locator(),
				}],
			})
		}
		DeviceCommand::EnterText {
			target,
			locator,
			text,
		} => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::EnterText {
					locator: locator.locator(),
					text,
				}],
			})
		}
		DeviceCommand::TypeText { target, text } => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::TypeText { text }],
			})
		}
		DeviceCommand::ExpectVisible { target, locator } => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::ExpectVisible {
					locator: locator.locator(),
				}],
			})
		}
		DeviceCommand::ExpectText {
			target,
			locator,
			text,
		} => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::ExpectText {
					locator: locator.locator(),
					text,
				}],
			})
		}
		DeviceCommand::Source { target } => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::Source],
			})
		}
		DeviceCommand::StartApp { target, identifier } => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::StartApp {
					identifier,
					arguments: Vec::new(),
					environment: BTreeMap::default(),
				}],
			})
		}
		DeviceCommand::StopApp { target, identifier } => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::StopApp { identifier }],
			})
		}
		DeviceCommand::Logs { target, tail } => {
			Ok(Document {
				schema_version: SCHEMA_VERSION,
				target: target.resolve()?,
				steps: vec![Step::Logs {
					tail,
					minutes: None,
				}],
			})
		}
	}
}

/// A target with just enough defaults for one device-only step.
fn device_only_target(kind: TargetKind, device: Option<String>) -> Target {
	Target {
		kind,
		device,
		application: None,
		url: None,
		browser: None,
		endpoint: None,
		capabilities: BTreeMap::default(),
		headless: true,
		startup_seconds: 300,
		command_seconds: 30,
		wait_seconds: 15,
		artifacts: PathBuf::from("target/leptoskit-control"),
	}
}

/// What `device boot <id>` is booting: an iOS simulator, or an Android AVD.
fn infer_kind(id: &str) -> Result<TargetKind> {
	let inventory = inventory::list();
	match inventory.devices.iter().find(|entry| entry.id == id) {
		Some(entry) => {
			match entry.kind.target() {
				Some(kind @ (TargetKind::IosSimulator | TargetKind::Android)) => Ok(kind),
				_ => bail!("{id:?} is an emulator that is already running; nothing to boot"),
			}
		}
		None => {
			bail!(
				"{id:?} is not in `{} device list`; AVDs and simulators appear there",
				crate::cli::invoked_name()
			)
		}
	}
}

/// Reads a document from a path, or standard input for `-`.
fn read_document(from: &str) -> Result<Document> {
	let bytes = if from == "-" {
		let mut bytes = Vec::new();
		std::io::stdin()
			.read_to_end(&mut bytes)
			.context("reading the document from standard input")?;
		bytes
	} else {
		std::fs::read(from).with_context(|| format!("reading {from}"))?
	};
	serde_json::from_slice(&bytes).with_context(|| format!("parsing {from} as a control document"))
}

/// Runs one document, prints its outcome, and fails the command on a failed
/// step.
pub fn execute(document: &Document) -> Result<()> {
	let outcome = leptoskit_testing::control::run(document);
	println!("{}", serde_json::to_string_pretty(&outcome)?);
	if !outcome.passed {
		std::process::exit(1);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn apps_resolve_by_shape() {
		assert!(matches!(
			application("dev.example.app"),
			Application::Identifier(id) if id == "dev.example.app"
		));
		assert!(matches!(
			application("build/App.app"),
			Application::Path(path) if path == "build/App.app"
		));
		assert!(matches!(
			application("C:/Apps/App.exe"),
			Application::Path(path) if path == "C:/Apps/App.exe"
		));
		assert!(matches!(application("desktop"), Application::Desktop));
	}

	#[test]
	fn exactly_one_locator_flag_builds_its_locator() {
		let locator = |args: LocatorArgs| args.locator();
		assert_eq!(
			locator(LocatorArgs {
				locator_accessibility_id: Some("save".to_owned()),
				locator_text: None,
				locator_id: None,
				locator_css: None,
				locator_xpath: None,
			}),
			Locator::AccessibilityId("save".to_owned())
		);
		assert_eq!(
			locator(LocatorArgs {
				locator_accessibility_id: None,
				locator_text: None,
				locator_id: None,
				locator_css: Some("#go".to_owned()),
				locator_xpath: None,
			}),
			Locator::Css("#go".to_owned())
		);
	}

	fn target_args(kind: Option<DeviceKindArg>, url: Option<&str>) -> TargetArgs {
		TargetArgs {
			kind,
			device: Some("emulator-5554".to_owned()),
			app: None,
			url: url.map(str::to_owned),
			browser: None,
			endpoint: None,
			headful: false,
		}
	}

	#[test]
	fn urls_win_and_kinds_apply_without_inventory_lookup() {
		let url = target_args(None, Some("https://example.com"))
			.resolve()
			.unwrap();
		assert_eq!(url.kind, TargetKind::Web);
		assert_eq!(url.url.as_deref(), Some("https://example.com"));
		let kind = target_args(Some(DeviceKindArg::IosDevice), None)
			.resolve()
			.unwrap();
		assert_eq!(kind.kind, TargetKind::IosDevice);
	}
}
