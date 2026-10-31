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
//! Solana wallets are the same everywhere too: [`use_wallet_apps`] lists
//! the wallets this device offers, [`link_wallet`] and [`wallet_sign_in`]
//! ask one to sign, and [`use_wallets`], [`update_wallet`], and
//! [`unlink_wallet`] manage the linked ones. A browser reaches wallets
//! through the Wallet Standard; a native app answers `unsupported` for the
//! calls that need a wallet to sign until a native wallet adapter exists,
//! and manages linked wallets like any other device.
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
pub use protocol::HttpResponse;
pub use protocol::Method;
use serde::Deserialize;
use serde::Serialize;

pub use self::qr::QrCode;
pub use self::qr::qr_code;
use crate::Device;
use crate::ErrorResponse;
use crate::Factor;
use crate::Identity;
use crate::IdentityInfo;
use crate::MergeMode;
use crate::MergeOffer;
use crate::MfaStatus;
use crate::Passkey;
use crate::PasskeyInfo;
use crate::ProviderInfo;
use crate::ProviderOutcome;
use crate::PublicSession;
use crate::RecoveryCodes;
use crate::RenamePasskey;
use crate::Session;
use crate::StartProvider;
use crate::TotpSetup;
use crate::UpdateWallet;
use crate::Wallet;
use crate::WalletInfo;
use crate::WalletLinked;
use crate::WalletSettings;

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
mod qr;
#[cfg(feature = "ssr")]
mod server;
#[cfg(all(test, feature = "axum", feature = "sqlite"))]
mod tests;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod wallet_standard;

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

/// How [`passkey_sign_in`] asks for a passkey.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PasskeyMode {
	/// In the browser's own dialog, when the person picks "Sign in with a
	/// passkey".
	#[default]
	Prompt,
	/// In the email field's autofill, which offers saved passkeys as soon
	/// as the sign-in page opens. Give the field
	/// `autocomplete="username webauthn"`. It waits until the person picks
	/// one, or [`cancel_passkey_sign_in`] stops it; in a browser without
	/// passkey autofill it fails at once with `unsupported`.
	Autofill,
}

/// A passkey that was just added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PasskeyAdded {
	/// The passkey.
	pub passkey: PasskeyInfo,
	/// The session, now at `aal2`.
	pub session: PublicSession,
	/// New recovery codes, to show once, when the account had none.
	pub recovery_codes: Option<RecoveryCodes>,
}

/// What [`merge_accounts`] needs: the person's go-ahead, and when the other
/// account has two-step verification, its second factor.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeInput {
	/// What happens to the other account's data.
	pub mode: Option<MergeMode>,
	/// Must be `true`: a merge can't be undone.
	pub confirm: bool,
	/// A code from the other account's authenticator app, or one of its
	/// recovery codes.
	pub code: Option<String>,
	/// Prove the other account with one of its passkeys instead.
	pub passkey: bool,
}

impl std::fmt::Debug for MergeInput {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("MergeInput")
			.field("mode", &self.mode)
			.field("confirm", &self.confirm)
			.field("passkey", &self.passkey)
			.finish_non_exhaustive()
	}
}

/// A wallet app this device offers, such as Phantom or Solflare: in the
/// browser, each one that registered through the Wallet Standard and
/// supports Solana.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WalletApp {
	/// Its name, which [`link_wallet`] and [`wallet_sign_in`] take.
	pub name: String,
	/// Its icon, as a `data:` URI.
	pub icon: String,
	/// Whether it signs Sign-In With Solana messages itself
	/// (`solana:signIn`), showing the person the site and the statement.
	/// Otherwise it signs the message's text (`solana:signMessage`).
	pub sign_in: bool,
}

/// What [`link_wallet`] needs: which wallet app, and what to call the
/// wallet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkWalletInput {
	/// The wallet app's [`WalletApp::name`].
	pub wallet: String,
	/// A name for the wallet, such as "Treasury".
	pub label: Option<String>,
}

/// What [`prove_wallet`] needs: which wallet app, and which linked
/// address must sign.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProveWalletInput {
	/// The wallet app's [`WalletApp::name`].
	pub wallet: String,
	/// The linked wallet's address.
	pub address: leptoskit_db::Pubkey,
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
	/// The `Authorization` header this device's next request carries.
	#[cfg(feature = "rpc")]
	fn authorization(&self) -> Pending<'_, Option<String>>;
	/// Renews after the server refused `sent`; whether to send again.
	#[cfg(feature = "rpc")]
	fn refused(&self, sent: Option<String>) -> Pending<'_, bool>;
}

