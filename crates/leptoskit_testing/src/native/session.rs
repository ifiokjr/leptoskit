use std::fmt::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use reqwest::Method;
use serde_json::Value;
use serde_json::json;

use super::Error;
use super::Locator;
use super::Options;
use super::Platform;
use crate::capture::Screenshot;
use crate::webdriver::Connection;
use crate::webdriver::DRIVER_REPORT_MARGIN_SECONDS;
use crate::webdriver::Limits;
use crate::webdriver::Session;

static SESSION_NUMBER: AtomicU64 = AtomicU64::new(0);

/// A real OS automation session. Call [`Self::run`] to retain failure evidence
/// and propagate cleanup failures, or [`Self::finish`] when managing it
/// manually.
///
/// This synchronous API must run outside an async runtime (or in its blocking
/// pool). Only reads are retried while waiting; actions are sent exactly once.
pub struct NativeTester {
	session: Session,
	options: Options,
	artifacts: PathBuf,
	captures: Vec<(String, String)>,
}

impl NativeTester {
	/// Connects to a provisioned platform driver and launches/attaches the app.
	pub fn connect(options: Options) -> Result<Self, Error> {
		let capabilities = options.driver_capabilities()?;
		let limits = Limits {
			startup: Duration::from_secs(options.startup_seconds),
			command: Duration::from_secs(
				options.driver_command_seconds() + DRIVER_REPORT_MARGIN_SECONDS,
			),
			wait: Duration::from_secs(options.wait_seconds),
			implicit_wait_ms: u64::from(options.platform == Platform::Linux) * 100,
			atspi_quirks: options.platform == Platform::Linux,
		};
		let connection = Connection::new(&options.endpoint)?;
		let artifacts = create_artifacts(&options.artifacts)?;
		let session = match Session::start(connection, &capabilities, limits) {
			Ok(session) => session,
			Err(error) => {
				// There is no session to capture, but the report still records
				// why startup failed alongside any driver logs the runner
				// keeps.
				let message = error.to_string();
				return combine(Err(error), write_report(&artifacts, &[], Some(&message))).map_err(
					|source| {
						Error::Evidence {
							directory: artifacts,
							source: Box::new(source),
						}
					},
				);
			}
		};
		Ok(Self {
			session,
			options,
			artifacts,
			captures: Vec::new(),
		})
	}

