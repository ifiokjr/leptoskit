//! The headless client for "Sign in with your app": the query and mutation
//! scopes a consent page, a device page, a "Connected apps" screen and a
//! developer portal are built from. The look is yours; these are the
//! behaviour, the same in the browser, in a native app, and while the
//! server renders.
//!
//! ```ignore
//! use leptoskit::idp::client::*;
//!
//! let request = use_consent(move || handle.get());          // the app asking, and what for
//! let decide = use_mutation(decide_consent());
//! decide.mutate(ConsentDecision { request: handle.get(), approve: true });
//! // then navigate to the answer's `redirect_to`
//!
//! let apps = use_connected_apps();                         // what they've connected
//! let disconnect = use_mutation(disconnect_app());
//! disconnect.mutate(app.app.client_id.clone());
//! ```
//!
//! The client sends its requests through `leptoskit_auth`'s client, with the
//! same session: the `HttpOnly` cookie in the browser, or the access token a
//! native app keeps, renewed when it runs out. Configure that once with
//! `leptoskit_auth::client::configure`; there's nothing to configure here.
//! While the server renders a page, the reads come from the request's own
//! session in process, and the changes answer `server`.
//!
//! Errors carry the provider's codes as [`ErrorResponse::error`], such as
//! `login_required` when no one is signed in, `not_found` for an unknown or
//! expired request, `invalid_client_metadata` for an app the portal won't
//! register, and `rate_limited`; plus the auth client's own `network`,
//! `config` and `storage`.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use leptoskit_auth::ErrorResponse;
use leptoskit_query::BoxFuture;
use leptoskit_query::MutationScope;
use leptoskit_query::QueryHandle;
use leptoskit_query::QueryScope;
use leptoskit_query::Retry;
use leptoskit_query::use_query;

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

mod remote;
#[cfg(feature = "ssr")]
mod server;
#[cfg(all(test, feature = "axum", feature = "sqlite"))]
mod tests;

/// A call to the server, in flight.
pub(crate) type Pending<'a, T> = BoxFuture<'a, Result<T, ErrorResponse>>;

/// What every target's client can do. The scopes call it.
pub(crate) trait Api: Send + Sync + 'static {
	fn consent(&self, request: String) -> Pending<'_, ConsentRequest>;
	fn decide_consent(&self, decision: ConsentDecision) -> Pending<'_, ConsentRedirect>;
	fn device(&self, user_code: String) -> Pending<'_, DeviceRequest>;
	fn decide_device(&self, decision: DeviceDecision) -> Pending<'_, ()>;
	fn connected_apps(&self) -> Pending<'_, Vec<ConnectedApp>>;
	fn disconnect(&self, client_id: String) -> Pending<'_, bool>;
	fn my_apps(&self) -> Pending<'_, Vec<ClientInfo>>;
	fn register_app(&self, app: NewApp) -> Pending<'_, RegisteredApp>;
	fn update_app(&self, changes: AppChanges) -> Pending<'_, ClientInfo>;
	fn rotate_app_secret(&self, client_id: String) -> Pending<'_, RegisteredApp>;
	fn delete_app(&self, client_id: String) -> Pending<'_, bool>;
}

static CLIENT: RwLock<Option<Arc<dyn Api>>> = RwLock::new(None);

#[cfg(test)]
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
	Arc::clone(client.get_or_insert_with(backend))
}

/// While the server renders: the request's session, in process.
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
fn backend() -> Arc<dyn Api> {
	Arc::new(server::Server)
}

/// Everywhere else: the auth client's transport, with its session.
#[cfg(not(all(feature = "ssr", not(target_arch = "wasm32"))))]
fn backend() -> Arc<dyn Api> {
	Arc::new(remote::Remote::new(leptoskit_auth::client::send))
}

/// Reads try again, up to three times, only when the failure may pass: the
/// network, or the server. An answer such as `not_found` or
/// `login_required` stands.
fn transient() -> Retry<ErrorResponse> {
	Retry::decide(|failures, error: &ErrorResponse| {
		failures < 3 && matches!(error.error.as_str(), "network" | "server")
	})
}

/// The consent request a consent page was opened with, by its `request`
/// handle: the app asking, what it asks to do, and where it'll send the
/// person back.
///
/// Fails with `login_required` when no one is signed in, and `not_found`
/// when the request is unknown, answered, expired, or someone else's.
pub fn consent() -> QueryScope<String, ConsentRequest, ErrorResponse> {
	QueryScope::new(|request: String| async move { api().consent(request).await })
		.with_retry(transient())
}

/// The consent request `request` names, as a query handle.
#[track_caller]
pub fn use_consent(
	request: impl Fn() -> String + Send + Sync + 'static,
) -> QueryHandle<ConsentRequest, ErrorResponse> {
	use_query(consent(), request)
}

