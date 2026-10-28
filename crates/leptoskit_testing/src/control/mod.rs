//! Device control: one JSON document that drives any attached target.
//!
//! A [`Document`] names a [`Target`] — an iOS simulator or device, an Android
//! device or emulator, a native app on the desktop, or a website — and lists
//! ordered [`Step`]s. Driver steps run through a W3C session (Appium for
//! native, a spawned browser driver for the web); device steps shell out to
//! `simctl`, `devicectl`, and `adb` directly, so screenshots and app
//! management work with no driver running at all. Execution stops on the
//! first failure and reports every step in an [`Outcome`].
//!
//! The same document reaches the executor from the `leptoskit device` CLI
//! commands, from JSON piped to the CLI, and from the `leptoskit mcp`
//! server's tools, so agents compose journeys once and run them anywhere.

mod device;
pub mod inventory;
mod tools;
mod web;

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
pub use web::Browser;

use crate::native::Application;
use crate::native::Locator as NativeLocator;
use crate::native::NativeTester;
use crate::native::Options as NativeOptions;
use crate::native::Platform;

/// The document schema's major version.
pub const SCHEMA_VERSION: u8 = 1;

/// The default Appium endpoint when none is configured.
pub const DEFAULT_APPIUM_ENDPOINT: &str = "http://127.0.0.1:4723";

/// The environment variable overriding the Appium endpoint.
pub const APPIUM_ENDPOINT_VARIABLE: &str = "LEPTOSKIT_APPIUM_ENDPOINT";

/// Device control failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Invalid target or step selection.
	#[error("device control configuration: {0}")]
	Configuration(String),
	/// Local files could not be read or written.
	#[error(transparent)]
	Io(#[from] std::io::Error),
	/// Malformed JSON.
	#[error(transparent)]
	Json(#[from] serde_json::Error),
	/// The driver rejected the operation.
	#[error("driver {code}: {message}")]
	Driver { code: String, message: String },
	/// Driver returned a structurally invalid response.
	#[error("driver response: {0}")]
	Response(String),
	/// A visible state failed to appear within its deadline.
	#[error("timed out: {0}")]
	Timeout(String),
	/// More than one control matched; no action was taken.
	#[error("selector {selector} matched {count} visible controls")]
	Ambiguous { selector: String, count: usize },
	/// An observable assertion failed.
	#[error("assertion: {0}")]
	Assertion(String),
	/// A platform tool (`simctl`, `adb`, …) failed.
	#[error("{name} {detail}")]
	Tool { name: String, detail: String },
	/// A driver session failed with retained evidence.
	#[error("session failed; evidence in {directory}: {message}")]
	Evidence { directory: PathBuf, message: String },
}

impl From<crate::webdriver::Error> for Error {
	fn from(error: crate::webdriver::Error) -> Self {
		match error {
			crate::webdriver::Error::Evidence { directory, source } => {
				Self::Evidence {
					directory,
					message: source.to_string(),
				}
			}
			crate::webdriver::Error::Configuration(message) => Self::Configuration(message),
			crate::webdriver::Error::Transport(source) => {
				Self::Driver {
					code: "transport".to_owned(),
					message: source.to_string(),
				}
			}
			crate::webdriver::Error::Io(source) => Self::Io(source),
			crate::webdriver::Error::Json(source) => Self::Json(source),
			crate::webdriver::Error::Driver { code, message } => Self::Driver { code, message },
			crate::webdriver::Error::Response(message) => Self::Response(message),
			crate::webdriver::Error::Timeout(message) => Self::Timeout(message),
			crate::webdriver::Error::Ambiguous { selector, count } => {
				Self::Ambiguous { selector, count }
			}
			crate::webdriver::Error::Assertion(message) => Self::Assertion(message),
			crate::webdriver::Error::Capture(source) => Self::Response(source.to_string()),
			crate::webdriver::Error::Combined { primary, secondary } => {
				Self::Driver {
					code: "combined".to_owned(),
					message: format!("{primary}; additionally: {secondary}"),
				}
			}
		}
	}
}

/// What a document drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TargetKind {
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

impl TargetKind {
	/// Whether the kind names a device `simctl` or `adb` can reach directly.
	fn is_mobile(self) -> bool {
		matches!(self, Self::IosSimulator | Self::IosDevice | Self::Android)
	}
}

/// Session setup for one target.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Target {
	/// What to drive.
	pub kind: TargetKind,
	/// The simulator UDID, device UDID, or adb serial. Required for the
	/// mobile kinds; the desktop and web kinds have none.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device: Option<String>,
	/// The native app to launch or attach to: a path, an installed identity,
	/// or the whole desktop on Windows and Linux. Required once any driver
	/// step runs; device-only documents can omit it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub application: Option<Application>,
	/// The page a web target opens.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub url: Option<String>,
	/// The browser a web target uses; Chrome when unset.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub browser: Option<Browser>,
	/// A W3C endpoint to use instead of the default: the Appium server for
	/// native kinds, or a remote browser driver for the web, which is then
	/// not spawned locally.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub endpoint: Option<String>,
	/// Extra namespaced driver capabilities.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub capabilities: BTreeMap<String, Value>,
	/// Whether a spawned browser runs headless. On by default; turn it off to
	/// watch a web journey.
	#[serde(default = "default_true")]
	pub headless: bool,
	/// Deadline for session creation, including driver and app start.
	#[serde(default = "default_startup")]
	pub startup_seconds: u64,
	/// Deadline for a single driver command.
	#[serde(default = "default_command")]
	pub command_seconds: u64,
	/// Deadline for observable UI conditions.
	#[serde(default = "default_wait")]
	pub wait_seconds: u64,
	/// Parent directory for this run's artifacts.
	#[serde(default = "default_artifacts")]
	pub artifacts: PathBuf,
}

