//! Who is calling between an app and its services: a request signed with a
//! service's token is identified by a caller name rather than as a person,
//! and one signed with a token the app minted for a signed-in person is
//! identified as that person.
//!
//! # Forwarded identity
//!
//! The app's server mints, for one service, a token that names the person a
//! request acts for (their [`Actor`]), lasts [`IDENTITY_LIFETIME`], and is
//! signed with a key only the app and that service hold:
//!
//! ```text
//! lkid1.<claims>.<signature>
//! claims    = base64url(JSON { "aud": service, "sub": person, "iat": secs, "exp": secs })
//! signature = base64url(HMAC-SHA256(key, "lkid1." + claims))
//! key       = HKDF-SHA256(salt = "leptoskit/service-identity/v1",
//!                         ikm  = the service's token,
//!                         info = the service's name)
//! ```
//!
//! The key comes from the token `leptoskit` already gives the app and the
//! service (`LEPTOSKIT_SERVICE_<NAME>_TOKEN` and `LEPTOSKIT_SERVICE_TOKEN`),
//! so nothing new is deployed. The label keeps it from ever equalling the
//! token itself or any other key made from it, and the service's name, in
//! the key and in `aud`, means a token minted for one service fails at every
//! other. A service can mint these for itself, which tells it nothing it
//! didn't know; it can't mint one any other service or the app accepts.

use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::Hmac;
use hmac::Mac as _;
use leptoskit_db::Actor;
use leptoskit_db::Context;
use leptoskit_query::BoxFuture;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;
use uuid::Uuid;

use super::Identify;
use super::Identity;
use super::IdentityLease;
use crate::Error;
use crate::ServiceToken;

type HmacSha256 = Hmac<Sha256>;

/// The caller name a service gives the app: what an operation reads from
/// `cx.identity().caller` when the app's server called it.
pub const APP_CALLER: &str = "app";

/// The prefix of the caller name the app gives a service:
/// `service:analytics`.
pub const SERVICE_CALLER_PREFIX: &str = "service:";

/// The prefix of the caller name a service gives a signed-in person the app
/// forwarded: `person:<id>`, with the person's [`Actor`] UUID.
pub const PERSON_CALLER_PREFIX: &str = "person:";

/// How long a token minted for a service lasts. Clients ask for a new one a
/// minute before it runs out; signing out stops them asking, and a token
/// already handed out works until it expires.
pub const IDENTITY_LIFETIME: Duration = Duration::from_secs(300);

/// The longest a service accepts a token for, whatever its claims say.
const MAX_LIFETIME: Duration = Duration::from_mins(15);

/// How far apart the app's and the service's clocks may be.
const CLOCK_LEEWAY: Duration = Duration::from_secs(30);

/// How long before a forwarded identity runs out a live connection asks
/// its client for a new token.
const REFRESH_BEFORE: Duration = Duration::from_secs(60);

/// What every forwarded-identity token starts with: its format's version.
const TOKEN_PREFIX: &str = "lkid1.";

/// The HKDF salt that keeps identity keys apart from the service token they
/// come from and from any other key made from it.
const KEY_LABEL: &[u8] = b"leptoskit/service-identity/v1";

/// The service's own variable for its token, as `leptoskit dev`, `leptoskit
/// serve`, and deploys set it.
const SERVICE_TOKEN: &str = "LEPTOSKIT_SERVICE_TOKEN";

/// The service's own variable for its name.
const SERVICE_NAME: &str = "LEPTOSKIT_SERVICE_NAME";

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
///
/// A service's callers can also be people the app forwards (see the
/// [module](self)): with [`forwarded_identity`](Self::forwarded_identity),
/// which [`for_service`](Self::for_service) sets up, a request with a token
/// the app minted for this service is identified as that person, with their
/// [`Actor`] and [`ForwardedIdentity`] in its context and the caller
/// `person:<id>`:
///
/// ```ignore
/// #[rpc::query(service = "analytics")]
/// pub async fn mine(cx: &rpc::QueryCx) -> rpc::Result<Vec<Event>> {
/// 	let Some(me) = cx.identity().context.get::<rpc::server::ForwardedIdentity>() else {
/// 		return Err(rpc::Error::unauthorized("sign in to see your events"));
/// 	};
/// 	// … the events of `me.person`
/// }
/// ```
///
/// A token for another service, an expired one, or one that was changed is
/// refused with `401`, which tells the client to fetch a new one.
pub struct ServiceCallers<I> {
	inner: I,
	/// Each token's SHA-256 and its caller name.
	callers: Vec<([u8; 32], String)>,
	/// The key of this service's forwarded identities, when it accepts them.
	identity: Option<IdentityKey>,
}

