//! A service: a separate process the app runs beside its server.
//!
//! An app declares its services in `[services]` in `leptoskit.toml`, and
//! `leptoskit dev`, `leptoskit serve`, and the deploy generators start each
//! one with its environment set. This module is the service's half of that
//! contract:
//!
//! - [`ServiceEnv`] reads what the service was started with: its name, the
//!   address to bind, the token its trusted callers present, the app's URL, its
//!   database, and its volumes.
//! - [`ServiceEnv::serve`] serves the service's router with `/healthz` and
//!   `/readyz`, and drains on `SIGTERM`.
//! - [`RequireToken`] guards the routes only the app may call.
//! - [`main!`](crate::service::main) writes the whole `main` and makes sure the
//!   service's operations library is linked, so its operations exist.
//!
//! And the app's half: [`service_url`] and [`service_token`] read where a
//! service is and how to call it.
//!
//! Everything the service serves to browsers and native apps goes under
//! [`ServiceEnv::public_prefix`], `/_leptoskit/services/<name>`. The app's
//! origin forwards that prefix to the service unchanged.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use axum::Router;
use axum::http::HeaderValue;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use subtle::ConstantTimeEq;
use tower::Layer;
use tower::Service;

use crate::GracefulShutdown;
use crate::Readiness;
use crate::ServeError;

/// Where every service's public paths live on the app's origin:
/// `/_leptoskit/services/<name>`.
pub const SERVICES_PREFIX: &str = "/_leptoskit/services";

/// The service's name: `LEPTOSKIT_SERVICE_NAME`.
pub const NAME_ENV: &str = "LEPTOSKIT_SERVICE_NAME";
/// The address the service binds: `LEPTOSKIT_SERVICE_ADDR`.
pub const ADDR_ENV: &str = "LEPTOSKIT_SERVICE_ADDR";
/// The token its trusted callers present: `LEPTOSKIT_SERVICE_TOKEN`.
pub const TOKEN_ENV: &str = "LEPTOSKIT_SERVICE_TOKEN";
/// The app's public origin: `LEPTOSKIT_PUBLIC_ORIGIN`.
pub const PUBLIC_ORIGIN_ENV: &str = "LEPTOSKIT_PUBLIC_ORIGIN";
/// The app server's private base URL: `LEPTOSKIT_APP_URL`.
pub const APP_URL_ENV: &str = "LEPTOSKIT_APP_URL";
/// The service's own database: `DATABASE_URL`.
pub const DATABASE_URL_ENV: &str = "DATABASE_URL";
/// The service database's pool size, when its config sets `pool`:
/// `LEPTOSKIT_SERVICE_DATABASE_POOL`.
pub const DATABASE_POOL_ENV: &str = "LEPTOSKIT_SERVICE_DATABASE_POOL";
/// How long, in milliseconds, [`ServiceEnv::serve`] keeps accepting after
/// `SIGTERM`: `LEPTOSKIT_SERVICE_DRAIN_MS`. `leptoskit dev` sets 0, since
/// no load balancer needs the drain there; unset, it's
/// [`GracefulShutdown::DEFAULT_DRAIN_DELAY`].
pub const DRAIN_MS_ENV: &str = "LEPTOSKIT_SERVICE_DRAIN_MS";
/// The prefix of each volume's variable: `LEPTOSKIT_SERVICE_VOLUME_<VOLUME>`.
pub const VOLUME_ENV_PREFIX: &str = "LEPTOSKIT_SERVICE_VOLUME_";

/// The public path prefix of the service called `name`:
/// `/_leptoskit/services/<name>`.
#[must_use]
pub fn public_prefix(name: &str) -> String {
	format!("{SERVICES_PREFIX}/{name}")
}

/// The variable the app reads one of a service's values from:
/// `LEPTOSKIT_SERVICE_<NAME>_<SUFFIX>`, with the name uppercased and `-`
/// written `_`.
///
/// ```
/// use leptoskit_axum::service::env_name;
///
/// assert_eq!(
/// 	env_name("event-log", "URL"),
/// 	"LEPTOSKIT_SERVICE_EVENT_LOG_URL"
/// );
/// ```
#[must_use]
pub fn env_name(name: &str, suffix: &str) -> String {
	format!("LEPTOSKIT_SERVICE_{}_{suffix}", env_part(name))
}

fn env_part(name: &str) -> String {
	name.chars()
		.map(|c| {
			if c == '-' {
				'_'
			} else {
				c.to_ascii_uppercase()
			}
		})
		.collect()
}