const fn default_true() -> bool {
	true
}
const fn default_startup() -> u64 {
	300
}
const fn default_command() -> u64 {
	30
}
const fn default_wait() -> u64 {
	15
}
fn default_artifacts() -> PathBuf {
	PathBuf::from("target/leptoskit-control")
}

/// A point in the driver's logical viewport units, or screenshot pixels for
/// device-level input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Point {
	/// Horizontal offset from the left edge.
	pub x: i64,
	/// Vertical offset from the top edge.
	pub y: i64,
}

/// A semantic selector, resolved afresh before every action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
	tag = "by",
	content = "value",
	rename_all = "snake_case",
	deny_unknown_fields
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Locator {
	/// Accessibility identifier, `AutomationId`, Android content description,
	/// or on the web `aria-label`/`data-testid`.
	AccessibilityId(String),
	/// Visible text or accessible name.
	Text(String),
	/// Native resource or HTML element id.
	Id(String),
	/// A CSS selector; web targets only.
	Css(String),
	/// An explicit `XPath` into the native tree or the DOM.
	Xpath(String),
}

impl Locator {
	/// The native locator this resolves to; CSS has no native equivalent.
	fn native(&self) -> Result<NativeLocator, Error> {
		let locator = match self {
			Self::AccessibilityId(value) => NativeLocator::AccessibilityId(value.clone()),
			Self::Text(value) => NativeLocator::Text(value.clone()),
			Self::Id(value) => NativeLocator::Id(value.clone()),
			Self::Xpath(value) => NativeLocator::Xpath(value.clone()),
			Self::Css(_) => {
				return Err(Error::Configuration(
					"css selectors need a web target".to_owned(),
				));
			}
		};
		Ok(locator)
	}
}

/// One ordered operation. Driver steps need a session; device steps run
/// through the platform tools directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Step {
	/// Click a visible enabled control.
	Tap {
		/// What to click.
		locator: Locator,
	},
	/// Replace the text of a visible enabled input.
	EnterText {
		/// The input to write into.
		locator: Locator,
		/// The full replacement text.
		text: String,
	},
	/// Type into whatever currently has focus.
	TypeText {
		/// The text to send as keystrokes.
		text: String,
	},
	/// Press one key or W3C key sequence on whatever has focus.
	PressKey {
		/// A single character, like `a`, or a W3C key like `\u{e006}` for
		/// enter.
		key: String,
	},
	/// Drag from one point to another.
	Swipe {
		/// Where the gesture starts.
		start: Point,
		/// Where the gesture ends.
		end: Point,
		/// How long the move takes; a short default when unset.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		duration_ms: Option<u64>,
	},
	/// Wait for exactly one visible control.
	ExpectVisible {
		/// What must appear.
		locator: Locator,
	},
	/// Wait for every matching control to disappear.
	ExpectAbsent {
		/// What must go away.
		locator: Locator,
	},
	/// Wait for an exact visible text value.
	ExpectText {
		/// The control to read.
		locator: Locator,
		/// The text it must show.
		text: String,
	},
	/// Wait for and read the visible alert's text.
	AlertText,
	/// Accept the visible alert.
	AcceptAlert,
	/// Dismiss the visible alert.
	DismissAlert,
	/// Save the session's pixels — the app on native, the viewport on the
	/// web — and record the artifact path.
	Screenshot {
		/// Names the saved file.
		label: String,
	},
	/// Read the accessibility tree (native) or page source (web).
	Source,
	/// Execute a driver extension: `mobile:` commands on Appium, JavaScript
	/// on the web.
	NativeCommand {
		/// The extension name or script.
		script: String,
		/// Its argument, passed through as JSON.
		arguments: Value,
	},
	/// Dispatch raw W3C pointer/key action sources.
	PerformActions {
		/// The action sources, verbatim.
		sources: Value,
	},

	/// Boot the target simulator and wait for it to finish.
	Boot,
	/// Shut the target simulator or emulator down.
	Shutdown,
	/// Start the emulator named here, detached, and wait for it to boot; the
	/// document's device then targets its `emulator-PORT` serial.
	StartEmulator {
		/// The AVD name, as `emulator -list-avds` prints it.
		avd: String,
		/// The console port; `5554` when unset. Only even ports 5554–5682
		/// work.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		port: Option<u16>,
	},
	/// Launch an installed app by identity: bundle id on Apple, package on
	/// Android.
	StartApp {
		/// The bundle identifier or package name.
		identifier: String,
		/// Arguments passed to the simulator launch.
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		arguments: Vec<String>,
		/// Environment variables set for the launched app, on simulators.
		#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
		environment: BTreeMap<String, String>,
	},
	/// Terminate an installed app by identity.
	StopApp {
		/// The bundle identifier or package name.
		identifier: String,
	},
	/// Save the device's whole screen with `simctl` or `screencap`, no
	/// driver needed.
	DeviceScreenshot {
		/// Names the saved file.
		label: String,
	},
	/// Tap absolute screenshot pixels with the platform input tool; Android
	/// only.
	DeviceTap {
		/// Horizontal offset from the left edge.
		x: i64,
		/// Vertical offset from the top edge.
		y: i64,
	},
	/// Drag between absolute screenshot pixels with the platform input
	/// tool; Android only.
	DeviceSwipe {
		/// Where the gesture starts.
		start: Point,
		/// Where the gesture ends.
		end: Point,
		/// How long the move takes; a short default when unset.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		duration_ms: Option<u64>,
	},
	/// Press one Android key event code, by name (`BACK`) or number (`4`).
	DeviceKey {
		/// The `input keyevent` code.
		key: String,
	},
	/// Read recent system logs: `logcat` on Android, the unified log on
	/// simulators.
	Logs {
		/// The most recent lines to keep; 500 when unset.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		tail: Option<u64>,
		/// How far back simulator logs reach, in minutes; 5 when unset.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		minutes: Option<u64>,
	},
}

