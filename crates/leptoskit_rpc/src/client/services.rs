//! The clients for operations another service runs, one per service.
//!
//! An operation declared with `#[rpc::query(service = "analytics")]` is
//! sent to the `analytics` service rather than the app's own server:
//!
//! - in the browser and in a native app, to
//!   `{origin}/_leptoskit/services/analytics/api/<name>`, on the app's origin,
//!   which the edge (or `leptoskit dev`) forwards to the service. The app's own
//!   credentials don't go along: only those
//!   [`ClientConfig::service_credentials`] names for the service do. In the
//!   browser the page's cookies still go with every same-origin request; the
//!   edge strips them for services the app doesn't trust.
//! - from a Rust server, such as the app's while it renders, to
//!   `$LEPTOSKIT_SERVICE_ANALYTICS_URL/_leptoskit/services/analytics/api/
//!   <name>`, as an anonymous caller: what the server renders is what the
//!   browser would fetch. [`call_as_app`](super::call_as_app) is the one way to
//!   call as the app, signed with `$LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`; no
//!   query or live hook ever does, and the token never reaches the browser.
//!
//! When the app forwards identity to the service (`forward_identity` in its
//! `[services.<name>]` table), the browser and a native app sign the
//! service's calls and its live connection with a token the app's server
//! mints for the signed-in person and that service alone ([`IdentityToken`]),
//! and the server signs the calls it makes while rendering a page with one
//! for the person it renders for. Either way the service sees the same person,
//! so a rendered page still holds only what that person's browser could fetch.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_query::BoxFuture;
use serde_json::Value;

use super::Api;
use super::ClientConfig;
use super::Key;
use crate::FnError;

/// The configuration [`configure`](super::configure) was last given, which
/// every service's client starts from.
static CONFIG: RwLock<Option<ClientConfig>> = RwLock::new(None);

/// Each service's client, made on its first call.
static CLIENTS: RwLock<Option<HashMap<&'static str, Arc<dyn Api>>>> = RwLock::new(None);

/// Keeps `config` for the services' clients, and forgets the ones made
/// from the previous configuration.
pub(crate) fn remember(config: &ClientConfig) {
	*CONFIG.write().unwrap_or_else(PoisonError::into_inner) = Some(config.clone());
	*CLIENTS.write().unwrap_or_else(PoisonError::into_inner) = None;
}

/// The configuration [`configure`](super::configure) was last given, or
/// the default: the page's own origin.
pub(crate) fn config() -> ClientConfig {
	CONFIG
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.clone()
		.unwrap_or_default()
}

/// The client for the service `service`.
pub(crate) fn api(service: &'static str) -> Arc<dyn Api> {
	if let Some(api) = CLIENTS
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.as_ref()
		.and_then(|clients| clients.get(service))
	{
		return Arc::clone(api);
	}

	let mut clients = CLIENTS.write().unwrap_or_else(PoisonError::into_inner);
	let api = clients
		.get_or_insert_with(HashMap::new)
		.entry(service)
		.or_insert_with(|| backend(service, &config()));
	Arc::clone(api)
}

/// The origin a service's routes live under, for a client whose origin is
/// `origin`: `{origin}/_leptoskit/services/<service>`.
pub(crate) fn service_origin(origin: Option<&str>, service: &str) -> String {
	format!(
		"{}{}",
		origin.unwrap_or_default().trim_end_matches('/'),
		crate::service_prefix(service)
	)
}

/// The app's configuration as a service's client sees it: the same
/// origin, the credentials [`ClientConfig::service_credentials`] names for
/// `service` and no others, and no offline queue. The queue, its storage
/// and its status belong to the app's own connection: a service's
/// mutations go over HTTP whenever its socket is down.
///
/// A service the app names no credentials for gets, in the browser and in
/// a native app, an [`IdentityToken`] for it: the signed-in person's token
/// for that service, when the app forwards identity to it, and nothing
/// otherwise.
pub(crate) fn for_service(config: &ClientConfig, service: &str) -> ClientConfig {
	ClientConfig {
		origin: config.origin.clone(),
		credentials: config
			.service_credentials
			.get(service)
			.cloned()
			.or_else(|| identity_credentials(service)),
		offline_queue: false,
		offline_store: None,
		service_credentials: std::collections::BTreeMap::new(),
	}
}

/// In the browser and a native app: asks the app for the signed-in person's
/// token for `service`.
#[cfg(target_arch = "wasm32")]
#[expect(
	clippy::unnecessary_wraps,
	reason = "a server's version has nothing to give"
)]
fn identity_credentials(service: &str) -> Option<Arc<dyn super::Credentials>> {
	Some(Arc::new(IdentityToken::new(service, super::api)))
}