/// A call to the server, in flight.
pub(crate) type Pending<'a, T> = leptoskit_query::BoxFuture<'a, Result<T, ErrorResponse>>;

static CLIENT: RwLock<Option<Arc<dyn Api>>> = RwLock::new(None);

/// Tells the client where the server is, and what this device is called.
/// Call it once, when the app starts, before anything signs in; calling it
/// again starts a fresh client. Without it, the client uses the page's own
/// origin, which works in the browser; a native app answers `config` until
/// it has an origin.
///
/// With the `rpc` feature it also configures `leptoskit_rpc`'s client: the
/// same origin, and calls signed the way sign-in signs its own, so queries
/// and mutations run as the signed-in person on every platform.
pub fn configure(config: ClientConfig) {
	#[cfg(feature = "rpc")]
	let origin = config.origin.clone();
	install(backend(config));
	#[cfg(feature = "rpc")]
	configure_rpc(origin);
}

/// Points `leptoskit_rpc`'s client at the same server, signed like this one.
#[cfg(feature = "rpc")]
fn configure_rpc(origin: Option<String>) {
	let mut config = leptoskit_rpc::client::ClientConfig::new().credentials(RpcCredentials);
	config.origin = origin;
	leptoskit_rpc::client::configure(config);
}

/// Signs `leptoskit_rpc`'s calls with this client's credentials: nothing in
/// the browser, where the cookie goes along, and the access token in a
/// native app, renewed as sign-in renews it.
#[cfg(feature = "rpc")]
struct RpcCredentials;

#[cfg(feature = "rpc")]
impl leptoskit_rpc::client::Credentials for RpcCredentials {
	fn authorization(
		&self,
	) -> leptoskit_query::BoxFuture<'_, Result<Option<String>, leptoskit_rpc::FnError>> {
		Box::pin(async { api().authorization().await.map_err(rpc_error) })
	}

	fn refused(
		&self,
		sent: Option<String>,
	) -> leptoskit_query::BoxFuture<'_, Result<bool, leptoskit_rpc::FnError>> {
		Box::pin(async move { api().refused(sent).await.map_err(rpc_error) })
	}
}

#[cfg(feature = "rpc")]
fn rpc_error(error: ErrorResponse) -> leptoskit_rpc::FnError {
	if error.error == "network" {
		leptoskit_rpc::FnError::Network(error.message)
	} else {
		leptoskit_rpc::FnError::Other(error.message)
	}
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
	Arc::new(
		protocol::Remote::new(
			config,
			native::Plugin,
			native::TokenKeeper::new(native::Keychain, leptoskit_query::now_ms),
		)
		.platform(native::System::default()),
	)
}

