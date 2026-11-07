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
//!
//! When the app forwards identity to the service (`forward_identity` in its
//! `[services.<name>]` table), each of those calls also carries a short-lived
//! token naming the signed-in person, which the app's server mints for that
//! service alone: see [`ServiceTokenArgs`].

use serde::Deserialize;
use serde::Serialize;

use crate::Kind;
use crate::Operation;

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

/// What `LEPTOSKIT_SERVICE_<NAME>_IDENTITY` holds when the app tells the
/// service who is signed in: `leptoskit dev`, `leptoskit serve`, and the
/// deploy generators set it for each service with `forward_identity` on.
pub const IDENTITY_FORWARDED: &str = "forward";

/// Whether the app forwards identity to `service`, by the variable `lookup`
/// reads: `LEPTOSKIT_SERVICE_<NAME>_IDENTITY=forward`.
#[cfg(feature = "db")]
pub(crate) fn forwards_identity(service: &str, lookup: impl Fn(&str) -> Option<String>) -> bool {
	lookup(&service_env_name(service, "IDENTITY"))
		.is_some_and(|value| value.trim() == IDENTITY_FORWARDED)
}

/// The name of the operation [`ServiceTokenArgs`] calls, which every build
/// of the client knows, with or without a database driver.
pub(crate) const SERVICE_TOKEN_OPERATION: &str = "rpc.service_token";

/// Asks the app's server for a token that tells the service `service` who
/// is signed in: `rpc.service_token`, which every app's server answers.
///
/// The client asks for one itself before it calls a service, and signs the
/// service's calls and its live connection with it; an app doesn't call
/// this. The answer is `None` for a caller nobody is signed in as, and a
/// `Forbidden` error when the app doesn't forward identity to that service.
///
/// It's an action, so it runs no transaction and keeps no receipt: a token
/// is never stored.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ServiceTokenArgs {
	/// The service the token is for, and the only one that accepts it.
	pub service: String,
}

/// A token that tells one service who is signed in, as
/// [`ServiceTokenArgs`] answers it.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ServiceToken {
	/// The token, which goes in `Authorization: Bearer <token>`.
	pub token: String,
	/// How many seconds from now it stops working.
	pub expires_in: u64,
}

impl std::fmt::Debug for ServiceToken {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ServiceToken")
			.field("expires_in", &self.expires_in)
			.finish_non_exhaustive()
	}
}

// A server build with no database driver has no `ActionCx` to name, so it
// mints nothing; everything else does.
#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
impl Operation for ServiceTokenArgs {
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	type Context = crate::ActionCx;
	type Output = Option<ServiceToken>;

	const KIND: Kind = Kind::Action;
	const NAME: &'static str = SERVICE_TOKEN_OPERATION;

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn run(self, cx: &Self::Context) -> impl Future<Output = crate::Result<Self::Output>> + Send {
		std::future::ready(crate::server::service_identity_token(
			&self.service,
			cx.identity(),
		))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[cfg(feature = "db")]
	#[test]
	fn identity_is_forwarded_only_when_the_variable_says_so() {
		let set = |value: &'static str| {
			move |name: &str| {
				(name == "LEPTOSKIT_SERVICE_EVENT_STORE_IDENTITY").then(|| value.to_owned())
			}
		};
		assert!(forwards_identity("event-store", set("forward")));
		assert!(forwards_identity("event-store", set(" forward\n")));
		assert!(!forwards_identity("event-store", set("1")));
		assert!(!forwards_identity("event-store", set("")));
		assert!(!forwards_identity("cards", set("forward")));
	}

	#[test]
	fn a_service_token_never_prints_itself() {
		let token = ServiceToken {
			token: "lkid1.secret".to_owned(),
			expires_in: 300,
		};
		assert!(!format!("{token:?}").contains("secret"));
	}

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
