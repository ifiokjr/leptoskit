//! Guards for leptos server functions, which read what the session layer
//! learned from the request leptos is serving.
//!
//! ```ignore
//! #[server]
//! pub async fn my_drafts() -> Result<Vec<PublicPost>, ServerFnError> {
//! 	auth::require_signed_in()?;
//! 	let db = auth::request_db()?;   // policies see the signed-in person
//! 	…
//! }
//! ```

use leptos::prelude::use_context;
use leptoskit_db::Db;

use crate::AuthError;
use crate::CurrentSession;
use crate::SignedIn;

/// What the session layer learned about the request being served.
///
/// # Errors
///
/// [`AuthError::Config`] outside a request, or when the server didn't add
/// [`Auth::layer`](crate::Auth::layer).
pub fn current_session() -> Result<CurrentSession, AuthError> {
	extension()
}

/// The signed-in person, if the request has a live session.
///
/// # Errors
///
/// As [`current_session`].
pub fn signed_in() -> Result<Option<SignedIn>, AuthError> {
	current_session().map(|current| current.signed_in)
}

/// The signed-in person, or [`AuthError::SignedOut`].
///
/// # Errors
///
/// [`AuthError::SignedOut`] for a signed-out request, or as
/// [`current_session`].
pub fn require_signed_in() -> Result<SignedIn, AuthError> {
	signed_in()?.ok_or(AuthError::SignedOut)
}

/// The database, acting for the request: its policies see the signed-in
/// person, or no one.
///
/// # Errors
///
/// As [`current_session`].
pub fn request_db() -> Result<Db, AuthError> {
	extension()
}

fn extension<T: Clone + Send + Sync + 'static>() -> Result<T, AuthError> {
	let parts = use_context::<axum::http::request::Parts>().ok_or_else(|| {
		AuthError::Config("auth guards only work while leptos serves a request".into())
	})?;
	parts
		.extensions
		.get::<T>()
		.cloned()
		.ok_or_else(|| AuthError::Config("add `auth.layer()` to the server's router".into()))
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
	use axum::http::Request;
	use leptos::prelude::Owner;
	use leptos::prelude::provide_context;

	use super::*;
	use crate::Aal;
	use crate::RequestInfo;

	fn parts(extensions: impl FnOnce(&mut axum::http::Extensions)) -> axum::http::request::Parts {
		let (mut parts, ()) = Request::new(()).into_parts();
		extensions(&mut parts.extensions);
		parts
	}

	#[tokio::test]
	async fn guards_read_the_request_leptos_serves() {
		let db = leptoskit_db::testing::sqlite().await;
		let me = SignedIn {
			account: leptoskit_db::Id::new(),
			session: leptoskit_db::Id::new(),
			aal: Aal::One,
			authenticated_at: leptoskit_db::now(),
			mfa_at: None,
		};
		let session = |signed_in: Option<SignedIn>| {
			CurrentSession {
				signed_in,
				request: RequestInfo::default(),
				token: None,
			}
		};
		let owner = Owner::new();

		owner.with(|| {
			assert!(matches!(require_signed_in(), Err(AuthError::Config(message)) if message.contains("while leptos serves")));

			provide_context(parts(|_| {}));
			assert!(matches!(request_db(), Err(AuthError::Config(message)) if message.contains("auth.layer()")));

			let person = me.clone();
			provide_context(parts(|extensions| {
				extensions.insert(session(Some(person)));
				extensions.insert(db.clone());
			}));
			assert_eq!(require_signed_in().unwrap(), me);
			assert!(request_db().unwrap().context().is_some());

			provide_context(parts(|extensions| {
				extensions.insert(session(None));
			}));
			assert_eq!(signed_in().unwrap(), None);
			assert!(matches!(require_signed_in(), Err(AuthError::SignedOut)));
		});
	}
}