/// Answers a consent request. On success, send the browser to
/// [`ConsentRedirect::redirect_to`]: back to the app, with a code or with
/// `access_denied`. The connected apps query refetches.
pub fn decide_consent() -> MutationScope<ConsentDecision, ConsentRedirect, ErrorResponse> {
	MutationScope::new(
		|decision: ConsentDecision| async move { api().decide_consent(decision).await },
	)
	.with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&connected_apps()).await;
		}
	})
}

/// What the device page shows for the code someone typed: the app on the
/// device, and what it asks to do. Codes are read in any case, with or
/// without the dash.
///
/// Fails with `login_required`, `not_found` for a code no device is waiting
/// for, or `rate_limited` after too many guesses.
pub fn device_request() -> QueryScope<String, DeviceRequest, ErrorResponse> {
	QueryScope::new(|user_code: String| async move { api().device(user_code).await })
		.with_retry(transient())
}

/// The device request for `user_code`, as a query handle.
#[track_caller]
pub fn use_device_request(
	user_code: impl Fn() -> String + Send + Sync + 'static,
) -> QueryHandle<DeviceRequest, ErrorResponse> {
	use_query(device_request(), user_code)
}

/// Approves or refuses a device. On success, the device's next poll gets
/// tokens or hears `access_denied`, and the connected apps query
/// refetches.
pub fn decide_device() -> MutationScope<DeviceDecision, (), ErrorResponse> {
	MutationScope::new(
		|decision: DeviceDecision| async move { api().decide_device(decision).await },
	)
	.with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&connected_apps()).await;
		}
	})
}

/// The apps the signed-in person let sign in with their account, most
/// recently connected first, for a "Connected apps" screen.
///
/// Fails with `login_required` when no one is signed in.
pub fn connected_apps() -> QueryScope<(), Vec<ConnectedApp>, ErrorResponse> {
	QueryScope::new(|()| async { api().connected_apps().await }).with_retry(transient())
}

/// The signed-in person's connected apps, as a query handle.
#[track_caller]
pub fn use_connected_apps() -> QueryHandle<Vec<ConnectedApp>, ErrorResponse> {
	use_query(connected_apps(), || ())
}

/// Disconnects the app with this client ID, ending every token it holds,
/// and says whether it was connected. On success, the connected apps query
/// refetches.
pub fn disconnect_app() -> MutationScope<String, bool, ErrorResponse> {
	MutationScope::new(|client_id: String| async move { api().disconnect(client_id).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&connected_apps()).await;
			}
		})
}

/// The apps the signed-in person registered in the developer portal,
/// oldest first.
///
/// Fails with `login_required`, or `not_found` when the portal is off.
pub fn my_apps() -> QueryScope<(), Vec<ClientInfo>, ErrorResponse> {
	QueryScope::new(|()| async { api().my_apps().await }).with_retry(transient())
}

/// The signed-in person's own apps, as a query handle.
#[track_caller]
pub fn use_my_apps() -> QueryHandle<Vec<ClientInfo>, ErrorResponse> {
	use_query(my_apps(), || ())
}

/// Registers an app in the developer portal. The answer holds its secret,
/// shown this once. On success, the apps query refetches.
///
/// Fails with `invalid_client_metadata` for an app that can't register, or
/// when the person already has as many as the portal allows.
pub fn register_app() -> MutationScope<NewApp, RegisteredApp, ErrorResponse> {
	MutationScope::new(|app: NewApp| async move { api().register_app(app).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&my_apps()).await;
			}
		})
}

/// Changes one of the person's own apps. Taking scopes away takes them from
/// everyone who connected it too. On success, the apps query refetches.
pub fn update_app() -> MutationScope<AppChanges, ClientInfo, ErrorResponse> {
	MutationScope::new(|changes: AppChanges| async move { api().update_app(changes).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&my_apps()).await;
			}
		})
}

/// Replaces the secret of one of the person's own apps, by client ID. The
/// answer holds the new secret, shown this once.
pub fn rotate_app_secret() -> MutationScope<String, RegisteredApp, ErrorResponse> {
	MutationScope::new(|client_id: String| async move { api().rotate_app_secret(client_id).await })
}

/// Deletes one of the person's own apps, by client ID, ending every grant
/// and token it holds, and says whether there was one. On success, the
/// apps query refetches.
pub fn delete_app() -> MutationScope<String, bool, ErrorResponse> {
	MutationScope::new(|client_id: String| async move { api().delete_app(client_id).await })
		.with_on_success_async(|args| {
			async move {
				args.client.invalidate_scope(&my_apps()).await;
			}
		})
}
