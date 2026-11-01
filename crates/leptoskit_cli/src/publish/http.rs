//! The requests the store clients make, and their answers.

use std::time::Duration;

use anyhow::Context;
use anyhow::Result;

/// A store's answer.
#[derive(Debug)]
pub struct Reply {
	pub status: u16,
	pub headers: Vec<(String, String)>,
	pub body: Vec<u8>,
}

impl Reply {
	/// The body as JSON, or `null` when empty.
	///
	/// # Errors
	///
	/// When the body isn't JSON.
	pub fn json(&self) -> Result<serde_json::Value> {
		if self.body.is_empty() {
			return Ok(serde_json::Value::Null);
		}

		serde_json::from_slice(&self.body).with_context(|| {
			format!(
				"the store answered {} with something that isn't JSON: {}",
				self.status,
				String::from_utf8_lossy(&self.body[..self.body.len().min(200)])
			)
		})
	}

	/// The header `name`, case-insensitively.
	#[must_use]
	pub fn header(&self, name: &str) -> Option<&str> {
		self.headers
			.iter()
			.find(|(key, _)| key.eq_ignore_ascii_case(name))
			.map(|(_, value)| value.as_str())
	}

	/// The reply, or an error naming the status and the store's message.
	///
	/// # Errors
	///
	/// When the status isn't a success.
	pub fn ok(self, action: &str) -> Result<Self> {
		if (200..300).contains(&self.status) {
			return Ok(self);
		}

		let body = String::from_utf8_lossy(&self.body);
		anyhow::bail!(
			"{action} failed with {}: {}",
			self.status,
			body.chars().take(500).collect::<String>()
		)
	}
}

/// A request body and its content type.
pub struct Body {
	pub content_type: String,
	pub bytes: Vec<u8>,
}

impl Body {
	/// A JSON body.
	#[must_use]
	pub fn json(value: &serde_json::Value) -> Self {
		Self {
			content_type: "application/json".to_owned(),
			bytes: value.to_string().into_bytes(),
		}
	}

	/// Any file, such as an Android App Bundle.
	#[must_use]
	pub fn bytes(bytes: Vec<u8>) -> Self {
		Self {
			content_type: "application/octet-stream".to_owned(),
			bytes,
		}
	}

	/// A file of `content_type`, such as `image/png`.
	#[must_use]
	pub fn typed(content_type: &str, bytes: Vec<u8>) -> Self {
		Self {
			content_type: content_type.to_owned(),
			bytes,
		}
	}

	/// A zip file.
	#[must_use]
	pub fn zip(bytes: Vec<u8>) -> Self {
		Self {
			content_type: "application/zip".to_owned(),
			bytes,
		}
	}

