//! What the client's server functions do on the server: read the request
//! the session layer saw, and set or clear the session cookie through the
//! response leptos is building.

#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
use crate::ErrorResponse;
#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
use crate::PublicSession;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod live {
	use axum::http::HeaderValue;
	use axum::http::header;
	use leptos::prelude::use_context;
	use leptos_axum::ResponseOptions;

	use crate::Auth;
	use crate::AuthError;
	use crate::ErrorResponse;
	use crate::PublicSession;
	use crate::Session;

	fn service() -> Result<Auth, AuthError> {
		crate::guards::extension()
	}

	fn set_cookie(value: HeaderValue) -> Result<(), AuthError> {
		let response = use_context::<ResponseOptions>().ok_or_else(|| {
			AuthError::Config("sign-in server functions need leptos_axum's response".into())
		})?;
		response.insert_header(header::SET_COOKIE, value);
		Ok(())
	}

	pub async fn current_session() -> Result<Option<PublicSession>, ErrorResponse> {
		let Some(me) = crate::signed_in()? else {
			return Ok(None);
		};
		let auth = service()?;
		let session = Session::find(&auth.inner.db, me.session)
			.await
			.map_err(AuthError::from)?;
		Ok(session.map(PublicSession::from))
	}

	pub async fn send_email_code(email: String) -> Result<(), ErrorResponse> {
		let current = crate::current_session()?;
		service()?.send_email_code(&email, &current.request).await?;
		Ok(())
	}

	pub async fn verify_email_code(
		email: String,
		code: String,
	) -> Result<PublicSession, ErrorResponse> {
		let current = crate::current_session()?;
		let auth = service()?;
		let sign_in = auth
			.verify_email_code(&email, &code, &current.request)
			.await?;
		set_cookie(auth.session_cookie(&sign_in.token))?;
		Ok(PublicSession::from(sign_in.session))
	}

	pub async fn sign_out() -> Result<(), ErrorResponse> {
		let current = crate::current_session()?;
		let auth = service()?;

		if let Some(token) = &current.token {
			auth.sign_out(token, &current.request).await?;
		}

		set_cookie(auth.cleared_cookie())?;
		Ok(())
	}
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use live::*;

/// Without a database there is no sign-in to run.
#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
fn no_database() -> ErrorResponse {
	ErrorResponse {
		error: "server".into(),
		message: "sign-in needs a database: turn on leptoskit_auth's `postgres` or `sqlite`".into(),
		retry_after: None,
	}
}

#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
pub async fn current_session() -> Result<Option<PublicSession>, ErrorResponse> {
	Err(no_database())
}

#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
pub async fn send_email_code(_email: String) -> Result<(), ErrorResponse> {
	Err(no_database())
}

#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
pub async fn verify_email_code(
	_email: String,
	_code: String,
) -> Result<PublicSession, ErrorResponse> {
	Err(no_database())
}

#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
pub async fn sign_out() -> Result<(), ErrorResponse> {
	Err(no_database())
}
