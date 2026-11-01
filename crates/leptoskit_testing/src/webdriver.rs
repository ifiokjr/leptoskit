//! The W3C `WebDriver` wire client shared by native drivers (Appium) and
//! browser drivers (chromedriver and friends).
//!
//! [`Connection`] speaks HTTP to one driver endpoint; [`Session`] owns one
//! session on it, with the bounded-wait discipline the testing and control
//! layers rely on: observations inside a wait are retried until the wait's
//! own deadline, while actions are sent exactly once.

use std::io::Read;
use std::time::Duration;
use std::time::Instant;

use reqwest::Method;
use reqwest::Url;
use serde::Serialize;
use serde_json::Value;

const MAX_RESPONSE: u64 = 64 * 1024 * 1024;
pub(crate) const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";
/// Time beyond a driver's own command bound for it to report a timeout.
pub(crate) const DRIVER_REPORT_MARGIN_SECONDS: u64 = 10;

/// An automation failure. Driver errors retain their W3C error code.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// A failed run with its retained evidence location and original error.
	#[error("{source}; evidence: {directory}")]
	Evidence {
		directory: std::path::PathBuf,
		#[source]
		source: Box<Self>,
	},
	/// Invalid session options or unsupported target selection.
	#[error("automation configuration: {0}")]
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
	/// The driver explicitly rejected the operation.
	#[error("driver {code}: {message}")]
	Driver { code: String, message: String },
	/// Driver returned a structurally invalid response.
	#[error("invalid driver response: {0}")]
	Response(String),
	/// A visible state failed to appear within its deadline.
	#[error("UI condition timed out: {0}")]
	Timeout(String),
	/// More than one control matches; no action was taken.
	#[error("selector {selector} matched {count} visible controls")]
	Ambiguous { selector: String, count: usize },
	/// An observable assertion failed.
	#[error("UI assertion: {0}")]
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

/// One resolved locator: a W3C locator strategy and its selector string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Selector {
	/// The strategy name, like `xpath` or `css selector`.
	pub using: String,
	/// The selector expression in that strategy's syntax.
	pub value: String,
}

impl Selector {
	pub(crate) fn new(using: &str, value: impl Into<String>) -> Self {
		Self {
			using: using.to_owned(),
			value: value.into(),
		}
	}
}

/// Quotes an `XPath` string literal, preferring the readable single-quote form.
pub(crate) fn xpath_literal(value: &str) -> String {
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

/// Deadlines and platform quirks one session runs with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
	/// Deadline for session creation, including driver boot.
	pub startup: Duration,
	/// Client deadline for one command: the driver's own bound plus a margin
	/// for it to report a timeout.
	pub command: Duration,
	/// Deadline for observable UI conditions, including every read the wait
	/// makes.
	pub wait: Duration,
	/// The implicit wait the driver itself applies. KDE's AT-SPI driver needs
	/// a positive interval to traverse the tree; every other driver gets zero
	/// so the client's outer wait owns the deadline.
	pub implicit_wait_ms: u64,
	/// Whether observations treat the AT-SPI driver's internal `unknown error`
	/// on a destroyed accessible as a vanished element.
	pub atspi_quirks: bool,
}

/// A live W3C session on one driver connection.
pub(crate) struct Session {
	connection: Connection,
	id: Option<String>,
	command: Duration,
	wait: Duration,
	atspi_quirks: bool,
}