impl<I: Identify> ServiceCallers<I> {
	/// Knows no tokens yet: add them with [`caller`](Self::caller).
	pub fn new(inner: I) -> Self {
		Self {
			inner,
			callers: Vec::new(),
			identity: None,
		}
	}

	/// In a service: knows the app, which signs its calls with the token
	/// in `LEPTOSKIT_SERVICE_TOKEN`, as the caller [`APP_CALLER`], and the
	/// people the app forwards to the service `LEPTOSKIT_SERVICE_NAME`
	/// names: see [`forwarded_identity`](Self::forwarded_identity).
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

	/// Knows the people the app forwards to the service `service`, by the
	/// key both derive from that service's `token` (see the
	/// [module](self)). Only the app mints these, and only for a service
	/// whose table turns `forward_identity` on, so accepting them lets in
	/// nobody the app didn't vouch for. An empty token is ignored.
	#[must_use]
	pub fn forwarded_identity(mut self, service: &str, token: impl AsRef<str>) -> Self {
		self.identity = IdentityKey::new(service, token.as_ref()).or(self.identity);
		self
	}

	/// [`for_service`](Self::for_service), reading the variables with
	/// `lookup`.
	pub(crate) fn service_with(inner: I, lookup: impl Fn(&str) -> Option<String>) -> Self {
		let mut callers = Self::new(inner);

		if let Some(token) = lookup(SERVICE_TOKEN) {
			if let Some(name) = lookup(SERVICE_NAME) {
				callers = callers.forwarded_identity(name.trim(), &token);
			}
			callers = callers.caller(token, APP_CALLER);
		}

		callers
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

	/// The caller name `token` belongs to, if any.
	fn known(&self, token: &str) -> Option<&str> {
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

/// The bearer token `request` carries, if any.
fn bearer(request: &http::request::Parts) -> Option<&str> {
	Some(
		request
			.headers
			.get(http::header::AUTHORIZATION)?
			.to_str()
			.ok()?
			.strip_prefix("Bearer ")?
			.trim(),
	)
}

impl<I: Identify> Identify for ServiceCallers<I> {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, Result<Identity, Error>> {
		let Some(token) = bearer(request) else {
			return self.inner.identify(request);
		};

		if let Some(key) = &self.identity
			&& token.starts_with(TOKEN_PREFIX)
		{
			let identity = key
				.verify(token, SystemTime::now())
				.map(|forwarded| forwarded.identity());
			return Box::pin(std::future::ready(identity));
		}

		match self.known(token) {
			Some(caller) => {
				let identity = Identity::anonymous().with_caller(caller);
				Box::pin(std::future::ready(Ok(identity)))
			}
			None => self.inner.identify(request),
		}
	}

	/// A forwarded person keeps a live connection until their token runs
	/// out, and is asked for a new one a minute before. Everyone else is as
	/// `inner` says.
	fn lease<'a>(&'a self, identity: &'a Identity) -> BoxFuture<'a, Result<IdentityLease, Error>> {
		match identity.context.get::<ForwardedIdentity>() {
			Some(forwarded) if self.identity.is_some() => {
				Box::pin(std::future::ready(Ok(forwarded.lease())))
			}
			_ => self.inner.lease(identity),
		}
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
			.field(
				"forwarded_identity",
				&self.identity.as_ref().map(IdentityKey::service),
			)
			.finish_non_exhaustive()
	}
}

/// A signed-in person the app forwarded to this service: what a
/// [`ServiceCallers`] puts in the context of a request signed with a token
/// the app minted, beside the person's [`Actor`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardedIdentity {
	/// The person, as the app knows them: their [`Actor`]'s UUID, such as
	/// a `leptoskit_auth` account's ID.
	pub person: Uuid,
	/// When the token stops working.
	pub expires_at: SystemTime,
}

impl ForwardedIdentity {
	/// The identity an operation sees: the person's [`Actor`], this, and the
	/// caller `person:<id>`.
	fn identity(&self) -> Identity {
		Identity::new(
			Context::new()
				.with(Actor::from_uuid(self.person))
				.with(self.clone()),
		)
		.with_caller(self.caller())
	}

	fn caller(&self) -> String {
		format!("{PERSON_CALLER_PREFIX}{}", self.person)
	}

	/// Valid until the token runs out; asked to refresh a minute before.
	/// Every token for the same person reads the same rows, so a renewed
	/// one keeps the connection's results.
	fn lease(&self) -> IdentityLease {
		IdentityLease {
			continuity: Some(self.caller()),
			reads: Vec::new(),
			expires_at: Some(self.expires_at),
			refresh_at: Some(
				self.expires_at
					.checked_sub(REFRESH_BEFORE)
					.unwrap_or(self.expires_at),
			),
		}
	}
}

/// The key the app and one service share for that service's forwarded
/// identities, derived from the service's token: see the [module](self).
///
/// The app's server mints with it through [`service_identity_token`]; a
/// service verifies with it through [`ServiceCallers`]. Build one by hand
/// only to mint tokens in a test, or from an app server that doesn't run
/// `leptoskit_rpc`'s own.
#[derive(Clone)]
pub struct IdentityKey {
	service: String,
	key: [u8; 32],
}

/// What a forwarded-identity token claims.
#[derive(Serialize, Deserialize)]
struct Claims {
	/// The one service that accepts it.
	aud: String,
	/// The person: their [`Actor`]'s UUID.
	sub: String,
	/// When it was minted, in seconds since the Unix epoch.
	iat: u64,
	/// When it stops working, in seconds since the Unix epoch.
	exp: u64,
}

impl IdentityKey {
	/// The key for the service `service`, whose token is `token`; `None`
	/// when the token is empty.
	#[must_use]
	pub fn new(service: &str, token: &str) -> Option<Self> {
		let token = token.trim();

		if token.is_empty() {
			return None;
		}

		// HKDF-SHA256 (RFC 5869) for one 32-byte block: extract with the
		// label as salt, then expand with the service's name as info.
		let mut extract = HmacSha256::new_from_slice(KEY_LABEL).expect("HMAC takes any key");
		extract.update(token.as_bytes());
		let pseudorandom = extract.finalize().into_bytes();
		let mut expand = HmacSha256::new_from_slice(&pseudorandom).expect("HMAC takes any key");
		expand.update(service.as_bytes());
		expand.update(&[1]);

		Some(Self {
			service: service.to_owned(),
			key: expand.finalize().into_bytes().into(),
		})
	}

	/// The service whose tokens it mints and verifies.
	#[must_use]
	pub fn service(&self) -> &str {
		&self.service
	}

	/// A token naming `person`, minted at `issued_at`, that this key's
	/// service accepts for `lifetime`.
	#[must_use]
	pub fn mint(&self, person: Uuid, issued_at: SystemTime, lifetime: Duration) -> String {
		let issued = seconds(issued_at);
		let claims = Claims {
			aud: self.service.clone(),
			sub: person.to_string(),
			iat: issued,
			exp: issued.saturating_add(lifetime.as_secs()),
		};
		let claims =
			URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("the claims are plain JSON"));
		let signature = URL_SAFE_NO_PAD.encode(self.sign(&claims).finalize().into_bytes());
		format!("{TOKEN_PREFIX}{claims}.{signature}")
	}

	/// The person `token` names, if this key's service may accept it at
	/// `now`.
	///
	/// # Errors
	///
	/// [`Error::Unauthorized`] when the token isn't one of this key's, was
	/// changed, is for another service, has expired, or claims to last
	/// longer than a service accepts.
	pub fn verify(&self, token: &str, now: SystemTime) -> Result<ForwardedIdentity, Error> {
		let refused =
			|| Error::unauthorized("the service's token for the signed-in person isn't valid");
		let (claims, signature) = token
			.strip_prefix(TOKEN_PREFIX)
			.and_then(|rest| rest.split_once('.'))
			.ok_or_else(refused)?;
		let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| refused())?;
		// `verify_slice` compares in constant time.
		self.sign(claims)
			.verify_slice(&signature)
			.map_err(|_| refused())?;
		let claims: Claims = URL_SAFE_NO_PAD
			.decode(claims)
			.ok()
			.and_then(|json| serde_json::from_slice(&json).ok())
			.ok_or_else(refused)?;

		if claims.aud != self.service {
			return Err(Error::unauthorized(format!(
				"the token is for the `{}` service, not this one",
				claims.aud
			)));
		}

		let now = seconds(now);
		let leeway = CLOCK_LEEWAY.as_secs();

		if claims.exp <= claims.iat || claims.exp - claims.iat > MAX_LIFETIME.as_secs() {
			return Err(refused());
		}

		if claims.iat > now.saturating_add(leeway) || claims.exp.saturating_add(leeway) <= now {
			return Err(Error::unauthorized(
				"the service's token for the signed-in person has expired",
			));
		}

		let person = Uuid::parse_str(&claims.sub).map_err(|_| refused())?;

		Ok(ForwardedIdentity {
			person,
			expires_at: UNIX_EPOCH + Duration::from_secs(claims.exp),
		})
	}

