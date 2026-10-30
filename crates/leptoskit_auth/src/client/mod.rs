//! The headless client: the query and mutation scopes a sign-in screen is
//! built from. The look is yours; these are the behaviour, and they are the
//! same in the browser and in a native app.
//!
//! ```ignore
//! client::configure(ClientConfig::new().origin("https://myapp.com"));   // once, at startup
//!
//! let auth = use_auth();                         // the session, or `None` when signed out
//! let send = use_mutation(auth::client::send_code());
//! let verify = use_mutation(auth::client::verify_code());
//!
//! send.mutate(email);
//! verify.mutate(EmailCode { email, code });   // SignInStep::SignedIn, or SecondFactor
//! let finish = use_mutation(auth::client::complete_sign_in());
//! finish.mutate(code_from_the_authenticator);
//!
//! let devices = use_devices();                   // where they're signed in, this device marked current
//! let revoke = use_mutation(auth::client::revoke_device());
//! revoke.mutate(device.session.id);
//! ```
//!
//! Signing in or out updates the session query, so everything reading
//! [`use_auth`] re-renders.
//!
//! Two-step verification is the same everywhere too: [`use_mfa`] says what's
//! on; [`enroll_totp`], [`confirm_totp`], [`disable_totp`], and
//! [`regenerate_recovery_codes`] change it; and [`step_up`] answers the
//! `step_up_required` that sensitive actions return.
//!
//! Every target speaks the `/auth` routes. What differs is where the
//! session lives, and the client picks that itself:
//!
//! - **In the browser** (`web`), in the `HttpOnly` cookie, which scripts can't
//!   read. Requests are same-origin `fetch`es that carry it.
//! - **In a native app** (`guest`), as an access token held in memory and a
//!   refresh token in the platform's secure store. The client refreshes before
//!   the access token runs out, or after a `401`, one refresh at a time.
//! - **While the server renders** (`ssr`), the request's own session, read in
//!   process.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use leptoskit_db::Id;
use leptoskit_query::MutationScope;
use leptoskit_query::QueryHandle;
use leptoskit_query::QueryScope;
use leptoskit_query::use_query;
use serde::Deserialize;
use serde::Serialize;

use crate::Device;
use crate::ErrorResponse;
use crate::Factor;
use crate::MfaStatus;
use crate::PublicSession;
use crate::RecoveryCodes;
use crate::Session;
use crate::TotpSetup;

#[cfg_attr(
	all(feature = "guest", target_arch = "wasm32"),
	allow(dead_code, reason = "a native app keeps tokens, not the cookie")
)]
mod browser;
#[cfg(any(
	all(test, feature = "axum", feature = "sqlite"),
	all(feature = "guest", target_arch = "wasm32"),
))]
mod native;
mod protocol;
#[cfg(feature = "ssr")]
mod server;
#[cfg(all(test, feature = "axum", feature = "sqlite"))]
mod tests;

/// Where the client finds the server, and what a native app calls its
/// device. Pass it to [`configure`] once, when the app starts; the same call
/// works on every platform.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientConfig {
	/// The server's origin, such as `https://myapp.com`. A native app needs
	/// it. In the browser, leave it unset: the page's own origin is the one
	/// its session cookie belongs to.
	pub origin: Option<String>,
	/// What a native app calls its device in the list of signed-in devices,
	/// such as "Ada's iPhone". Browsers are named from their user agent.
	pub device_name: Option<String>,
}

impl ClientConfig {
	/// No origin and no device name: the page's own origin, in the browser.
	pub fn new() -> Self {
		Self::default()
	}

	/// The server's origin, such as `https://myapp.com`.
	#[must_use]
	pub fn origin(mut self, origin: impl Into<String>) -> Self {
		self.origin = Some(origin.into());
		self
	}

	/// What this device is called in the list of signed-in devices.
	#[must_use]
	pub fn device_name(mut self, name: impl Into<String>) -> Self {
		self.device_name = Some(name.into());
		self
	}
}

