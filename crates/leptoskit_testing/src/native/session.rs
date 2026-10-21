use std::fmt::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use base64::Engine;
use reqwest::Method;
use serde_json::Value;
use serde_json::json;

use super::Error;
use super::Locator;
use super::Options;
use super::Platform;
use super::protocol::Connection;
use super::protocol::ELEMENT_KEY;
use crate::capture::Screenshot;

static SESSION_NUMBER: AtomicU64 = AtomicU64::new(0);

/// A real OS automation session. Call [`Self::run`] to retain failure evidence
/// and propagate cleanup failures, or [`Self::finish`] when managing it
/// manually.
///
/// This synchronous API must run outside an async runtime (or in its blocking
/// pool). Only reads are retried while waiting; actions are sent exactly once.
pub struct NativeTester {
	connection: Connection,
	id: Option<String>,
	options: Options,
	artifacts: PathBuf,
	captures: Vec<(String, String)>,
}

impl NativeTester {
	/// Connects to a provisioned platform driver and launches/attaches the app.
	pub fn connect(options: Options) -> Result<Self, Error> {
		let capabilities = options.driver_capabilities()?;
		let connection = Connection::new(&options.endpoint)?;
		let artifacts = create_artifacts(&options.artifacts)?;
		let deadline = Instant::now() + Duration::from_secs(options.startup_seconds);
		let id = match start_session(&connection, &capabilities, deadline) {
			Ok(id) => id,
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
		let tester = Self {
			connection,
			id: Some(id),
			options,
			artifacts,
			captures: Vec::new(),
		};
		// KDE's AT-SPI driver needs a positive implicit interval to traverse
		// the tree. Our outer wait owns the deadline on every platform.
		let (path, timeouts) = if tester.options.platform == Platform::Linux {
			(vec!["timeouts", "implicit_wait"], json!({"ms": 100}))
		} else {
			(vec!["timeouts"], json!({"implicit": 0}))
		};
		if let Err(error) = tester.command(Method::POST, &path, Some(&timeouts)) {
			return tester.finish(Err(error));
		}
		Ok(tester)
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
		let id = self.wait_for(
			locator,
			"to be visible and enabled",
			|tester, id, timeout| tester.element_bool(id, "enabled", timeout),
		)?;
		self.command(Method::POST, &["element", &id, "click"], Some(&json!({})))?;
		Ok(())
	}

	/// Replaces the text of one visible, enabled native input.
	pub fn enter_text(&self, locator: &Locator, text: &str) -> Result<(), Error> {
		let id = self.wait_for(
			locator,
			"to be visible and enabled",
			|tester, id, timeout| tester.element_bool(id, "enabled", timeout),
		)?;
		self.command(Method::POST, &["element", &id, "clear"], Some(&json!({})))?;
		self.command(
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
		self.wait_for(locator, "to be visible", |_, _, _| Ok(true))?;
		Ok(())
	}

	/// Waits until no visible control matches.
	pub fn expect_absent(&self, locator: &Locator) -> Result<(), Error> {
		self.wait(&format!("{locator:?} to disappear"), |remaining| {
			Ok(self
				.visible_elements(locator, remaining)?
				.is_empty()
				.then_some(()))
		})
	}

	/// Waits until the native control's visible text matches exactly.
	pub fn expect_text(&self, locator: &Locator, expected: &str) -> Result<(), Error> {
		let purpose = format!("to show text {expected:?}");
		self.wait_for(locator, &purpose, |tester, id, remaining| {
			let value = tester.request(Method::GET, &["element", id, "text"], None, remaining)?;
			Ok(value
				.as_str()
				.ok_or_else(|| Error::Response("element text is not a string".to_owned()))?
				== expected)
		})?;
		Ok(())
	}

	/// Waits for the driver's native alert. Permission sheets can also be
	/// addressed through their visible labels with [`Self::tap`].
	pub fn alert_text(&self) -> Result<String, Error> {
		self.wait("native alert", |remaining| {
			match self.request(Method::GET, &["alert", "text"], None, remaining) {
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
		self.command(Method::POST, &["alert", "accept"], Some(&json!({})))?;
		Ok(())
	}

	/// Dismisses the currently visible native alert, once.
	pub fn dismiss_alert(&self) -> Result<(), Error> {
		self.alert_text()?;
		self.command(Method::POST, &["alert", "dismiss"], Some(&json!({})))?;
		Ok(())
	}

	/// Executes a documented driver extension, such as `mobile: pressButton`
	/// or `mobile: openNotifications`. Unsupported operations remain errors.
	pub fn native_command(&self, script: &str, arguments: &Value) -> Result<Value, Error> {
		self.command(
			Method::POST,
			&["execute", "sync"],
			Some(&json!({"script": script, "args": [arguments]})),
		)
	}

	/// Performs W3C native pointer/key actions. Coordinates use the driver's
	/// logical viewport units, not screenshot physical pixels.
	/// Include matching key-up/pointer-up actions to release held input.
	pub fn perform_actions(&self, sources: &Value) -> Result<(), Error> {
		self.command(
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
		let value = self.command(Method::GET, &["screenshot"], None)?;
		let encoded = value
			.as_str()
			.ok_or_else(|| Error::Response("screenshot is not base64 text".to_owned()))?;
		let bytes = base64::engine::general_purpose::STANDARD
			.decode(encoded)
			.map_err(|error| Error::Response(format!("invalid screenshot base64: {error}")))?;
		Ok(Screenshot::from_png(bytes)?)
	}

	/// Saves a real screenshot and accessibility tree and updates the visual
	/// HTML report. Labels are escaped and never used as filesystem paths.
	pub fn checkpoint(&mut self, label: &str) -> Result<(), Error> {
		let stem = format!("capture-{:03}", self.captures.len() + 1);
		self.screenshot()?
			.save(self.artifacts.join(format!("{stem}.png")))?;
		let tree = self.command(Method::GET, &["source"], None)?;
		let source = tree
			.as_str()
			.ok_or_else(|| Error::Response("source is not XML text".to_owned()))?;
		std::fs::write(self.artifacts.join(format!("{stem}.xml")), source)?;
		// Record the capture only once both files exist, so the report never
		// links to evidence that was not written.
		self.captures.push((label.to_owned(), stem));
		self.write_report(None)
	}

	/// Preserves failure evidence, writes the report, and closes the native
	/// session. Both test and cleanup errors are retained if both fail.
	pub fn finish<T>(mut self, mut result: Result<T, Error>) -> Result<T, Error> {
		if result.is_err() {
			result = combine(result, self.checkpoint("Failure: actual native screen"));
		}
		let message = result.as_ref().err().map(ToString::to_string);
		result = combine(result, self.close());
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

	fn close(&mut self) -> Result<(), Error> {
		if let Some(id) = self.id.take() {
			self.connection.request(
				Method::DELETE,
				&["session", &id],
				None,
				Duration::from_secs(self.options.command_seconds),
			)?;
		}
		Ok(())
	}

	fn write_report(&self, failure: Option<&str>) -> Result<(), Error> {
		write_report(&self.artifacts, &self.captures, failure)
	}

	fn command(&self, method: Method, tail: &[&str], body: Option<&Value>) -> Result<Value, Error> {
		self.request(
			method,
			tail,
			body,
			Duration::from_secs(self.options.command_seconds),
		)
	}

	fn request(
		&self,
		method: Method,
		tail: &[&str],
		body: Option<&Value>,
		timeout: Duration,
	) -> Result<Value, Error> {
		let id = self
			.id
			.as_deref()
			.ok_or_else(|| Error::Configuration("session is closed".to_owned()))?;
		let mut path = vec!["session", id];
		path.extend_from_slice(tail);
		self.connection.request(
			method,
			&path,
			body,
			timeout.min(Duration::from_secs(self.options.command_seconds)),
		)
	}

	fn element_bool(&self, id: &str, property: &str, timeout: Duration) -> Result<bool, Error> {
		self.request(Method::GET, &["element", id, property], None, timeout)?
			.as_bool()
			.ok_or_else(|| Error::Response(format!("element {property} is not boolean")))
	}

	fn visible_elements(&self, locator: &Locator, timeout: Duration) -> Result<Vec<String>, Error> {
		let start = Instant::now();
		let value = self.request(
			Method::POST,
			&["elements"],
			Some(&locator.wire(self.options.platform)),
			timeout,
		)?;
		let elements = value
			.as_array()
			.ok_or_else(|| Error::Response("elements is not an array".to_owned()))?;
		let mut visible = Vec::new();
		for element in elements {
			let id = element
				.get(ELEMENT_KEY)
				.and_then(Value::as_str)
				.ok_or_else(|| Error::Response("missing W3C element ID".to_owned()))?;
			let remaining = timeout
				.checked_sub(start.elapsed())
				.filter(|time| !time.is_zero())
				.ok_or_else(|| Error::Timeout(format!("resolving {locator:?}")))?;
			if self.element_bool(id, "displayed", remaining)? {
				visible.push(id.to_owned());
			}
		}
		Ok(visible)
	}

	fn wait_for(
		&self,
		locator: &Locator,
		purpose: &str,
		condition: impl Fn(&Self, &str, Duration) -> Result<bool, Error>,
	) -> Result<String, Error> {
		self.wait(&format!("{locator:?} {purpose}"), |remaining| {
			let start = Instant::now();
			let ids = self.visible_elements(locator, remaining)?;
			if ids.len() > 1 {
				return Err(Error::Ambiguous {
					locator: locator.clone(),
					count: ids.len(),
				});
			}
			let Some(id) = ids.into_iter().next() else {
				return Ok(None);
			};
			let remaining = remaining
				.checked_sub(start.elapsed())
				.filter(|time| !time.is_zero())
				.ok_or_else(|| Error::Timeout(format!("checking {locator:?}")))?;
			Ok(condition(self, &id, remaining)?.then_some(id))
		})
	}

	fn wait<T>(
		&self,
		description: &str,
		observe: impl Fn(Duration) -> Result<Option<T>, Error>,
	) -> Result<T, Error> {
		let deadline = Instant::now() + Duration::from_secs(self.options.wait_seconds);
		loop {
			let remaining = deadline
				.checked_duration_since(Instant::now())
				.filter(|time| !time.is_zero())
				.ok_or_else(|| Error::Timeout(description.to_owned()))?;
			match observe(remaining) {
				Ok(Some(value)) => return Ok(value),
				Ok(None) => {}
				Err(Error::Driver { code, .. })
					if code == "stale element reference" || code == "no such element" => {}
				// An observation cut short by this wait's deadline is this wait's
				// timeout, whichever nested check or request noticed it first.
				Err(Error::Timeout(_)) => return Err(Error::Timeout(description.to_owned())),
				Err(Error::Transport(error))
					if error.is_timeout() && Instant::now() >= deadline =>
				{
					return Err(Error::Timeout(description.to_owned()));
				}
				Err(error) => return Err(error),
			}
			// Poll a condition; this is not a fixed delay between UI actions.
			std::thread::sleep(
				deadline
					.saturating_duration_since(Instant::now())
					.min(Duration::from_millis(100)),
			);
		}
	}
}

impl Drop for NativeTester {
	fn drop(&mut self) {
		if let Err(error) = self.close() {
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

/// Waits for the driver, then creates a session within one startup deadline
/// that covers driver boot, app installation, and launch.
fn start_session(
	connection: &Connection,
	capabilities: &Value,
	deadline: Instant,
) -> Result<String, Error> {
	let remaining = || {
		deadline
			.checked_duration_since(Instant::now())
			.filter(|time| !time.is_zero())
			.ok_or_else(|| Error::Timeout("native session startup".to_owned()))
	};
	connection.wait_ready(remaining()?)?;
	let response =
		connection.request(Method::POST, &["session"], Some(capabilities), remaining()?)?;
	Ok(response
		.get("sessionId")
		.and_then(Value::as_str)
		.filter(|id| !id.is_empty())
		.ok_or_else(|| Error::Response("missing sessionId".to_owned()))?
		.to_owned())
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