/// A server signs its calls to a service itself: see [`FromEnvironment`].
#[cfg(not(target_arch = "wasm32"))]
fn identity_credentials(_service: &str) -> Option<Arc<dyn super::Credentials>> {
	None
}

/// Bumped whenever the app's sign-in may have changed
/// ([`reauthenticate`](super::reauthenticate)), so a token fetched for
/// whoever was signed in before is never sent after.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Forgets every service's token: the person they named may have signed
/// out, or someone else signed in.
pub(crate) fn forget_identities() {
	GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// How long before a token runs out the client asks for a new one.
const RENEW_BEFORE_MS: u64 = 60_000;

/// How long the client goes without asking again after the app gave no
/// token: nobody is signed in, the app doesn't forward identity to the
/// service, or the app couldn't be asked. A sign-in asks again at once.
const UNSIGNED_FOR_MS: u64 = 300_000;

/// What signs a service's calls in the browser and in a native app: the
/// signed-in person's token for that service, which the app's server mints
/// (`rpc.service_token`, over the app's own client and credentials) and
/// which no other service accepts.
///
/// It keeps the token until a minute before it runs out, asks again when
/// the service refuses it, and forgets it on
/// [`reauthenticate`](super::reauthenticate), which `leptoskit_auth`'s client
/// calls on every sign-in and sign-out. When the app gives no token, calls
/// go without one: they're anonymous, never more than that.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(dead_code, reason = "a server signs its calls to a service itself")
)]
pub(crate) struct IdentityToken {
	service: String,
	/// The app's own client, which asks for the token.
	app: Arc<dyn Fn() -> Arc<dyn Api> + Send + Sync>,
	held: Mutex<Option<Held>>,
}

/// What a service's client holds for it now.
enum Kept {
	/// `Bearer <token>`.
	Token(String),
	/// The app gave no token: call anonymously.
	Unsigned,
	/// Nothing current: ask the app.
	Nothing,
}

/// A token, or the lack of one, and how long to keep it.
struct Held {
	generation: u64,
	/// `Bearer <token>`, or `None` to call anonymously.
	authorization: Option<String>,
	/// When to ask again, in [`leptoskit_query::now_ms`] milliseconds.
	until_ms: u64,
}

#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(dead_code, reason = "a server signs its calls to a service itself")
)]
impl IdentityToken {
	pub(crate) fn new(
		service: &str,
		app: impl Fn() -> Arc<dyn Api> + Send + Sync + 'static,
	) -> Self {
		Self {
			service: service.to_owned(),
			app: Arc::new(app),
			held: Mutex::new(None),
		}
	}

	/// What's held, if it's still current.
	fn current(&self) -> Kept {
		let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
		let current = held.as_ref().filter(|held| {
			held.generation == GENERATION.load(Ordering::SeqCst)
				&& leptoskit_query::now_ms() < held.until_ms
		});

		match current {
			Some(Held {
				authorization: Some(authorization),
				..
			}) => Kept::Token(authorization.clone()),
			Some(_) => Kept::Unsigned,
			None => Kept::Nothing,
		}
	}

	/// Asks the app for a token, and keeps the answer. A sign-in or
	/// sign-out while it was asked makes the answer someone else's, so the
	/// call then goes without one.
	async fn fetch(&self) -> Option<String> {
		let generation = GENERATION.load(Ordering::SeqCst);
		let asked = leptoskit_query::now_ms();
		let answer = (self.app)()
			.call(
				crate::service::SERVICE_TOKEN_OPERATION,
				serde_json::json!({ "service": self.service }),
				Key::None,
			)
			.await;
		let minted = answer
			.ok()
			.and_then(|value| serde_json::from_value::<Option<crate::ServiceToken>>(value).ok())
			.flatten();
		let (authorization, until_ms) = match minted {
			Some(minted) => {
				let lasts = minted
					.expires_in
					.saturating_mul(1000)
					.saturating_sub(RENEW_BEFORE_MS);
				(
					Some(format!("Bearer {}", minted.token)),
					asked.saturating_add(lasts),
				)
			}
			None => (None, asked.saturating_add(UNSIGNED_FOR_MS)),
		};

		if GENERATION.load(Ordering::SeqCst) != generation {
			return None;
		}

		*self.held.lock().unwrap_or_else(PoisonError::into_inner) = Some(Held {
			generation,
			authorization: authorization.clone(),
			until_ms,
		});
		authorization
	}
}