/// In the browser: `fetch`, and the cookie.
#[cfg(all(feature = "web", not(feature = "guest"), target_arch = "wasm32"))]
fn backend(config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(
		protocol::Remote::new(config, browser::Fetch, browser::Cookie).platform(browser::Browser),
	)
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

/// Signs in with a passkey: in the browser's dialog or the device's own
/// passkey sheet, or from the email field's autofill. A passkey that
/// verifies its user is two factors in one, so there's no second step. On
/// success, the session query holds the new session.
///
/// Fails with `passkey_failed`, `cancelled` when the person closes the
/// prompt or [`cancel_passkey_sign_in`] stops it, `disabled`, `locked`,
/// `rate_limited`, or `unsupported` where the device has no passkeys, and
/// for [`PasskeyMode::Autofill`] in native apps, whose text fields offer no
/// passkey suggestions.
pub fn passkey_sign_in() -> MutationScope<PasskeyMode, PublicSession, ErrorResponse> {
	MutationScope::new(|mode: PasskeyMode| async move { api().passkey_sign_in(mode).await })
		.with_on_success(|args| {
			args.client.set_query_data(&session(), (), Some(args.data));
		})
}

/// Stops a passkey request that's waiting, such as the autofill one when
/// the person signs in another way.
pub fn cancel_passkey_sign_in() {
	api().cancel_passkey();
}

/// Adds a passkey to the signed-in person's account, named as given or
/// after the device. On success, the session query holds the elevated
/// session, the passkeys and two-step queries refetch, and an account that
/// had no recovery codes gets them, to show once.
///
/// Fails with `reauthenticate`, `step_up_required`, `passkey_failed`,
/// `cancelled`, or `unsupported` where the device has no passkeys.
pub fn add_passkey() -> MutationScope<Option<String>, PasskeyAdded, ErrorResponse> {
	MutationScope::new(|name: Option<String>| async move { api().add_passkey(name).await })
		.with_on_success_async(|args| {
			async move {
				args.client
					.set_query_data(&session(), (), Some(args.data.session.clone()));
				args.client.invalidate_scope(&passkeys()).await;
				args.client.invalidate_scope(&mfa()).await;
			}
		})
}

/// Finishes signing in with one of the account's passkeys, after
/// [`verify_code`] answered [`SignInStep::SecondFactor`] with
/// [`Factor::Passkey`] among its factors.
///
/// Fails with `passkey_failed` (try again), `sign_in_expired`,
/// `cancelled`, or `unsupported`.
pub fn complete_sign_in_with_passkey() -> MutationScope<(), PublicSession, ErrorResponse> {
	MutationScope::new(|()| async { api().complete_sign_in_with_passkey().await }).with_on_success(
		|args| {
			args.client.set_query_data(&session(), (), Some(args.data));
		},
	)
}

/// Confirms it's still the signed-in person with one of their passkeys,
/// when an action answered `step_up_required`. Then try the action again.
///
/// Fails with `passkey_failed`, `cancelled`, `not_enrolled`, or
/// `unsupported`.
pub fn step_up_with_passkey() -> MutationScope<(), PublicSession, ErrorResponse> {
	MutationScope::new(|()| async { api().step_up_with_passkey().await }).with_on_success(|args| {
		args.client.set_query_data(&session(), (), Some(args.data));
	})
}

/// The passkeys query: the signed-in person's passkeys, newest first.
pub fn passkeys() -> QueryScope<(), Vec<PasskeyInfo>, ErrorResponse> {
	QueryScope::new(|()| async { api().passkeys().await })
}

/// The signed-in person's passkeys, as a query handle.
#[track_caller]
pub fn use_passkeys() -> QueryHandle<Vec<PasskeyInfo>, ErrorResponse> {
	use_query(passkeys(), || ())
}

/// Renames a passkey. On success, the passkeys query refetches.
///
/// Fails with `invalid_name` or `not_found`.
pub fn rename_passkey() -> MutationScope<RenamePasskey, (), ErrorResponse> {
	MutationScope::new(|input: RenamePasskey| async move { api().rename_passkey(input).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&passkeys()).await;
			}
		})
}

/// Removes a passkey, after step-up. Every other device is signed out. On
/// success, the passkeys, two-step, and devices queries refetch.
///
/// Fails with `step_up_required`, `not_found`, or `last_sign_in_method`.
pub fn remove_passkey() -> MutationScope<Id<Passkey>, (), ErrorResponse> {
	MutationScope::new(|id: Id<Passkey>| async move { api().remove_passkey(id).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&passkeys()).await;
				args.client.invalidate_scope(&mfa()).await;
				args.client.invalidate_scope(&devices()).await;
			}
		})
}

/// The providers query: the third-party sign-ins the server offers.
pub fn providers() -> QueryScope<(), Vec<ProviderInfo>, ErrorResponse> {
	QueryScope::new(|()| async { api().providers().await })
}

/// The server's providers, as a query handle, for a row of "Continue with
/// …" buttons.
#[track_caller]
pub fn use_providers() -> QueryHandle<Vec<ProviderInfo>, ErrorResponse> {
	use_query(providers(), || ())
}

/// Signs in with a provider. In the browser, the page goes to its sign-in
/// page, and comes back to `redirect_to` with the outcome in its query. A
/// native app goes there in the system browser (never a web view), or uses
/// Sign in with Apple on the device for Apple, and resolves once it's back.
/// Either way, [`provider_outcome`] then says how it ended, and the session
/// query refetches.
///
/// Fails with `unknown_provider`, `invalid_redirect`, `rate_limited`, or
/// in a native app `config` without its `web-auth` plugin.
pub fn sign_in_with_provider() -> MutationScope<StartProvider, (), ErrorResponse> {
	MutationScope::new(
		|input: StartProvider| async move { api().start_provider(input, false).await },
	)
	.with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&session()).await;
		}
	})
}

/// Connects a provider to the signed-in person's account, as
/// [`sign_in_with_provider`] does. Coming back, the outcome is `linked`,
/// `already_linked`, or `merge` when the provider's account is another
/// account here: then [`merge_offer`] describes it.
///
/// Also fails with `reauthenticate` or `step_up_required`.
pub fn link_provider() -> MutationScope<StartProvider, (), ErrorResponse> {
	MutationScope::new(
		|input: StartProvider| async move { api().start_provider(input, true).await },
	)
	.with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&identities()).await;
			args.client.invalidate_scope(&merge_offer()).await;
		}
	})
}