impl Session {
	/// Waits for the driver, then creates a session and sets its implicit
	/// wait within one startup deadline.
	pub fn start(
		connection: Connection,
		capabilities: &Value,
		limits: Limits,
	) -> Result<Self, Error> {
		let deadline = Instant::now() + limits.startup;
		let remaining = || {
			deadline
				.checked_duration_since(Instant::now())
				.filter(|duration| !duration.is_zero())
				.ok_or_else(|| Error::Timeout("session startup".to_owned()))
		};
		connection.wait_ready(remaining()?)?;
		let response =
			connection.request(Method::POST, &["session"], Some(capabilities), remaining()?)?;
		let id = response
			.get("sessionId")
			.and_then(Value::as_str)
			.filter(|id| !id.is_empty())
			.ok_or_else(|| Error::Response("missing sessionId".to_owned()))?
			.to_owned();
		let session = Self {
			connection,
			id: Some(id),
			command: limits.command,
			wait: limits.wait,
			atspi_quirks: limits.atspi_quirks,
		};
		// Compatibility names for the same field: current drivers answer
		// `implicit` at `timeouts`, the AT-SPI driver reads `ms` at
		// `timeouts/implicit_wait`.
		let timeouts = if limits.implicit_wait_ms > 0 {
			(
				vec!["timeouts", "implicit_wait"],
				serde_json::json!({"ms": limits.implicit_wait_ms}),
			)
		} else {
			(vec!["timeouts"], serde_json::json!({"implicit": 0}))
		};

		if let Err(error) = session.command(Method::POST, &timeouts.0, Some(&timeouts.1)) {
			let mut closed = session;
			let _ = closed.close();

			return Err(error);
		}

		Ok(session)
	}

	/// Deletes the session, once. Further requests report the session closed.
	pub fn close(&mut self) -> Result<(), Error> {
		if let Some(id) = self.id.take() {
			self.connection
				.request(Method::DELETE, &["session", &id], None, self.command)?;
		}

		Ok(())
	}

	/// Marks the session closed without deleting it, for a driver that already
	/// ended it on its own.
	pub fn abandon(&mut self) {
		self.id = None;
	}

	/// Sends one session request with the per-command deadline.
	pub fn command(
		&self,
		method: Method,
		tail: &[&str],
		body: Option<&Value>,
	) -> Result<Value, Error> {
		self.request(method, tail, body, self.command)
	}

	/// Sends one session request. Observations inside a wait pass the wait's
	/// remaining time, so a slow but answering driver fails only when the
	/// wait itself expires.
	pub fn request(
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
		self.connection.request(method, &path, body, timeout)
	}

	/// Captures the driver's actual pixels as PNG bytes.
	pub fn screenshot_png(&self) -> Result<Vec<u8>, Error> {
		use base64::Engine;
		let value = self.command(Method::GET, &["screenshot"], None)?;
		let encoded = value
			.as_str()
			.ok_or_else(|| Error::Response("screenshot is not base64 text".to_owned()))?;
		base64::engine::general_purpose::STANDARD
			.decode(encoded)
			.map_err(|error| Error::Response(format!("invalid screenshot base64: {error}")))
	}

	/// Types one string of keys to whatever has focus, as a single key
	/// action: drivers spawn an input synthesizer per request, so one action
	/// per string keeps typing cheap.
	pub fn type_text(&self, text: &str) -> Result<(), Error> {
		if text.is_empty() {
			return Err(Error::Configuration("cannot type empty input".to_owned()));
		}

		let keys: Vec<_> = text
			.chars()
			.flat_map(|character| {
				[
					serde_json::json!({"type": "keyDown", "value": character.to_string()}),
					serde_json::json!({"type": "keyUp", "value": character.to_string()}),
				]
			})
			.collect();
		self.command(
			Method::POST,
			&["actions"],
			Some(&serde_json::json!({"actions": [{
				"type": "key",
				"id": "leptoskit-typing",
				"actions": keys
			}]})),
		)?;
		Ok(())
	}

	pub(crate) fn element_bool(
		&self,
		id: &str,
		property: &str,
		timeout: Duration,
	) -> Result<bool, Error> {
		self.request(Method::GET, &["element", id, property], None, timeout)?
			.as_bool()
			.ok_or_else(|| Error::Response(format!("element {property} is not boolean")))
	}

	pub(crate) fn visible_elements(
		&self,
		selector: &Selector,
		timeout: Duration,
	) -> Result<Vec<String>, Error> {
		let start = Instant::now();
		let body = serde_json::to_value(selector)?;
		let value = self.request(Method::POST, &["elements"], Some(&body), timeout)?;
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
				.ok_or_else(|| Error::Timeout(format!("resolving {selector:?}")))?;

			if self.element_bool(id, "displayed", remaining)? {
				visible.push(id.to_owned());
			}
		}