/// What a person types to sign in: their address, and the code emailed to
/// it.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailCode {
	/// The address the code was sent to.
	pub email: String,
	/// The code, as typed.
	pub code: String,
}

impl std::fmt::Debug for EmailCode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("EmailCode")
			.field("email", &self.email)
			.finish_non_exhaustive()
	}
}

/// Where signing in with an emailed code got to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[allow(
	variant_size_differences,
	reason = "one is made per sign-in, and never kept in bulk"
)]
pub enum SignInStep {
	/// Signed in.
	SignedIn(PublicSession),
	/// The account has two-step verification: finish with
	/// [`complete_sign_in`] and a code from the authenticator app or a
	/// recovery code. The client keeps what the server needs to match the
	/// two steps, so the code is all it takes.
	SecondFactor(SecondFactorPrompt),
}

/// What the second step of signing in accepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondFactorPrompt {
	/// The factors the account can use, best first.
	pub factors: Vec<Factor>,
	/// Seconds until the step expires and signing in starts again.
	pub expires_in: u64,
}

/// An authenticator app that was just turned on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TotpConfirmation {
	/// The session, now at `aal2`.
	pub session: PublicSession,
	/// New recovery codes, to show once, when the account had none.
	pub recovery_codes: Option<RecoveryCodes>,
}

/// What every target's client can do. The scopes call it.
pub(crate) trait Api: Send + Sync + 'static {
	fn session(&self) -> Pending<'_, Option<PublicSession>>;
	fn send_code(&self, email: String) -> Pending<'_, ()>;
	fn verify_code(&self, input: EmailCode) -> Pending<'_, SignInStep>;
	fn complete_sign_in(&self, code: String) -> Pending<'_, PublicSession>;
	fn mfa(&self) -> Pending<'_, MfaStatus>;
	fn step_up(&self, code: String) -> Pending<'_, PublicSession>;
	fn begin_totp(&self) -> Pending<'_, TotpSetup>;
	fn confirm_totp(&self, code: String) -> Pending<'_, TotpConfirmation>;
	fn disable_totp(&self) -> Pending<'_, ()>;
	fn regenerate_recovery_codes(&self) -> Pending<'_, RecoveryCodes>;
	fn sign_out(&self) -> Pending<'_, ()>;
	fn devices(&self) -> Pending<'_, Vec<Device>>;
	fn revoke(&self, id: Id<Session>) -> Pending<'_, bool>;
	fn revoke_others(&self) -> Pending<'_, ()>;
	fn sign_out_everywhere(&self) -> Pending<'_, ()>;
}

/// A call to the server, in flight.
pub(crate) type Pending<'a, T> = leptoskit_query::BoxFuture<'a, Result<T, ErrorResponse>>;

static CLIENT: RwLock<Option<Arc<dyn Api>>> = RwLock::new(None);

/// Tells the client where the server is, and what this device is called.
/// Call it once, when the app starts, before anything signs in; calling it
/// again starts a fresh client. Without it, the client uses the page's own
/// origin, which works in the browser; a native app answers `config` until
/// it has an origin.
pub fn configure(config: ClientConfig) {
	install(backend(config));
}

pub(crate) fn install(api: Arc<dyn Api>) {
	*CLIENT.write().unwrap_or_else(PoisonError::into_inner) = Some(api);
}

fn api() -> Arc<dyn Api> {
	if let Some(api) = CLIENT
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.as_ref()
	{
		return Arc::clone(api);
	}

	let mut client = CLIENT.write().unwrap_or_else(PoisonError::into_inner);
	Arc::clone(client.get_or_insert_with(|| backend(ClientConfig::default())))
}

/// While the server renders: the request's session, in process.
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
fn backend(_config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(server::Server)
}