	/// The MAC over a token's encoded claims.
	fn sign(&self, claims: &str) -> HmacSha256 {
		let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC takes any key");
		mac.update(TOKEN_PREFIX.as_bytes());
		mac.update(claims.as_bytes());
		mac
	}
}

impl std::fmt::Debug for IdentityKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("IdentityKey")
			.field("service", &self.service)
			.finish_non_exhaustive()
	}
}

/// In the app's server: a token that tells the service `service` who
/// `identity` is, for [`IDENTITY_LIFETIME`], or `None` when nobody is signed
/// in. It's what `rpc.service_token` answers browsers and native apps, and
/// what the app's server signs its own calls with while it renders a page
/// for someone.
///
/// The person is the identity's [`Actor`], which `leptoskit_auth` sets for
/// every signed-in request. Nothing else about them goes to the service.
///
/// # Errors
///
/// [`Error::Forbidden`] when the app doesn't forward identity to `service`
/// (`LEPTOSKIT_SERVICE_<NAME>_IDENTITY` isn't `forward`, because its table
/// doesn't turn `forward_identity` on), and a server error when the
/// service's token isn't set.
pub fn service_identity_token(
	service: &str,
	identity: &Identity,
) -> Result<Option<ServiceToken>, Error> {
	service_identity_token_with(service, identity, SystemTime::now(), |name| {
		std::env::var(name).ok()
	})
}

