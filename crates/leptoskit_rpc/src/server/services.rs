//! Who is calling between an app and its services: a request signed with a
//! service's token is identified by a caller name rather than as a person.

use leptoskit_query::BoxFuture;
use sha2::Digest as _;
use sha2::Sha256;

use super::Identify;
use super::Identity;
use super::IdentityLease;
use crate::Error;

/// The caller name a service gives the app: what an operation reads from
/// `cx.identity().caller` when the app's server called it.
pub const APP_CALLER: &str = "app";

/// The prefix of the caller name the app gives a service:
/// `service:analytics`.
pub const SERVICE_CALLER_PREFIX: &str = "service:";

/// The app's own variable for a service's token, as `leptoskit dev`,
/// `leptoskit serve`, and deploys set it for each service.
const SERVICE_TOKEN: &str = "LEPTOSKIT_SERVICE_TOKEN";

/// An [`Identify`] that knows the tokens an app and its services sign
/// their calls to each other with, and asks `inner` about everyone else.
///
/// A request with `Authorization: Bearer <token>` for a known token is
/// identified as an anonymous request with that token's caller name, so an
/// operation can let only that caller in:
///
/// ```ignore
/// #[rpc::query(service = "analytics")]
/// pub async fn summary(cx: &rpc::QueryCx, days: u32) -> rpc::Result<Summary> {
/// 	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
/// 		return Err(rpc::Error::forbidden("only the app reads the summary"));
/// 	}
/// 	// …
/// }
/// ```
///
/// In a service, [`for_service`](Self::for_service) knows the app, by the
/// service's own `LEPTOSKIT_SERVICE_TOKEN`. In the app,
/// [`for_app`](Self::for_app) knows every service, by each
/// `LEPTOSKIT_SERVICE_<NAME>_TOKEN`, as `service:<name>`.
pub struct ServiceCallers<I> {
	inner: I,
	/// Each token's SHA-256 and its caller name.
	callers: Vec<([u8; 32], String)>,
}

impl<I: Identify> ServiceCallers<I> {
	/// Knows no tokens yet: add them with [`caller`](Self::caller).
	pub fn new(inner: I) -> Self {
		Self {
			inner,
			callers: Vec::new(),
		}
	}

	/// In a service: knows the app, which signs its calls with the token
	/// in `LEPTOSKIT_SERVICE_TOKEN`, as the caller [`APP_CALLER`].
	pub fn for_service(inner: I) -> Self {
		Self::service_with(inner, |name| std::env::var(name).ok())
	}

	/// In the app: knows each service, which signs its calls with the token
	/// in `LEPTOSKIT_SERVICE_<NAME>_TOKEN`, as the caller `service:<name>`,
	/// with the name lowercased and `_` written as `-`.
	pub fn for_app(inner: I) -> Self {
		Self::app_with(inner, std::env::vars())
	}

	/// Knows `token` as the caller `name`. An empty token is ignored.
	#[must_use]
	pub fn caller(mut self, token: impl AsRef<str>, name: impl Into<String>) -> Self {
		let token = token.as_ref().trim();

		if !token.is_empty() {
			self.callers.push((digest(token), name.into()));
		}

		self
	}

	/// [`for_service`](Self::for_service), reading the variable with
	/// `lookup`.
	pub(crate) fn service_with(inner: I, lookup: impl Fn(&str) -> Option<String>) -> Self {
		let callers = Self::new(inner);

		match lookup(SERVICE_TOKEN) {
			Some(token) => callers.caller(token, APP_CALLER),
			None => callers,
		}
	}

	/// [`for_app`](Self::for_app), over the variables `vars`.
	pub(crate) fn app_with(inner: I, vars: impl IntoIterator<Item = (String, String)>) -> Self {
		let mut callers = Self::new(inner);

		for (name, value) in vars {
			let Some(service) = name
				.strip_prefix("LEPTOSKIT_SERVICE_")
				.and_then(|rest| rest.strip_suffix("_TOKEN"))
				.filter(|service| !service.is_empty())
			else {
				continue;
			};
			let service = service.to_ascii_lowercase().replace('_', "-");
			callers = callers.caller(value, format!("{SERVICE_CALLER_PREFIX}{service}"));
		}

		callers
	}

