//! Real native UI automation through platform `WebDriver` servers.
//!
//! iOS uses `XCUITest`, Android UI Automator, Windows NovaWindows/UI
//! Automation, and Linux KDE's AT-SPI `WebDriver`. These drivers interact with
//! native views and system UI outside the application process. Screenshots are
//! driver PNGs.

mod protocol;
mod session;
pub mod suite;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
pub use session::NativeTester;

/// The OS whose native automation driver handles this session.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
	/// Appium `XCUITest`, hosted on macOS.
	Ios,
	/// Appium UI Automator 2.
	Android,
	/// Appium `NovaWindows`, hosted in an interactive Windows desktop.
	Windows,
	/// KDE Selenium AT-SPI, hosted in a Linux graphical session.
	Linux,
}

/// An application artifact, installed identity, or the whole desktop.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
	tag = "kind",
	content = "value",
	rename_all = "snake_case",
	deny_unknown_fields
)]
pub enum Application {
	/// An app path on the driver host: .app/.ipa, .apk, .exe, or Linux command.
	Path(String),
	/// Installed bundle/package identity. Android also needs
	/// `appium:appActivity`.
	Identifier(String),
	/// The desktop accessibility root, including dialogs from other processes.
	Desktop,
}

/// Session setup. Paths refer to the driver host, which may be a remote
/// machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
	/// W3C `WebDriver` endpoint, including any server base path.
	pub endpoint: String,
	/// Native target OS.
	pub platform: Platform,
	/// Application to launch or attach to.
	pub application: Application,
	/// Explicit device ID for mobile sessions.
	#[serde(default)]
	pub device: Option<String>,
	/// Additional namespaced driver capabilities.
	#[serde(default)]
	pub capabilities: BTreeMap<String, Value>,
	/// Deadline for session creation, including driver boot/build.
	#[serde(default = "default_startup")]
	pub startup_seconds: u64,
	/// Deadline for a single action, capture, or session request. Reads made
	/// while waiting for a UI condition share that wait's deadline instead.
	#[serde(default = "default_command")]
	pub command_seconds: u64,
	/// Deadline for observable UI conditions, including every read the wait
	/// makes.
	#[serde(default = "default_wait")]
	pub wait_seconds: u64,
	/// Parent directory; every session creates an isolated child.
	#[serde(default = "default_artifacts")]
	pub artifacts: PathBuf,
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
	PathBuf::from("target/leptoskit-native-tests")
}

impl Options {
	/// Creates an explicit native session configuration with bounded defaults.
	pub fn new(endpoint: impl Into<String>, platform: Platform, application: Application) -> Self {
		Self {
			endpoint: endpoint.into(),
			platform,
			application,
			device: None,
			capabilities: BTreeMap::new(),
			startup_seconds: default_startup(),
			command_seconds: default_command(),
			wait_seconds: default_wait(),
			artifacts: default_artifacts(),
		}
	}

