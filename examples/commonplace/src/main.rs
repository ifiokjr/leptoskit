//! Commonplace's server: the JSON API, sign-in, and the built website.
//!
//! The website renders in the browser (`app.render = "csr"`), the same way
//! the native apps and the extension do, so every client talks to this one
//! API. `leptoskit build` puts the website in `dist/`, and the server serves
//! it, falling back to `index.html` for the app's own routes.

use std::path::Path;

use leptoskit::auth::Auth;
use leptoskit::config::LeptoskitConfig;
use leptoskit::server::tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let config =
		LeptoskitConfig::from_toml(include_str!("../leptoskit.toml"), Path::new("commonplace"))?;
	let site = std::env::var("LEPTOS_SITE_ROOT").unwrap_or_else(|_| "dist".to_owned());
	let address = std::env::var("COMMONPLACE_ADDRESS")
		.unwrap_or_else(|_| format!("{}:{}", config.serve.address, config.serve.port));

	// The database, migrated before the first request.
	let url = std::env::var("DATABASE_URL")
		.unwrap_or_else(|_| "sqlite://commonplace.sqlite3?mode=rwc".to_owned());
	let db = leptoskit::db::Db::connect(&url).await?;
	for name in db.migrate(commonplace_db::MIGRATIONS).await? {
		println!("applied migration {name}");
	}

	let auth = sign_in(&db, &config)?.build()?;
	let app = commonplace_backend::router()
		.merge(auth.router())
		.fallback_service(leptoskit::server::static_files(site.as_ref()))
		.layer(auth.layer());

	let listener = tokio::net::TcpListener::bind(&address).await?;
	println!("listening on http://{address}");
	// Serves until Ctrl-C or the SIGTERM a container runtime sends, finishing
	// the requests in flight first.
	leptoskit::server::serve_listener(listener, app).await?;
	Ok(())
}

/// Sign-in as `[auth]` in `leptoskit.toml` describes it.
///
/// A debug build without `AUTH_ENCRYPTION_KEYS` uses a fixed development key,
/// keeps every email at `/_leptoskit/mail`, and offers the development
/// sign-in, so `leptoskit dev` works with no setup. A release build has none
/// of that, and refuses to start without its keys and a way to send email.
fn sign_in(
	db: &leptoskit::db::Db,
	config: &LeptoskitConfig,
) -> Result<leptoskit::auth::AuthBuilder, leptoskit::auth::AuthError> {
	#[cfg(debug_assertions)]
	if let Some(auth) = &config.auth
		&& std::env::var_os(&auth.keys_env).is_none()
	{
		println!("{} is not set: using the development key", auth.keys_env);
		let mut builder = Auth::builder(db, leptoskit::auth::Keys::single("dev", [7; 32]));
		if let Some(name) = &auth.app_name {
			builder = builder.app_name(name.clone());
		}
		if let Some(origin) = &auth.origin {
			builder = builder.origin(origin.clone());
		}
		return Ok(builder.dev_mailbox().dev_sign_in());
	}

	let builder = Auth::from_config(db, config)?;
	#[cfg(debug_assertions)]
	let builder = builder.dev_mailbox().dev_sign_in();
	Ok(builder)
}
