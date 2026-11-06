//! Operations that run in another service: the paths and environment
//! variable names every side agrees on.
//!
//! An operation declared with `#[rpc::query(service = "analytics")]` runs in
//! the `analytics` service, a separate process the app declares in
//! `leptoskit.toml`. The app calls it with the same client it uses for its
//! own operations:
//!
//! - from the browser, at `/_leptoskit/services/analytics/api/<name>` on the
//!   page's own origin, which the edge (or `leptoskit dev`) sends to the
//!   service;
//! - from a native app, at the same path on the configured origin;
//! - from the app's server, at `$LEPTOSKIT_SERVICE_ANALYTICS_URL` with the same
//!   path, signed with `$LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`.

/// The path every service's public routes live under:
/// `/_leptoskit/services/<name>`.
pub const SERVICES_PREFIX: &str = "/_leptoskit/services";

/// The path a service's public routes live under, such as
/// `/_leptoskit/services/analytics`.
///
/// ```
/// assert_eq!(
/// 	leptoskit_rpc::service_prefix("analytics"),
/// 	"/_leptoskit/services/analytics"
/// );
/// ```
#[must_use]
pub fn service_prefix(name: &str) -> String {
	format!("{SERVICES_PREFIX}/{name}")
}

/// The environment variable that carries `suffix` for the service `name`:
/// `LEPTOSKIT_SERVICE_<NAME>_<SUFFIX>`, with the name uppercased and `-`
/// written as `_`. `leptoskit dev`, `leptoskit serve`, and the deploy
/// generators set `URL` and `TOKEN` for each service.
///
/// ```
/// assert_eq!(
/// 	leptoskit_rpc::service_env_name("event-store", "URL"),
/// 	"LEPTOSKIT_SERVICE_EVENT_STORE_URL"
/// );
/// ```
#[must_use]
pub fn service_env_name(name: &str, suffix: &str) -> String {
	let name: String = name
		.chars()
		.map(|character| {
			if character == '-' {
				'_'
			} else {
				character.to_ascii_uppercase()
			}
		})
		.collect();
	format!("LEPTOSKIT_SERVICE_{name}_{suffix}")
}

/// Whether `name` can name a service: a lowercase ASCII letter, then
/// lowercase letters, digits, and `-`.
///
/// ```
/// assert!(leptoskit_rpc::is_service_name("event-store"));
/// assert!(!leptoskit_rpc::is_service_name("Events"));
/// ```
#[must_use]
pub fn is_service_name(name: &str) -> bool {
	let mut characters = name.chars();
	characters
		.next()
		.is_some_and(|first| first.is_ascii_lowercase())
		&& characters.all(|character| {
			character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_and_paths() {
		assert_eq!(service_prefix("cards"), "/_leptoskit/services/cards");
		assert_eq!(
			service_env_name("cards", "TOKEN"),
			"LEPTOSKIT_SERVICE_CARDS_TOKEN"
		);
		assert!(is_service_name("a1-b"));
		for name in ["", "1a", "-a", "a_b", "aB", "a.b"] {
			assert!(!is_service_name(name), "{name:?}");
		}
	}
}