	/// The caller name `request`'s bearer token belongs to, if any.
	fn known(&self, request: &http::request::Parts) -> Option<&str> {
		let token = request
			.headers
			.get(http::header::AUTHORIZATION)?
			.to_str()
			.ok()?
			.strip_prefix("Bearer ")?
			.trim();
		let sent = digest(token);
		// Every token is compared, so the time taken doesn't say which matched.
		let mut found = None;

		for (known, name) in &self.callers {
			if equal(known, &sent) {
				found = Some(name.as_str());
			}
		}

		found
	}
}

impl<I: Identify> Identify for ServiceCallers<I> {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, Result<Identity, Error>> {
		match self.known(request) {
			Some(caller) => {
				let identity = Identity::anonymous().with_caller(caller);
				Box::pin(std::future::ready(Ok(identity)))
			}
			None => self.inner.identify(request),
		}
	}

	fn lease<'a>(&'a self, identity: &'a Identity) -> BoxFuture<'a, Result<IdentityLease, Error>> {
		self.inner.lease(identity)
	}
}

impl<I> std::fmt::Debug for ServiceCallers<I> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ServiceCallers")
			.field(
				"callers",
				&self
					.callers
					.iter()
					.map(|(_, name)| name.as_str())
					.collect::<Vec<_>>(),
			)
			.finish_non_exhaustive()
	}
}

/// Whether `identity` is one of the app's services calling, as
/// [`ServiceCallers::for_app`] names them.
#[cfg(any(feature = "live", test))]
pub(crate) fn is_service(identity: &Identity) -> bool {
	identity
		.caller
		.as_deref()
		.is_some_and(|caller| caller.starts_with(SERVICE_CALLER_PREFIX))
}

fn digest(token: &str) -> [u8; 32] {
	Sha256::digest(token.as_bytes()).into()
}

/// Compares two digests in the same time whatever they hold.
fn equal(a: &[u8; 32], b: &[u8; 32]) -> bool {
	a.iter()
		.zip(b)
		.fold(0_u8, |difference, (x, y)| difference | (x ^ y))
		== 0
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::server::Anonymous;

	fn request(authorization: Option<&str>) -> http::request::Parts {
		let mut request = http::Request::builder().uri("/api/summary");

		if let Some(authorization) = authorization {
			request = request.header(http::header::AUTHORIZATION, authorization);
		}

		request.body(()).unwrap().into_parts().0
	}

	async fn caller(identify: &impl Identify, authorization: Option<&str>) -> Option<String> {
		identify
			.identify(&request(authorization))
			.await
			.unwrap()
			.caller
	}

	#[tokio::test]
	async fn a_service_knows_the_app_by_its_own_token() {
		let callers = ServiceCallers::service_with(Anonymous, |name| {
			(name == "LEPTOSKIT_SERVICE_TOKEN").then(|| "s3cret".to_owned())
		});

		assert_eq!(
			caller(&callers, Some("Bearer s3cret")).await.as_deref(),
			Some("app")
		);
		assert_eq!(caller(&callers, Some("Bearer wrong")).await, None);
		assert_eq!(caller(&callers, None).await, None);
	}

	#[tokio::test]
	async fn the_app_knows_each_service_by_its_token() {
		let callers = ServiceCallers::app_with(
			Anonymous,
			[
				("LEPTOSKIT_SERVICE_EVENT_STORE_TOKEN", "a"),
				("LEPTOSKIT_SERVICE_CARDS_TOKEN", "b"),
				("LEPTOSKIT_SERVICE_CARDS_URL", "http://127.0.0.1:1"),
				("LEPTOSKIT_SERVICE_TOKEN", "own"),
				("LEPTOSKIT_SERVICE_EMPTY_TOKEN", " "),
			]
			.map(|(name, value)| (name.to_owned(), value.to_owned())),
		);

		assert_eq!(
			caller(&callers, Some("Bearer a")).await.as_deref(),
			Some("service:event-store")
		);
		assert_eq!(
			caller(&callers, Some("Bearer b")).await.as_deref(),
			Some("service:cards")
		);
		assert_eq!(caller(&callers, Some("Bearer own")).await, None);
		assert_eq!(caller(&callers, Some("Bearer ")).await, None);
		assert_eq!(format!("{callers:?}").matches("service:").count(), 2);
	}

	#[test]
	fn a_service_caller_is_recognized() {
		assert!(is_service(
			&Identity::anonymous().with_caller("service:cards")
		));
		assert!(!is_service(&Identity::anonymous().with_caller("app")));
		assert!(!is_service(&Identity::anonymous()));
	}
}