/// In a native app: the HTTP plugin, and the secure store.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
fn backend(config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(protocol::Remote::new(
		config,
		native::Plugin,
		native::TokenKeeper::new(native::Keychain, leptoskit_query::now_ms),
	))
}

/// In the browser: `fetch`, and the cookie.
#[cfg(all(feature = "web", not(feature = "guest"), target_arch = "wasm32"))]
fn backend(config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(protocol::Remote::new(
		config,
		browser::Fetch,
		browser::Cookie,
	))
}

/// Anywhere else there's no way to reach the server, and each call says so.
#[cfg(not(any(
	all(feature = "ssr", not(target_arch = "wasm32")),
	all(any(feature = "guest", feature = "web"), target_arch = "wasm32"),
)))]
fn backend(config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(protocol::Remote::new(
		config,
		protocol::Unreachable,
		browser::Cookie,
	))
}

/// The session query: the request's session, cached and shared by
/// everything that reads it.
pub fn session() -> QueryScope<(), Option<PublicSession>, ErrorResponse> {
	QueryScope::new(|()| async { api().session().await })
}

/// The current session, as a query handle: `data()` is `Some(Some(session))`
/// when signed in, and `Some(None)` when signed out.
#[track_caller]
pub fn use_auth() -> QueryHandle<Option<PublicSession>, ErrorResponse> {
	use_query(session(), || ())
}

/// Emails a sign-in code to the address it's given.
///
/// Fails with `invalid_email`, `rate_limited`, or `mail`, as
/// [`ErrorResponse`] says.
pub fn send_code() -> MutationScope<String, (), ErrorResponse> {
	MutationScope::new(|email: String| async move { api().send_code(email).await })
}

/// Signs in with an emailed code. When the account has no second factor,
/// that's it: the session query holds the new session. Otherwise it answers
/// [`SignInStep::SecondFactor`], and [`complete_sign_in`] finishes.
///
/// Fails with `invalid_code`, `rate_limited`, `disabled`, `locked`, or
/// `sign_up`, as [`ErrorResponse`] says.
pub fn verify_code() -> MutationScope<EmailCode, SignInStep, ErrorResponse> {
	MutationScope::new(|input: EmailCode| async move { api().verify_code(input).await })
		.with_on_success(|args| {
			if let SignInStep::SignedIn(signed_in) = args.data {
				args.client.set_query_data(&session(), (), Some(signed_in));
			}
		})
}

/// Finishes signing in with a code from the authenticator app, or a
/// recovery code, after [`verify_code`] answered
/// [`SignInStep::SecondFactor`]. On success, the session query holds the
/// new session.
///
/// Fails with `invalid_code` (try again), `sign_in_expired` (start again
/// from the email), or `rate_limited`.
pub fn complete_sign_in() -> MutationScope<String, PublicSession, ErrorResponse> {
	MutationScope::new(|code: String| async move { api().complete_sign_in(code).await })
		.with_on_success(|args| {
			args.client.set_query_data(&session(), (), Some(args.data));
		})
}

/// The two-step verification query: whether an authenticator app is on,
/// and how many recovery codes are left.
pub fn mfa() -> QueryScope<(), MfaStatus, ErrorResponse> {
	QueryScope::new(|()| async { api().mfa().await })
}

/// The signed-in person's two-step verification, as a query handle.
#[track_caller]
pub fn use_mfa() -> QueryHandle<MfaStatus, ErrorResponse> {
	use_query(mfa(), || ())
}

/// Confirms it's still the signed-in person, with a code from the
/// authenticator app or a recovery code, when an action answered
/// `step_up_required`. Then try the action again. On success, the session
/// query holds the session, now with a fresh second factor.
///
/// Fails with `invalid_code` or `rate_limited`.
pub fn step_up() -> MutationScope<String, PublicSession, ErrorResponse> {
	MutationScope::new(|code: String| async move { api().step_up(code).await }).with_on_success(
		|args| {
			args.client.set_query_data(&session(), (), Some(args.data));
		},
	)
}

