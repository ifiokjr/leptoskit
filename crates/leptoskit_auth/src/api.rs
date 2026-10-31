//! What the `/auth` routes send and receive. Clients build these too, so
//! they need no server code.

use leptoskit_db::Id;
use leptoskit_db::Pubkey;
use leptoskit_db::Signature;
use leptoskit_db::Timestamp;
use serde::Deserialize;
use serde::Serialize;
pub use webauthn_rs_proto::CreationChallengeResponse;
pub use webauthn_rs_proto::PublicKeyCredential;
pub use webauthn_rs_proto::RegisterPublicKeyCredential;
pub use webauthn_rs_proto::RequestChallengeResponse;

use crate::Identity;
use crate::Passkey;
use crate::PublicSession;
use crate::SiwsInput;
use crate::Wallet;

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
	/// One of the account's passkeys.
	Passkey,
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
	/// How many passkeys the account has.
	#[serde(default)]
	pub passkeys: u32,
	/// Whether the account must have a second factor, as `[auth] mfa` or
	/// `required_for` says. Until it has one, its sessions may only set one
	/// up.
	#[serde(default)]
	pub required: bool,
}

impl MfaStatus {
	/// Whether the account has a second factor: an authenticator app or a
	/// passkey.
	pub const fn enrolled(&self) -> bool {
		self.totp || self.passkeys > 0
	}
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
	pub id: Id<crate::Session>,
}

/// Why an `/auth` request failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
	/// What went wrong, for code to match on: `invalid_email`,
	/// `invalid_code`, `rate_limited`, `disabled`, `locked`, `signed_out`,
	/// `sign_in_expired`, `step_up_required`, `reauthenticate`,
	/// `already_enrolled`, `not_enrolled`, `mfa_required`, `passkey_failed`,
	/// `last_sign_in_method`, `not_found`, `invalid_name`,
	/// `unknown_provider`, `provider`, `invalid_redirect`, `cancelled`,
	/// `wallet_failed`, `wallet_in_use`, `unknown_wallet`, `proof_required`,
	/// `merge_conflict`, `unsupported`, `forbidden`, `mail`, `sign_up`, or
	/// `server`; and from
	/// the client, `network` when the server can't be reached, `config` when
	/// the app isn't set up to reach it, `storage` when a native app's secure
	/// store fails, `cancelled` when the person dismissed a passkey or wallet
	/// prompt, `no_wallet` when the browser has no wallet by that name, or
	/// `unsupported` when this device can't do that yet, such as wallets in
	/// a native app.
	pub error: String,
	/// What went wrong, for people.
	pub message: String,
	/// For `rate_limited`, how many seconds until the next attempt counts.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub retry_after: Option<u64>,
}

/// One of the signed-in person's passkeys, as `GET /auth/passkeys` lists
/// them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyInfo {
	/// The passkey.
	pub id: Id<Passkey>,
	/// What the person calls it.
	pub name: String,
	/// Whether it's synced to their other devices, such as through a
	/// password manager.
	pub backed_up: bool,
	/// When it was added.
	pub created_at: Timestamp,
	/// When it last signed in or confirmed it's them.
	pub last_used_at: Option<Timestamp>,
}

/// What a device needs to ask its passkeys for a signature: the WebAuthn
/// options, and for a native app the token that matches its answer to this
/// request. A browser gets that in an `HttpOnly` cookie instead.
#[derive(Clone, Serialize, Deserialize)]
pub struct PasskeyOptions {
	/// The options for `navigator.credentials.get`.
	pub options: RequestChallengeResponse,
	/// What a native app sends back with the signature.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
}

impl std::fmt::Debug for PasskeyOptions {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PasskeyOptions").finish_non_exhaustive()
	}
}

/// The body of `POST /auth/passkey/sign-in/options` and
/// `POST /auth/passkey/second-factor/options`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyStep {
	/// For the second step, a native app's [`SecondFactor::token`]. A
	/// browser's comes in its cookie.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
	/// How the device keeps its session, and so where the ceremony's own
	/// token goes: a cookie, or the body.
	#[serde(default)]
	pub transport: Transport,
}

impl std::fmt::Debug for PasskeyStep {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PasskeyStep").finish_non_exhaustive()
	}
}

