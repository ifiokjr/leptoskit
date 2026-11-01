//! Sign-in on the server, from the `[auth]` table in `leptoskit.toml`,
//! embedded so the server needs no file at runtime.
//!
//! `leptoskit generate auth` wrote the first version of this file. Huddle
//! keeps every email in its own mailbox (see [`crate::start`]), the
//! invitations as well as the sign-in codes, so it builds sign-in from the
//! configuration's settings itself instead of `Auth::from_config`, which
//! would pick a mailer of its own.

use std::time::Duration;

use leptoskit::db::Db;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthBuilder;
use leptoskit_auth::AuthError;
use leptoskit_auth::Keys;
use leptoskit_auth::SessionLifetimes;
use leptoskit_config::LeptoskitConfig;

const DAY: u64 = 24 * 60 * 60;

/// Sign-in for `db`, configured from `[auth]`.
///
/// The keys come from the variable `keys_env` names, `AUTH_ENCRYPTION_KEYS`
/// by default. A debug build without it uses a development key, so
/// `leptoskit dev` works with nothing set; a release build refuses to
/// start.
///
/// # Errors
///
/// When `[auth]` is missing or invalid, or a release build has no keys.
pub fn builder(db: &Db) -> Result<AuthBuilder, AuthError> {
	let config = config()?;
	let auth = config
		.auth
		.as_ref()
		.ok_or_else(|| AuthError::Config("add an [auth] table to leptoskit.toml".into()))?;
	let keys = match Keys::from_env(&auth.keys_env) {
		Ok(keys) => keys,
		#[cfg(debug_assertions)]
		Err(_) => Keys::single("development", [7; 32]),
		#[cfg(not(debug_assertions))]
		Err(error) => return Err(error),
	};
	let mut builder = Auth::builder(db, keys).sessions(SessionLifetimes {
		idle: Duration::from_secs(u64::from(auth.idle_days) * DAY),
		absolute: Duration::from_secs(u64::from(auth.absolute_days) * DAY),
		access: Duration::from_secs(u64::from(auth.access_minutes) * 60),
	});

	if let Some(origin) = std::env::var("HUDDLE_ORIGIN")
		.ok()
		.or_else(|| auth.origin.clone())
	{
		builder = builder.origin(origin);
	}

	if let Some(header) = std::env::var("HUDDLE_CLIENT_IP_HEADER")
		.ok()
		.or_else(|| auth.client_ip_header.clone())
	{
		builder = builder.client_ip_header(header);
	}

	Ok(builder)
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

/// The project's `leptoskit.toml`.
///
/// # Errors
///
/// When it doesn't parse.
pub fn config() -> Result<LeptoskitConfig, AuthError> {
	LeptoskitConfig::from_toml(
		include_str!("../../leptoskit.toml"),
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
	)
	.map_err(|error| AuthError::Config(error.to_string()))
}