	fn driver_capabilities(&self) -> Result<Value, Error> {
		if [
			self.startup_seconds,
			self.command_seconds,
			self.wait_seconds,
		]
		.iter()
		.any(|value| !(1..=3600).contains(value))
		{
			return Err(Error::Configuration(
				"deadlines must be between 1 and 3600 seconds".to_owned(),
			));
		}
		let (platform, driver) = match self.platform {
			Platform::Ios => ("iOS", "XCUITest"),
			Platform::Android => ("Android", "UiAutomator2"),
			Platform::Windows => ("Windows", "NovaWindows"),
			Platform::Linux => ("linux", "atspi"),
		};
		let mut capabilities = BTreeMap::from([
			("platformName".to_owned(), json!(platform)),
			("appium:automationName".to_owned(), json!(driver)),
		]);
		if matches!(self.platform, Platform::Ios | Platform::Android) {
			let device = self
				.device
				.as_deref()
				.filter(|id| !id.trim().is_empty())
				.ok_or_else(|| {
					Error::Configuration("mobile sessions require an explicit device ID".to_owned())
				})?;
			capabilities.insert("appium:udid".to_owned(), json!(device));
			capabilities.insert("appium:noReset".to_owned(), json!(true));
			capabilities.insert(
				"appium:newCommandTimeout".to_owned(),
				json!(self.command_seconds + self.wait_seconds + 60),
			);
		}
		match &self.application {
			Application::Path(path) => {
				if path.trim().is_empty() {
					return Err(Error::Configuration("application path is empty".to_owned()));
				}
				capabilities.insert("appium:app".to_owned(), json!(path));
			}
			Application::Identifier(id) => {
				if id.trim().is_empty() {
					return Err(Error::Configuration(
						"application identity is empty".to_owned(),
					));
				}
				let key = match self.platform {
					Platform::Ios => "appium:bundleId",
					Platform::Android => "appium:appPackage",
					Platform::Windows | Platform::Linux => "appium:app",
				};
				capabilities.insert(key.to_owned(), json!(id));
			}
			Application::Desktop => {
				if !matches!(self.platform, Platform::Windows | Platform::Linux) {
					return Err(Error::Configuration(
						"desktop sessions require Windows or Linux".to_owned(),
					));
				}
				capabilities.insert("appium:app".to_owned(), json!("Root"));
				if self.platform == Platform::Windows {
					capabilities.insert("appium:shouldCloseApp".to_owned(), json!(false));
				}
			}
		}
		if self.platform == Platform::Ios {
			capabilities.insert(
				"appium:settings[respectSystemAlerts]".to_owned(),
				json!(true),
			);
			capabilities.insert("appium:autoAcceptAlerts".to_owned(), json!(false));
			capabilities.insert("appium:autoDismissAlerts".to_owned(), json!(false));
		}
		if self.platform == Platform::Android {
			capabilities.insert("appium:autoGrantPermissions".to_owned(), json!(false));
		}
		for (key, value) in &self.capabilities {
			if !key.contains(':')
				|| matches!(
					key.as_str(),
					"appium:automationName"
						| "appium:app"
						| "appium:udid"
						| "appium:bundleId"
						| "appium:appPackage"
				) {
				return Err(Error::Configuration(format!(
					"capability {key:?} must use the typed session fields or a driver namespace"
				)));
			}
			capabilities.insert(key.clone(), value.clone());
		}
		Ok(json!({"capabilities": {"alwaysMatch": capabilities, "firstMatch": [{}]}}))
	}
}

/// A semantic native selector, resolved afresh before every action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
	tag = "by",
	content = "value",
	rename_all = "snake_case",
	deny_unknown_fields
)]
pub enum Locator {
	/// Accessibility identifier / `AutomationId` / Android content description.
	AccessibilityId(String),
	/// Visible text or accessible name.
	Text(String),
	/// Android resource ID or the driver's native ID strategy.
	Id(String),
	/// Explicit native-tree `XPath` for cases without semantic identifiers.
	Xpath(String),
}

impl Locator {
	/// Matches an explicit accessibility identifier.
	pub fn accessibility_id(value: impl Into<String>) -> Self {
		Self::AccessibilityId(value.into())
	}

	/// Matches visible text or an accessible name.
	pub fn text(value: impl Into<String>) -> Self {
		Self::Text(value.into())
	}

	fn wire(&self, platform: Platform) -> Value {
		let (strategy, selector) = match self {
			Self::AccessibilityId(value) => ("accessibility id", value.clone()),
			Self::Id(value) => ("id", value.clone()),
			Self::Xpath(value) => ("xpath", value.clone()),
			Self::Text(value) => {
				match platform {
					Platform::Android => ("xpath", format!("//*[@text={}]", xpath_literal(value))),
					Platform::Ios => {
						(
							"xpath",
							format!(
								"//*[@label={} and @accessible='true']",
								xpath_literal(value)
							),
						)
					}
					_ => ("name", value.clone()),
				}
			}
		};
		json!({"using": strategy, "value": selector})
	}
}

