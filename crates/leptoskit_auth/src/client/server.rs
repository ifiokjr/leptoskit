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
use super::Pending;
use super::protocol::error;
use crate::Device;
use crate::PublicSession;
use crate::Session;

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

	fn verify_code(&self, _input: EmailCode) -> Pending<'_, PublicSession> {
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
		let Some(me) = crate::signed_in()? else {
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
}
