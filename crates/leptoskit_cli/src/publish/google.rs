//! Signing in to Google's APIs with a service account, as the Chrome Web
//! Store and Google Play take.

use anyhow::Context;
use anyhow::Result;
use serde_json::Value;
use serde_json::json;

use super::http::Body;
use super::http::Http;
use super::secrets::Secret;
use super::secrets::issued_at;
use super::secrets::rs256;

/// Google's OAuth token endpoint, which a service account's signed token is
/// exchanged at.
pub const OAUTH: &str = "https://oauth2.googleapis.com/token";

/// Exchanges a token signed with the service account's key for an access
/// token to `scope`. `account` is the JSON key, or its path, from the
/// credential `name`.
///
/// # Errors
///
/// When the key is unusable or Google refuses it.
pub fn access_token(
	http: &Http,
	oauth: &str,
	account: &Secret,
	name: &str,
	scope: &str,
	store: &str,
) -> Result<Secret> {
	let text = account.expose().trim();
	let text = if text.starts_with('{') {
		text.to_owned()
	} else {
		std::fs::read_to_string(text)
			.with_context(|| format!("{name} names a file that can't be read"))?
	};

	let account: Value = serde_json::from_str(&text)
		.with_context(|| format!("{name} isn't a service account's JSON key"))?;
	let email = account["client_email"]
		.as_str()
		.with_context(|| format!("{name} has no client_email"))?;
	let key = account["private_key"]
		.as_str()
		.with_context(|| format!("{name} has no private_key"))?;
	let issued = issued_at();
	let assertion = rs256(
		&json!({
			"iss": email,
			"scope": scope,
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
		.ok(&format!("signing in to {store}"))?
		.json()?;
	let token = reply["access_token"]
		.as_str()
		.context("Google answered without an access token")?;
	Ok(Secret::new(token.to_owned()))
}
