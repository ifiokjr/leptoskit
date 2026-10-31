//! The client while the server renders a page: it reads the session the
//! layer resolved for the request leptos is serving, with no HTTP hop.
//! Signing in and out happen in the browser, so only the reads work here.

#![cfg_attr(
	target_arch = "wasm32",
	allow(dead_code, reason = "server rendering never runs in Wasm")
)]

use leptoskit_db::Id;

use super::Api;
use super::EmailCode;
use super::HttpResponse;
use super::LinkWalletInput;
use super::MergeInput;
use super::Method;
use super::PasskeyAdded;
use super::PasskeyMode;
use super::Pending;
use super::ProveWalletInput;
use super::SignInStep;
use super::TotpConfirmation;
use super::WalletApp;
use super::protocol::error;
use crate::Device;
use crate::Identity;
use crate::IdentityInfo;
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

/// The request's own session, in process.
pub(crate) struct Server;

fn in_the_browser<T: Send + 'static>() -> Pending<'static, T> {
	Box::pin(std::future::ready(Err(error(
		"server",
		"signing in and out runs in the browser or the app, not while the server renders",
	))))
}

impl Api for Server {
	fn session(&self) -> Pending<'_, Option<PublicSession>> {
		Box::pin(reads::session())
	}

	fn send_code(&self, _email: String) -> Pending<'_, ()> {
		in_the_browser()
	}

	fn verify_code(&self, _input: EmailCode) -> Pending<'_, SignInStep> {
		in_the_browser()
	}

	fn complete_sign_in(&self, _code: String) -> Pending<'_, PublicSession> {
		in_the_browser()
	}

	fn mfa(&self) -> Pending<'_, MfaStatus> {
		Box::pin(reads::mfa())
	}

	fn step_up(&self, _code: String) -> Pending<'_, PublicSession> {
		in_the_browser()
	}

	fn begin_totp(&self) -> Pending<'_, TotpSetup> {
		in_the_browser()
	}

	fn confirm_totp(&self, _code: String) -> Pending<'_, TotpConfirmation> {
		in_the_browser()
	}

	fn disable_totp(&self) -> Pending<'_, ()> {
		in_the_browser()
	}

	fn regenerate_recovery_codes(&self) -> Pending<'_, RecoveryCodes> {
		in_the_browser()
	}

	fn sign_out(&self) -> Pending<'_, ()> {
		in_the_browser()
	}

	fn devices(&self) -> Pending<'_, Vec<Device>> {
		Box::pin(reads::devices())
	}

	fn revoke(&self, _id: Id<Session>) -> Pending<'_, bool> {
		in_the_browser()
	}

	fn revoke_others(&self) -> Pending<'_, ()> {
		in_the_browser()
	}

	fn sign_out_everywhere(&self) -> Pending<'_, ()> {
		in_the_browser()
	}

	/// The RPC identifies the request being rendered itself.
	#[cfg(feature = "rpc")]
	fn authorization(&self) -> Pending<'_, Option<String>> {
		Box::pin(std::future::ready(Ok(None)))
	}

	#[cfg(feature = "rpc")]
	fn refused(&self, _sent: Option<String>) -> Pending<'_, bool> {
		Box::pin(std::future::ready(Ok(false)))
	}
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod reads {
	use crate::Auth;
	use crate::AuthError;
	use crate::Device;
	use crate::ErrorResponse;
	use crate::PublicSession;
	use crate::Session;

	pub(super) async fn session() -> Result<Option<PublicSession>, ErrorResponse> {
		// A session that may only set up two-step verification is still
		// the person's session.
		let current = crate::current_session()?;
		let Some(me) = current.signed_in.or(current.enrolling) else {
			return Ok(None);
		};
		let auth: Auth = crate::guards::extension()?;
		let session = Session::find(&auth.inner.db, me.session)
			.await
			.map_err(AuthError::from)?;
		Ok(session.map(PublicSession::from))
	}

	pub(super) async fn devices() -> Result<Vec<Device>, ErrorResponse> {
		let me = crate::require_signed_in()?;
		let auth: Auth = crate::guards::extension()?;
		Ok(auth.devices(&me).await?)
	}

	pub(super) async fn mfa() -> Result<crate::MfaStatus, ErrorResponse> {
		let me = enrolling()?;
		let auth: Auth = crate::guards::extension()?;
		Ok(auth.mfa_status(me.account).await?)
	}

	pub(super) async fn passkeys() -> Result<Vec<crate::PasskeyInfo>, ErrorResponse> {
		let me = enrolling()?;
		let auth: Auth = crate::guards::extension()?;
		Ok(auth.passkeys(me.account).await?)
	}

	#[allow(clippy::unused_async, reason = "the reads share a shape")]
	pub(super) async fn providers() -> Result<Vec<crate::ProviderInfo>, ErrorResponse> {
		let auth: Auth = crate::guards::extension()?;
		Ok(auth.providers())
	}

	pub(super) async fn identities() -> Result<Vec<crate::IdentityInfo>, ErrorResponse> {
		let me = crate::require_signed_in()?;
		let auth: Auth = crate::guards::extension()?;
		Ok(auth.identities(me.account).await?)
	}

	pub(super) async fn merge_offer() -> Result<Option<crate::MergeOffer>, ErrorResponse> {
		let me = crate::require_signed_in()?;
		let auth: Auth = crate::guards::extension()?;
		Ok(auth.merge_offer(&me).await?)
	}

	#[allow(clippy::unused_async, reason = "the reads share a shape")]
	pub(super) async fn wallet_settings() -> Result<crate::WalletSettings, ErrorResponse> {
		let auth: Auth = crate::guards::extension()?;
		Ok(auth.wallet_settings())
	}

	pub(super) async fn wallets() -> Result<Vec<crate::WalletInfo>, ErrorResponse> {
		let me = crate::require_signed_in()?;
		let auth: Auth = crate::guards::extension()?;
		Ok(auth.wallets(me.account).await?)
	}

	/// The signed-in person, including a session that may only set up a
	/// second factor, as the routes that do that accept it.
	fn enrolling() -> Result<crate::SignedIn, ErrorResponse> {
		let current = crate::current_session()?;
		current
			.signed_in
			.or(current.enrolling)
			.ok_or_else(|| AuthError::SignedOut.into())
	}
}

/// Without a database there is no sign-in to read, and the reads say so.
#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
mod reads {
	#![allow(
		clippy::unused_async,
		reason = "they stand in for the async functions with a database"
	)]

	use super::error;
	use crate::Device;
	use crate::ErrorResponse;
	use crate::PublicSession;

	fn no_database() -> ErrorResponse {
		error(
			"server",
			"sign-in needs a database: turn on leptoskit_auth's `postgres` or `sqlite`",
		)
	}

	pub(super) async fn session() -> Result<Option<PublicSession>, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn devices() -> Result<Vec<Device>, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn mfa() -> Result<crate::MfaStatus, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn passkeys() -> Result<Vec<crate::PasskeyInfo>, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn providers() -> Result<Vec<crate::ProviderInfo>, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn identities() -> Result<Vec<crate::IdentityInfo>, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn merge_offer() -> Result<Option<crate::MergeOffer>, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn wallet_settings() -> Result<crate::WalletSettings, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn wallets() -> Result<Vec<crate::WalletInfo>, ErrorResponse> {
		Err(no_database())
	}
}
