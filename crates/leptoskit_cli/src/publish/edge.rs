//! Microsoft Edge Add-ons, through its Publish API with an API key.

use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use serde_json::Value;
use serde_json::json;

use super::http::Body;
use super::http::Http;
use super::http::Reply;
use super::http::poll;
use super::secrets::Secret;

/// The API's address.
pub const API: &str = "https://api.addons.microsoftedge.microsoft.com";
/// The credentials, from Partner Center's Publish API page.
pub const CLIENT_ID: &str = "LEPTOSKIT_PUBLISH_EDGE_CLIENT_ID";
pub const API_KEY: &str = "LEPTOSKIT_PUBLISH_EDGE_API_KEY";
/// Where to make them.
pub const HOW: &str = "open Partner Center's Publish API page for Microsoft Edge, turn the API \
                       on, and create an API key (keys expire after 72 days)";

/// A client for one product.
pub struct Edge<'a> {
	http: &'a Http,
	api: String,
	product: String,
	client_id: Secret,
	api_key: Secret,
}

impl<'a> Edge<'a> {
	#[must_use]
	pub fn new(
		http: &'a Http,
		api: &str,
		product: &str,
		client_id: Secret,
		api_key: Secret,
	) -> Self {
		Self {
			http,
			api: api.trim_end_matches('/').to_owned(),
			product: product.to_owned(),
			client_id,
			api_key,
		}
	}

	fn send(&self, method: &str, path: &str, body: Option<Body>) -> Result<Reply> {
		let authorization = format!("ApiKey {}", self.api_key.expose());
		let reply = self.http.send(
			method,
			&format!("{}/v1/products/{}{path}", self.api, self.product),
			&[
				("authorization", &authorization),
				("x-clientid", self.client_id.expose()),
			],
			body,
		)?;
		if reply.status == 401 {
			anyhow::bail!(
				"Edge Add-ons refused the API key; keys expire after 72 days, so make a new one \
				 on Partner Center's Publish API page"
			);
		}
		Ok(reply)
	}

	/// The operation ID a `202 Accepted` names in its `Location`.
	fn operation(reply: &Reply) -> Result<String> {
		reply
			.header("location")
			.and_then(|location| location.trim_end_matches('/').rsplit('/').next())
			.map(str::to_owned)
			.context("Edge Add-ons accepted the request without naming its operation")
	}

	/// Waits for an operation at `path` to finish.
	fn wait(&self, what: &str, path: &str) -> Result<Value> {
		poll(what, 120, Duration::from_secs(5), || {
			let status = self.send("GET", path, None)?.ok(what)?.json()?;
			match status["status"].as_str() {
				Some("InProgress") | None => Ok(None),
				Some("Succeeded") => Ok(Some(status)),
				Some(_) => {
					anyhow::bail!(
						"{what} failed: {}",
						status["message"]
							.as_str()
							.map_or_else(|| status.to_string(), str::to_owned)
					)
				}
			}
		})
	}

	/// Uploads a new package as the product's draft, and waits until it is
	/// processed.
	///
	/// # Errors
	///
	/// When Edge Add-ons refuses it, with its reason.
	pub fn upload(&self, zip: Vec<u8>) -> Result<()> {
		let reply = self
			.send("POST", "/submissions/draft/package", Some(Body::zip(zip)))?
			.ok("uploading to Edge Add-ons")?;
		let operation = Self::operation(&reply)?;
		self.wait(
			"Edge Add-ons' processing",
			&format!("/submissions/draft/package/operations/{operation}"),
		)
		.map(drop)
	}

	/// Submits the draft for certification, with release notes.
	///
	/// # Errors
	///
	/// When Edge Add-ons refuses, such as when the listing is incomplete.
	pub fn submit(&self, notes: &str) -> Result<()> {
		let reply = self
			.send(
				"POST",
				"/submissions",
				Some(Body::json(&json!({ "notes": notes }))),
			)?
			.ok("submitting to Edge Add-ons")?;
		let operation = Self::operation(&reply)?;
		self.wait(
			"Edge Add-ons' submission",
			&format!("/submissions/operations/{operation}"),
		)
		.map(drop)
	}
}
