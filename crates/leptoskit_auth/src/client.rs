//! The headless client: server functions over the sign-in service, and the
//! query and mutation scopes a sign-in screen is built from. The look is
//! yours; these are the behaviour.
//!
//! ```ignore
//! let auth = use_auth();                         // the session, or `None` when signed out
//! let send = use_mutation(auth::client::send_code());
//! let verify = use_mutation(auth::client::verify_code());
//!
//! send.mutate(email);
//! verify.mutate(VerifyCode { email, code, ..Default::default() });
//!
//! let devices = use_devices();                   // where they're signed in, this device marked current
//! let revoke = use_mutation(auth::client::revoke_device());
//! revoke.mutate(device.session.id);
//! ```
//!
//! Signing in or out updates the session query, so everything reading
//! [`use_auth`] re-renders.

use leptos::server;
use leptos::server_fn::codec::JsonEncoding;
use leptos::server_fn::error::FromServerFnError;
use leptos::server_fn::error::ServerFnErrorErr;
use leptoskit_db::Id;
use leptoskit_query::MutationScope;
use leptoskit_query::QueryHandle;
use leptoskit_query::QueryScope;
use leptoskit_query::use_query;

use crate::Device;
use crate::ErrorResponse;
use crate::PublicSession;
use crate::Session;
use crate::VerifyCode;

impl FromServerFnError for ErrorResponse {
	type Encoder = JsonEncoding;

	fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
		let (error, message) = match &value {
			ServerFnErrorErr::Request(_) => ("network", "couldn't reach the server; try again"),
			ServerFnErrorErr::Args(_)
			| ServerFnErrorErr::MissingArg(_)
			| ServerFnErrorErr::Deserialization(_) => ("invalid", "the request didn't make sense"),
			_ => ("server", "something went wrong; try again"),
		};

		Self {
			error: error.to_owned(),
			message: message.to_owned(),
			retry_after: None,
		}
	}
}

/// The request's session, or `None` when it's signed out.
///
/// # Errors
///
/// When the server can't be reached or fails.
#[server(prefix = "/auth/rpc", endpoint = "session")]
pub async fn current_session() -> Result<Option<PublicSession>, ErrorResponse> {
	crate::client_server::current_session().await
}

/// Emails a sign-in code to `email`.
///
/// # Errors
///
/// `invalid_email`, `rate_limited`, or `mail`, as [`ErrorResponse`] says.
#[server(prefix = "/auth/rpc", endpoint = "send_code")]
pub async fn send_email_code(email: String) -> Result<(), ErrorResponse> {
	crate::client_server::send_email_code(email).await
}

/// Signs in with an emailed code, and starts a session in the browser's
/// cookie.
///
/// # Errors
///
/// `invalid_code`, `rate_limited`, `disabled`, `locked`, or `sign_up`, as
/// [`ErrorResponse`] says.
#[server(prefix = "/auth/rpc", endpoint = "verify_code")]
pub async fn verify_email_code(
	email: String,
	code: String,
) -> Result<PublicSession, ErrorResponse> {
	crate::client_server::verify_email_code(email, code).await
}

/// Ends the current session, and clears its cookie.
///
/// # Errors
///
/// When the server can't be reached or fails.
#[server(prefix = "/auth/rpc", endpoint = "sign_out")]
pub async fn sign_out() -> Result<(), ErrorResponse> {
	crate::client_server::sign_out().await
}

/// The signed-in person's devices: one live session each, most recently
/// used first, with this one marked current.
///
/// # Errors
///
/// `signed_out` when no one is signed in.
#[server(prefix = "/auth/rpc", endpoint = "sessions")]
pub async fn list_sessions() -> Result<Vec<Device>, ErrorResponse> {
	crate::client_server::list_sessions().await
}

/// Ends one of the signed-in person's sessions, such as a lost phone's.
/// Returns whether it was theirs and still live.
///
/// # Errors
///
/// `signed_out` when no one is signed in.
#[server(prefix = "/auth/rpc", endpoint = "revoke_session")]
pub async fn revoke_session(id: Id<Session>) -> Result<bool, ErrorResponse> {
	crate::client_server::revoke_session(id).await
}

/// Ends every session but this one, and returns how many ended.
///
/// # Errors
///
/// `signed_out` when no one is signed in.
#[server(prefix = "/auth/rpc", endpoint = "revoke_other_sessions")]
pub async fn revoke_other_sessions() -> Result<u64, ErrorResponse> {
	crate::client_server::revoke_other_sessions().await
}

/// Ends every session, this one included, clears the cookie, and returns
/// how many ended.
///
/// # Errors
///
/// `signed_out` when no one is signed in.
#[server(prefix = "/auth/rpc", endpoint = "sign_out_everywhere")]
pub async fn sign_out_everywhere() -> Result<u64, ErrorResponse> {
	crate::client_server::sign_out_everywhere().await
}

/// The session query: [`current_session`], cached and shared by everything
/// that reads it.
pub fn session() -> QueryScope<(), Option<PublicSession>, ErrorResponse> {
	QueryScope::new(|()| current_session())
}

/// The current session, as a query handle: `data()` is `Some(Some(session))`
/// when signed in, and `Some(None)` when signed out.
#[track_caller]
pub fn use_auth() -> QueryHandle<Option<PublicSession>, ErrorResponse> {
	use_query(session(), || ())
}

/// Sends a code to the email address it's given.
pub fn send_code() -> MutationScope<String, (), ErrorResponse> {
	MutationScope::new(send_email_code)
}

/// Signs in with a code. On success, the session query holds the new
/// session.
pub fn verify_code() -> MutationScope<VerifyCode, PublicSession, ErrorResponse> {
	MutationScope::new(|input: VerifyCode| verify_email_code(input.email, input.code))
		.with_on_success(|args| {
			args.client.set_query_data(&session(), (), Some(args.data));
		})
}

/// Signs out. On success, the session query holds `None`.
pub fn sign_out_mutation() -> MutationScope<(), (), ErrorResponse> {
	MutationScope::new(|()| sign_out()).with_on_success(|args| {
		args.client.set_query_data(&session(), (), None);
	})
}

/// The devices query: [`list_sessions`], for a "where you're signed in"
/// screen.
pub fn devices() -> QueryScope<(), Vec<Device>, ErrorResponse> {
	QueryScope::new(|()| list_sessions())
}

/// The signed-in person's devices, as a query handle.
#[track_caller]
pub fn use_devices() -> QueryHandle<Vec<Device>, ErrorResponse> {
	use_query(devices(), || ())
}

/// Ends the session it's given. On success, the devices query refetches.
pub fn revoke_device() -> MutationScope<Id<Session>, bool, ErrorResponse> {
	MutationScope::new(revoke_session).with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&devices()).await;
		}
	})
}

/// Ends every session but this one. On success, the devices query
/// refetches.
pub fn revoke_other_devices() -> MutationScope<(), u64, ErrorResponse> {
	MutationScope::new(|()| revoke_other_sessions()).with_on_success_async(|args| {
		async move {
			args.client.invalidate_scope(&devices()).await;
		}
	})
}

/// Ends every session, this one included. On success, the session query
/// holds `None` and the devices query is empty.
pub fn sign_out_everywhere_mutation() -> MutationScope<(), u64, ErrorResponse> {
	MutationScope::new(|()| sign_out_everywhere()).with_on_success(|args| {
		args.client.set_query_data(&session(), (), None);
		args.client.set_query_data(&devices(), (), Vec::new());
	})
}
