//! Browser sessions for website control: chromedriver, geckodriver, or
//! safaridriver, spawned and cleaned up by the run, or a remote endpoint.

use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use reqwest::Method;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use super::Error;
use super::Locator;
use super::Target;
use crate::webdriver::Connection;
use crate::webdriver::DRIVER_REPORT_MARGIN_SECONDS;
use crate::webdriver::Limits;
use crate::webdriver::Selector;
use crate::webdriver::Session;

/// The browser a web target drives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Browser {
	/// Chrome through chromedriver.
	#[default]
	Chrome,
	/// Firefox through geckodriver.
	Firefox,
	/// Safari through the macOS safaridriver.
	Safari,
}

impl Browser {
	/// The driver executable this browser needs.
	pub fn driver(self) -> &'static str {
		match self {
			Self::Chrome => "chromedriver",
			Self::Firefox => "geckodriver",
			Self::Safari => "safaridriver",
		}
	}

	fn capabilities(self, headless: bool) -> Value {
		match self {
			Self::Chrome => {
				let mut options = json!({"browserName": "chrome", "goog:chromeOptions": {}});
				if headless {
					options["goog:chromeOptions"]["args"] = json!(["--headless=new"]);
				}
				options
			}
			Self::Firefox => {
				let mut options = json!({"browserName": "firefox", "moz:firefoxOptions": {}});
				if headless {
					options["moz:firefoxOptions"]["args"] = json!(["-headless"]);
				}
				options
			}
			// Safari's driver has no headless mode.
			Self::Safari => json!({"browserName": "safari"}),
		}
	}
}

/// A spawned browser driver, killed when the session ends.
struct DriverGuard {
	child: std::process::Child,
}

impl DriverGuard {
	fn spawn(browser: Browser, path: &std::path::Path, port: u16) -> Result<Self, Error> {
		let mut command = Command::new(path);
		match browser {
			Browser::Chrome => {
				command.args([
					format!("--port={port}"),
					// Keep the driver loopback-only where the flag exists.
					"--allowed-ips=127.0.0.1".to_owned(),
				]);
			}
			Browser::Firefox | Browser::Safari => {
				command.arg("--port").arg(port.to_string());
			}
		}
		let child = command
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|error| {
				Error::Configuration(format!(
					"starting {} for a web target: {error}",
					browser.driver()
				))
			})?;
		Ok(Self { child })
	}
}

impl Drop for DriverGuard {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// One website automation session.
pub(crate) struct WebSession {
	session: Session,
	_driver: Option<DriverGuard>,
}

impl WebSession {
	/// Connects to a browser driver, spawns one when no endpoint is
	/// configured, and navigates to the target's URL.
	pub fn connect(target: &Target) -> Result<Self, Error> {
		let browser = target.browser.unwrap_or_default();
		let url = target
			.url
			.as_deref()
			.ok_or_else(|| Error::Configuration("web targets require a url".to_owned()))?;
		let (endpoint, driver) = if let Some(endpoint) = target.endpoint.as_deref() {
			(endpoint.to_owned(), None)
		} else {
			let program = browser.driver();
			let path: PathBuf = crate::control::tools::find_in_path(program).ok_or_else(|| {
				Error::Configuration(format!(
					"{program} is not installed; it drives {} targets",
					browser_name(browser)
				))
			})?;
			let port = crate::control::tools::free_port().ok_or_else(|| {
				Error::Configuration("no free loopback port for the browser driver".to_owned())
			})?;
			let guard = DriverGuard::spawn(browser, &path, port)?;
			(format!("http://127.0.0.1:{port}"), Some(guard))
		};
		let limits = Limits {
			startup: Duration::from_secs(target.startup_seconds.min(120)),
			command: Duration::from_secs(
				target.command_seconds.max(target.wait_seconds) + DRIVER_REPORT_MARGIN_SECONDS,
			),
			wait: Duration::from_secs(target.wait_seconds),
			implicit_wait_ms: 0,
			atspi_quirks: false,
		};
		let mut always_match = browser.capabilities(target.headless);
		for (key, value) in &target.capabilities {
			merge_capability(&mut always_match, key, value);
		}
		let capabilities = json!({"capabilities": {"alwaysMatch": always_match}});
		let connection = Connection::new(&endpoint)?;
		let session = Session::start(connection, &capabilities, limits)?;
		let web = Self {
			session,
			_driver: driver,
		};
		web.session
			.command(Method::POST, &["url"], Some(&json!({ "url": url })))?;
		Ok(web)
	}

	/// Deletes the session; the spawned driver, if any, dies with it.
	pub fn close(&mut self) -> Result<(), Error> {
		self.session.close().map_err(Error::from)
	}

	/// Waits for one visible enabled element and clicks it.
	pub fn tap(&self, locator: &Locator) -> Result<(), Error> {
		let id = self.wait(
			locator,
			"to be visible and enabled",
			|session, id, timeout| session.element_bool(id, "enabled", timeout),
		)?;
		self.session
			.command(Method::POST, &["element", &id, "click"], Some(&json!({})))?;
		Ok(())
	}

	/// Replaces the text of one visible enabled input.
	pub fn enter_text(&self, locator: &Locator, text: &str) -> Result<(), Error> {
		let id = self.wait(
			locator,
			"to be visible and enabled",
			|session, id, timeout| session.element_bool(id, "enabled", timeout),
		)?;
		self.session
			.command(Method::POST, &["element", &id, "clear"], Some(&json!({})))?;
		self.session.command(
			Method::POST,
			&["element", &id, "value"],
			Some(&json!({
				"text": text,
				"value": text.chars().map(|c| c.to_string()).collect::<Vec<_>>()
			})),
		)?;
		Ok(())
	}