impl Step {
	/// The step's JSON action name.
	pub fn action_name(&self) -> &'static str {
		match self {
			Self::Tap { .. } => "tap",
			Self::EnterText { .. } => "enter_text",
			Self::TypeText { .. } => "type_text",
			Self::PressKey { .. } => "press_key",
			Self::Swipe { .. } => "swipe",
			Self::ExpectVisible { .. } => "expect_visible",
			Self::ExpectAbsent { .. } => "expect_absent",
			Self::ExpectText { .. } => "expect_text",
			Self::AlertText => "alert_text",
			Self::AcceptAlert => "accept_alert",
			Self::DismissAlert => "dismiss_alert",
			Self::Screenshot { .. } => "screenshot",
			Self::Source => "source",
			Self::NativeCommand { .. } => "native_command",
			Self::PerformActions { .. } => "perform_actions",
			Self::Boot => "boot",
			Self::Shutdown => "shutdown",
			Self::StartEmulator { .. } => "start_emulator",
			Self::StartApp { .. } => "start_app",
			Self::StopApp { .. } => "stop_app",
			Self::DeviceScreenshot { .. } => "device_screenshot",
			Self::DeviceTap { .. } => "device_tap",
			Self::DeviceSwipe { .. } => "device_swipe",
			Self::DeviceKey { .. } => "device_key",
			Self::Logs { .. } => "logs",
		}
	}
}

/// An ordered control journey.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Document {
	/// The document schema's major version; [`SCHEMA_VERSION`] today.
	#[serde(default = "default_schema_version")]
	pub schema_version: u8,
	/// What the steps run against.
	pub target: Target,
	/// The steps, in order, stopping on the first failure. Empty documents
	/// are connectivity checks.
	pub steps: Vec<Step>,
}

fn default_schema_version() -> u8 {
	SCHEMA_VERSION
}

impl Document {
	/// Rejects targets no step could run against.
	pub fn validate(&self) -> Result<(), Error> {
		let target = &self.target;
		for deadline in [
			target.startup_seconds,
			target.command_seconds,
			target.wait_seconds,
		] {
			if !(1..=3600).contains(&deadline) {
				return Err(Error::Configuration(
					"deadlines must be between 1 and 3600 seconds".to_owned(),
				));
			}
		}
		if target.kind == TargetKind::Web {
			if target.url.is_none() {
				return Err(Error::Configuration("web targets require a url".to_owned()));
			}
			if target.device.is_some() || target.application.is_some() {
				return Err(Error::Configuration(
					"web targets take no device or application".to_owned(),
				));
			}
		} else {
			if target.url.is_some() || target.browser.is_some() {
				return Err(Error::Configuration(
					"url and browser belong to web targets".to_owned(),
				));
			}
			// Every step but StartEmulator reaches the device itself: driver
			// steps through `appium:udid`, device steps through the tools.
			// StartEmulator picks its own serial, so a boot-only document
			// needs none.
			let needs_device = self
				.steps
				.iter()
				.any(|step| !matches!(step, Step::StartEmulator { .. }));
			if target.kind.is_mobile() && needs_device {
				let empty = target
					.device
					.as_deref()
					.is_none_or(|device| device.trim().is_empty());
				if empty {
					return Err(Error::Configuration(format!(
						"{:?} targets require a device id",
						target.kind
					)));
				}
			}
		}
		Ok(())
	}
}