/// Where the service called `name` is, for the app: the private base URL
/// from `LEPTOSKIT_SERVICE_<NAME>_URL`, with no trailing slash.
#[must_use]
pub fn service_url(name: &str) -> Option<String> {
	non_empty(std::env::var(env_name(name, "URL")).ok())
		.map(|url| url.trim_end_matches('/').to_owned())
}

/// The token the app presents to the service called `name`, from
/// `LEPTOSKIT_SERVICE_<NAME>_TOKEN`.
#[must_use]
pub fn service_token(name: &str) -> Option<String> {
	non_empty(std::env::var(env_name(name, "TOKEN")).ok())
}

fn non_empty(value: Option<String>) -> Option<String> {
	value.filter(|value| !value.is_empty())
}

/// What a service was started with.
///
/// `leptoskit dev`, `leptoskit serve`, and the deploy generators set these
/// variables; [`from_env`](Self::from_env) reads them.
///
/// ```no_run
/// use leptoskit_axum::axum::Router;
/// use leptoskit_axum::axum::routing::get;
/// use leptoskit_axum::service::ServiceEnv;
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let env = ServiceEnv::from_env()?;
/// let public = Router::new().route("/hello", get(|| async { "hello" }));
/// let app = Router::new().nest(&env.public_prefix(), public);
/// env.serve(app).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct ServiceEnv {
	name: String,
	address: SocketAddr,
	token: String,
	public_origin: Option<String>,
	app_url: Option<String>,
	database_url: Option<String>,
	database_pool: Option<usize>,
	drain_delay: Option<Duration>,
	volumes: BTreeMap<String, PathBuf>,
	readiness: Readiness,
}

impl ServiceEnv {
	/// Reads the service's environment.
	///
	/// # Errors
	///
	/// When `LEPTOSKIT_SERVICE_NAME`, `LEPTOSKIT_SERVICE_ADDR`, or
	/// `LEPTOSKIT_SERVICE_TOKEN` is missing or empty (the error names every
	/// one), or the address isn't a socket address such as `127.0.0.1:3100`.
	pub fn from_env() -> Result<Self, ServiceEnvError> {
		Self::from_vars(std::env::vars())
	}

	/// Reads the service's environment from `vars` instead of the process's,
	/// as a test does.
	///
	/// # Errors
	///
	/// As [`from_env`](Self::from_env).
	pub fn from_vars<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Result<Self, ServiceEnvError>
	where
		K: Into<String>,
		V: Into<String>,
	{
		let mut name = None;
		let mut address = None;
		let mut token = None;
		let mut public_origin = None;
		let mut app_url = None;
		let mut database_url = None;
		let mut database_pool = None;
		let mut drain_delay = None;
		let mut volumes = BTreeMap::new();

		for (key, value) in vars {
			let key = key.into();
			let value = Some(value.into());

			match key.as_str() {
				NAME_ENV => name = non_empty(value),
				ADDR_ENV => address = non_empty(value),
				TOKEN_ENV => token = non_empty(value),
				PUBLIC_ORIGIN_ENV => public_origin = non_empty(value),
				APP_URL_ENV => app_url = non_empty(value),
				DATABASE_URL_ENV => database_url = non_empty(value),
				DRAIN_MS_ENV => {
					drain_delay = non_empty(value)
						.and_then(|ms| ms.trim().parse().ok())
						.map(Duration::from_millis);
				}
				DATABASE_POOL_ENV => {
					database_pool = non_empty(value).and_then(|pool| pool.trim().parse().ok());
				}
				_ => {
					if let (Some(volume), Some(path)) =
						(key.strip_prefix(VOLUME_ENV_PREFIX), non_empty(value))
						&& !volume.is_empty()
					{
						volumes.insert(volume.to_ascii_lowercase(), PathBuf::from(path));
					}
				}
			}
		}

		let missing: Vec<&'static str> = [
			(NAME_ENV, name.is_none()),
			(ADDR_ENV, address.is_none()),
			(TOKEN_ENV, token.is_none()),
		]
		.into_iter()
		.filter_map(|(variable, absent)| absent.then_some(variable))
		.collect();

		let (Some(name), Some(address), Some(token)) = (name, address, token) else {
			return Err(ServiceEnvError::Missing(missing));
		};
		let address = address
			.parse()
			.map_err(|source| ServiceEnvError::InvalidAddress { address, source })?;