/// A passkey's signature, for `POST /auth/passkey/sign-in`,
/// `/auth/passkey/second-factor`, and `/auth/passkey/step-up`.
#[derive(Clone, Serialize, Deserialize)]
pub struct PasskeyAssertion {
	/// What `navigator.credentials.get` returned.
	pub credential: PublicKeyCredential,
	/// A native app's [`PasskeyOptions::token`], or for the second step its
	/// [`SecondFactor::token`].
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
	/// How the device keeps its session.
	#[serde(default)]
	pub transport: Transport,
	/// What a native app calls its device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device_name: Option<String>,
}

impl std::fmt::Debug for PasskeyAssertion {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PasskeyAssertion")
			.field("transport", &self.transport)
			.field("device_name", &self.device_name)
			.finish_non_exhaustive()
	}
}

/// The body of `POST /auth/passkey/register`: the new passkey, and what to
/// call it.
#[derive(Clone, Serialize, Deserialize)]
pub struct AddPasskey {
	/// What `navigator.credentials.create` returned.
	pub credential: RegisterPublicKeyCredential,
	/// A name, such as "Work laptop". Without one, it's named after the
	/// device that added it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
}

impl std::fmt::Debug for AddPasskey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AddPasskey")
			.field("name", &self.name)
			.finish_non_exhaustive()
	}
}

/// The answer to `POST /auth/passkey/register`: the passkey, the session,
/// now at `aal2` with a new token, and recovery codes when the account had
/// none.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PasskeyAddedResponse {
	/// The new passkey.
	pub passkey: PasskeyInfo,
	/// The session.
	pub session: PublicSession,
	/// A native app's new tokens.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tokens: Option<Tokens>,
	/// New recovery codes, shown once.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub recovery_codes: Option<RecoveryCodes>,
}

/// The body of `POST /auth/passkeys/rename`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenamePasskey {
	/// The passkey.
	pub id: Id<Passkey>,
	/// Its new name: 1 to 100 characters.
	pub name: String,
}

/// The body of `POST /auth/passkeys/remove`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovePasskey {
	/// The passkey.
	pub id: Id<Passkey>,
}

/// A third-party sign-in provider the server has set up, as
/// `GET /auth/providers` lists them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderInfo {
	/// Its ID in `[auth.providers]`, such as `google`.
	pub id: String,
	/// Its name, for a button: "Google".
	pub name: String,
}

/// Which provider to sign in with, and where to come back to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartProvider {
	/// The provider's ID, such as `google`.
	pub provider: String,
	/// The path on this site to come back to, such as `/settings`. Anything
	/// else is refused. Defaults to `/`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub redirect_to: Option<String>,
}

/// What the client sends `POST /auth/oauth/start` and `POST /auth/oauth/link`:
/// the app's [`StartProvider`], and for a native app how it goes through the
/// provider's sign-in. The client fills that in; app code never does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartProviderRequest {
	/// The provider, and where to come back to.
	#[serde(flatten)]
	pub start: StartProvider,
	/// How a native app signs in, which also says it keeps its session as
	/// [`Tokens`]. `None` in a browser.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub native: Option<NativeStart>,
}

/// How a native app goes through a provider's sign-in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeStart {
	/// In the system browser, never a web view in the app (RFC 8252). The
	/// provider sends the browser back to this site, which sends it on to
	/// `return_to`: the app's own address on this site's
	/// `/auth/native/return`, a loopback address with that path for a
	/// desktop app, or `<package id>:/auth/native/return`.
	Browser {
		/// Where the system browser comes back to the app.
		return_to: String,
	},
	/// With Sign in with Apple on the device itself, which answers with an
	/// ID token for the app's bundle ID.
	AppleId,
}

/// Where to send the browser to sign in with a provider.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRedirect {
	/// The provider's authorization page.
	pub url: String,
	/// For a native app: what binds the flow to it, sent back to
	/// `POST /auth/oauth/finish`. A browser keeps it in an `HttpOnly`
	/// cookie instead, so this is `None` there.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
	/// For [`NativeStart::AppleId`]: the nonce to give Sign in with Apple,
	/// which its ID token must carry.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub nonce: Option<String>,
}

impl std::fmt::Debug for ProviderRedirect {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ProviderRedirect")
			.field("url", &self.url)
			.finish_non_exhaustive()
	}
}

