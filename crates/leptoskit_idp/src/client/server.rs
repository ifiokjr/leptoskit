//! The client while the server renders a page: it reads the provider and
//! the session the layers put on the request leptos is serving, with no
//! HTTP hop. Answering requests and managing apps happen in the browser or
//! the app, so only the reads work here.

#![cfg_attr(
	target_arch = "wasm32",
	allow(dead_code, reason = "server rendering never runs in Wasm")
)]

use super::Api;
use super::Pending;
use super::remote::error;
use crate::AppChanges;
use crate::ClientInfo;
use crate::ConnectedApp;
use crate::ConsentDecision;
use crate::ConsentRedirect;
use crate::ConsentRequest;
use crate::DeviceDecision;
use crate::DeviceRequest;
use crate::NewApp;
use crate::RegisteredApp;

/// The request's own session, in process.
pub(crate) struct Server;

fn in_the_browser<T: Send + 'static>() -> Pending<'static, T> {
	Box::pin(std::future::ready(Err(error(
		"server",
		"answering and changing runs in the browser or the app, not while the server renders",
	))))
}

impl Api for Server {
	fn consent(&self, request: String) -> Pending<'_, ConsentRequest> {
		Box::pin(async move { reads::consent(&request).await })
	}

	fn decide_consent(&self, _decision: ConsentDecision) -> Pending<'_, ConsentRedirect> {
		in_the_browser()
	}

	fn device(&self, user_code: String) -> Pending<'_, DeviceRequest> {
		Box::pin(async move { reads::device(&user_code).await })
	}

	fn decide_device(&self, _decision: DeviceDecision) -> Pending<'_, ()> {
		in_the_browser()
	}

	fn connected_apps(&self) -> Pending<'_, Vec<ConnectedApp>> {
		Box::pin(reads::connected_apps())
	}

	fn disconnect(&self, _client_id: String) -> Pending<'_, bool> {
		in_the_browser()
	}

	fn my_apps(&self) -> Pending<'_, Vec<ClientInfo>> {
		Box::pin(reads::my_apps())
	}

	fn register_app(&self, _app: NewApp) -> Pending<'_, RegisteredApp> {
		in_the_browser()
	}

	fn update_app(&self, _changes: AppChanges) -> Pending<'_, ClientInfo> {
		in_the_browser()
	}

	fn rotate_app_secret(&self, _client_id: String) -> Pending<'_, RegisteredApp> {
		in_the_browser()
	}

	fn delete_app(&self, _client_id: String) -> Pending<'_, bool> {
		in_the_browser()
	}
}

/// An error as the routes would answer it.
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
fn answer(failure: &crate::IdpError) -> leptoskit_auth::ErrorResponse {
	leptoskit_auth::ErrorResponse {
		error: failure.code().to_owned(),
		message: failure.description(),
		retry_after: failure.retry_after().map(|wait| wait.as_secs().max(1)),
	}
}

#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
mod reads {
	use leptoskit_auth::ErrorResponse;
	use leptoskit_auth::SignedIn;

	use super::answer;
	use crate::ClientInfo;
	use crate::ConnectedApp;
	use crate::ConsentRequest;
	use crate::DeviceRequest;
	use crate::Idp;
	use crate::IdpError;

	/// The provider and the person, as the routes see them: a third-party
	/// token never counts as someone signed in here.
	fn context() -> Result<(Idp, SignedIn), ErrorResponse> {
		// Outside a request, or without the layers, nothing will change by
		// trying again: these are `config`, which isn't retried.
		let outside = || {
			super::error(
				"config",
				"the provider reads only while leptos serves a request through `auth.layer()`",
			)
		};
		let current = leptoskit_auth::current_session().map_err(|_| outside())?;
		let parts =
			leptos::prelude::use_context::<axum::http::request::Parts>().ok_or_else(outside)?;
		let idp =
			parts.extensions.get::<Idp>().cloned().ok_or_else(|| {
				super::error("config", "add `idp.layer()` to the server's router")
			})?;
		let me = current
			.signed_in
			.filter(|_| current.delegation.is_none())
			.ok_or_else(|| answer(&IdpError::SignedOut))?;
		Ok((idp, me))
	}

	pub(super) async fn consent(request: &str) -> Result<ConsentRequest, ErrorResponse> {
		let (idp, me) = context()?;
		idp.consent_request(&me, request)
			.await
			.map_err(|failure| answer(&failure))
	}

	pub(super) async fn device(user_code: &str) -> Result<DeviceRequest, ErrorResponse> {
		let (idp, me) = context()?;
		idp.device_request(&me, user_code)
			.await
			.map_err(|failure| answer(&failure))
	}

	pub(super) async fn connected_apps() -> Result<Vec<ConnectedApp>, ErrorResponse> {
		let (idp, me) = context()?;
		idp.connected_apps(&me)
			.await
			.map_err(|failure| answer(&failure))
	}

	pub(super) async fn my_apps() -> Result<Vec<ClientInfo>, ErrorResponse> {
		let (idp, me) = context()?;
		idp.my_apps(&me).await.map_err(|failure| answer(&failure))
	}
}

/// Without a database there is no provider to read, and the reads say so.
#[cfg(not(all(feature = "axum", any(feature = "postgres", feature = "sqlite"))))]
mod reads {
	#![allow(
		clippy::unused_async,
		reason = "they stand in for the async functions with a database"
	)]

	use leptoskit_auth::ErrorResponse;

	use super::error;
	use crate::ClientInfo;
	use crate::ConnectedApp;
	use crate::ConsentRequest;
	use crate::DeviceRequest;

	fn no_database() -> ErrorResponse {
		error(
			"server",
			"the provider needs a database: turn on leptoskit_idp's `postgres` or `sqlite`, and \
			 `axum`",
		)
	}

	pub(super) async fn consent(_request: &str) -> Result<ConsentRequest, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn device(_user_code: &str) -> Result<DeviceRequest, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn connected_apps() -> Result<Vec<ConnectedApp>, ErrorResponse> {
		Err(no_database())
	}

	pub(super) async fn my_apps() -> Result<Vec<ClientInfo>, ErrorResponse> {
		Err(no_database())
	}
}