		Ok(Self {
			name,
			address,
			token,
			public_origin: public_origin.map(|origin| origin.trim_end_matches('/').to_owned()),
			app_url: app_url.map(|url| url.trim_end_matches('/').to_owned()),
			database_url,
			database_pool,
			drain_delay,
			volumes,
			readiness: Readiness::new(),
		})
	}

	/// The service's name, as `[services.<name>]` declares it.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.name
	}

	/// Checks the service runs under `expected`, the `service = "…"` its
	/// operations are declared with. Under any other name every call the app
	/// makes goes to a path and variables this process doesn't answer, so a
	/// service checks this first, before it serves anything.
	///
	/// ```
	/// # use leptoskit_axum::service::ServiceEnv;
	/// let env = ServiceEnv::from_vars([
	/// 	("LEPTOSKIT_SERVICE_NAME", "stats"),
	/// 	("LEPTOSKIT_SERVICE_ADDR", "127.0.0.1:3100"),
	/// 	("LEPTOSKIT_SERVICE_TOKEN", "secret"),
	/// ])
	/// .expect("env");
	/// assert!(env.expect_name("analytics").is_err());
	/// ```
	///
	/// # Errors
	///
	/// When the name differs, with a message naming both.
	pub fn expect_name(&self, expected: &str) -> Result<(), WrongName> {
		if self.name == expected {
			Ok(())
		} else {
			Err(WrongName {
				expected: expected.to_owned(),
				actual: self.name.clone(),
			})
		}
	}

	/// The address to bind.
	#[must_use]
	pub fn address(&self) -> SocketAddr {
		self.address
	}

	/// The token the service's trusted callers present as
	/// `Authorization: Bearer <token>`. The app presents it, and the service
	/// presents it to the app in turn.
	#[must_use]
	pub fn token(&self) -> &str {
		&self.token
	}

	/// The app's public origin, such as `https://example.com`, with no
	/// trailing slash.
	#[must_use]
	pub fn public_origin(&self) -> Option<&str> {
		self.public_origin.as_deref()
	}

	/// The app server's private base URL, with no trailing slash: where the
	/// service calls the app's operations.
	#[must_use]
	pub fn app_url(&self) -> Option<&str> {
		self.app_url.as_deref()
	}

	/// The service's own database, when it declares one.
	#[must_use]
	pub fn database_url(&self) -> Option<&str> {
		self.database_url.as_deref()
	}

	/// How many connections the service's database pool should hold, when
	/// the app's config sets `pool` for it.
	#[must_use]
	pub fn database_pool(&self) -> Option<usize> {
		self.database_pool
	}

	/// The directory of the volume called `name`. Names compare as the
	/// variables spell them, so `exports`, `EXPORTS`, and `my-data` for
	/// `MY_DATA` all match.
	#[must_use]
	pub fn volume(&self, name: &str) -> Option<&Path> {
		self.volumes
			.get(&env_part(name).to_ascii_lowercase())
			.map(PathBuf::as_path)
	}

	/// Every volume, by name.
	#[must_use]
	pub fn volumes(&self) -> &BTreeMap<String, PathBuf> {
		&self.volumes
	}

	/// Where the service's public paths go: `/_leptoskit/services/<name>`.
	#[must_use]
	pub fn public_prefix(&self) -> String {
		public_prefix(&self.name)
	}

	/// The readiness `/readyz` reports. Add a check for anything the service
	/// must finish before it takes traffic, such as its migrations.
	#[must_use]
	pub fn readiness(&self) -> &Readiness {
		&self.readiness
	}

	/// A layer that lets through only requests carrying this service's
	/// token.
	#[must_use]
	pub fn require_token(&self) -> RequireToken {
		RequireToken::new(&self.token)
	}

	/// Serves `router` on [`address`](Self::address) with `/healthz` and
	/// `/readyz` merged in, until Ctrl-C or `SIGTERM`. On the signal,
	/// `/readyz` fails first, then the server drains as
	/// [`GracefulShutdown::new`] does, for [`DRAIN_MS_ENV`] when it's set.
	///
	/// # Errors
	///
	/// When the address can't be bound, serving fails, or requests were
	/// still in flight at the end of the grace period.
	///
	/// # Panics
	///
	/// When `router` already has `/healthz` or `/readyz`.
	pub async fn serve(self, router: Router) -> Result<(), ServeError> {
		let shutdown = match self.drain_delay {
			Some(delay) => GracefulShutdown::new().drain_delay(delay),
			None => GracefulShutdown::new(),
		};
		self.serve_with(router, shutdown).await
	}

	/// [`serve`](Self::serve), stopping the way `shutdown` says. The
	/// service's readiness is attached to it.
	///
	/// # Errors
	///
	/// As [`serve`](Self::serve).
	///
	/// # Panics
	///
	/// As [`serve`](Self::serve).
	pub async fn serve_with(
		self,
		router: Router,
		shutdown: GracefulShutdown,
	) -> Result<(), ServeError> {
		let app = router.merge(self.readiness.routes());
		let listener = tokio::net::TcpListener::bind(self.address)
			.await
			.map_err(|source| {
				ServeError::Bind {
					address: self.address,
					source,
				}
			})?;
		let address = listener.local_addr().unwrap_or(self.address);
		println!(
			"service `{}` listening on http://{address}{}",
			self.name,
			self.public_prefix()
		);
		crate::serve_listener_with(listener, app, shutdown.readiness(&self.readiness)).await
	}
}