	/// Types into whatever has focus.
	pub fn type_text(&self, text: &str) -> Result<(), Error> {
		self.session.type_text(text).map_err(Error::from)
	}

	/// Waits for exactly one visible element.
	pub fn expect_visible(&self, locator: &Locator) -> Result<(), Error> {
		self.wait(locator, "to be visible", |_, _, _| Ok(true))?;
		Ok(())
	}

	/// Waits until no element matches.
	pub fn expect_absent(&self, locator: &Locator) -> Result<(), Error> {
		let selector = locator.web();
		self.session
			.wait(&format!("{selector:?} to disappear"), |remaining| {
				Ok(self
					.session
					.visible_elements(&selector, remaining)?
					.is_empty()
					.then_some(()))
			})
			.map_err(Error::from)
	}

	/// Waits for an element's exact text.
	pub fn expect_text(&self, locator: &Locator, expected: &str) -> Result<(), Error> {
		self.wait(
			locator,
			&format!("to show text {expected:?}"),
			|session, id, remaining| {
				let value =
					session.request(Method::GET, &["element", id, "text"], None, remaining)?;
				Ok(value.as_str().ok_or_else(|| {
					crate::webdriver::Error::Response("element text is not a string".to_owned())
				})? == expected)
			},
		)?;
		Ok(())
	}

	/// Reads the visible dialog's text.
	pub fn alert_text(&self) -> Result<String, Error> {
		self.session
			.wait("alert", |remaining| {
				match self
					.session
					.request(Method::GET, &["alert", "text"], None, remaining)
				{
					Ok(value) => {
						value
							.as_str()
							.map(|text| Some(text.to_owned()))
							.ok_or_else(|| {
								crate::webdriver::Error::Response(
									"alert text is not a string".to_owned(),
								)
							})
					}
					Err(crate::webdriver::Error::Driver { code, .. })
						if code == "no such alert" =>
					{
						Ok(None)
					}
					Err(error) => Err(error),
				}
			})
			.map_err(Error::from)
	}

	/// Accepts the visible dialog.
	pub fn accept_alert(&self) -> Result<(), Error> {
		self.alert_text()?;
		self.session
			.command(Method::POST, &["alert", "accept"], Some(&json!({})))?;
		Ok(())
	}

	/// Dismisses the visible dialog.
	pub fn dismiss_alert(&self) -> Result<(), Error> {
		self.alert_text()?;
		self.session
			.command(Method::POST, &["alert", "dismiss"], Some(&json!({})))?;
		Ok(())
	}

	/// The viewport's actual pixels, PNG-encoded.
	pub fn screenshot_png(&self) -> Result<Vec<u8>, Error> {
		Ok(self.session.screenshot_png()?)
	}

	/// The page's current source.
	pub fn source(&self) -> Result<String, Error> {
		let tree = self.session.command(Method::GET, &["source"], None)?;
		tree.as_str()
			.map(str::to_owned)
			.ok_or_else(|| Error::Response("source is not text".to_owned()))
	}

	/// Dispatches raw W3C actions.
	pub fn perform_actions(&self, sources: &Value) -> Result<(), Error> {
		self.session
			.command(
				Method::POST,
				&["actions"],
				Some(&json!({"actions": sources})),
			)
			.map_err(Error::from)?;
		Ok(())
	}

	/// Runs synchronous JavaScript with one JSON argument.
	pub fn execute(&self, script: &str, arguments: &Value) -> Result<Value, Error> {
		self.session
			.command(
				Method::POST,
				&["execute", "sync"],
				Some(&json!({"script": script, "args": [arguments]})),
			)
			.map_err(Error::from)
	}

	fn wait(
		&self,
		locator: &Locator,
		purpose: &str,
		condition: impl Fn(&Session, &str, Duration) -> Result<bool, crate::webdriver::Error>,
	) -> Result<String, Error> {
		self.session
			.wait_for(&locator.web(), purpose, condition)
			.map_err(Error::from)
	}
}

fn browser_name(browser: Browser) -> &'static str {
	match browser {
		Browser::Chrome => "Chrome",
		Browser::Firefox => "Firefox",
		Browser::Safari => "Safari",
	}
}

/// Merges one namespaced capability into the defaults, combining object
/// values so a partial `goog:chromeOptions` keeps the computed flags.
fn merge_capability(capabilities: &mut Value, key: &str, value: &Value) {
	match (capabilities.get_mut(key), value) {
		(Some(Value::Object(defaults)), Value::Object(extra)) => {
			for (name, entry) in extra {
				defaults.insert(name.clone(), entry.clone());
			}
		}
		_ => {
			capabilities[key] = value.clone();
		}
	}
}

impl Locator {
	/// Resolves the locator for a web session.
	pub(crate) fn web(&self) -> Selector {
		match self {
			Self::AccessibilityId(value) => {
				Selector::new(
					"xpath",
					format!(
						"//*[@aria-label={} or @data-testid={}]",
						crate::webdriver::xpath_literal(value),
						crate::webdriver::xpath_literal(value)
					),
				)
			}
			Self::Text(value) => {
				Selector::new(
					"xpath",
					format!("//*[text()={}]", crate::webdriver::xpath_literal(value)),
				)
			}
			Self::Id(value) => {
				Selector::new(
					"xpath",
					format!("//*[@id={}]", crate::webdriver::xpath_literal(value)),
				)
			}
			Self::Css(value) => Selector::new("css selector", value.clone()),
			Self::Xpath(value) => Selector::new("xpath", value.clone()),
		}
	}
}