impl super::Credentials for IdentityToken {
	fn authorization(&self) -> BoxFuture<'_, Result<Option<String>, FnError>> {
		Box::pin(async move {
			match self.current() {
				Kept::Token(authorization) => Ok(Some(authorization)),
				Kept::Unsigned => Ok(None),
				Kept::Nothing => Ok(self.fetch().await),
			}
		})
	}

	/// The service refused `sent`, or its live connection asked for a new
	/// one: asks the app again, unless another call already has.
	fn refused(&self, sent: Option<String>) -> BoxFuture<'_, Result<bool, FnError>> {
		Box::pin(async move {
			if let Kept::Token(current) = self.current()
				&& Some(&current) != sent.as_ref()
			{
				return Ok(true);
			}

			let renewed = self.fetch().await;
			Ok(renewed.is_some() && renewed != sent)
		})
	}
}

/// The app's configuration, pointed at `service`'s routes: see
/// [`for_service`].
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(dead_code, reason = "a server reaches services by their environment")
)]
pub(crate) fn at_service(config: &ClientConfig, service: &str) -> ClientConfig {
	let config = for_service(config, service);
	ClientConfig {
		origin: Some(service_origin(config.origin.as_deref(), service)),
		..config
	}
}

/// In a native app: the HTTP plugin, to the app's origin.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
fn backend(service: &'static str, config: &ClientConfig) -> Arc<dyn Api> {
	Arc::new(super::protocol::Remote::new(
		at_service(config, service),
		super::native::Plugin,
	))
}

/// In the browser: `fetch`, to the page's origin.
#[cfg(all(feature = "web", not(feature = "guest"), target_arch = "wasm32"))]
fn backend(service: &'static str, config: &ClientConfig) -> Arc<dyn Api> {
	Arc::new(super::protocol::Remote::new(
		at_service(config, service),
		super::browser::Fetch,
	))
}

/// From a server: HTTP to the service's private URL.
#[cfg(all(feature = "services", not(target_arch = "wasm32")))]
fn backend(service: &'static str, _config: &ClientConfig) -> Arc<dyn Api> {
	Arc::new(FromEnvironment::new(
		service,
		super::reqwest::Reqwest::shared(),
		|name| std::env::var(name).ok(),
	))
}

/// From a server, as the app: HTTP to the service's private URL, signed
/// with the service's token. What [`call_as_app`](super::call_as_app)
/// sends with; made for each call, since only explicit calls use it.
#[cfg(all(feature = "services", not(target_arch = "wasm32")))]
pub(crate) fn as_app(service: &'static str) -> Arc<dyn Api> {
	Arc::new(
		FromEnvironment::new(service, super::reqwest::Reqwest::shared(), |name| {
			std::env::var(name).ok()
		})
		.signed_as_app(),
	)
}

/// Anywhere else there's no way to reach a service, and each call says so.
#[cfg(not(any(
	all(any(feature = "guest", feature = "web"), target_arch = "wasm32"),
	all(feature = "services", not(target_arch = "wasm32")),
)))]
fn backend(service: &'static str, _config: &ClientConfig) -> Arc<dyn Api> {
	Arc::new(Refuse(format!(
		"this build can't reach the `{service}` service: turn on leptoskit_rpc's `web` feature \
		 for the browser, `guest` for a native app, or `services` for a server"
	)))
}

/// Answers every call with the same error.
#[cfg_attr(
	any(
		all(any(feature = "guest", feature = "web"), target_arch = "wasm32"),
		all(feature = "services", not(target_arch = "wasm32")),
	),
	allow(
		dead_code,
		reason = "only a build with no way to reach a service uses it"
	)
)]
struct Refuse(String);

impl Api for Refuse {
	fn call(
		&self,
		_name: &'static str,
		_arguments: Value,
		_key: Key,
	) -> BoxFuture<'static, Result<Value, FnError>> {
		Box::pin(std::future::ready(Err(FnError::Other(self.0.clone()))))
	}
}

/// Reads an environment variable by name.
type Lookup = dyn Fn(&str) -> Option<String> + Send + Sync;

/// A server's client for one service: reads where the service is, and its
/// token, from the environment on each call, so a service started after the
/// app is still found. It calls as an anonymous caller unless made with
/// [`signed_as_app`](Self::signed_as_app).
#[cfg_attr(
	not(all(feature = "services", not(target_arch = "wasm32"))),
	allow(
		dead_code,
		reason = "only a server reaches services by their environment"
	)
)]
pub(crate) struct FromEnvironment<H> {
	service: &'static str,
	http: H,
	/// Whether calls carry the service's token, so the service identifies
	/// them as the app.
	as_app: bool,
	/// Reads an environment variable: the process's own, or a test's.
	lookup: Arc<Lookup>,
}