/// The body of `POST /auth/oauth/finish`: a native app's provider flow,
/// back from the system browser or from Sign in with Apple.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinishProvider {
	/// The provider's ID, such as `google`.
	pub provider: String,
	/// [`ProviderRedirect::token`], from starting the flow.
	pub token: String,
	/// The `state` the browser came back with.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub state: Option<String>,
	/// The authorization code it came back with.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub code: Option<String>,
	/// Why the provider refused, such as `access_denied`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
	/// Sign in with Apple's name, sent on the first sign-in only: the web
	/// flow's `user` JSON, or `{"name":{"firstName":…,"lastName":…}}` from
	/// the device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user: Option<String>,
	/// The ID token Sign in with Apple gave the device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub id_token: Option<String>,
	/// What the app calls this device, for the list of signed-in devices.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device_name: Option<String>,
}

impl std::fmt::Debug for FinishProvider {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("FinishProvider")
			.field("provider", &self.provider)
			.field("error", &self.error)
			.finish_non_exhaustive()
	}
}

/// What `POST /auth/oauth/finish` answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderFinished {
	/// How it ended, as a browser's return page would say.
	pub outcome: ProviderOutcome,
	/// The new session and its tokens, when it signed in.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub signed_in: Option<SignedInResponse>,
	/// The second step, when the account has a second factor.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub second_factor: Option<SecondFactor>,
}

/// The body a native app sends to `POST /auth/oauth/continue`, after the
/// outcome [`ProviderOutcome::EmailInUse`]. A browser sends none: its
/// binding is in a cookie.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinueProvider {
	/// [`ProviderRedirect::token`], from starting the flow. Without it, the
	/// browser's cookie binds the sign-up.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
	/// What the app calls this device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device_name: Option<String>,
}

impl std::fmt::Debug for ContinueProvider {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ContinueProvider")
			.field("device_name", &self.device_name)
			.finish_non_exhaustive()
	}
}

/// One of the signed-in person's connected accounts, as
/// `GET /auth/identities` lists them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityInfo {
	/// The connection.
	pub id: Id<Identity>,
	/// The provider's ID, such as `google`.
	pub provider: String,
	/// The provider's name, such as "Google".
	pub provider_name: String,
	/// Their name or username there.
	pub name: Option<String>,
	/// The address the provider gave.
	pub email: Option<String>,
	/// When it was connected.
	pub created_at: Timestamp,
	/// When it last signed in.
	pub last_used_at: Option<Timestamp>,
}

/// The body of `POST /auth/identities/unlink`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlinkIdentity {
	/// The connection to remove.
	pub id: Id<Identity>,
}

/// How a third-party sign-in ended, as the `auth` query parameter on the
/// page it returns to says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOutcome {
	/// Signed in: the session query is out of date.
	SignedIn,
	/// The account has two-step verification: finish signing in with one
	/// of these factors.
	SecondFactor(Vec<Factor>),
	/// The provider's account is now connected to the signed-in one.
	Linked,
	/// It was already connected to the signed-in one.
	AlreadyLinked,
	/// It belongs to another account, which can be merged into this one.
	Merge,
	/// An account with the provider's email address already exists. Sign in
	/// to it to connect the provider, or continue with a separate account.
	EmailInUse,
	/// It failed, with the [`ErrorResponse::error`] code.
	Failed(String),
}

impl ProviderOutcome {
	/// The outcome a page's query string, such as `?auth=signed_in`, holds.
	pub fn from_query(query: &str) -> Option<Self> {
		let mut outcome = None;
		let mut error = None;
		let mut factors = Vec::new();

		for pair in query.trim_start_matches('?').split('&') {
			match pair.split_once('=') {
				Some(("auth", value)) => outcome = Some(value),
				Some(("auth_error", value)) => error = Some(value),
				Some(("auth_factors", value)) => {
					factors = value
						.split(',')
						.filter_map(|factor| {
							match factor {
								"passkey" => Some(Factor::Passkey),
								"totp" => Some(Factor::Totp),
								"recovery_code" => Some(Factor::RecoveryCode),
								_ => None,
							}
						})
						.collect();
				}
				_ => {}
			}
		}

		Some(match outcome? {
			"signed_in" => Self::SignedIn,
			"second_factor" => Self::SecondFactor(factors),
			"linked" => Self::Linked,
			"already_linked" => Self::AlreadyLinked,
			"merge" => Self::Merge,
			"email_in_use" => Self::EmailInUse,
			"error" => {
				let code = error.filter(|code| {
					!code.is_empty()
						&& code
							.bytes()
							.all(|byte| byte.is_ascii_lowercase() || byte == b'_')
				});
				Self::Failed(code.unwrap_or("provider").to_owned())
			}
			_ => return None,
		})
	}