/// After the outcome `email_in_use`: creates a separate account with the
/// provider, without the address another account already has. On success,
/// the session query holds the new session.
///
/// Fails with `sign_in_expired`.
pub fn continue_provider_sign_up() -> MutationScope<(), PublicSession, ErrorResponse> {
	MutationScope::new(|()| async { api().continue_provider_sign_up().await }).with_on_success(
		|args| {
			args.client.set_query_data(&session(), (), Some(args.data));
		},
	)
}

/// How the last provider sign-in ended: from the page's query in the
/// browser, which came back from the provider, or from the system browser
/// in a native app, once [`sign_in_with_provider`] or [`link_provider`]
/// resolves. `None` when there wasn't one.
pub fn provider_outcome() -> Option<ProviderOutcome> {
	api().provider_outcome()
}

/// The connected accounts query: the providers the signed-in person can
/// sign in with.
pub fn identities() -> QueryScope<(), Vec<IdentityInfo>, ErrorResponse> {
	QueryScope::new(|()| async { api().identities().await })
}

/// The signed-in person's connected accounts, as a query handle.
#[track_caller]
pub fn use_identities() -> QueryHandle<Vec<IdentityInfo>, ErrorResponse> {
	use_query(identities(), || ())
}

/// Disconnects a provider, after a fresh sign-in or step-up. On success,
/// the connected accounts query refetches.
///
/// Fails with `reauthenticate`, `step_up_required`, `not_found`, or
/// `last_sign_in_method`.
pub fn unlink_identity() -> MutationScope<Id<Identity>, (), ErrorResponse> {
	MutationScope::new(|id: Id<Identity>| async move { api().unlink_identity(id).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&identities()).await;
			}
		})
}

/// The merge offer query: what merging would do, after connecting a
/// provider found another account, or `None`.
pub fn merge_offer() -> QueryScope<(), Option<MergeOffer>, ErrorResponse> {
	QueryScope::new(|()| async { api().merge_offer().await })
}

/// The merge offer, as a query handle.
#[track_caller]
pub fn use_merge_offer() -> QueryHandle<Option<MergeOffer>, ErrorResponse> {
	use_query(merge_offer(), || ())
}

/// Merges the other account into this one, as the offer says. It can't be
/// undone: `confirm` must be `true`. On success, the merge offer, connected
/// accounts, passkeys, and two-step queries refetch.
///
/// Fails with `reauthenticate` or `step_up_required` (for this account, or
/// when the other account's second factor is missing), `invalid_code` or
/// `passkey_failed` (for the other account's), `sign_in_expired` when the
/// offer is gone, or `unsupported` without `confirm`.
pub fn merge_accounts() -> MutationScope<MergeInput, (), ErrorResponse> {
	MutationScope::new(|input: MergeInput| async move { api().merge_accounts(input).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&merge_offer()).await;
				args.client.invalidate_scope(&identities()).await;
				args.client.invalidate_scope(&passkeys()).await;
				args.client.invalidate_scope(&mfa()).await;
			}
		})
}

/// Drops the merge offer. On success, the merge offer query refetches.
pub fn cancel_merge() -> MutationScope<(), (), ErrorResponse> {
	MutationScope::new(|()| async { api().cancel_merge().await }).with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&merge_offer()).await;
		}
	})
}

/// The wallet settings query: whether wallets work on this server, whether
/// they may sign in, and the Solana cluster.
pub fn wallet_settings() -> QueryScope<(), WalletSettings, ErrorResponse> {
	QueryScope::new(|()| async { api().wallet_settings().await })
}

/// The server's wallet settings, as a query handle.
#[track_caller]
pub fn use_wallet_settings() -> QueryHandle<WalletSettings, ErrorResponse> {
	use_query(wallet_settings(), || ())
}

/// The wallet apps query: the Solana wallets this device offers.
///
/// Fails with `unsupported` in a native app, until a native wallet adapter
/// exists.
pub fn wallet_apps() -> QueryScope<(), Vec<WalletApp>, ErrorResponse> {
	QueryScope::new(|()| async { api().wallet_apps().await })
}

/// The wallet apps this device offers, as a query handle, for a row of
/// "Phantom", "Solflare", … buttons.
#[track_caller]
pub fn use_wallet_apps() -> QueryHandle<Vec<WalletApp>, ErrorResponse> {
	use_query(wallet_apps(), || ())
}

/// The linked wallets query: the signed-in person's wallets, newest first.
pub fn wallets() -> QueryScope<(), Vec<WalletInfo>, ErrorResponse> {
	QueryScope::new(|()| async { api().wallets().await })
}