	/// Runs a scenario and always records failures and closes the session.
	pub fn run<T>(
		options: Options,
		scenario: impl FnOnce(&mut Self) -> Result<T, Error>,
	) -> Result<T, Error> {
		let mut tester = Self::connect(options)?;
		// Assertion macros panic. Retain the original panic after collecting
		// evidence and closing the driver, so the Rust test still fails.
		match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scenario(&mut tester))) {
			Ok(result) => tester.finish(result),
			Err(payload) => {
				let message = payload
					.downcast_ref::<String>()
					.map(String::as_str)
					.or_else(|| payload.downcast_ref::<&str>().copied())
					.unwrap_or("non-string panic");
				if let Err(error) = tester.finish::<()>(Err(Error::Assertion(format!(
					"scenario panicked: {message}"
				)))) {
					eprintln!("{error}");
				}
				std::panic::resume_unwind(payload)
			}
		}
	}

	/// Directory containing this session's evidence and `index.html`.
	pub fn artifacts(&self) -> &Path {
		&self.artifacts
	}

	/// Waits for exactly one visible native control, then performs one OS
	/// click.
	pub fn tap(&self, locator: &Locator) -> Result<(), Error> {
		let id = self.session.wait_for(
			&locator.wire(self.options.platform),
			"to be visible and enabled",
			|session, id, timeout| session.element_bool(id, "enabled", timeout),
		)?;
		self.session
			.command(Method::POST, &["element", &id, "click"], Some(&json!({})))?;
		Ok(())
	}

	/// Presses and releases one key, delivered to whatever the compositor has
	/// focused. GTK's AT-SPI bridge reports every component at the origin and
	/// the KDE driver never performs GTK's own `click` action, so focused
	/// GTK controls are driven with keys instead of clicks.
	pub fn input_key(&self, key: char) -> Result<(), Error> {
		self.input_type(&key.to_string())
	}

	/// Types one string of keys, delivered to whatever the compositor has
	/// focused, as a single driver action.
	pub fn input_type(&self, text: &str) -> Result<(), Error> {
		self.session.type_text(text)
	}

	/// Replaces the text of one visible, enabled native input.
	pub fn enter_text(&self, locator: &Locator, text: &str) -> Result<(), Error> {
		let id = self.session.wait_for(
			&locator.wire(self.options.platform),
			"to be visible and enabled",
			|session, id, timeout| session.element_bool(id, "enabled", timeout),
		)?;
		self.session
			.command(Method::POST, &["element", &id, "clear"], Some(&json!({})))?;
		self.session.command(
			Method::POST,
			&["element", &id, "value"],
			Some(
				&json!({"text": text, "value": text.chars().map(|c| c.to_string()).collect::<Vec<_>>()}),
			),
		)?;
		Ok(())
	}

	/// Waits for one visible control, rejecting ambiguous matches.
	pub fn expect_visible(&self, locator: &Locator) -> Result<(), Error> {
		self.session.wait_for(
			&locator.wire(self.options.platform),
			"to be visible",
			|_, _, _| Ok(true),
		)?;
		Ok(())
	}

	/// Waits until no visible control matches.
	pub fn expect_absent(&self, locator: &Locator) -> Result<(), Error> {
		let selector = locator.wire(self.options.platform);
		self.session
			.wait(&format!("{locator:?} to disappear"), |remaining| {
				Ok(self
					.session
					.visible_elements(&selector, remaining)?
					.is_empty()
					.then_some(()))
			})
	}

	/// Waits until the native control's visible text matches exactly.
	pub fn expect_text(&self, locator: &Locator, expected: &str) -> Result<(), Error> {
		let purpose = format!("to show text {expected:?}");
		self.session.wait_for(
			&locator.wire(self.options.platform),
			&purpose,
			|session, id, remaining| {
				let value =
					session.request(Method::GET, &["element", id, "text"], None, remaining)?;
				Ok(value
					.as_str()
					.ok_or_else(|| Error::Response("element text is not a string".to_owned()))?
					== expected)
			},
		)?;
		Ok(())
	}

	/// Waits for the driver's native alert. Permission sheets can also be
	/// addressed through their visible labels with [`Self::tap`].
	pub fn alert_text(&self) -> Result<String, Error> {
		self.session.wait("native alert", |remaining| {
			match self
				.session
				.request(Method::GET, &["alert", "text"], None, remaining)
			{
				Ok(value) => {
					value
						.as_str()
						.map(|text| Some(text.to_owned()))
						.ok_or_else(|| Error::Response("alert text is not a string".to_owned()))
				}
				Err(Error::Driver { code, .. }) if code == "no such alert" => Ok(None),
				Err(error) => Err(error),
			}
		})
	}

	/// Accepts the currently visible native alert, once.
	pub fn accept_alert(&self) -> Result<(), Error> {
		self.alert_text()?;
		self.session
			.command(Method::POST, &["alert", "accept"], Some(&json!({})))?;
		Ok(())
	}

	/// Dismisses the currently visible native alert, once.
	pub fn dismiss_alert(&self) -> Result<(), Error> {
		self.alert_text()?;
		self.session
			.command(Method::POST, &["alert", "dismiss"], Some(&json!({})))?;
		Ok(())
	}

	/// Executes a documented driver extension, such as `mobile: pressButton`
	/// or `mobile: openNotifications`. Unsupported operations remain errors.
	pub fn native_command(&self, script: &str, arguments: &Value) -> Result<Value, Error> {
		self.session.command(
			Method::POST,
			&["execute", "sync"],
			Some(&json!({"script": script, "args": [arguments]})),
		)
	}

	/// Performs W3C native pointer/key actions. Coordinates use the driver's
	/// logical viewport units, not screenshot physical pixels.
	/// Include matching key-up/pointer-up actions to release held input.
	pub fn perform_actions(&self, sources: &Value) -> Result<(), Error> {
		self.session.command(
			Method::POST,
			&["actions"],
			Some(&json!({"actions": sources})),
		)?;
		Ok(())
	}

	/// Captures actual driver pixels, including visible OS UI. Never
	/// synthesizes widget rectangles or substitutes an earlier image after
	/// a driver failure.
	pub fn screenshot(&self) -> Result<Screenshot, Error> {
		Ok(Screenshot::from_png(self.session.screenshot_png()?)?)
	}

	/// Saves a real screenshot and accessibility tree and updates the visual
	/// HTML report. Labels are escaped and never used as filesystem paths.
	pub fn checkpoint(&mut self, label: &str) -> Result<(), Error> {
		let stem = format!("capture-{:03}", self.captures.len() + 1);
		self.screenshot()?
			.save(self.artifacts.join(format!("{stem}.png")))?;
		let source = self.source()?;
		std::fs::write(self.artifacts.join(format!("{stem}.xml")), source)?;
		// Record the capture only once both files exist, so the report never
		// links to evidence that was not written.
		self.captures.push((label.to_owned(), stem));
		self.write_report(None)
	}

	/// Reads the driver's current accessibility tree as XML text.
	pub fn source(&self) -> Result<String, Error> {
		let tree = self.session.command(Method::GET, &["source"], None)?;
		tree.as_str()
			.map(str::to_owned)
			.ok_or_else(|| Error::Response("source is not XML text".to_owned()))
	}

	/// Preserves failure evidence, writes the report, and closes the native
	/// session. Both test and cleanup errors are retained if both fail.
	pub fn finish<T>(mut self, mut result: Result<T, Error>) -> Result<T, Error> {
		if result.is_err() {
			match self.checkpoint("Failure: actual native screen") {
				// Appium ends a session itself after a driver command times
				// out. Nothing is left to capture or close, so report only the
				// failure that ended it rather than a chain of follow-on
				// errors.
				Err(Error::Driver { code, .. }) if code == "invalid session id" => {
					self.session.abandon();
				}
				capture => result = combine(result, capture),
			}
		}
		let message = result.as_ref().err().map(ToString::to_string);
		result = combine(result, self.session.close());
		let message = result.as_ref().err().map(ToString::to_string).or(message);
		result = combine(
			result,
			write_report(&self.artifacts, &self.captures, message.as_deref()),
		);
		result.map_err(|source| {
			Error::Evidence {
				directory: self.artifacts.clone(),
				source: Box::new(source),
			}
		})
	}

	fn write_report(&self, failure: Option<&str>) -> Result<(), Error> {
		write_report(&self.artifacts, &self.captures, failure)
	}
}