	/// The query parameters that carry it.
	pub fn query(&self) -> String {
		match self {
			Self::SignedIn => "auth=signed_in".into(),
			Self::SecondFactor(factors) => {
				let factors: Vec<&str> = factors
					.iter()
					.map(|factor| {
						match factor {
							Factor::Passkey => "passkey",
							Factor::Totp => "totp",
							Factor::RecoveryCode => "recovery_code",
						}
					})
					.collect();
				format!("auth=second_factor&auth_factors={}", factors.join(","))
			}
			Self::Linked => "auth=linked".into(),
			Self::AlreadyLinked => "auth=already_linked".into(),
			Self::Merge => "auth=merge".into(),
			Self::EmailInUse => "auth=email_in_use".into(),
			Self::Failed(code) => format!("auth=error&auth_error={code}"),
		}
	}
}

/// What merging would do, from `GET /auth/merge`: the other account, which
/// the person just proved they control, and what it still needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeOffer {
	/// The provider whose sign-in found the other account.
	pub provider: String,
	/// When the other account was created.
	pub created_at: Timestamp,
	/// Its verified email addresses, which move here.
	pub emails: Vec<String>,
	/// Its connected accounts' providers, which move here.
	pub identities: Vec<String>,
	/// How many passkeys move here.
	pub passkeys: u32,
	/// Its linked wallets' addresses, which move here.
	#[serde(default)]
	pub wallets: Vec<Pubkey>,
	/// Its data, table by table, for "bring its data": what moves here, and
	/// what's dropped because this account already has its own.
	#[serde(default)]
	pub data: Vec<MergeData>,
	/// The other account's second factors, one of which the merge needs,
	/// since merging mustn't get around them. Empty when it has none.
	pub second_factor: Vec<Factor>,
	/// What a merge may do with the other account's data.
	pub modes: Vec<MergeMode>,
	/// Seconds until the offer expires.
	pub expires_in: u64,
}

/// Some of the other account's data, as the merge preview counts it,
/// such as "12 posts".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeData {
	/// What it is, such as `posts`: the app's own label, or the table's
	/// name.
	pub label: String,
	/// How many there are.
	pub count: u64,
	/// What "bring its data" does with them.
	pub fate: MergeFate,
}

/// What "bring its data" does with some of the other account's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeFate {
	/// It moves here.
	Moves,
	/// It moves here, except what duplicates this account's own.
	MovesUnlessDuplicate,
	/// Whichever of the two accounts' is newer stays.
	NewestStays,
	/// It's dropped: this account's own stays.
	Dropped,
	/// The app decides.
	AppDecides,
}

/// What a merge does with the other account's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMode {
	/// Move its sign-in methods here, and delete its data.
	MoveLogin,
	/// Move its sign-in methods and its data here, as each table's
	/// `on_account_merge` says, then delete what's left. Only offered when
	/// every table says.
	BringData,
}

/// Which Solana cluster a wallet is asked to sign for, and whether the
/// server lets wallets sign in: `GET /auth/wallets/config`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSettings {
	/// Whether wallets can be linked here: the server has the `solana`
	/// feature and an origin to bind messages to.
	pub enabled: bool,
	/// Whether a wallet whose owner turned it on may sign in.
	pub sign_in: bool,
	/// The cluster messages name, such as `mainnet`.
	pub chain_id: String,
}

/// One of the signed-in person's linked wallets, as `GET /auth/wallets`
/// lists them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletInfo {
	/// The link.
	pub id: Id<Wallet>,
	/// The chain: `solana`.
	pub chain: String,
	/// The address.
	pub address: Pubkey,
	/// What the person calls it.
	pub label: Option<String>,
	/// Whether it may sign in to this account.
	pub can_sign_in: bool,
	/// Whether it shows on the person's public profile.
	pub is_public: bool,
	/// When it was linked.
	pub verified_at: Timestamp,
	/// When it last signed a proof that the person still controls it.
	pub last_proved_at: Timestamp,
}

