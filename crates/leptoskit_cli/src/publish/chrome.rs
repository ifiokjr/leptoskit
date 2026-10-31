//! The Chrome Web Store, through its v2 API with a service account.

use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use serde_json::Value;
use serde_json::json;

use super::http::Body;
use super::http::Http;
use super::http::poll;
use super::secrets::Secret;
use super::secrets::issued_at;
use super::secrets::rs256;

/// The API's address.
pub const API: &str = "https://chromewebstore.googleapis.com";
/// Google's OAuth token endpoint, which a service account's signed token is
/// exchanged at.
pub const OAUTH: &str = "https://oauth2.googleapis.com/token";
/// The credential: the service account's JSON key, or the path to it.
pub const CREDENTIAL: &str = "LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT";
/// Where to make one.
pub const HOW: &str = "create a Google Cloud service account with a JSON key, then add its email \
                       in the Chrome Web Store developer dashboard, under Account, API access";

/// A signed-in client for one item.
pub struct Chrome<'a> {
	http: &'a Http,
	api: String,
	publisher: String,
	item: String,
	token: Secret,
}

impl<'a> Chrome<'a> {
	/// Exchanges the service account's signed token for an access token.
	///
	/// # Errors
	///
	/// When the key is unusable or Google refuses it.
	pub fn connect(
		http: &'a Http,
		api: &str,
		oauth: &str,
		account: &Secret,
		publisher: &str,
		item: &str,
	) -> Result<Self> {
		let text = account.expose().trim();
		let text = if text.starts_with('{') {
			text.to_owned()
		} else {
			std::fs::read_to_string(text)
				.with_context(|| format!("{CREDENTIAL} names a file that can't be read"))?
		};
		let account: Value = serde_json::from_str(&text)
			.with_context(|| format!("{CREDENTIAL} isn't a service account's JSON key"))?;
		let email = account["client_email"]
			.as_str()
			.with_context(|| format!("{CREDENTIAL} has no client_email"))?;
		let key = account["private_key"]
			.as_str()
			.with_context(|| format!("{CREDENTIAL} has no private_key"))?;
		let issued = issued_at();
		let assertion = rs256(
			&json!({
				"iss": email,
				"scope": "https://www.googleapis.com/auth/chromewebstore",
				"aud": oauth,
				"iat": issued,
				"exp": issued + 3600,
			}),
			key,
		)?;
		let reply = http
			.send(
				"POST",
				oauth,
				&[],
				Some(Body::form(&[
					("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
					("assertion", &assertion),
				])),
			)?
			.ok("signing in to the Chrome Web Store")?
			.json()?;
		let token = reply["access_token"]
			.as_str()
			.context("Google answered without an access token")?;
		Ok(Self {
			http,
			api: api.trim_end_matches('/').to_owned(),
			publisher: publisher.to_owned(),
			item: item.to_owned(),
			token: Secret::from(token),
		})
	}

	fn item_url(&self, upload: bool, action: &str) -> String {
		let prefix = if upload { "/upload" } else { "" };
		format!(
			"{}{prefix}/v2/publishers/{}/items/{}:{action}",
			self.api, self.publisher, self.item
		)
	}

	fn bearer(&self) -> String {
		format!("Bearer {}", self.token.expose())
	}

	/// The item's state: what was uploaded, submitted, and published.
	///
	/// # Errors
	///
	/// When the store refuses.
	pub fn status(&self) -> Result<Value> {
		self.http
			.send(
				"GET",
				&self.item_url(false, "fetchStatus"),
				&[("authorization", &self.bearer())],
				None,
			)?
			.ok("reading the item's status")?
			.json()
	}

	/// Uploads a new package as the item's draft, and waits until the store
	/// has processed it. Returns the version the store read from it.
	///
	/// # Errors
	///
	/// When the store refuses the package, with its reason.
	pub fn upload(&self, zip: Vec<u8>) -> Result<String> {
		let reply = self
			.http
			.send(
				"POST",
				&self.item_url(true, "upload"),
				&[("authorization", &self.bearer())],
				Some(Body::zip(zip)),
			)?
			.ok("uploading to the Chrome Web Store")?
			.json()?;
		let state = reply["uploadState"].as_str().unwrap_or_default().to_owned();
		if state == "IN_PROGRESS" {
			poll(
				"The Chrome Web Store's processing",
				60,
				Duration::from_secs(5),
				|| {
					let status = self.status()?;
					Ok(match status["lastAsyncUploadState"].as_str() {
						Some("IN_PROGRESS") => None,
						Some("FAILED") => {
							anyhow::bail!(
								"the Chrome Web Store couldn't process the package: {status}"
							)
						}
						_ => Some(()),
					})
				},
			)?;
		} else if state != "SUCCEEDED" {
			anyhow::bail!("the Chrome Web Store refused the package: {reply}");
		}
		Ok(reply["crxVersion"].as_str().unwrap_or_default().to_owned())
	}

	/// Submits the draft for review. With `staged`, an approved version
	/// waits to be published by hand.
	///
	/// # Errors
	///
	/// When the store refuses, such as when the listing is incomplete.
	pub fn submit(&self, staged: bool) -> Result<String> {
		let reply = self
			.http
			.send(
				"POST",
				&self.item_url(false, "publish"),
				&[("authorization", &self.bearer())],
				Some(Body::json(&json!({
					"publishType": if staged { "STAGED_PUBLISH" } else { "DEFAULT_PUBLISH" },
				}))),
			)?
			.ok("submitting to the Chrome Web Store")?
			.json()?;
		Ok(reply["state"].as_str().unwrap_or("submitted").to_owned())
	}
}

impl From<&str> for Secret {
	fn from(value: &str) -> Self {
		Secret::new(value.to_owned())
	}
}