/// Starts turning on an authenticator app: the secret to show as a QR code
/// or type in. [`confirm_totp`] turns it on.
///
/// Fails with `reauthenticate` when the person signed in too long ago (sign
/// in again), `step_up_required` when another factor is already on, or
/// `already_enrolled`.
pub fn enroll_totp() -> MutationScope<(), TotpSetup, ErrorResponse> {
	MutationScope::new(|()| async { api().begin_totp().await })
}

/// Turns the authenticator app on with a code from it. On success, the
/// session query holds the elevated session, the two-step query refetches,
/// and an account that had no recovery codes gets them, to show once.
///
/// Fails with `invalid_code`, `not_enrolled`, or as [`enroll_totp`].
pub fn confirm_totp() -> MutationScope<String, TotpConfirmation, ErrorResponse> {
	MutationScope::new(|code: String| async move { api().confirm_totp(code).await })
		.with_on_success_async(|args| {
			async move {
				args.client
					.set_query_data(&session(), (), Some(args.data.session.clone()));
				args.client.invalidate_scope(&mfa()).await;
			}
		})
}

/// Turns the authenticator app off and deletes the recovery codes. Every
/// other device is signed out. On success, the two-step and devices queries
/// refetch.
///
/// Fails with `step_up_required` or `not_enrolled`.
pub fn disable_totp() -> MutationScope<(), (), ErrorResponse> {
	MutationScope::new(|()| async { api().disable_totp().await }).with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&mfa()).await;
			args.client.invalidate_scope(&devices()).await;
		}
	})
}

/// Replaces the recovery codes with ten new ones, to show once; the old
/// ones stop working. On success, the two-step query refetches.
///
/// Fails with `step_up_required` or `not_enrolled`.
pub fn regenerate_recovery_codes() -> MutationScope<(), RecoveryCodes, ErrorResponse> {
	MutationScope::new(|()| async { api().regenerate_recovery_codes().await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&mfa()).await;
			}
		})
}

/// Signs out. On success, the session query holds `None`. A native app
/// forgets its tokens even when the server can't be reached.
pub fn sign_out_mutation() -> MutationScope<(), (), ErrorResponse> {
	MutationScope::new(|()| async { api().sign_out().await }).with_on_success(|args| {
		args.client.set_query_data(&session(), (), None);
	})
}

/// The devices query, for a "where you're signed in" screen: one live
/// session each, most recently used first, with this one marked current.
///
/// Fails with `signed_out` when no one is signed in.
pub fn devices() -> QueryScope<(), Vec<Device>, ErrorResponse> {
	QueryScope::new(|()| async { api().devices().await })
}

/// The signed-in person's devices, as a query handle.
#[track_caller]
pub fn use_devices() -> QueryHandle<Vec<Device>, ErrorResponse> {
	use_query(devices(), || ())
}

/// Ends the session it's given, such as a lost phone's, and says whether it
/// was theirs and still live. On success, the devices query refetches.
pub fn revoke_device() -> MutationScope<Id<Session>, bool, ErrorResponse> {
	MutationScope::new(|id: Id<Session>| async move { api().revoke(id).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&devices()).await;
			}
		})
}

/// Ends every session but this one. On success, the devices query
/// refetches.
pub fn revoke_other_devices() -> MutationScope<(), (), ErrorResponse> {
	MutationScope::new(|()| async { api().revoke_others().await }).with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&devices()).await;
		}
	})
}

/// Ends every session, this one included. On success, the session query
/// holds `None` and the devices query is empty.
pub fn sign_out_everywhere_mutation() -> MutationScope<(), (), ErrorResponse> {
	MutationScope::new(|()| async { api().sign_out_everywhere().await }).with_on_success(|args| {
		args.client.set_query_data(&session(), (), None);
		args.client.set_query_data(&devices(), (), Vec::new());
	})
}