impl fmt::Debug for ServiceEnv {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ServiceEnv")
			.field("name", &self.name)
			.field("address", &self.address)
			.field("token", &"…")
			.field("public_origin", &self.public_origin)
			.field("app_url", &self.app_url)
			.field("database_url", &self.database_url.as_ref().map(|_| "…"))
			.field("volumes", &self.volumes)
			.finish_non_exhaustive()
	}
}

/// [`ServiceEnv::expect_name`]: the service runs under another name than the
/// one its operations are declared with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrongName {
	/// The name its operations use.
	pub expected: String,
	/// The name `[services.<name>]` gave it.
	pub actual: String,
}

impl fmt::Display for WrongName {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"this service's operations are declared `service = \"{}\"`, but it runs as `{}`: name \
			 its table `[services.{}]` in leptoskit.toml",
			self.expected, self.actual, self.expected
		)
	}
}

impl std::error::Error for WrongName {}

/// What's wrong with a service's environment.
#[derive(Debug, thiserror::Error)]
pub enum ServiceEnvError {
	/// Required variables are missing or empty. A service started by
	/// `leptoskit dev`, `leptoskit serve`, or a deploy has them all.
	#[error(
		"the service's environment is missing {}: run it through `leptoskit dev` or `leptoskit \
		 serve`, which set them",
		.0.join(", ")
	)]
	Missing(Vec<&'static str>),
	/// `LEPTOSKIT_SERVICE_ADDR` isn't a socket address.
	#[error("LEPTOSKIT_SERVICE_ADDR `{address}` is not a socket address such as 127.0.0.1:3100")]
	InvalidAddress {
		/// The value as set.
		address: String,
		/// The parse failure.
		source: std::net::AddrParseError,
	},
}

/// A layer that answers `401 Unauthorized` unless the request carries
/// `Authorization: Bearer <token>`, compared in constant time. Put it on the
/// routes only the app may call.
///
/// ```
/// use leptoskit_axum::axum::Router;
/// use leptoskit_axum::axum::routing::get;
/// use leptoskit_axum::service::RequireToken;
///
/// let private: Router = Router::new()
/// 	.route("/summary", get(|| async { "{}" }))
/// 	.route_layer(RequireToken::new("secret"));
/// ```
///
/// An empty token lets nothing through.
#[derive(Clone)]
pub struct RequireToken {
	expected: Option<Arc<[u8]>>,
}

impl RequireToken {
	/// Requires `token`.
	#[must_use]
	pub fn new(token: &str) -> Self {
		Self {
			expected: (!token.is_empty())
				.then(|| Arc::from(format!("Bearer {token}").into_bytes().into_boxed_slice())),
		}
	}

	fn allows(&self, header: Option<&HeaderValue>) -> bool {
		match (&self.expected, header) {
			(Some(expected), Some(header)) => bool::from(expected.ct_eq(header.as_bytes())),
			_ => false,
		}
	}
}

impl fmt::Debug for RequireToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("RequireToken").finish_non_exhaustive()
	}
}

impl<S> Layer<S> for RequireToken {
	type Service = RequireTokenService<S>;

	fn layer(&self, inner: S) -> Self::Service {
		RequireTokenService {
			inner,
			require: self.clone(),
		}
	}
}

/// The service [`RequireToken`] wraps around a route.
#[derive(Clone, Debug)]
pub struct RequireTokenService<S> {
	inner: S,
	require: RequireToken,
}