fn xpath_literal(value: &str) -> String {
	if !value.contains('\'') {
		format!("'{value}'")
	} else if !value.contains('"') {
		format!("\"{value}\"")
	} else {
		format!(
			"concat({})",
			value
				.split('\'')
				.map(|part| format!("'{part}'"))
				.collect::<Vec<_>>()
				.join(",\"'\",")
		)
	}
}

/// Native automation failure. Driver errors retain their W3C error code.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// A failed run with its retained evidence location and original error.
	#[error("{source}; evidence: {directory}")]
	Evidence {
		directory: PathBuf,
		#[source]
		source: Box<Self>,
	},
	/// Invalid session options or unsupported target selection.
	#[error("native testing configuration: {0}")]
	Configuration(String),
	/// Network or driver request failure.
	#[error(transparent)]
	Transport(#[from] reqwest::Error),
	/// Local evidence could not be read or written.
	#[error(transparent)]
	Io(#[from] std::io::Error),
	/// Malformed driver response.
	#[error(transparent)]
	Json(#[from] serde_json::Error),
	/// Native driver explicitly rejected the operation.
	#[error("native driver {code}: {message}")]
	Driver { code: String, message: String },
	/// Driver returned a structurally invalid response.
	#[error("invalid native driver response: {0}")]
	Response(String),
	/// A visible state failed to appear within its deadline.
	#[error("native UI condition timed out: {0}")]
	Timeout(String),
	/// More than one control matches; no action was taken.
	#[error("native selector {locator:?} matched {count} visible controls")]
	Ambiguous { locator: Locator, count: usize },
	/// An observable assertion failed.
	#[error("native UI assertion: {0}")]
	Assertion(String),
	/// Screenshot bytes are not a valid PNG.
	#[error(transparent)]
	Capture(#[from] crate::capture::CaptureError),
	/// Evidence or session cleanup failed in addition to another failure.
	/// The primary failure keeps its original variant.
	#[error("{primary}; additionally: {secondary}")]
	Combined {
		#[source]
		primary: Box<Self>,
		secondary: Box<Self>,
	},
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn text_queries_use_visible_labels_even_when_ios_has_an_identifier() {
		let label = "It's a \"label\"";
		let selector = Locator::text(label).wire(Platform::Ios);
		assert_eq!(selector["using"], "xpath");
		assert_eq!(
			selector["value"],
			"//*[@label=concat('It',\"'\",'s a \"label\"') and @accessible='true']"
		);
		assert_eq!(
			Locator::text(label).wire(Platform::Android)["using"],
			"xpath"
		);
		assert_eq!(Locator::text(label).wire(Platform::Windows)["value"], label);
	}

	#[test]
	fn native_platforms_keep_permission_dialogs_visible_and_require_explicit_devices() {
		for platform in [Platform::Ios, Platform::Android] {
			let mut options = Options::new(
				"http://localhost:4723",
				platform,
				Application::Identifier("dev.example".to_owned()),
			);
			assert!(options.driver_capabilities().is_err());
			options.device = Some("selected-device".to_owned());
			let capabilities = options.driver_capabilities().unwrap();
			let capabilities = &capabilities["capabilities"]["alwaysMatch"];
			assert_eq!(capabilities["appium:udid"], "selected-device");
			let key = if platform == Platform::Ios {
				"appium:autoAcceptAlerts"
			} else {
				"appium:autoGrantPermissions"
			};
			assert_eq!(capabilities[key], false);
			options
				.capabilities
				.insert("appium:udid".to_owned(), json!("different-device"));
			assert!(options.driver_capabilities().is_err());
		}
	}

	fn always_match(options: &Options) -> Value {
		options.driver_capabilities().unwrap()["capabilities"]["alwaysMatch"].clone()
	}

	fn configuration_error(options: &Options) -> String {
		match options.driver_capabilities() {
			Err(Error::Configuration(message)) => message,
			other => panic!("expected a configuration error, got {other:?}"),
		}
	}

	#[test]
	fn locators_map_to_each_platform_strategy() {
		let wire = |locator: Locator| locator.wire(Platform::Linux);
		assert_eq!(
			wire(Locator::accessibility_id("save")),
			json!({"using": "accessibility id", "value": "save"})
		);
		assert_eq!(
			wire(Locator::Id("android:id/button1".to_owned())),
			json!({"using": "id", "value": "android:id/button1"})
		);
		assert_eq!(
			wire(Locator::Xpath("//Button".to_owned())),
			json!({"using": "xpath", "value": "//Button"})
		);
		assert_eq!(
			wire(Locator::text("Open")),
			json!({"using": "name", "value": "Open"})
		);
		assert_eq!(
			Locator::text("Plain").wire(Platform::Android)["value"],
			"//*[@text='Plain']"
		);
		assert_eq!(xpath_literal("Don't"), "\"Don't\"");
	}

	#[test]
	fn deadlines_and_application_targets_are_validated() {
		let mut options = Options::new(
			"http://localhost:4723",
			Platform::Windows,
			Application::Path(" ".to_owned()),
		);
		assert_eq!(configuration_error(&options), "application path is empty");
		options.application = Application::Identifier(String::new());
		assert_eq!(
			configuration_error(&options),
			"application identity is empty"
		);
		options.application = Application::Desktop;
		for deadline in [0, 3601] {
			options.wait_seconds = deadline;
			assert_eq!(
				configuration_error(&options),
				"deadlines must be between 1 and 3600 seconds"
			);
		}
		options.wait_seconds = 15;
		options
			.capabilities
			.insert("noNamespace".to_owned(), json!(true));
		assert!(configuration_error(&options).contains("noNamespace"));
		options.capabilities.clear();
		for platform in [Platform::Ios, Platform::Android] {
			let mut mobile = options.clone();
			mobile.platform = platform;
			mobile.device = Some("device".to_owned());
			assert_eq!(
				configuration_error(&mobile),
				"desktop sessions require Windows or Linux"
			);
		}
	}

	#[test]
	fn each_platform_receives_its_driver_and_application_identity() {
		let identity = |platform, application| {
			let mut options = Options::new("http://localhost:4723", platform, application);
			options.device = Some("device".to_owned());
			options
				.capabilities
				.insert("appium:newCommandTimeout".to_owned(), json!(600));
			always_match(&options)
		};
		let ios = identity(Platform::Ios, Application::Identifier("dev.app".to_owned()));
		assert_eq!(ios["appium:automationName"], "XCUITest");
		assert_eq!(ios["appium:bundleId"], "dev.app");
		assert_eq!(ios["appium:settings[respectSystemAlerts]"], true);
		// Explicit namespaced capabilities override the computed defaults.
		assert_eq!(ios["appium:newCommandTimeout"], 600);
		let android = identity(
			Platform::Android,
			Application::Identifier("dev.app".to_owned()),
		);
		assert_eq!(android["appium:automationName"], "UiAutomator2");
		assert_eq!(android["appium:appPackage"], "dev.app");
		let windows = identity(
			Platform::Windows,
			Application::Path("C:/App.exe".to_owned()),
		);
		assert_eq!(windows["appium:automationName"], "NovaWindows");
		assert_eq!(windows["appium:app"], "C:/App.exe");
		assert!(windows.get("appium:udid").is_none());
		let linux = identity(Platform::Linux, Application::Identifier("kate".to_owned()));
		assert_eq!(linux["platformName"], "linux");
		assert_eq!(linux["appium:app"], "kate");
		let desktop = identity(Platform::Linux, Application::Desktop);
		assert_eq!(desktop["appium:app"], "Root");
		assert!(desktop.get("appium:shouldCloseApp").is_none());
	}
}