/// [`service_identity_token`] at `now`, reading the variables with
/// `lookup`.
pub(crate) fn service_identity_token_with(
	service: &str,
	identity: &Identity,
	now: SystemTime,
	lookup: impl Fn(&str) -> Option<String>,
) -> Result<Option<ServiceToken>, Error> {
	if !crate::is_service_name(service) {
		return Err(Error::invalid(format!(
			"`{service}` isn't a service's name"
		)));
	}

	if !crate::service::forwards_identity(service, &lookup) {
		return Err(Error::forbidden(format!(
			"the app doesn't tell the `{service}` service who is signed in; turn on 			 \
			 `forward_identity` in [services.{service}] if it should know"
		)));
	}

	let Some(actor) = identity.context.get::<Actor>() else {
		return Ok(None);
	};
	let variable = crate::service_env_name(service, "TOKEN");
	let key = lookup(&variable)
		.and_then(|token| IdentityKey::new(service, &token))
		.ok_or_else(|| {
			Error::server(format!(
				"{variable} isn't set, so the app can't sign for the `{service}` service"
			))
		})?;

	Ok(Some(ServiceToken {
		token: key.mint(actor.uuid(), now, IDENTITY_LIFETIME),
		expires_in: IDENTITY_LIFETIME.as_secs(),
	}))
}

