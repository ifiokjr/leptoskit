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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyCode {
	/// The address the code was sent to.
	pub email: String,
	/// The code, as typed.
	pub code: String,
	/// How the device keeps its session.
	#[serde(default)]
	pub transport: Transport,
}

/// How a device keeps its session token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
	/// In an `HttpOnly` cookie, which scripts can't read: for browsers.
	#[default]
	Cookie,
	/// In the response body, for a native app to keep in its secure store
	/// and send as `Authorization: Bearer`.
	Bearer,
}

/// The answer to a successful sign-in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignedInResponse {
	/// The new session.
	pub session: PublicSession,
	/// Whether the account was created by this sign-in.
	pub signed_up: bool,
	/// The session token, only for [`Transport::Bearer`].
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
}

/// The answer to `GET /auth/session`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionResponse {
	/// The request's session, or `None` when it's signed out.
	pub session: Option<PublicSession>,
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
	/// `forbidden`, `mail`, `sign_up`, or `server`.
	pub error: String,
	/// What went wrong, for people.
	pub message: String,
	/// For `rate_limited`, how many seconds until the next attempt counts.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub retry_after: Option<u64>,
}