	/// A form, `application/x-www-form-urlencoded`.
	#[must_use]
	pub fn form(fields: &[(&str, &str)]) -> Self {
		let encode = |text: &str| {
			text.bytes()
				.map(|byte| {
					match byte {
						b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
							char::from(byte).to_string()
						}
						_ => format!("%{byte:02X}"),
					}
				})
				.collect::<String>()
		};
		let bytes = fields
			.iter()
			.map(|(key, value)| format!("{}={}", encode(key), encode(value)))
			.collect::<Vec<_>>()
			.join("&")
			.into_bytes();
		Self {
			content_type: "application/x-www-form-urlencoded".to_owned(),
			bytes,
		}
	}

	/// A `multipart/form-data` body of text fields and files.
	#[must_use]
	pub fn multipart(fields: &[(&str, Part<'_>)]) -> Self {
		let boundary = format!("leptoskit-{:016x}", fastrand_seed());
		let mut bytes = Vec::new();

		for (name, part) in fields {
			bytes.extend_from_slice(format!("--{boundary}\r\n").as_bytes());

			match part {
				Part::Text(value) => {
					bytes.extend_from_slice(
						format!(
							"Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
						)
						.as_bytes(),
					);
				}
				Part::File {
					filename,
					content_type,
					bytes: content,
				} => {
					bytes.extend_from_slice(
						format!(
							"Content-Disposition: form-data; name=\"{name}\"; \
							 filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
						)
						.as_bytes(),
					);
					bytes.extend_from_slice(content);
					bytes.extend_from_slice(b"\r\n");
				}
			}
		}

		bytes.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
		Self {
			content_type: format!("multipart/form-data; boundary={boundary}"),
			bytes,
		}
	}
}

/// One part of a multipart form.
pub enum Part<'a> {
	Text(&'a str),
	File {
		filename: &'a str,
		content_type: &'a str,
		bytes: &'a [u8],
	},
}

fn fastrand_seed() -> u64 {
	use ring::rand::SecureRandom;

	let mut bytes = [0_u8; 8];
	let _ = ring::rand::SystemRandom::new().fill(&mut bytes);
	u64::from_le_bytes(bytes)
}

/// Sends requests to the stores.
pub struct Http {
	agent: ureq::Agent,
}

impl Default for Http {
	fn default() -> Self {
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_global(Some(Duration::from_mins(5)))
			.http_status_as_error(false)
			.build()
			.into();
		Self { agent }
	}
}

impl Http {
	/// Sends one request.
	///
	/// # Errors
	///
	/// When it never got an answer.
	pub fn send(
		&self,
		method: &str,
		url: &str,
		headers: &[(&str, &str)],
		body: Option<Body>,
	) -> Result<Reply> {
		let response = match (method, body) {
			(method @ ("GET" | "DELETE"), _) => {
				let mut request = if method == "GET" {
					self.agent.get(url)
				} else {
					self.agent.delete(url)
				};
				for (name, value) in headers {
					request = request.header(*name, *value);
				}

				request.call()
			}
			(method, body) => {
				let mut request = match method {
					"PATCH" => self.agent.patch(url),
					"PUT" => self.agent.put(url),
					_ => self.agent.post(url),
				};
				for (name, value) in headers {
					request = request.header(*name, *value);
				}
				match body {
					Some(body) => {
						request
							.header("content-type", body.content_type.as_str())
							.send(body.bytes.as_slice())
					}
					None => request.send_empty(),
				}
			}
		}
		.with_context(|| format!("could not reach {}", url.split('?').next().unwrap_or(url)))?;
		let status = response.status().as_u16();
		let headers = response
			.headers()
			.iter()
			.filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
			.collect();
		let body = response
			.into_body()
			.with_config()
			.limit(64 * 1024 * 1024)
			.read_to_vec()
			.context("could not read the store's answer")?;
		Ok(Reply {
			status,
			headers,
			body,
		})
	}
}

/// Waits `interval` between polls, at most `attempts` times, until `poll`
/// returns `Some`.
///
/// # Errors
///
/// When `poll` fails, or never finishes.
pub fn poll<T>(
	what: &str,
	attempts: u32,
	interval: Duration,
	mut poll: impl FnMut() -> Result<Option<T>>,
) -> Result<T> {
	// Tests talk to a local server, which answers at once.
	let interval = if cfg!(test) {
		Duration::from_millis(5)
	} else {
		interval
	};

	for _ in 0..attempts {
		if let Some(done) = poll()? {
			return Ok(done);
		}

		std::thread::sleep(interval);
	}

	anyhow::bail!("{what} didn't finish; check the store's dashboard")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn forms_and_multipart_bodies() {
		let form = Body::form(&[("grant_type", "urn:x"), ("assertion", "a.b+c")]);
		assert_eq!(
			String::from_utf8(form.bytes).unwrap(),
			"grant_type=urn%3Ax&assertion=a.b%2Bc"
		);
		let multipart = Body::multipart(&[
			("channel", Part::Text("listed")),
			(
				"upload",
				Part::File {
					filename: "firefox.zip",
					content_type: "application/zip",
					bytes: b"PK",
				},
			),
		]);
		let boundary = multipart
			.content_type
			.rsplit('=')
			.next()
			.unwrap()
			.to_owned();
		let text = String::from_utf8(multipart.bytes).unwrap();
		assert!(text.starts_with(&format!("--{boundary}\r\n")));
		assert!(text.contains("name=\"channel\"\r\n\r\nlisted\r\n"));
		assert!(text.contains("filename=\"firefox.zip\"\r\nContent-Type: application/zip\r\n"));

		assert!(text.ends_with(&format!("--{boundary}--\r\n")));
	}
}