		Ok(visible)
	}

	/// Waits for exactly one visible control matching `selector` that also
	/// satisfies `condition`, returning its element ID.
	pub fn wait_for(
		&self,
		selector: &Selector,
		purpose: &str,
		condition: impl Fn(&Self, &str, Duration) -> Result<bool, Error>,
	) -> Result<String, Error> {
		self.wait(&format!("{selector:?} {purpose}"), |remaining| {
			let start = Instant::now();
			let ids = self.visible_elements(selector, remaining)?;
			if ids.len() > 1 {
				return Err(Error::Ambiguous {
					selector: format!("{selector:?}"),
					count: ids.len(),
				});
			}

			let Some(id) = ids.into_iter().next() else {
				return Ok(None);
			};
			let remaining = remaining
				.checked_sub(start.elapsed())
				.filter(|time| !time.is_zero())
				.ok_or_else(|| Error::Timeout(format!("checking {selector:?}")))?;
			Ok(condition(self, &id, remaining)?.then_some(id))
		})
	}

	/// Whether an observation failed only because an element it was reading
	/// disappeared, which the next poll sees through. Standard drivers report
	/// that as `stale element reference` or `no such element`. The Linux AT-SPI
	/// driver (selenium-webdriver-at-spi) instead raises an internal `unknown
	/// error: 'NoneType' object is not iterable` when an accessible is
	/// destroyed while it walks the tree, as a portal dialog's are while the
	/// dialog closes. Only observations inside a bounded wait treat it so; an
	/// action on a vanished element still fails.
	fn vanished_during_observation(&self, error: &Error) -> bool {
		let Error::Driver { code, message } = error else {
			return false;
		};
		code == "stale element reference"
			|| code == "no such element"
			|| (self.atspi_quirks
				&& code == "unknown error"
				&& message.contains("'NoneType' object is not iterable"))
	}

	/// Polls `observe` until it returns `Some` or the wait deadline passes.
	pub fn wait<T>(
		&self,
		description: &str,
		observe: impl Fn(Duration) -> Result<Option<T>, Error>,
	) -> Result<T, Error> {
		let deadline = Instant::now() + self.wait;

		loop {
			let remaining = deadline
				.checked_duration_since(Instant::now())
				.filter(|time| !time.is_zero())
				.ok_or_else(|| Error::Timeout(description.to_owned()))?;

			match observe(remaining) {
				Ok(Some(value)) => return Ok(value),
				Ok(None) => {}

				Err(error) if self.vanished_during_observation(&error) => {}
				// An observation cut short by this wait's deadline is this
				// wait's timeout, whichever nested check or request noticed it
				// first.
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

/// One driver endpoint's HTTP transport.
pub(crate) struct Connection {
	client: reqwest::blocking::Client,
	endpoint: Url,
}

impl Connection {
	pub fn wait_ready(&self, timeout: Duration) -> Result<(), Error> {
		let deadline = Instant::now() + timeout;

		loop {
			let remaining = deadline
				.checked_duration_since(Instant::now())
				.filter(|duration| !duration.is_zero())
				.ok_or_else(|| Error::Timeout("driver readiness".to_owned()))?;
			match self.request(
				Method::GET,
				&["status"],
				None,
				remaining.min(Duration::from_secs(5)),
			) {
				// KDE AT-SPI currently serializes readiness as a string.
				Ok(value)
					if value.get("ready") == Some(&Value::Bool(true))
						|| value.get("ready") == Some(&Value::String("true".to_owned())) =>
				{
					return Ok(());
				}

				Ok(value)
					if value.get("ready") == Some(&Value::Bool(false))
						|| value.get("ready") == Some(&Value::String("false".to_owned())) => {}
				Ok(_) => {
					return Err(Error::Response(
						"status has no boolean ready field".to_owned(),
					));
				}

				Err(Error::Transport(error)) if error.is_connect() || error.is_timeout() => {}

				Err(error) => return Err(error),
			}
			std::thread::sleep(
				deadline
					.saturating_duration_since(Instant::now())
					.min(Duration::from_millis(100)),
			);
		}
	}

	pub fn new(endpoint: &str) -> Result<Self, Error> {
		let endpoint =
			Url::parse(endpoint).map_err(|error| Error::Configuration(error.to_string()))?;

		if !matches!(endpoint.scheme(), "http" | "https")
			|| endpoint.host_str().is_none()
			|| endpoint.query().is_some()
			|| endpoint.fragment().is_some()
		{
			return Err(Error::Configuration(
				"driver endpoint must be an HTTP(S) URL without query or fragment".to_owned(),
			));
		}
		let client = reqwest::blocking::Client::builder()
			.connect_timeout(Duration::from_secs(10))
			.redirect(reqwest::redirect::Policy::none())
			.build()?;
		Ok(Self { client, endpoint })
	}

	pub fn request(
		&self,
		method: Method,
		segments: &[&str],
		body: Option<&Value>,
		timeout: Duration,
	) -> Result<Value, Error> {
		let mut url = self.endpoint.clone();
		url.path_segments_mut()
			.map_err(|()| Error::Configuration("endpoint cannot contain path segments".to_owned()))?
			.pop_if_empty()
			.extend(segments);
		let mut request = self.client.request(method, url).timeout(timeout);

		if let Some(body) = body {
			request = request.json(body);
		}

		let response = request.send()?;
		let status = response.status();
		let mut bytes = Vec::new();
		response
			.take(MAX_RESPONSE + 1)
			.read_to_end(&mut bytes)
			.map_err(body_error)?;

		if bytes.len() as u64 > MAX_RESPONSE {
			return Err(Error::Response("response exceeded 64 MiB".to_owned()));
		}

		let envelope: Value = match serde_json::from_slice(&bytes) {
			Ok(envelope) => envelope,
			// Proxies and crashed servers answer with HTML or plain text; the
			// status explains the failure better than the parse position.
			Err(_) if !status.is_success() => {
				let body = String::from_utf8_lossy(&bytes);
				let excerpt: String = body.chars().take(512).collect();
				return Err(Error::Response(format!("HTTP {status}: {excerpt}")));
			}
			Err(error) => return Err(error.into()),
		};

		let value = envelope
			.get("value")
			.ok_or_else(|| Error::Response("missing W3C value".to_owned()))?;

		if let Some(code) = value.get("error").and_then(Value::as_str) {
			return Err(Error::Driver {
				code: code.to_owned(),
				message: value
					.get("message")
					.and_then(Value::as_str)
					.unwrap_or("no driver message")
					.to_owned(),
			});
		}

		if !status.is_success() {
			return Err(Error::Response(format!("HTTP {status}: {value}")));
		}

		Ok(value.clone())
	}
}

/// reqwest's blocking body reader reports its own failures, including the
/// request timeout, as `io::Error`. Unwrap them so a wait still recognises a
/// response its deadline cut short as a transport timeout.
fn body_error(error: std::io::Error) -> Error {
	match error.downcast::<reqwest::Error>() {
		Ok(error) => Error::Transport(error),
		Err(error) => Error::Io(error),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn body_errors_reqwest_did_not_raise_stay_io_errors() {
		let error = body_error(std::io::Error::new(
			std::io::ErrorKind::InvalidData,
			"truncated chunk",
		));
		assert!(
			matches!(&error, Error::Io(source) if source.kind() == std::io::ErrorKind::InvalidData),
			"{error}"
		);
	}

	#[test]
	fn xpath_literals_stay_readable_until_quotes_force_concat() {
		assert_eq!(xpath_literal("Plain"), "'Plain'");
		assert_eq!(xpath_literal("Don't"), "\"Don't\"");
		assert_eq!(
			xpath_literal("It's a \"label\""),
			"concat('It',\"'\",'s a \"label\"')"
		);
	}
}