/// The signed-in person's linked wallets, as a query handle.
#[track_caller]
pub fn use_wallets() -> QueryHandle<Vec<WalletInfo>, ErrorResponse> {
	use_query(wallets(), || ())
}

/// Links a wallet to the signed-in person's account: the wallet app asks
/// them to sign a message bound to this site, and the server checks it. It
/// can't sign in until they turn that on with [`update_wallet`]. On
/// success, the linked wallets query refetches, and after
/// [`WalletLinked::Merge`] the merge offer query too.
///
/// Fails with `reauthenticate` or `step_up_required` (sign in again, or step
/// up, first), `cancelled` when the person declines in the wallet,
/// `no_wallet` when the device has no wallet app by that name,
/// `wallet_failed`, `wallet_in_use` when another account has it and it
/// can't sign in there, `rate_limited`, or `unsupported` in a native app.
pub fn link_wallet() -> MutationScope<LinkWalletInput, WalletLinked, ErrorResponse> {
	MutationScope::new(|input: LinkWalletInput| async move { api().link_wallet(input).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&wallets()).await;

				if args.data == WalletLinked::Merge {
					args.client.invalidate_scope(&merge_offer()).await;
				}
			}
		})
}

/// Asks a linked wallet to sign again, so actions that need a fresh proof,
/// such as a payout, accept it for a while. Answer `proof_required` with
/// this. On success, the linked wallets query refetches.
///
/// Fails with `cancelled`, `no_wallet`, `wallet_failed` when the wallet app
/// signed with another address, `not_found`, or `unsupported` in a native
/// app.
pub fn prove_wallet() -> MutationScope<ProveWalletInput, WalletInfo, ErrorResponse> {
	MutationScope::new(|input: ProveWalletInput| async move { api().prove_wallet(input).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&wallets()).await;
			}
		})
}

/// Renames a linked wallet, shows or hides it on the person's profile, or
/// turns signing in with it on or off. On success, the linked wallets query
/// refetches.
///
/// Fails with `not_found`, `reauthenticate` or `step_up_required` when
/// changing whether it signs in, `last_sign_in_method`, or `unsupported`
/// when the server doesn't let wallets sign in.
pub fn update_wallet() -> MutationScope<UpdateWallet, WalletInfo, ErrorResponse> {
	MutationScope::new(|input: UpdateWallet| async move { api().update_wallet(input).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&wallets()).await;
			}
		})
}

/// Unlinks a wallet, after a fresh sign-in or step-up. On success, the
/// linked wallets query refetches.
///
/// Fails with `reauthenticate`, `step_up_required`, `not_found`, or
/// `last_sign_in_method`.
pub fn unlink_wallet() -> MutationScope<Id<Wallet>, (), ErrorResponse> {
	MutationScope::new(|id: Id<Wallet>| async move { api().unlink_wallet(id).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&wallets()).await;
			}
		})
}

/// Signs in with the wallet app it's given, when its owner let that wallet
/// sign in. A wallet is one factor: an account with two-step verification
/// answers [`SignInStep::SecondFactor`], and [`complete_sign_in`] finishes.
/// On success, the session query holds the new session.
///
/// Fails with `cancelled`, `no_wallet`, `wallet_failed`, `unknown_wallet`
/// when no account lets this wallet sign in, `disabled`, `locked`,
/// `rate_limited`, or `unsupported` when the server doesn't let wallets
/// sign in, or in a native app.
pub fn wallet_sign_in() -> MutationScope<String, SignInStep, ErrorResponse> {
	MutationScope::new(|wallet: String| async move { api().wallet_sign_in(wallet).await })
		.with_on_success(|args| {
			if let SignInStep::SignedIn(signed_in) = args.data {
				args.client.set_query_data(&session(), (), Some(signed_in));
			}
		})
}

/// Sends a request to a route of the app's server next to `/auth`, the way
/// this client sends its own: with this device's session, renewed and sent
/// again once if the server answers `401`. For crates that add routes next
/// to sign-in's, such as `leptoskit_idp`'s connected apps, so their hooks
/// work on every platform with no code of their own for each.
///
/// `path` is the route's path, such as `/oauth/grants`, and `body` a JSON
/// body for a `POST`. The answer comes back as it was sent, whatever its
/// status; an error means the request never got an answer: `network`,
/// `config`, or `storage`. While the server renders a page, it answers
/// `server`: read in process there instead.
pub fn send(
	method: Method,
	path: impl Into<String>,
	body: Option<String>,
) -> leptoskit_query::BoxFuture<'static, Result<HttpResponse, ErrorResponse>> {
	let path = path.into();
	let api = api();
	Box::pin(async move { api.send(method, path, body).await })
}