/// One driver session a document's steps run through: Appium-backed
/// [`NativeTester`] or a spawned-driver [`web::WebSession`].
pub trait ControlDriver {
	/// Waits for one visible enabled control and clicks it.
	fn tap(&self, locator: &Locator) -> Result<(), Error>;
	/// Replaces the text of one visible enabled input.
	fn enter_text(&self, locator: &Locator, text: &str) -> Result<(), Error>;
	/// Types into whatever has focus.
	fn type_text(&self, text: &str) -> Result<(), Error>;
	/// Waits for exactly one visible control.
	fn expect_visible(&self, locator: &Locator) -> Result<(), Error>;
	/// Waits until no control matches.
	fn expect_absent(&self, locator: &Locator) -> Result<(), Error>;
	/// Waits for an exact visible text.
	fn expect_text(&self, locator: &Locator, text: &str) -> Result<(), Error>;
	/// Reads the visible alert's text.
	fn alert_text(&self) -> Result<String, Error>;
	/// Accepts the visible alert.
	fn accept_alert(&self) -> Result<(), Error>;
	/// Dismisses the visible alert.
	fn dismiss_alert(&self) -> Result<(), Error>;
	/// The session's actual pixels, PNG-encoded.
	fn screenshot_png(&self) -> Result<Vec<u8>, Error>;
	/// The accessibility tree or page source.
	fn source(&self) -> Result<String, Error>;
	/// Dispatches raw W3C actions.
	fn perform_actions(&self, sources: &Value) -> Result<(), Error>;
	/// Executes a driver extension or script.
	fn execute(&self, script: &str, arguments: &Value) -> Result<Value, Error>;
	/// Closes the session. `failure` names the step error that ended the run,
	/// so native sessions can retain their evidence.
	fn close(self: Box<Self>, failure: Option<&Error>) -> Result<(), Error>;

	/// Drags between two points in one pointer gesture.
	fn swipe(&self, start: Point, end: Point, duration_ms: Option<u64>) -> Result<(), Error> {
		let duration = duration_ms.unwrap_or(250);
		// The raw action sources: perform_actions adds the envelope.
		let sources = json!([{
			"type": "pointer",
			"id": "leptoskit-swipe",
			"parameters": {"pointerType": "touch"},
			"actions": [
				{"type": "pointerMove", "duration": 0, "x": start.x, "y": start.y},
				{"type": "pointerDown", "button": 0},
				{"type": "pause", "duration": duration},
				{"type": "pointerMove", "duration": duration, "origin": "pointer",
				 "x": end.x - start.x, "y": end.y - start.y},
				{"type": "pointerUp", "button": 0}
			]
		}]);
		self.perform_actions(&sources)
	}
}

/// The result of one step, as the outcome reports it.
#[derive(Debug, Default)]
struct StepValue {
	value: Option<Value>,
	artifact: Option<PathBuf>,
}

/// One step's recorded outcome.
#[derive(Debug, Serialize)]
pub struct StepOutcome {
	/// The step's position in the document.
	pub index: usize,
	/// The JSON action name.
	pub action: &'static str,
	/// `ok` or `error`.
	pub status: &'static str,
	/// What the step read: alert text, source, command results, logs.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub value: Option<Value>,
	/// The file the step wrote, when it wrote one.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub artifact: Option<PathBuf>,
	/// Why the step failed.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
}

/// A document run's complete report.
#[derive(Debug, Serialize)]
pub struct Outcome {
	/// The outcome schema's major version.
	pub schema_version: u8,
	/// Whether every step ran.
	pub passed: bool,
	/// The target the steps ran against.
	pub target: Target,
	/// One entry per step that ran, in order.
	pub steps: Vec<StepOutcome>,
	/// This run's artifact directory, when anything was written.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub artifacts: Option<PathBuf>,
	/// Why the run ended early.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
}

/// How much source and log text a step value carries; files keep everything.
const VALUE_TEXT_LIMIT: usize = 200_000;

/// The control document's public JSON Schema (draft 2020-12) with its asset
/// id, for editors, the MCP tool contract, and the published schema file.
#[cfg(feature = "schema")]
pub fn document_schema(id: &str) -> schemars::Schema {
	let mut schema = schemars::generate::SchemaSettings::draft2020_12()
		.for_deserialize()
		.into_generator()
		.into_root_schema_for::<Document>();
	schema.insert("$id".into(), Value::String(id.into()));
	schema.insert(
		"title".into(),
		Value::String("leptoskit control document".into()),
	);
	schema
}

/// Runs the document, stopping on the first failure, and returns the report.
///
/// Like [`NativeTester`], this blocking executor must run outside an async
/// runtime or in its blocking pool.
pub fn run(document: &Document) -> Outcome {
	let mut outcome = Outcome {
		schema_version: SCHEMA_VERSION,
		passed: false,
		target: document.target.clone(),
		steps: Vec::new(),
		artifacts: None,
		error: None,
	};
	let mut context = match document
		.validate()
		.and_then(|()| ExecutionContext::new(&document.target))
	{
		Ok(context) => context,
		Err(error) => {
			outcome.error = Some(format!("{error:#}"));
			return outcome;
		}
	};
	let mut failure = None;
	for (index, step) in document.steps.iter().enumerate() {
		let result = context.step(step, index);
		let action = step.action_name();
		match result {
			Ok(step_value) => {
				outcome.steps.push(StepOutcome {
					index,
					action,
					status: "ok",
					value: step_value.value,
					artifact: step_value.artifact,
					error: None,
				});
			}
			Err(error) => {
				outcome.steps.push(StepOutcome {
					index,
					action,
					status: "error",
					value: None,
					artifact: None,
					error: Some(format!("{error:#}")),
				});
				failure = Some(error);
				break;
			}
		}
	}
	let artifacts = context.artifacts.clone();
	if let Err(error) = context.close(failure.as_ref()) {
		eprintln!("closing the session also failed: {error:#}");
	}
	outcome.artifacts = Some(artifacts);
	outcome.passed = failure.is_none();
	outcome.error = failure.map(|error| format!("{error:#}"));
	outcome
}

