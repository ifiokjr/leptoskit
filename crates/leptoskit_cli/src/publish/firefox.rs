//! addons.mozilla.org, through its v5 API with a JWT per request.

use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use serde_json::Value;

use super::http::Body;
use super::http::Http;
use super::http::Part;
use super::http::Reply;
use super::http::poll;
use super::secrets::Secret;
use super::secrets::hs256;

/// The API's address.
pub const API: &str = "https://addons.mozilla.org/api/v5";
/// The credentials, from addons.mozilla.org's API key page.
pub const ISSUER: &str = "LEPTOSKIT_PUBLISH_FIREFOX_JWT_ISSUER";
pub const SECRET: &str = "LEPTOSKIT_PUBLISH_FIREFOX_JWT_SECRET";
/// Where to make them.
pub const HOW: &str =
	"generate credentials at https://addons.mozilla.org/developers/addon/api/key/";

/// A client for one add-on.
pub struct Firefox<'a> {
	http: &'a Http,
	api: String,
	guid: String,
	issuer: Secret,
	secret: Secret,
}

/// A path segment, percent-encoded except for what add-on IDs use.
fn segment(text: &str) -> String {
	text.bytes()
		.map(|byte| {
			match byte {
				b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'@' => {
					char::from(byte).to_string()
				}
				_ => format!("%{byte:02X}"),
			}
		})
		.collect()
}

impl<'a> Firefox<'a> {
	#[must_use]
	pub fn new(http: &'a Http, api: &str, guid: &str, issuer: Secret, secret: Secret) -> Self {
		Self {
			http,
			api: api.trim_end_matches('/').to_owned(),
			guid: guid.to_owned(),
			issuer,
			secret,
		}
	}

	fn send(&self, method: &str, path: &str, body: Option<Body>) -> Result<Reply> {
		let authorization = format!("JWT {}", hs256(self.issuer.expose(), &self.secret)?);
		self.http.send(
			method,
			&format!("{}{path}", self.api),
			&[("authorization", &authorization)],
			body,
		)
	}

	/// Uploads the package for validation, and waits for it. Returns the
	/// upload's ID, which a version is created from.
	///
	/// # Errors
	///
	/// When validation fails, with addons.mozilla.org's messages.
	pub fn upload(&self, zip: &[u8], channel: &str) -> Result<String> {
		let reply = self
			.send(
				"POST",
				"/addons/upload/",
				Some(Body::multipart(&[
					(
						"upload",
						Part::File {
							filename: "firefox.zip",
							bytes: zip,
						},
					),
					("channel", Part::Text(channel)),
				])),
			)?
			.ok("uploading to addons.mozilla.org")?
			.json()?;
		let uuid = reply["uuid"]
			.as_str()
			.context("addons.mozilla.org answered without an upload ID")?
			.to_owned();
		let upload = poll(
			"addons.mozilla.org's validation",
			120,
			Duration::from_secs(5),
			|| {
				let upload = self
					.send("GET", &format!("/addons/upload/{uuid}/"), None)?
					.ok("reading the upload's validation")?
					.json()?;
				Ok(upload["processed"]
					.as_bool()
					.unwrap_or(false)
					.then_some(upload))
			},
		)?;
		if !upload["valid"].as_bool().unwrap_or(false) {
			let messages: Vec<String> = upload["validation"]["messages"]
				.as_array()
				.into_iter()
				.flatten()
				.filter(|message| message["type"] == "error")
				.filter_map(|message| message["message"].as_str().map(str::to_owned))
				.collect();
			anyhow::bail!(
				"addons.mozilla.org's validation failed: {}",
				messages.join("; ")
			);
		}
		Ok(uuid)
	}

	/// Creates a version of the add-on from an upload, with the source it was
	/// built from. A listed version goes into review.
	///
	/// # Errors
	///
	/// When the add-on doesn't exist yet, which its first submission on the
	/// site creates, or addons.mozilla.org refuses.
	pub fn submit(&self, upload: &str, source: &[u8]) -> Result<Value> {
		let reply = self.send(
			"POST",
			&format!("/addons/addon/{}/versions/", segment(&self.guid)),
			Some(Body::multipart(&[
				("upload", Part::Text(upload)),
				(
					"source",
					Part::File {
						filename: "firefox-source.zip",
						bytes: source,
					},
				),
			])),
		)?;
		if reply.status == 404 {
			anyhow::bail!(
				"addons.mozilla.org has no add-on {} yet: submit its first version on the site, \
				 then publish updates from here",
				self.guid
			);
		}
		reply.ok("submitting to addons.mozilla.org")?.json()
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn add_on_ids_keep_their_shape_in_paths() {
		assert_eq!(super::segment("notes@example.com"), "notes@example.com");
		assert_eq!(super::segment("{abc}"), "%7Babc%7D");
	}
}