impl<S, B> Service<Request<B>> for RequireTokenService<S>
where
	S: Service<Request<B>, Response = Response>,
	S::Error: Send + 'static,
	S::Future: Send + 'static,
{
	type Error = S::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;
	type Response = Response;

	fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		self.inner.poll_ready(cx)
	}

	fn call(&mut self, request: Request<B>) -> Self::Future {
		if self
			.require
			.allows(request.headers().get(header::AUTHORIZATION))
		{
			Box::pin(self.inner.call(request))
		} else {
			let response = (
				StatusCode::UNAUTHORIZED,
				[(header::WWW_AUTHENTICATE, "Bearer")],
				"this route needs the service token",
			)
				.into_response();
			Box::pin(std::future::ready(Ok(response)))
		}
	}
}

/// Runs a service's `main`: the work behind [`main!`](crate::service::main).
#[doc(hidden)]
pub fn __run<F, Fut, E>(run: F) -> ExitCode
where
	F: FnOnce(ServiceEnv) -> Fut,
	Fut: Future<Output = Result<Router, E>>,
	E: fmt::Display,
{
	let runtime = match tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
	{
		Ok(runtime) => runtime,
		Err(error) => {
			eprintln!("could not start the async runtime: {error}");
			return ExitCode::FAILURE;
		}
	};

	runtime.block_on(async move {
		let env = match ServiceEnv::from_env() {
			Ok(env) => env,
			Err(error) => {
				eprintln!("{error}");
				return ExitCode::FAILURE;
			}
		};
		let name = env.name().to_owned();
		let router = match run(env.clone()).await {
			Ok(router) => router,
			Err(error) => {
				eprintln!("service `{name}` could not start: {error}");
				return ExitCode::FAILURE;
			}
		};

		match env.serve(router).await {
			Ok(()) => ExitCode::SUCCESS,
			Err(error) => {
				eprintln!("service `{name}` stopped: {error}");
				ExitCode::FAILURE
			}
		}
	})
}

/// Writes a service's `main`.
///
/// `main!(library, run)`:
///
/// 1. links `library`, the crate that declares the service's operations, so
///    their registrations exist: a binary that never names its library has
///    none, and answers every call with `404`;
/// 2. starts a multi-threaded runtime and reads the [`ServiceEnv`];
/// 3. awaits `run(env)`, an `async fn(ServiceEnv) -> Result<Router, E>` with
///    `E: Display`, which builds the router;
/// 4. serves it with [`ServiceEnv::serve`], exiting non-zero with the error
///    printed when anything fails.
///
/// In `src/main.rs` of a package whose library is `analytics_service`:
///
/// ```ignore
/// use leptoskit::server::axum::Router;
/// use leptoskit::service::ServiceEnv;
///
/// leptoskit::service::main!(analytics_service, run);
///
/// async fn run(env: ServiceEnv) -> Result<Router, String> {
/// 	let public = Router::new(); // the service's operations and live routes
/// 	Ok(Router::new().nest(&env.public_prefix(), public))
/// }
/// ```
#[doc(hidden)]
#[macro_export]
macro_rules! __service_main {
	($library:ident, $run:path $(,)?) => {
		extern crate $library;

		fn main() -> ::std::process::ExitCode {
			$crate::service::__run($run)
		}
	};
}

#[doc(inline)]
pub use crate::__service_main as main;

#[cfg(test)]
mod tests {
	use axum::body::Body;
	use axum::routing::get;
	use tower::ServiceExt as _;

	use super::*;

	fn required() -> Vec<(&'static str, &'static str)> {
		vec![
			(NAME_ENV, "event-log"),
			(ADDR_ENV, "127.0.0.1:3100"),
			(TOKEN_ENV, "secret"),
		]
	}