/// Seconds since the Unix epoch, or 0 before it.
fn seconds(time: SystemTime) -> u64 {
	time.duration_since(UNIX_EPOCH)
		.map_or(0, |since| since.as_secs())
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[allow(deprecated)]
const _: () = {
	inventory::submit! {
		super::Registration {
			name: <crate::ServiceTokenArgs as crate::Operation>::NAME,
			kind: <crate::ServiceTokenArgs as crate::Operation>::KIND,
			module_path: module_path!(),
			service: None,
			handler: super::handle::<crate::ServiceTokenArgs>,
		}
	}
};

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

	const PERSON: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);

	fn at(seconds: u64) -> SystemTime {
		UNIX_EPOCH + Duration::from_secs(seconds)
	}

	fn key() -> IdentityKey {
		IdentityKey::new("echo", "s3cret").expect("a key")
	}

	#[test]
	fn a_minted_token_names_its_person_to_its_service_alone() {
		let token = key().mint(PERSON, at(1_000), IDENTITY_LIFETIME);
		assert!(token.starts_with("lkid1."), "{token}");
		assert!(!token.contains("s3cret"));

		let forwarded = key().verify(&token, at(1_010)).expect("valid");
		assert_eq!(forwarded.person, PERSON);
		assert_eq!(forwarded.expires_at, at(1_300));

		// Another service's key: another name, or another token.
		for other in [
			IdentityKey::new("other", "s3cret").unwrap(),
			IdentityKey::new("echo", "different").unwrap(),
		] {
			assert!(matches!(
				other.verify(&token, at(1_010)),
				Err(Error::Unauthorized(_))
			));
		}

		// The key isn't the token, nor the other service's key.
		assert_ne!(key().key, digest("s3cret"));
		assert_ne!(key().key, IdentityKey::new("other", "s3cret").unwrap().key);
		assert!(IdentityKey::new("echo", "  ").is_none());
		assert!(!format!("{:?}", key()).contains("key:"));
	}

	#[test]
	fn a_changed_token_is_refused() {
		let token = key().mint(PERSON, at(1_000), IDENTITY_LIFETIME);
		let (head, signature) = token.rsplit_once('.').unwrap();
		let (_, claims) = head.split_once('.').unwrap();

		// Another person, signed with the original signature.
		let json = String::from_utf8(URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap();
		let forged = json.replace(&PERSON.to_string(), &Uuid::nil().to_string());
		let forged = format!(
			"lkid1.{}.{signature}",
			URL_SAFE_NO_PAD.encode(forged.as_bytes())
		);
		// A flipped signature, a missing one, and something else entirely.
		let mut flipped = token.clone().into_bytes();
		let last = flipped.len() - 1;
		flipped[last] = if flipped[last] == b'A' { b'B' } else { b'A' };
		let flipped = String::from_utf8(flipped).unwrap();

		for bad in [
			forged.as_str(),
			flipped.as_str(),
			head,
			"lkid1.",
			"lkid1.e30.",
			"s3cret",
		] {
			assert!(
				matches!(key().verify(bad, at(1_010)), Err(Error::Unauthorized(_))),
				"{bad}"
			);
		}
	}

	#[test]
	fn a_token_works_only_while_it_lasts_give_or_take_the_clocks() {
		let token = key().mint(PERSON, at(1_000), IDENTITY_LIFETIME);
		let verify = |now| key().verify(&token, at(now));

		// A service's clock a little behind the app's, or a little ahead.
		assert!(verify(980).is_ok());
		assert!(verify(1_329).is_ok());
		assert!(matches!(verify(1_330), Err(Error::Unauthorized(_))));
		assert!(matches!(verify(960), Err(Error::Unauthorized(_))));

		// A token that claims to last longer than a service accepts.
		let long = key().mint(PERSON, at(1_000), Duration::from_secs(3_600));
		assert!(key().verify(&long, at(1_010)).is_err());
		let none = key().mint(PERSON, at(1_000), Duration::ZERO);
		assert!(key().verify(&none, at(1_000)).is_err());
	}

	#[tokio::test]
	async fn a_service_identifies_a_forwarded_person_and_keeps_them_while_the_token_lasts() {
		let callers = ServiceCallers::service_with(Anonymous, |name| {
			match name {
				"LEPTOSKIT_SERVICE_TOKEN" => Some("s3cret".to_owned()),
				"LEPTOSKIT_SERVICE_NAME" => Some("echo".to_owned()),
				_ => None,
			}
		});
		let token = key().mint(PERSON, SystemTime::now(), IDENTITY_LIFETIME);
		let identity = callers
			.identify(&request(Some(&format!("Bearer {token}"))))
			.await
			.expect("a person");
		assert_eq!(
			identity.caller.as_deref(),
			Some(format!("person:{PERSON}").as_str())
		);
		assert_eq!(
			identity.context.get::<Actor>().map(Actor::uuid),
			Some(PERSON)
		);
		assert!(!is_service(&identity));

		let lease = callers.lease(&identity).await.unwrap();
		let forwarded = identity.context.get::<ForwardedIdentity>().unwrap();
		assert_eq!(lease.expires_at, Some(forwarded.expires_at));
		assert_eq!(
			lease.refresh_at,
			Some(forwarded.expires_at - Duration::from_secs(60))
		);
		assert_eq!(lease.continuity.as_deref(), identity.caller.as_deref());

		// The app still signs as the app, and a bad token is refused, not
		// passed on as anonymous.
		assert_eq!(
			caller(&callers, Some("Bearer s3cret")).await.as_deref(),
			Some("app")
		);
		let other = IdentityKey::new("other", "s3cret").unwrap().mint(
			PERSON,
			SystemTime::now(),
			IDENTITY_LIFETIME,
		);
		assert!(matches!(
			callers
				.identify(&request(Some(&format!("Bearer {other}"))))
				.await,
			Err(Error::Unauthorized(_))
		));
		assert!(format!("{callers:?}").contains("echo"));

		// Without a key, such a token is nobody special.
		let app = ServiceCallers::new(Anonymous);
		assert_eq!(caller(&app, Some(&format!("Bearer {token}"))).await, None);
	}

	#[test]
	fn the_app_mints_only_for_services_it_forwards_identity_to() {
		let signed_in = Identity::new(Context::new().with(Actor::from_uuid(PERSON)));
		let lookup = |forward: bool| {
			move |name: &str| {
				match name {
					"LEPTOSKIT_SERVICE_ECHO_TOKEN" => Some("s3cret".to_owned()),
					"LEPTOSKIT_SERVICE_ECHO_IDENTITY" if forward => Some("forward".to_owned()),
					_ => None,
				}
			}
		};

		let minted = service_identity_token_with("echo", &signed_in, at(1_000), lookup(true))
			.unwrap()
			.expect("a token");
		assert_eq!(minted.expires_in, 300);
		assert_eq!(
			key().verify(&minted.token, at(1_000)).unwrap().person,
			PERSON
		);

		// Nobody signed in: nothing to forward.
		assert_eq!(
			service_identity_token_with("echo", &Identity::anonymous(), at(1_000), lookup(true)),
			Ok(None)
		);
		// A service the app doesn't forward identity to.
		assert!(matches!(
			service_identity_token_with("echo", &signed_in, at(1_000), lookup(false)),
			Err(Error::Forbidden(_))
		));
		// Forwarded, but without its token.
		let no_token =
			|name: &str| (name == "LEPTOSKIT_SERVICE_ECHO_IDENTITY").then(|| "forward".to_owned());
		assert!(matches!(
			service_identity_token_with("echo", &signed_in, at(1_000), no_token),
			Err(Error::Server(_))
		));
		assert!(matches!(
			service_identity_token_with("Echo!", &signed_in, at(1_000), lookup(true)),
			Err(Error::Invalid(_))
		));
	}
}