/// Runs one document against a resolved endpoint, holding the lazily created
/// driver session and this run's artifacts.
struct ExecutionContext {
	target: Target,
	artifacts: PathBuf,
	driver: Option<Box<dyn ControlDriver>>,
}

impl ExecutionContext {
	fn new(target: &Target) -> Result<Self, Error> {
		let parent = &target.artifacts;
		std::fs::create_dir_all(parent)?;
		let stamp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map_err(|error| Error::Configuration(error.to_string()))?
			.as_nanos();
		// Concurrent runs in one process can read the same coarse timestamp;
		// the counter keeps every run's directory its own.
		static RUN_NUMBER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
		let run = RUN_NUMBER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		let artifacts = parent.join(format!("{stamp}-{}-{run}", std::process::id()));
		std::fs::create_dir(&artifacts)?;
		Ok(Self {
			target: target.clone(),
			artifacts,
			driver: None,
		})
	}

	/// The deadline platform tools run under; log collection can outlast a
	/// single UI command, so it never runs shorter than a minute.
	fn tool_timeout(&self) -> Duration {
		Duration::from_secs(self.target.command_seconds.max(60))
	}

	#[expect(
		clippy::too_many_lines,
		reason = "one arm per step kind keeps the whole dispatch table in one place"
	)]
	fn step(&mut self, step: &Step, index: usize) -> Result<StepValue, Error> {
		match step {
			Step::Tap { locator } => self.driver()?.tap(locator).map(|()| StepValue::default()),
			Step::EnterText { locator, text } => {
				self.driver()?
					.enter_text(locator, text)
					.map(|()| StepValue::default())
			}
			Step::TypeText { text } => {
				self.driver()?
					.type_text(text)
					.map(|()| StepValue::default())
			}
			Step::PressKey { key } => self.driver()?.type_text(key).map(|()| StepValue::default()),
			Step::Swipe {
				start,
				end,
				duration_ms,
			} => {
				self.driver()?
					.swipe(*start, *end, *duration_ms)
					.map(|()| StepValue::default())
			}
			Step::ExpectVisible { locator } => {
				self.driver()?
					.expect_visible(locator)
					.map(|()| StepValue::default())
			}
			Step::ExpectAbsent { locator } => {
				self.driver()?
					.expect_absent(locator)
					.map(|()| StepValue::default())
			}
			Step::ExpectText { locator, text } => {
				self.driver()?
					.expect_text(locator, text)
					.map(|()| StepValue::default())
			}
			Step::AlertText => {
				self.driver()?.alert_text().map(|text| {
					StepValue {
						value: Some(Value::String(text)),
						artifact: None,
					}
				})
			}
			Step::AcceptAlert => self.driver()?.accept_alert().map(|()| StepValue::default()),
			Step::DismissAlert => {
				self.driver()?
					.dismiss_alert()
					.map(|()| StepValue::default())
			}
			Step::Screenshot { label } => {
				let bytes = self.driver()?.screenshot_png()?;
				let path = self.save_png(index, label, &bytes)?;
				Ok(StepValue {
					value: None,
					artifact: Some(path),
				})
			}
			Step::Source => self.capture_source(index),
			Step::NativeCommand { script, arguments } => {
				self.driver()?.execute(script, arguments).map(|value| {
					StepValue {
						value: Some(value),
						artifact: None,
					}
				})
			}
			Step::PerformActions { sources } => {
				self.driver()?
					.perform_actions(sources)
					.map(|()| StepValue::default())
			}
			Step::Boot => {
				device::boot(&self.target, self.tool_timeout()).map(|()| StepValue::default())
			}
			Step::Shutdown => {
				device::shutdown(&self.target, self.tool_timeout()).map(|()| StepValue::default())
			}
			Step::StartEmulator { avd, port } => {
				device::start_emulator(
					&self.target,
					avd,
					*port,
					Duration::from_secs(self.target.startup_seconds),
				)
				.map(|()| StepValue::default())
			}
			Step::StartApp {
				identifier,
				arguments,
				environment,
			} => {
				device::start_app(
					&self.target,
					identifier,
					arguments,
					environment,
					self.tool_timeout(),
				)
				.map(|value| {
					StepValue {
						value: Some(Value::String(value)),
						artifact: None,
					}
				})
			}
			Step::StopApp { identifier } => {
				device::stop_app(&self.target, identifier, self.tool_timeout())
					.map(|()| StepValue::default())
			}
			Step::DeviceScreenshot { label } => {
				let bytes = device::screenshot(&self.target, self.tool_timeout())?;
				let path = self.save_png(index, label, &bytes)?;
				Ok(StepValue {
					value: None,
					artifact: Some(path),
				})
			}
			Step::DeviceTap { x, y } => {
				device::tap_point(&self.target, *x, *y, self.tool_timeout())
					.map(|()| StepValue::default())
			}
			Step::DeviceSwipe {
				start,
				end,
				duration_ms,
			} => {
				device::swipe(
					&self.target,
					*start,
					*end,
					*duration_ms,
					self.tool_timeout(),
				)
				.map(|()| StepValue::default())
			}
			Step::DeviceKey { key } => {
				device::key(&self.target, key, self.tool_timeout()).map(|()| StepValue::default())
			}
			Step::Logs { tail, minutes } => {
				let text = device::logs(&self.target, *tail, *minutes, self.tool_timeout())?;
				let path = self.artifacts.join(format!("{index:02}-logs.txt"));
				std::fs::write(&path, &text)?;
				Ok(StepValue {
					value: Some(Value::String(truncate_chars(&text, VALUE_TEXT_LIMIT))),
					artifact: Some(path),
				})
			}
		}
	}

	/// Saves the driver's source while keeping the returned text bounded.
	fn capture_source(&mut self, index: usize) -> Result<StepValue, Error> {
		let source = self.driver()?.source()?;
		let path = self.artifacts.join(format!("{index:02}-source"));
		std::fs::write(&path, &source)?;
		Ok(StepValue {
			value: Some(Value::String(truncate_chars(&source, VALUE_TEXT_LIMIT))),
			artifact: Some(path),
		})
	}

	/// Validates the PNG and writes it under a sanitized label.
	fn save_png(&self, index: usize, label: &str, bytes: &[u8]) -> Result<PathBuf, Error> {
		let write = |path: &Path| {
			crate::capture::Screenshot::from_png(bytes.to_vec())
				.and_then(|screenshot| screenshot.save(path))
				.map_err(|error| Error::Response(error.to_string()))
		};
		let sanitized: String = label
			.chars()
			.map(|character| {
				if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
					character
				} else {
					'-'
				}
			})
			.collect();
		let path = self.artifacts.join(format!("{index:02}-{sanitized}.png"));
		write(&path)?;
		Ok(path)
	}

	fn driver(&mut self) -> Result<&mut dyn ControlDriver, Error> {
		if self.driver.is_none() {
			self.driver = Some(connect_driver(&self.target)?);
		}
		Ok(self.driver.as_deref_mut().expect("just connected"))
	}

	fn close(self, failure: Option<&Error>) -> Result<(), Error> {
		match self.driver {
			None => Ok(()),
			Some(driver) => driver.close(failure),
		}
	}
}