#[cfg_attr(
	not(all(feature = "services", not(target_arch = "wasm32"))),
	allow(
		dead_code,
		reason = "only a server reaches services by their environment"
	)
)]
impl<H: super::protocol::Http + Clone> FromEnvironment<H> {
	pub(crate) fn new(
		service: &'static str,
		http: H,
		lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
	) -> Self {
		Self {
			service,
			http,
			as_app: false,
			lookup: Arc::new(lookup),
		}
	}

	/// Calls as the app: with `$LEPTOSKIT_SERVICE_<NAME>_TOKEN`.
	#[must_use]
	pub(crate) fn signed_as_app(mut self) -> Self {
		self.as_app = true;
		self
	}

	fn read(&self, suffix: &str) -> Option<String> {
		(self.lookup)(&crate::service_env_name(self.service, suffix))
			.filter(|value| !value.trim().is_empty())
	}
}

impl<H: super::protocol::Http + Clone> Api for FromEnvironment<H> {
	/// Anonymous, signed as the app, or, while the server renders a page for
	/// someone and the app forwards identity to the service, signed with
	/// that person's token for it.
	fn call(
		&self,
		name: &'static str,
		arguments: Value,
		key: Key,
	) -> BoxFuture<'static, Result<Value, FnError>> {
		let Some(url) = self.read("URL") else {
			let variable = crate::service_env_name(self.service, "URL");
			return Box::pin(std::future::ready(Err(FnError::Other(format!(
				"`{name}` runs in the `{}` service, and {variable} isn't set: run the app with \
				 `leptoskit dev` or `leptoskit serve`, or set it to the service's private URL",
				self.service
			)))));
		};
		let credentials = if self.as_app {
			let Some(token) = self.read("TOKEN") else {
				let variable = crate::service_env_name(self.service, "TOKEN");
				return Box::pin(std::future::ready(Err(FnError::Other(format!(
					"`{name}` is called as the app, and {variable} isn't set: run the app with \
					 `leptoskit dev` or `leptoskit serve`, or set it to the `{}` service's token",
					self.service
				)))));
			};
			Some(Arc::new(Bearer(format!("Bearer {token}"))) as Arc<dyn super::Credentials>)
		} else {
			None
		};
		let origin = service_origin(Some(&url), self.service);

		#[cfg(any(feature = "postgres", feature = "sqlite"))]
		if !self.as_app && crate::service::forwards_identity(self.service, &*self.lookup) {
			// The request being rendered, read before the first await, while
			// its context is current.
			let parts = leptos::prelude::use_context::<http::request::Parts>();
			let (service, lookup, http) =
				(self.service, Arc::clone(&self.lookup), self.http.clone());
			return Box::pin(async move {
				let credentials = match parts {
					Some(parts) => forwarded(service, &parts, &*lookup).await,
					None => None,
				};
				let config = ClientConfig {
					origin: Some(origin),
					credentials,
					..ClientConfig::default()
				};
				super::protocol::Remote::new(config, http)
					.call(name, arguments, key)
					.await
			});
		}

		let config = ClientConfig {
			origin: Some(origin),
			credentials,
			..ClientConfig::default()
		};
		super::protocol::Remote::new(config, self.http.clone()).call(name, arguments, key)
	}
}

/// While the server renders a page: the token for `service` that names the
/// person `parts` is from, as the installed server identifies them, or
/// `None` to call anonymously, as that person's browser would when nobody is
/// signed in.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
async fn forwarded(
	service: &str,
	parts: &http::request::Parts,
	lookup: &Lookup,
) -> Option<Arc<dyn super::Credentials>> {
	let server = crate::server::installed()?;
	let identity = server.identify_request(parts).await.ok()?;
	let minted = crate::server::service_identity_token_with(
		service,
		&identity,
		std::time::SystemTime::now(),
		lookup,
	)
	.ok()??;
	Some(Arc::new(Bearer(format!("Bearer {}", minted.token))))
}

/// A fixed `Authorization` header: a service's token.
#[cfg_attr(
	not(all(feature = "services", not(target_arch = "wasm32"))),
	allow(
		dead_code,
		reason = "only a server reaches services by their environment"
	)
)]
struct Bearer(String);

impl super::Credentials for Bearer {
	fn authorization(&self) -> BoxFuture<'_, Result<Option<String>, FnError>> {
		Box::pin(std::future::ready(Ok(Some(self.0.clone()))))
	}
}
