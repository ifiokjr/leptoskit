//! The variables the runtime contract sets: on each service, and on the app
//! for each service. A service reads these through
//! `leptoskit_axum::service`; the spellings here are the contract.

use leptoskit_config::services::env_segment;

/// The service's name.
pub const NAME: &str = "LEPTOSKIT_SERVICE_NAME";
/// The address the service binds.
pub const ADDR: &str = "LEPTOSKIT_SERVICE_ADDR";
/// The secret the service's trusted callers present, and the service
/// presents to the app.
pub const TOKEN: &str = "LEPTOSKIT_SERVICE_TOKEN";
/// The app's public origin.
pub const PUBLIC_ORIGIN: &str = "LEPTOSKIT_PUBLIC_ORIGIN";
/// The app server's private base URL.
pub const APP_URL: &str = "LEPTOSKIT_APP_URL";
/// The service's own database.
pub const DATABASE_URL: &str = "DATABASE_URL";
/// The most connections the service's database pool opens, when its table
/// or manifest sets `pool`.
pub const DATABASE_POOL: &str = "LEPTOSKIT_SERVICE_DATABASE_POOL";

/// How long, in milliseconds, the service keeps accepting after `SIGTERM`
/// before it stops: `leptoskit dev` sets 0, so a rebuild doesn't wait on a
/// drain no load balancer needs. Unset, the runtime's default applies.
pub const DRAIN_MS: &str = "LEPTOSKIT_SERVICE_DRAIN_MS";

/// Where the service finds its volume `volume`.
#[must_use]
pub fn volume(volume: &str) -> String {
	format!("LEPTOSKIT_SERVICE_VOLUME_{}", env_segment(volume))
}

/// The app's variable holding `service`'s private base URL.
#[must_use]
pub fn url(service: &str) -> String {
	format!("LEPTOSKIT_SERVICE_{}_URL", env_segment(service))
}

/// The app's variable holding `service`'s token.
#[must_use]
pub fn token(service: &str) -> String {
	format!("LEPTOSKIT_SERVICE_{}_TOKEN", env_segment(service))
}

/// The app's variable saying whether it tells `service` who is signed in:
/// set to [`IDENTITY_FORWARDED`] when `forward_identity` is on, and absent
/// otherwise. The app's server reads it before minting a token for the
/// service.
#[must_use]
pub fn identity(service: &str) -> String {
	format!("LEPTOSKIT_SERVICE_{}_IDENTITY", env_segment(service))
}

/// The value of [`identity`] that lets the app forward identity.
pub const IDENTITY_FORWARDED: &str = "forward";

/// `value` with `{url}`, `{token}`, and `{public_path}` filled in.
#[must_use]
pub fn substitute(value: &str, url: &str, token: &str, public_path: &str) -> String {
	value
		.replace("{url}", url)
		.replace("{token}", token)
		.replace("{public_path}", public_path)
}
