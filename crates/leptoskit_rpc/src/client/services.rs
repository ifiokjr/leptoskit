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

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

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
pub(crate) fn for_service(config: &ClientConfig, service: &str) -> ClientConfig {
	ClientConfig {
		origin: config.origin.clone(),
		credentials: config.service_credentials.get(service).cloned(),
		offline_queue: false,
		offline_store: None,
		service_credentials: std::collections::BTreeMap::new(),
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
		let config = ClientConfig {
			origin: Some(service_origin(Some(&url), self.service)),
			credentials,
			..ClientConfig::default()
		};
		super::protocol::Remote::new(config, self.http.clone()).call(name, arguments, key)
	}
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
