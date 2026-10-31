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

/// What `POST /auth/email/verify` answers: signed in, or the second step.
/// Untagged, so a completed sign-in reads exactly as [`SignedInResponse`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VerifyResponse {
	/// Signed in.
	SignedIn(SignedInResponse),
	/// The account has a second factor; send a code from it to
	/// `POST /auth/second-factor`.
	SecondFactor(SecondFactorResponse),
}

/// The second step of signing in, as `POST /auth/email/verify` describes
/// it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondFactorResponse {
	/// What the step accepts.
	pub second_factor: SecondFactor,
}

/// The second step of signing in.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondFactor {
	/// The factors the account can use, best first.
	pub factors: Vec<Factor>,
	/// Seconds until the step expires and signing in starts again.
	pub expires_in: u64,
	/// What a native app sends back with the code. A browser gets it in an
	/// `HttpOnly` cookie instead, so this is `None` there.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
}

impl std::fmt::Debug for SecondFactor {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SecondFactor")
			.field("factors", &self.factors)
			.field("expires_in", &self.expires_in)
			.finish_non_exhaustive()
	}
}

/// A way to prove who you are a second time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Factor {
	/// A six-digit code from an authenticator app.
	Totp,
	/// One of the account's recovery codes, which each work once.
	RecoveryCode,
}

/// The body of `POST /auth/second-factor`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondFactorCode {
	/// A code from the authenticator app, or a recovery code, as typed.
	pub code: String,
	/// A native app's [`SecondFactor::token`]. A browser's comes in its
	/// cookie.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
	/// How the device keeps its session.
	#[serde(default)]
	pub transport: Transport,
	/// What a native app calls its device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device_name: Option<String>,
}

impl std::fmt::Debug for SecondFactorCode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SecondFactorCode")
			.field("transport", &self.transport)
			.field("device_name", &self.device_name)
			.finish_non_exhaustive()
	}
}

/// The body of `POST /auth/step-up` and `POST /auth/totp/confirm`: a code
/// from the authenticator app, or for step-up a recovery code.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactorCode {
	/// The code, as typed.
	pub code: String,
}

impl std::fmt::Debug for FactorCode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("FactorCode").finish_non_exhaustive()
	}
}

/// The answer to `GET /auth/mfa`: the second factors the account has on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MfaStatus {
	/// Whether an authenticator app is on.
	pub totp: bool,
	/// How many unused recovery codes are left.
	pub recovery_codes: u32,
}

/// A new authenticator app's secret, from `POST /auth/totp`. Show `uri` as
/// a QR code, or as a link on the phone itself, and `secret` for typing in.
/// Its `Debug` leaves both out.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotpSetup {
	/// The secret, in base32, as authenticator apps take it typed in.
	pub secret: String,
	/// The `otpauth://` link that carries the secret, the app's name, and
	/// the account's address.
	pub uri: String,
}

impl std::fmt::Debug for TotpSetup {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("TotpSetup").finish_non_exhaustive()
	}
}

/// Recovery codes, shown once: ten, each working once in place of a code
/// from the authenticator app. Its `Debug` gives only how many.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCodes {
	/// The codes, such as `K7QMR-2XDPA`.
	pub codes: Vec<String>,
}

impl std::fmt::Debug for RecoveryCodes {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RecoveryCodes")
			.field("count", &self.codes.len())
			.finish_non_exhaustive()
	}
}

/// The answer to `POST /auth/totp/confirm`: the session, now at `aal2` with
/// a new token, and recovery codes when the account had none.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TotpConfirmedResponse {
	/// The session.
	pub session: PublicSession,
	/// A native app's new tokens.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tokens: Option<Tokens>,
	/// New recovery codes, shown once.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub recovery_codes: Option<RecoveryCodes>,
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
	/// `sign_in_expired`, `step_up_required`, `reauthenticate`,
	/// `already_enrolled`, `not_enrolled`, `forbidden`, `mail`, `sign_up`, or
	/// `server`; and from the client,
	/// `network` when the server can't be reached, `config` when the app
	/// isn't set up to reach it, or `storage` when a native app's secure
	/// store fails.
	pub error: String,
	/// What went wrong, for people.
	pub message: String,
	/// For `rate_limited`, how many seconds until the next attempt counts.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub retry_after: Option<u64>,
}