/// Connects the driver a target needs, validating the application once any
/// driver step runs.
fn connect_driver(target: &Target) -> Result<Box<dyn ControlDriver>, Error> {
	if target.kind == TargetKind::Web {
		return Ok(Box::new(web::WebSession::connect(target)?));
	}
	let application = target.application.clone().ok_or_else(|| {
		Error::Configuration(format!(
			"{:?} driver steps need target.application: a path, an installed identity, or desktop",
			target.kind
		))
	})?;
	let platform = match target.kind {
		TargetKind::IosSimulator | TargetKind::IosDevice => Platform::Ios,
		TargetKind::Android => Platform::Android,
		TargetKind::Macos => Platform::Macos,
		TargetKind::Windows => Platform::Windows,
		TargetKind::Linux => Platform::Linux,
		TargetKind::Web => unreachable!("web handled above"),
	};
	let endpoint = target
		.endpoint
		.clone()
		.or_else(|| std::env::var(APPIUM_ENDPOINT_VARIABLE).ok())
		.unwrap_or_else(|| DEFAULT_APPIUM_ENDPOINT.to_owned());
	let mut options = NativeOptions::new(endpoint, platform, application);
	options.device.clone_from(&target.device);
	options.capabilities.clone_from(&target.capabilities);
	options.startup_seconds = target.startup_seconds;
	options.command_seconds = target.command_seconds;
	options.wait_seconds = target.wait_seconds;
	options.artifacts.clone_from(&target.artifacts);
	Ok(Box::new(NativeTester::connect(options)?))
}

impl ControlDriver for NativeTester {
	fn tap(&self, locator: &Locator) -> Result<(), Error> {
		NativeTester::tap(self, &locator.native()?).map_err(Error::from)
	}

	fn enter_text(&self, locator: &Locator, text: &str) -> Result<(), Error> {
		NativeTester::enter_text(self, &locator.native()?, text).map_err(Error::from)
	}

	fn type_text(&self, text: &str) -> Result<(), Error> {
		NativeTester::input_type(self, text).map_err(Error::from)
	}

	fn expect_visible(&self, locator: &Locator) -> Result<(), Error> {
		NativeTester::expect_visible(self, &locator.native()?).map_err(Error::from)
	}

	fn expect_absent(&self, locator: &Locator) -> Result<(), Error> {
		NativeTester::expect_absent(self, &locator.native()?).map_err(Error::from)
	}

	fn expect_text(&self, locator: &Locator, text: &str) -> Result<(), Error> {
		NativeTester::expect_text(self, &locator.native()?, text).map_err(Error::from)
	}

	fn alert_text(&self) -> Result<String, Error> {
		NativeTester::alert_text(self).map_err(Error::from)
	}

	fn accept_alert(&self) -> Result<(), Error> {
		NativeTester::accept_alert(self).map_err(Error::from)
	}

	fn dismiss_alert(&self) -> Result<(), Error> {
		NativeTester::dismiss_alert(self).map_err(Error::from)
	}

	fn screenshot_png(&self) -> Result<Vec<u8>, Error> {
		Ok(NativeTester::screenshot(self)
			.map_err(Error::from)?
			.png()
			.to_vec())
	}

	fn source(&self) -> Result<String, Error> {
		NativeTester::source(self).map_err(Error::from)
	}

	fn perform_actions(&self, sources: &Value) -> Result<(), Error> {
		NativeTester::perform_actions(self, sources).map_err(Error::from)
	}

