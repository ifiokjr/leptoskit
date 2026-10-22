//! Real native UI automation through platform `WebDriver` servers.
//!
//! iOS uses `XCUITest`, Android UI Automator, macOS `XCTest` through Mac2,
//! Windows NovaWindows/UI Automation, and Linux KDE's AT-SPI `WebDriver`.
//! These drivers interact with
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
	/// Appium Mac2 (`XCTest` for macOS), hosted in an interactive macOS
	/// session. Sessions are scoped to one application, which includes the
	/// sheets and alerts it presents, and quit that application when they
	/// end.
	Macos,
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
	/// macOS needs an application bundle, not a bare executable.
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
	/// Drivers may take up to the longer of this and `wait_seconds` for any
	/// one command; see [`Options::driver_command_seconds`].
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

	/// The longest any one driver command may take: the per-command deadline
	/// or the wait deadline, whichever is longer, since a wait's reads use the
	/// latter. iOS sessions ask `WebDriverAgent` to enforce this bound itself,
	/// and the client allows a margin beyond it, so a slow action is reported
	/// by the driver rather than abandoned while it may still be running.
	pub fn driver_command_seconds(&self) -> u64 {
		self.command_seconds.max(self.wait_seconds)
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
			Platform::Macos => ("mac", "Mac2"),
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
				let key = if self.platform == Platform::Macos {
					"appium:appPath"
				} else {
					"appium:app"
				};
				capabilities.insert(key.to_owned(), json!(path));
				if self.platform == Platform::Macos {
					// WebDriverAgentMac terminates the apps it owns by bundle
					// identifier when a session ends. A path-launched app
					// carries none, so WDA leaves it running after
					// DELETE /session and the next journey starts with it
					// still on screen. An explicit skipAppKill claims the app
					// so teardown quits it.
					capabilities.insert("appium:skipAppKill".to_owned(), json!(false));
				}
			}
			Application::Identifier(id) => {
				if id.trim().is_empty() {
					return Err(Error::Configuration(
						"application identity is empty".to_owned(),
					));
				}
				let key = match self.platform {
					Platform::Ios | Platform::Macos => "appium:bundleId",
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
			capabilities.insert(
				"appium:commandTimeouts".to_owned(),
				json!((self.driver_command_seconds() * 1000).to_string()),
			);
			// Before each action XCTest waits for the app to go idle. System
			// animations settle well within a second, while continuous ones
			// (a focused field's caret) never do, so a longer wait only adds a
			// fixed delay; tests already wait for observable state instead.
			capabilities.insert("appium:waitForIdleTimeout".to_owned(), json!(1));
			// While a system alert is up, the accessibility tree comes from
			// SpringBoard, and WebDriverAgent's per-element visibility and
			// accessibility checks can stall there past any deadline after an
			// accessibility error. Evidence trees keep every element's type,
			// name, label, value, and frame without those two attributes.
			capabilities.insert(
				"appium:settings[pageSourceExcludedAttributes]".to_owned(),
				json!("visible,accessible"),
			);
		}
		if self.platform == Platform::Android {
			capabilities.insert("appium:autoGrantPermissions".to_owned(), json!(false));
		}
		if self.platform == Platform::Macos {
			// The driver builds and launches its XCTest runner on first use,
			// which the session's own startup budget already covers.
			capabilities.insert(
				"appium:serverStartupTimeout".to_owned(),
				json!(self.startup_seconds * 1000),
			);
		}
		for (key, value) in &self.capabilities {
			if !key.contains(':')
				|| matches!(
					key.as_str(),
					"appium:automationName"
						| "appium:app"
						| "appium:appPath"
						| "appium:commandTimeouts"
						| "appium:udid"
						| "appium:bundleId"
						| "appium:appPackage"
						| "appium:skipAppKill"
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
					// Mac2 resolves `name` to the accessibility identifier.
					// AppKit puts button text in `title` and label text in
					// `value`.
					Platform::Macos => {
						let value = xpath_literal(value);
						(
							"xpath",
							format!("//*[@title={value} or @label={value} or @value={value}]"),
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
		assert_eq!(
			Locator::text("Open").wire(Platform::Macos),
			json!({
				"using": "xpath",
				"value": "//*[@title='Open' or @label='Open' or @value='Open']"
			})
		);
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

	#[test]
	fn ios_drivers_enforce_the_command_bound_and_skip_long_idle_waits() {
		let mut options = Options::new(
			"http://localhost:4723",
			Platform::Ios,
			Application::Identifier("dev.app".to_owned()),
		);
		options.device = Some("device".to_owned());
		options.wait_seconds = 60;
		assert_eq!(options.driver_command_seconds(), 60);
		let ios = always_match(&options);
		assert_eq!(ios["appium:commandTimeouts"], "60000");
		assert_eq!(ios["appium:waitForIdleTimeout"], 1);
		options
			.capabilities
			.insert("appium:commandTimeouts".to_owned(), json!("1000"));
		assert!(configuration_error(&options).contains("commandTimeouts"));
		options.capabilities.clear();
		options
			.capabilities
			.insert("appium:waitForIdleTimeout".to_owned(), json!(3));
		assert_eq!(always_match(&options)["appium:waitForIdleTimeout"], 3);
	}

	#[test]
	fn ios_evidence_trees_skip_attributes_that_stall_on_system_alerts() {
		let mut options = Options::new(
			"http://localhost:4723",
			Platform::Ios,
			Application::Identifier("dev.app".to_owned()),
		);
		options.device = Some("device".to_owned());
		let key = "appium:settings[pageSourceExcludedAttributes]";
		assert_eq!(always_match(&options)[key], "visible,accessible");
		// A test that needs those attributes can still ask for them.
		options.capabilities.insert(key.to_owned(), json!(""));
		assert_eq!(always_match(&options)[key], "");
		options.capabilities.clear();
		options.platform = Platform::Android;
		assert!(always_match(&options).get(key).is_none());
	}

	#[test]
	fn macos_path_sessions_quit_their_app_when_they_end() {
		let options = Options::new(
			"http://localhost:4723",
			Platform::Macos,
			Application::Path("/tmp/Lab.app".to_owned()),
		);
		assert_eq!(always_match(&options)["appium:skipAppKill"], false);
		// Identifier sessions already name an app WDA owns, and other
		// platforms terminate their launched apps by default.
		let installed = Options::new(
			"http://localhost:4723",
			Platform::Macos,
			Application::Identifier("dev.lab".to_owned()),
		);
		assert!(always_match(&installed).get("appium:skipAppKill").is_none());
		let windows = Options::new(
			"http://localhost:4723",
			Platform::Windows,
			Application::Path("C:/App.exe".to_owned()),
		);
		assert!(always_match(&windows).get("appium:skipAppKill").is_none());
		// Session teardown behavior is typed, never a raw capability.
		let mut overridden = options.clone();
		overridden
			.capabilities
			.insert("appium:skipAppKill".to_owned(), json!(true));
		assert!(configuration_error(&overridden).contains("skipAppKill"));
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
		for platform in [Platform::Ios, Platform::Android, Platform::Macos] {
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
		let macos = identity(
			Platform::Macos,
			Application::Path("/tmp/Lab.app".to_owned()),
		);
		assert_eq!(macos["platformName"], "mac");
		assert_eq!(macos["appium:automationName"], "Mac2");
		assert_eq!(macos["appium:appPath"], "/tmp/Lab.app");
		assert!(macos.get("appium:app").is_none());
		assert!(macos.get("appium:udid").is_none());
		let installed = identity(
			Platform::Macos,
			Application::Identifier("dev.lab".to_owned()),
		);
		assert_eq!(installed["appium:bundleId"], "dev.lab");
	}

	#[test]
	fn macos_sessions_bound_runner_startup_by_the_session_budget() {
		let mut options = Options::new(
			"http://localhost:4723",
			Platform::Macos,
			Application::Path("/tmp/Lab.app".to_owned()),
		);
		options.startup_seconds = 600;
		assert_eq!(
			always_match(&options)["appium:serverStartupTimeout"],
			600_000
		);
		// The bundle path is a typed field, never a raw capability.
		options
			.capabilities
			.insert("appium:appPath".to_owned(), json!("/tmp/Other.app"));
		assert!(configuration_error(&options).contains("appPath"));
		options.platform = Platform::Windows;
		options.capabilities.clear();
		assert!(
			always_match(&options)
				.get("appium:serverStartupTimeout")
				.is_none()
		);
	}
}