/// A wallet someone chose to show, as their profile lists it: the address
/// and its label, nothing else.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWallet {
	/// The chain: `solana`.
	pub chain: String,
	/// The address.
	pub address: Pubkey,
	/// What its owner calls it.
	pub label: Option<String>,
}

/// What a wallet must sign: `POST /auth/wallets/link/options`,
/// `/auth/wallets/proof/options`, and `/auth/wallets/sign-in/options`.
#[derive(Clone, Serialize, Deserialize)]
pub struct WalletChallenge {
	/// The Sign-In With Solana fields, for the wallet's `solana:signIn`; or,
	/// with [`SiwsInput::message`], the text for `solana:signMessage`.
	pub input: SiwsInput,
	/// For signing in, what a native app sends back with the signature. A
	/// browser gets it in an `HttpOnly` cookie instead.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
}

impl std::fmt::Debug for WalletChallenge {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WalletChallenge")
			.field("input", &self.input)
			.finish_non_exhaustive()
	}
}

/// What a wallet signed: the address, the exact message, and its ed25519
/// signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletProof {
	/// The address that signed.
	pub address: Pubkey,
	/// The message, exactly as signed.
	pub message: String,
	/// The signature over the message's bytes.
	pub signature: Signature,
}

/// The body of `POST /auth/wallets/link`: a proof, and what to call the
/// wallet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkWallet {
	/// The signed challenge.
	pub proof: WalletProof,
	/// A name, such as "Treasury". At most 100 characters.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub label: Option<String>,
}

/// What linking a wallet did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome", content = "wallet")]
#[allow(
	variant_size_differences,
	reason = "one is made per link, and never kept in bulk"
)]
pub enum WalletLinked {
	/// It's linked to the signed-in account now.
	Linked(WalletInfo),
	/// It was already linked to the signed-in account; its proof is fresh
	/// now.
	AlreadyLinked(WalletInfo),
	/// It belongs to another account that it can sign in to, which can be
	/// merged into this one: `GET /auth/merge` describes it.
	Merge,
}

/// The body of `POST /auth/wallets/proof/options`: which linked wallet must
/// prove it's still the person's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProveWallet {
	/// The wallet's address.
	pub address: Pubkey,
}

/// The body of `POST /auth/wallets/sign-in/options`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSignInStart {
	/// How the device keeps its session, and so where the challenge's token
	/// goes: a cookie, or the body.
	#[serde(default)]
	pub transport: Transport,
}

/// The body of `POST /auth/wallets/sign-in`.
#[derive(Clone, Serialize, Deserialize)]
pub struct WalletSignIn {
	/// The signed challenge.
	pub proof: WalletProof,
	/// A native app's [`WalletChallenge::token`].
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
	/// How the device keeps its session.
	#[serde(default)]
	pub transport: Transport,
	/// What a native app calls its device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device_name: Option<String>,
}

impl std::fmt::Debug for WalletSignIn {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WalletSignIn")
			.field("address", &self.proof.address)
			.field("transport", &self.transport)
			.field("device_name", &self.device_name)
			.finish_non_exhaustive()
	}
}

/// The body of `POST /auth/wallets/update`: what to change about a linked
/// wallet. Fields left out stay as they are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateWallet {
	/// The wallet.
	pub id: Id<Wallet>,
	/// A new name; an empty one clears it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub label: Option<String>,
	/// Whether it may sign in. Turning it on needs a fresh sign-in or
	/// step-up.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub can_sign_in: Option<bool>,
	/// Whether it shows on the person's public profile.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub is_public: Option<bool>,
}

/// The body of `POST /auth/wallets/unlink`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlinkWallet {
	/// The wallet to unlink.
	pub id: Id<Wallet>,
}

/// The body of `POST /auth/merge`: the person's explicit go-ahead, and the
/// other account's second factor when it has one.
#[derive(Clone, Serialize, Deserialize)]
pub struct MergeAccounts {
	/// What to do with the other account's data.
	pub mode: MergeMode,
	/// Must be `true`: a merge can't be undone.
	pub confirm: bool,
	/// A code from the other account's authenticator app, or one of its
	/// recovery codes.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub code: Option<String>,
	/// Or a signature from one of its passkeys, for the options
	/// `POST /auth/merge/passkey/options` gave.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub credential: Option<PublicKeyCredential>,
}

