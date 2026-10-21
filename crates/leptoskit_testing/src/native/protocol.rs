use std::io::Read;
use std::time::Duration;
use std::time::Instant;

use reqwest::Method;
use reqwest::Url;
use serde_json::Value;

use super::Error;

const MAX_RESPONSE: u64 = 64 * 1024 * 1024;
pub(super) const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

pub(super) struct Connection {
	client: reqwest::blocking::Client,
	endpoint: Url,
}

impl Connection {
	pub(super) fn wait_ready(&self, timeout: Duration) -> Result<(), Error> {
		let deadline = Instant::now() + timeout;
		loop {
			let remaining = deadline
				.checked_duration_since(Instant::now())
				.filter(|duration| !duration.is_zero())
				.ok_or_else(|| Error::Timeout("native driver readiness".to_owned()))?;
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

	pub(super) fn new(endpoint: &str) -> Result<Self, Error> {
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

	pub(super) fn request(
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
