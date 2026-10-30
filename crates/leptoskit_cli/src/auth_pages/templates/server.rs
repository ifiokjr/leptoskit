//! Sign-in on the server, written by `leptoskit generate auth`. It reads the
//! `[auth]` table in `leptoskit.toml`, embedded so the server needs no file
//! at runtime.

use leptoskit::auth::Auth;
use leptoskit::auth::AuthError;
use leptoskit::auth::OnSignUp;
use leptoskit::config::LeptoskitConfig;
use leptoskit::db::Db;

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
	let config = LeptoskitConfig::from_toml(
		include_str!("__CONFIG_PATH__"),
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
	)
	.map_err(|error| AuthError::Config(error.to_string()))?;
	let builder = Auth::from_config(db, &config)?.on_sign_up::<U>();
	#[cfg(debug_assertions)]
	let builder = builder.dev_mailbox().dev_sign_in();
	builder.build()
}