	fn execute(&self, script: &str, arguments: &Value) -> Result<Value, Error> {
		NativeTester::native_command(self, script, arguments).map_err(Error::from)
	}

	fn close(self: Box<Self>, failure: Option<&Error>) -> Result<(), Error> {
		let tester = *self;
		let result = match failure {
			// The step error was already recorded; this run only asks the
			// session to retain its evidence and end.
			Some(_) => {
				tester.finish::<()>(Err(crate::webdriver::Error::Assertion(
					"a control step failed; see the outcome".to_owned(),
				)))
			}
			None => tester.finish::<()>(Ok(())),
		};
		match result {
			Ok(()) => Ok(()),
			// The Evidence wrapper around our placeholder is the expected
			// shape of a failure-path close; the real error was recorded.
			Err(crate::webdriver::Error::Evidence { .. }) if failure.is_some() => Ok(()),
			Err(error) => Err(Error::from(error)),
		}
	}
}

impl ControlDriver for web::WebSession {
	fn tap(&self, locator: &Locator) -> Result<(), Error> {
		web::WebSession::tap(self, locator)
	}

	fn enter_text(&self, locator: &Locator, text: &str) -> Result<(), Error> {
		web::WebSession::enter_text(self, locator, text)
	}

	fn type_text(&self, text: &str) -> Result<(), Error> {
		web::WebSession::type_text(self, text)
	}

	fn expect_visible(&self, locator: &Locator) -> Result<(), Error> {
		web::WebSession::expect_visible(self, locator)
	}

	fn expect_absent(&self, locator: &Locator) -> Result<(), Error> {
		web::WebSession::expect_absent(self, locator)
	}

	fn expect_text(&self, locator: &Locator, text: &str) -> Result<(), Error> {
		web::WebSession::expect_text(self, locator, text)
	}

	fn alert_text(&self) -> Result<String, Error> {
		web::WebSession::alert_text(self)
	}

	fn accept_alert(&self) -> Result<(), Error> {
		web::WebSession::accept_alert(self)
	}

	fn dismiss_alert(&self) -> Result<(), Error> {
		web::WebSession::dismiss_alert(self)
	}

	fn screenshot_png(&self) -> Result<Vec<u8>, Error> {
		web::WebSession::screenshot_png(self)
	}

	fn source(&self) -> Result<String, Error> {
		web::WebSession::source(self)
	}

	fn perform_actions(&self, sources: &Value) -> Result<(), Error> {
		web::WebSession::perform_actions(self, sources)
	}

	fn execute(&self, script: &str, arguments: &Value) -> Result<Value, Error> {
		web::WebSession::execute(self, script, arguments)
	}

	fn close(self: Box<Self>, _failure: Option<&Error>) -> Result<(), Error> {
		let mut session = *self;
		session.close()?;
		Ok(())
	}
}

/// Cuts text on a character boundary so values stay valid strings.
fn truncate_chars(text: &str, limit: usize) -> String {
	if text.len() <= limit {
		return text.to_owned();
	}
	let mut end = limit;
	while !text.is_char_boundary(end) {
		end -= 1;
	}
	let mut cut = text[..end].to_owned();
	cut.push('…');
	cut
}