	#[test]
	fn reads_every_variable() {
		let mut vars = required();
		vars.extend([
			(PUBLIC_ORIGIN_ENV, "https://example.com/"),
			(APP_URL_ENV, "http://127.0.0.1:3000/"),
			(DATABASE_URL_ENV, "sqlite://event-log.sqlite3"),
			(DATABASE_POOL_ENV, "4"),
			(DRAIN_MS_ENV, "0"),
			("LEPTOSKIT_SERVICE_VOLUME_EXPORTS", "/data/exports"),
			("LEPTOSKIT_SERVICE_VOLUME_MY_DATA", "/data/mine"),
			("LEPTOSKIT_SERVICE_VOLUME_", "/ignored"),
			("UNRELATED", "x"),
		]);
		let env = ServiceEnv::from_vars(vars).expect("env");

		assert_eq!(env.name(), "event-log");
		assert_eq!(env.address(), "127.0.0.1:3100".parse().unwrap());
		assert_eq!(env.token(), "secret");
		assert_eq!(env.public_origin(), Some("https://example.com"));
		assert_eq!(env.app_url(), Some("http://127.0.0.1:3000"));
		assert_eq!(env.database_url(), Some("sqlite://event-log.sqlite3"));
		assert_eq!(env.database_pool(), Some(4));
		assert_eq!(env.drain_delay, Some(Duration::ZERO));
		assert_eq!(env.public_prefix(), "/_leptoskit/services/event-log");
		assert_eq!(env.volume("exports"), Some(Path::new("/data/exports")));
		assert_eq!(env.volume("EXPORTS"), Some(Path::new("/data/exports")));
		assert_eq!(env.volume("my-data"), Some(Path::new("/data/mine")));
		assert_eq!(env.volume("missing"), None);
		assert_eq!(env.volumes().len(), 2);
	}

	#[test]
	fn names_every_missing_variable() {
		let error = ServiceEnv::from_vars([(NAME_ENV, "x"), (TOKEN_ENV, "")]).unwrap_err();
		let ServiceEnvError::Missing(missing) = &error else {
			panic!("unexpected error: {error}");
		};
		assert_eq!(missing, &[ADDR_ENV, TOKEN_ENV]);
		assert!(
			error
				.to_string()
				.contains("LEPTOSKIT_SERVICE_ADDR, LEPTOSKIT_SERVICE_TOKEN")
		);
	}

	#[test]
	fn rejects_an_address_without_a_port() {
		let mut vars = required();
		vars[1] = (ADDR_ENV, "127.0.0.1");
		let error = ServiceEnv::from_vars(vars).unwrap_err();
		assert!(
			matches!(error, ServiceEnvError::InvalidAddress { .. }),
			"{error}"
		);
	}

	#[test]
	fn debug_hides_the_token_and_database() {
		let mut vars = required();
		vars.push((DATABASE_URL_ENV, "postgres://user:password@host/db"));
		let debug = format!("{:?}", ServiceEnv::from_vars(vars).unwrap());
		assert!(!debug.contains("secret"), "{debug}");
		assert!(!debug.contains("password"), "{debug}");
	}

	#[test]
	fn env_names_follow_the_service_name() {
		assert_eq!(
			env_name("analytics", "URL"),
			"LEPTOSKIT_SERVICE_ANALYTICS_URL"
		);
		assert_eq!(
			env_name("event-log", "TOKEN"),
			"LEPTOSKIT_SERVICE_EVENT_LOG_TOKEN"
		);
		assert_eq!(public_prefix("analytics"), "/_leptoskit/services/analytics");
	}

	async fn status(router: &Router, authorization: Option<&str>) -> StatusCode {
		let mut request = Request::get("/private");

		if let Some(authorization) = authorization {
			request = request.header(header::AUTHORIZATION, authorization);
		}

		router
			.clone()
			.oneshot(request.body(Body::empty()).unwrap())
			.await
			.unwrap()
			.status()
	}

	#[tokio::test]
	async fn the_token_layer_lets_only_the_token_through() {
		let router = Router::new()
			.route("/private", get(|| async { "ok" }))
			.route_layer(RequireToken::new("secret"));

		assert_eq!(status(&router, Some("Bearer secret")).await, StatusCode::OK);
		assert_eq!(status(&router, None).await, StatusCode::UNAUTHORIZED);
		assert_eq!(
			status(&router, Some("Bearer wrong")).await,
			StatusCode::UNAUTHORIZED
		);
		assert_eq!(
			status(&router, Some("Bearer secret2")).await,
			StatusCode::UNAUTHORIZED
		);
		assert_eq!(
			status(&router, Some("secret")).await,
			StatusCode::UNAUTHORIZED
		);
	}

	#[tokio::test]
	async fn an_empty_token_lets_nothing_through() {
		let router = Router::new()
			.route("/private", get(|| async { "ok" }))
			.route_layer(RequireToken::new(""));

		assert_eq!(
			status(&router, Some("Bearer ")).await,
			StatusCode::UNAUTHORIZED
		);
		assert_eq!(status(&router, None).await, StatusCode::UNAUTHORIZED);
	}
}