impl std::fmt::Debug for MergeAccounts {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("MergeAccounts")
			.field("mode", &self.mode)
			.field("confirm", &self.confirm)
			.finish_non_exhaustive()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn provider_outcomes_round_trip_through_the_query() {
		for outcome in [
			ProviderOutcome::SignedIn,
			ProviderOutcome::SecondFactor(vec![
				Factor::Passkey,
				Factor::Totp,
				Factor::RecoveryCode,
			]),
			ProviderOutcome::Linked,
			ProviderOutcome::AlreadyLinked,
			ProviderOutcome::Merge,
			ProviderOutcome::EmailInUse,
			ProviderOutcome::Failed("sign_in_expired".into()),
		] {
			assert_eq!(
				ProviderOutcome::from_query(&format!("?tab=2&{}", outcome.query())),
				Some(outcome)
			);
		}

		// Only the outcome's own words come through.
		assert_eq!(ProviderOutcome::from_query("?tab=2"), None);
		assert_eq!(ProviderOutcome::from_query("auth=nonsense"), None);
		assert_eq!(
			ProviderOutcome::from_query("auth=error&auth_error=<script>"),
			Some(ProviderOutcome::Failed("provider".into()))
		);
		assert_eq!(
			ProviderOutcome::from_query("auth=second_factor&auth_factors=totp,sms"),
			Some(ProviderOutcome::SecondFactor(vec![Factor::Totp]))
		);
	}

	#[test]
	fn enrolled_means_an_authenticator_or_a_passkey() {
		assert!(!MfaStatus::default().enrolled());
		assert!(
			MfaStatus {
				passkeys: 1,
				..MfaStatus::default()
			}
			.enrolled()
		);
	}

	#[test]
	fn secrets_stay_out_of_debug() {
		let merge = MergeAccounts {
			mode: MergeMode::MoveLogin,
			confirm: true,
			code: Some("123456".into()),
			credential: None,
		};
		assert!(!format!("{merge:?}").contains("123456"));
		let step = PasskeyStep {
			token: Some("secret-token".into()),
			transport: Transport::Bearer,
		};
		assert!(!format!("{step:?}").contains("secret-token"));
		let options: RequestChallengeResponse = serde_json::from_value(serde_json::json!({
			"publicKey": { "challenge": "AAAA", "rpId": "notes.example", "allowCredentials": [], "userVerification": "required" }
		}))
		.unwrap();
		let options = PasskeyOptions {
			options,
			token: Some("secret-token".into()),
		};
		assert_eq!(format!("{options:?}"), "PasskeyOptions { .. }");
		let credential: PublicKeyCredential = serde_json::from_value(serde_json::json!({
			"id": "AA", "rawId": "AA", "type": "public-key", "extensions": {},
			"response": { "authenticatorData": "AA", "clientDataJSON": "AA", "signature": "AA" }
		}))
		.unwrap();
		let assertion = PasskeyAssertion {
			credential,
			token: Some("secret-token".into()),
			transport: Transport::Bearer,
			device_name: None,
		};
		assert!(!format!("{assertion:?}").contains("secret-token"));
		let added: RegisterPublicKeyCredential = serde_json::from_value(serde_json::json!({
			"id": "AA", "rawId": "AA", "type": "public-key", "extensions": {},
			"response": { "attestationObject": "AA", "clientDataJSON": "AA" }
		}))
		.unwrap();
		let add = AddPasskey {
			credential: added,
			name: Some("Laptop".into()),
		};
		assert_eq!(
			format!("{add:?}"),
			r#"AddPasskey { name: Some("Laptop"), .. }"#
		);
		let challenge = WalletChallenge {
			input: SiwsInput::default(),
			token: Some("secret-token".into()),
		};
		assert!(!format!("{challenge:?}").contains("secret-token"));
		let address = Pubkey::from_bytes([7; 32]);
		let sign_in = WalletSignIn {
			proof: WalletProof {
				address: address.clone(),
				message: "message".into(),
				signature: Signature::from_bytes([1; 64]),
			},
			token: Some("secret-token".into()),
			transport: Transport::Bearer,
			device_name: None,
		};
		let printed = format!("{sign_in:?}");
		assert!(!printed.contains("secret-token"), "{printed}");
		assert!(printed.contains(address.as_str()), "{printed}");
	}
}