impl Drop for NativeTester {
	fn drop(&mut self) {
		if let Err(error) = self.session.close() {
			eprintln!("native session cleanup failed: {error}");
		}
	}
}

fn create_artifacts(parent: &Path) -> Result<PathBuf, Error> {
	std::fs::create_dir_all(parent)?;
	let timestamp = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_err(|error| Error::Configuration(error.to_string()))?
		.as_nanos();
	let number = SESSION_NUMBER.fetch_add(1, Ordering::Relaxed);
	let path = parent.join(format!("{timestamp}-{}-{number}", std::process::id()));
	std::fs::create_dir(&path)?;
	Ok(path)
}

fn write_report(
	artifacts: &Path,
	captures: &[(String, String)],
	failure: Option<&str>,
) -> Result<(), Error> {
	let mut html = String::from(
		"<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" \
		 content=\"width=device-width\"><title>Leptoskit native test</title><style>body{font:16px \
		 system-ui;max-width:1100px;margin:40px auto;padding:0 \
		 24px;background:#f5f5f7;color:#202027}img{max-width:100%;max-height:850px;object-fit:\
		 contain;background:white;border:1px solid \
		 #ccc}pre{white-space:pre-wrap}figure{margin:24px 0}</style><h1>Leptoskit native \
		 test</h1><p>Actual operating-system screenshots. Images retain their original pixels.</p>",
	);
	if let Some(failure) = failure {
		write!(html, "<h2>Failed</h2><pre>{}</pre>", escape(failure))
			.expect("writing to a String cannot fail");
	}
	for (label, stem) in captures {
		write!(
			html,
			"<figure><figcaption>{}</figcaption><a href=\"{stem}.png\"><img src=\"{stem}.png\" \
			 alt=\"{}\"></a><p><a href=\"{stem}.xml\">Accessibility tree</a></p></figure>",
			escape(label),
			escape(label)
		)
		.expect("writing to a String cannot fail");
	}
	html.push_str("</html>");
	std::fs::write(artifacts.join("index.html"), html)?;
	Ok(())
}

fn combine<T>(primary: Result<T, Error>, secondary: Result<(), Error>) -> Result<T, Error> {
	match (primary, secondary) {
		(Ok(value), Ok(())) => Ok(value),
		(Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
		(Err(primary), Err(secondary)) => {
			Err(Error::Combined {
				primary: Box::new(primary),
				secondary: Box::new(secondary),
			})
		}
	}
}

fn escape(value: &str) -> String {
	value
		.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
		.replace('\'', "&#39;")
}
