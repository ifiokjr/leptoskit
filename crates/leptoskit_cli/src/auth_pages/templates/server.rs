//! Sign-in on the server, written by `leptoskit generate auth`. It reads the
//! `[auth]` table in `leptoskit.toml`, embedded so the server needs no file
//! at runtime.

use leptoskit::auth::Auth;
use leptoskit::auth::AuthError;
use leptoskit::auth::OnSignUp;
use leptoskit::config::LeptoskitConfig;
use leptoskit::db::Db;
// leptoskit: idp
use leptoskit::idp::Idp;
use leptoskit::idp::IdpBuilder;
use leptoskit::idp::IdpError;
// leptoskit: end idp
/// The sign-in service. `U` is the table that gets a row for each new
/// account, through its `OnSignUp`.
///
/// Debug builds keep every email at `/_leptoskit/mail` instead of sending
/// it, and add `POST /auth/dev/sign-in`, which signs in to any address
/// without a code. Release builds have neither.
///
/// # Errors
///
/// When `[auth]` is missing or invalid, or `AUTH_ENCRYPTION_KEYS` isn't set.
pub fn service<U: OnSignUp + 'static>(db: &Db) -> Result<Auth, AuthError> {
	let builder = Auth::from_config(db, &config()?)?.on_sign_up::<U>();
	#[cfg(debug_assertions)]
	let builder = builder.dev_mailbox().dev_sign_in();
	builder.build()
}

/// `/.well-known/apple-app-site-association` and
/// `/.well-known/assetlinks.json`, from `[native]`, so the iOS, macOS, and
/// Android apps can use this site's passkeys and sign-in return address.
/// Each is served once `[native]` names the Apple team or the Android
/// signing certificates.
///
/// # Errors
///
/// When `leptoskit.toml` is invalid.
pub fn associations<S>() -> Result<leptoskit::server::axum::Router<S>, AuthError>
where
	S: Clone + Send + Sync + 'static,
{
	Ok(leptoskit::server::app_associations(&config()?.native))
}

fn config() -> Result<LeptoskitConfig, AuthError> {
	LeptoskitConfig::from_toml(
		include_str!("__CONFIG_PATH__"),
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
	)
	.map_err(|error| AuthError::Config(error.to_string()))
}

// leptoskit: idp
/// The provider other apps sign in with, from `[auth.idp]`. Add your scopes
/// and claims, then build it:
///
/// ```ignore
/// let idp = sign_in::provider(&db, &auth)?
/// 	.scopes::<Scope>()
/// 	.claims::<User>()
/// 	.build()?;
/// ```
///
/// # Errors
///
/// When `[auth.idp]` is missing or invalid.
pub fn provider(db: &Db, auth: &Auth) -> Result<IdpBuilder, IdpError> {
	let config = config().map_err(|error| {
		match error {
			AuthError::Config(message) => IdpError::Config(message),
			error => error.into(),
		}
	})?;
	Idp::from_config(db, auth, &config)
}
// leptoskit: end idp
