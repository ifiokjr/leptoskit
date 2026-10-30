//! What the `/auth` routes send and receive. Clients build these too, so
//! they need no server code.

use serde::Deserialize;
use serde::Serialize;

use crate::PublicSession;

/// The body of `POST /auth/email/send`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendCode {
	/// The address to send a code to.
	pub email: String,
}

/// The body of `POST /auth/email/verify`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyCode {
	/// The address the code was sent to.
	pub email: String,
	/// The code, as typed.
	pub code: String,
	/// How the device keeps its session.
	#[serde(default)]
	pub transport: Transport,
	/// What a native app calls its device, such as "Ada's iPhone", for the
	/// list of signed-in devices. Browsers are named from their user agent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device_name: Option<String>,
}

/// How a device keeps its session token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
	/// In an `HttpOnly` cookie, which scripts can't read: for browsers.
	#[default]
	Cookie,
	/// As [`Tokens`] in the response body, for a native app: an access token
	/// it sends as `Authorization: Bearer`, and a refresh token it keeps in
	/// its secure store and trades for new ones at `POST /auth/refresh`.
	Bearer,
}

/// A native app's tokens. Keep the refresh token in the platform's secure
/// store; it works once, and trades for a new pair before the access token
/// expires.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
	/// Sent with each request as `Authorization: Bearer`.
	pub access_token: String,
	/// Trades for new tokens at `POST /auth/refresh`, once.
	pub refresh_token: String,
	/// Seconds until the access token stops working.
	pub expires_in: u64,
}

impl std::fmt::Debug for Tokens {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Tokens")
			.field("expires_in", &self.expires_in)
			.finish_non_exhaustive()
	}
}

/// The body of `POST /auth/refresh`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshTokens {
	/// The refresh token from the last sign-in or refresh.
	pub refresh_token: String,
}

impl std::fmt::Debug for RefreshTokens {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RefreshTokens").finish_non_exhaustive()
	}
}

/// The answer to a successful sign-in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignedInResponse {
	/// The new session.
	pub session: PublicSession,
	/// Whether the account was created by this sign-in.
	pub signed_up: bool,
	/// The new tokens, only for [`Transport::Bearer`].
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tokens: Option<Tokens>,
}

/// The answer to `GET /auth/session`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionResponse {
	/// The request's session, or `None` when it's signed out.
	pub session: Option<PublicSession>,
}

/// One of the signed-in person's sessions, as `GET /auth/sessions` lists
/// them: one per device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
	/// The session.
	#[serde(flatten)]
	pub session: PublicSession,
	/// Whether it's the session the request came from.
	pub current: bool,
}

/// The body of `POST /auth/sessions/revoke`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeSession {
	/// The session to end.
	pub id: leptoskit_db::Id<crate::Session>,
}

/// Why an `/auth` request failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
	/// What went wrong, for code to match on: `invalid_email`,
	/// `invalid_code`, `rate_limited`, `disabled`, `locked`, `signed_out`,
	/// `forbidden`, `mail`, `sign_up`, or `server`; and from the client,
	/// `network` or `invalid`.
	pub error: String,
	/// What went wrong, for people.
	pub message: String,
	/// For `rate_limited`, how many seconds until the next attempt counts.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub retry_after: Option<u64>,
}
