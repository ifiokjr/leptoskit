//! Passkeys and Sign in with Apple, with the platform's own credential UI.
//!
//! Passkey calls carry WebAuthn's standard JSON both ways, as
//! `navigator.credentials` would in a browser: [`CREATE`] takes
//! `PublicKeyCredentialCreationOptionsJSON` and answers
//! `RegistrationResponseJSON`, and [`GET`] takes
//! `PublicKeyCredentialRequestOptionsJSON` and answers
//! `AuthenticationResponseJSON`. The relying party is the options' `rp.id`
//! (or `rpId`), which the app must be associated with: the site's
//! `apple-app-site-association` lists it under `webcredentials`, and its
//! `assetlinks.json` lets the Android app get its credentials.
//!
//! | Operation         | iOS, macOS                            | Android            | Windows, Linux |
//! | ----------------- | ------------------------------------- | ------------------ | -------------- |
//! | Create a passkey  | `ASAuthorizationPlatformPublicKey…`   | Credential Manager | unavailable    |
//! | Sign with one     | `ASAuthorizationPlatformPublicKey…`   | Credential Manager | unavailable    |
//! | Sign in with Apple | `ASAuthorizationAppleIDProvider`     | unavailable        | unavailable    |
//!
//! Where a call is unavailable, the sign-in client goes through the system
//! browser instead.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use serde::Deserialize;
use serde::Serialize;

/// Stable plugin registration name.
pub const NAME: &str = "passkeys";
/// Creation options JSON; answers the registration JSON.
pub const CREATE: MethodId = MethodId(0);
/// Request options JSON; answers the assertion JSON.
pub const GET: MethodId = MethodId(1);
/// A UTF-8 nonce; answers an [`AppleIdCredential`] as JSON.
pub const APPLE_ID: MethodId = MethodId(2);
/// Exported methods, independently grantable by the host.
pub const METHODS: &[MethodId] = &[CREATE, GET, APPLE_ID];
/// The largest options or answer accepted.
pub const MAX_JSON_BYTES: usize = 64 * 1024;
/// The longest nonce Sign in with Apple is given.
pub const MAX_NONCE_BYTES: usize = 128;

/// What Sign in with Apple on the device answered.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppleIdCredential {
	/// The ID token, a JWT Apple signed for the app's bundle ID.
	pub id_token: String,
	/// Their given name, shared on the first sign-in only.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub given_name: Option<String>,
	/// Their family name, shared on the first sign-in only.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub family_name: Option<String>,
}

impl core::fmt::Debug for AppleIdCredential {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		f.debug_struct("AppleIdCredential").finish_non_exhaustive()
	}
}

impl AppleIdCredential {
	/// Reads the plugin's answer, checking the token is a JWT's shape.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_JSON_BYTES {
			return Err(Error::InvalidArgument);
		}

		let credential: Self =
			serde_json::from_slice(payload).map_err(|_| Error::InvalidArgument)?;
		let parts = credential.id_token.split('.').count();

		if parts != 3
			|| !credential
				.id_token
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
		{
			return Err(Error::InvalidArgument);
		}

		Ok(credential)
	}

	/// The name as Sign in with Apple's web flow sends it, for the server:
	/// `{"name":{"firstName":…,"lastName":…}}`, or `None` without one.
	#[must_use]
	pub fn user(&self) -> Option<String> {
		if self.given_name.is_none() && self.family_name.is_none() {
			return None;
		}

		Some(
			serde_json::json!({
				"name": { "firstName": self.given_name, "lastName": self.family_name },
			})
			.to_string(),
		)
	}
}

/// Checks a nonce for Sign in with Apple: 1 to 128 printable ASCII
/// characters.
pub fn nonce(value: &str) -> Result<&str, Error> {
	if value.is_empty()
		|| value.len() > MAX_NONCE_BYTES
		|| !value.bytes().all(|byte| byte.is_ascii_graphic())
	{
		return Err(Error::InvalidArgument);
	}

	Ok(value)
}