/// Runs one platform tool or fails with its output.
fn tool(command: &mut Command, timeout: Duration) -> Result<tools::ToolOutput, Error> {
	let output = tools::run(command, timeout)?;
	if !output.success() {
		return Err(Error::Tool {
			name: command.get_program().to_string_lossy().into_owned(),
			detail: output.failure(),
		});
	}
	Ok(output)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn web_targets_need_urls_and_native_targets_need_devices() {
		let mut document = Document {
			schema_version: SCHEMA_VERSION,
			target: Target {
				kind: TargetKind::Web,
				device: None,
				application: None,
				url: None,
				browser: None,
				endpoint: None,
				capabilities: BTreeMap::new(),
				headless: true,
				startup_seconds: 300,
				command_seconds: 30,
				wait_seconds: 15,
				artifacts: PathBuf::from("target/leptoskit-control"),
			},
			steps: vec![],
		};
		assert!(
			document
				.validate()
				.unwrap_err()
				.to_string()
				.contains("require a url")
		);
		document.target.url = Some("https://example.com".to_owned());
		document.validate().unwrap();
		document.target.device = Some("x".to_owned());
		assert!(
			document
				.validate()
				.unwrap_err()
				.to_string()
				.contains("take no device")
		);
		document.target.device = None;
		document.target.kind = TargetKind::Android;
		document.target.url = None;
		// No step reaches the device, so boot-only documents need no serial.
		document.validate().unwrap();
		document.steps = vec![Step::DeviceScreenshot {
			label: "home".to_owned(),
		}];
		assert!(
			document
				.validate()
				.unwrap_err()
				.to_string()
				.contains("require a device id")
		);
		document.target.device = Some("emulator-5554".to_owned());
		document.validate().unwrap();
	}

	#[test]
	fn validation_names_each_mistake() {
		let base = Document {
			schema_version: SCHEMA_VERSION,
			target: Target {
				kind: TargetKind::Web,
				device: None,
				application: None,
				url: Some("https://example.com".to_owned()),
				browser: None,
				endpoint: None,
				capabilities: BTreeMap::new(),
				headless: true,
				startup_seconds: 300,
				command_seconds: 30,
				wait_seconds: 15,
				artifacts: PathBuf::from("target/leptoskit-control"),
			},
			steps: vec![],
		};
		let mut document = base.clone();
		document.target.kind = TargetKind::Android;
		document.target.url = None;
		document.target.device = Some("emulator-5554".to_owned());
		document.target.browser = Some(Browser::Chrome);
		assert!(
			document
				.validate()
				.unwrap_err()
				.to_string()
				.contains("belong to web targets")
		);
		document = base.clone();
		document.target.application = Some(Application::Desktop);
		assert!(
			document
				.validate()
				.unwrap_err()
				.to_string()
				.contains("take no device")
		);
		document = base.clone();
		document.target.wait_seconds = 0;
		assert!(
			document
				.validate()
				.unwrap_err()
				.to_string()
				.contains("between 1 and 3600")
		);
	}

	#[test]
	fn webdriver_errors_translate_losslessly() {
		let converted = Error::from(crate::webdriver::Error::Ambiguous {
			selector: "two".to_owned(),
			count: 2,
		});
		assert!(converted.to_string().contains("matched 2"));
		let converted = Error::from(crate::webdriver::Error::Driver {
			code: "no such alert".to_owned(),
			message: "nothing is up".to_owned(),
		});
		assert!(converted.to_string().contains("no such alert"));
		let converted = Error::from(crate::webdriver::Error::Evidence {
			directory: PathBuf::from("/tmp/here"),
			source: Box::new(crate::webdriver::Error::Timeout("wait".to_owned())),
		});
		assert!(converted.to_string().contains("/tmp/here"));
		let converted = Error::from(crate::webdriver::Error::Combined {
			primary: Box::new(crate::webdriver::Error::Timeout("first".to_owned())),
			secondary: Box::new(crate::webdriver::Error::Timeout("second".to_owned())),
		});
		assert!(converted.to_string().contains("additionally"));
	}

	#[test]
	fn native_locators_cover_every_strategy() {
		let all = [
			Locator::AccessibilityId("a".to_owned()),
			Locator::Text("b".to_owned()),
			Locator::Id("c".to_owned()),
			Locator::Xpath("//d".to_owned()),
		];
		for locator in all {
			assert!(locator.native().is_ok(), "{locator:?}");
		}
	}

	#[test]
	fn truncation_lands_on_character_boundaries() {
		// é is two bytes; cut inside it and the walker steps back to the edge.
		let text = "é".repeat(3);
		let cut = truncate_chars(&text, 3);
		assert_eq!(cut.chars().count(), 2, "one é survives, then the ellipsis");
		assert!(cut.ends_with('…'));
	}

	#[test]
	fn documents_parse_with_defaults_and_reject_unknown_fields() {
		let document: Document = serde_json::from_str(
			r##"{
				"schema_version": 1,
				"target": {
					"kind": "web",
					"url": "https://example.com",
					"browser": "firefox",
					"headless": false
				},
				"steps": [
					{"action": "tap", "locator": {"by": "text", "value": "Sign in"}},
					{"action": "enter_text", "locator": {"by": "css", "value": "#email"}, "text": "a@b.c"},
					{"action": "swipe", "start": {"x": 10, "y": 20}, "end": {"x": 10, "y": 200}},
					{"action": "device_screenshot", "label": "home"}
				]
			}"##,
		)
		.unwrap();
		assert_eq!(document.target.wait_seconds, 15);
		assert_eq!(document.target.browser, Some(Browser::Firefox));
		assert!(!document.target.headless);
		assert_eq!(document.steps.len(), 4);
		assert!(matches!(
			document.steps[2],
			Step::Swipe {
				duration_ms: None,
				..
			}
		));
		let unknown = serde_json::from_str::<Document>(
			r#"{"schema_version": 1, "target": {"kind": "web", "url": "x"}, "steps": [], "wat": 1}"#,
		);
		assert!(unknown.is_err());
	}

	#[test]
	fn css_locators_are_web_only() {
		assert!(Locator::Css("#a".to_owned()).native().is_err());
		assert!(Locator::Text("Go".to_owned()).native().is_ok());
	}

	#[test]
	fn web_locators_map_to_standard_strategies() {
		assert_eq!(
			Locator::Text("Sign in".to_owned()).web(),
			crate::webdriver::Selector::new("xpath", "//*[text()='Sign in']")
		);
		assert_eq!(
			Locator::Id("email".to_owned()).web(),
			crate::webdriver::Selector::new("xpath", "//*[@id='email']")
		);
		assert_eq!(
			Locator::Css("#email".to_owned()).web(),
			crate::webdriver::Selector::new("css selector", "#email")
		);
		assert_eq!(
			Locator::AccessibilityId("send".to_owned()).web(),
			crate::webdriver::Selector::new(
				"xpath",
				"//*[@aria-label='send' or @data-testid='send']"
			)
		);
		assert_eq!(
			Locator::Xpath("//a".to_owned()).web(),
			crate::webdriver::Selector::new("xpath", "//a")
		);
	}

	#[test]
	fn text_truncates_on_character_boundaries() {
		let text = "é".repeat(6);
		let cut = truncate_chars(&text, 4);
		assert!(cut.chars().count() <= 5);
		assert!(cut.ends_with('…'));
		assert_eq!(truncate_chars("short", 100), "short");
	}
}