/// Checks WebAuthn options or an answer: a JSON object, at most
/// [`MAX_JSON_BYTES`], with a `challenge` for options.
pub fn json_object(payload: &[u8], challenge: bool) -> Result<&str, Error> {
	if payload.is_empty() || payload.len() > MAX_JSON_BYTES {
		return Err(Error::InvalidArgument);
	}

	let text = core::str::from_utf8(payload).map_err(|_| Error::InvalidArgument)?;
	let value: serde_json::Value =
		serde_json::from_str(text).map_err(|_| Error::InvalidArgument)?;
	let object = value.as_object().ok_or(Error::InvalidArgument)?;

	if challenge
		&& !object
			.get("challenge")
			.is_some_and(serde_json::Value::is_string)
	{
		return Err(Error::InvalidArgument);
	}

	Ok(text)
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Passkeys(leptoskit_bridge::guest::Client);

	impl Passkeys {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Makes a passkey with the creation options JSON, and answers the
		/// registration JSON. Fails with `Cancelled` when the person closes
		/// the sheet.
		pub async fn create(self, options: &str) -> Result<String, Error> {
			json_object(options.as_bytes(), true)?;
			let answer = self.0.call(CREATE, options.as_bytes())?.await?;
			json_object(&answer, false).map(str::to_owned)
		}

		/// Signs with a passkey for the request options JSON, and answers
		/// the assertion JSON.
		pub async fn get(self, options: &str) -> Result<String, Error> {
			json_object(options.as_bytes(), true)?;
			let answer = self.0.call(GET, options.as_bytes())?.await?;
			json_object(&answer, false).map(str::to_owned)
		}

		/// Signs in with Apple on the device, with the server's `nonce`.
		/// `Unavailable` off Apple platforms.
		pub async fn apple_id(self, nonce: &str) -> Result<AppleIdCredential, Error> {
			let nonce = super::nonce(nonce)?;
			AppleIdCredential::decode(&self.0.call(APPLE_ID, nonce.as_bytes())?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn options_and_answers_are_bounded_json_objects() {
		let options = r#"{"challenge":"AAEC","rp":{"id":"example.com"}}"#;
		assert_eq!(json_object(options.as_bytes(), true), Ok(options));
		assert!(json_object(br#"{"id":"x"}"#, false).is_ok());

		for bad in [
			&b""[..],
			b"[]",
			b"\"text\"",
			b"{not json",
			br#"{"challenge":5}"#,
			br#"{"rp":{}}"#,
			b"\xff",
		] {
			assert_eq!(json_object(bad, true), Err(Error::InvalidArgument));
		}

		let huge = format!(r#"{{"challenge":"{}"}}"#, "a".repeat(MAX_JSON_BYTES));
		assert_eq!(
			json_object(huge.as_bytes(), true),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn apple_id_answers_carry_a_token_and_maybe_a_name() {
		let credential =
			AppleIdCredential::decode(br#"{"id_token":"aaa.bbb.c-_","given_name":"Ada"}"#).unwrap();
		assert_eq!(credential.id_token, "aaa.bbb.c-_");
		assert_eq!(
			credential.user().unwrap(),
			r#"{"name":{"firstName":"Ada","lastName":null}}"#
		);
		assert!(!format!("{credential:?}").contains("aaa"));
		let unnamed = AppleIdCredential::decode(br#"{"id_token":"a.b.c"}"#).unwrap();
		assert_eq!(unnamed.user(), None);

		for bad in [
			&br#"{"id_token":"a.b"}"#[..],
			br#"{"id_token":"a.b.c d"}"#,
			br#"{"token":"a.b.c"}"#,
			b"nope",
		] {
			assert_eq!(AppleIdCredential::decode(bad), Err(Error::InvalidArgument));
		}

		assert_eq!(nonce("abc-_"), Ok("abc-_"));
		assert_eq!(nonce(""), Err(Error::InvalidArgument));
		assert_eq!(nonce("a b"), Err(Error::InvalidArgument));
		assert_eq!(nonce(&"a".repeat(129)), Err(Error::InvalidArgument));
		assert_eq!(METHODS, &[CREATE, GET, APPLE_ID]);
	}
}
