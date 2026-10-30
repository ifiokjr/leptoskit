//! Health and readiness probes, and the graceful shutdown that uses them.
//!
//! A load balancer asks two questions of each instance. `/healthz` asks
//! whether the process is up, and answers `200` for as long as it is.
//! `/readyz` asks whether the instance should get traffic, and answers
//! `200` only while every named check in its [`Readiness`] passes. A check
//! starts failing; the app passes it once the work it stands for is done,
//! such as running migrations.
//!
//! On `SIGTERM` the server fails readiness first, so the load balancer stops
//! sending new traffic, waits [`GracefulShutdown::drain_delay`] for it to
//! notice, and only then stops accepting connections and finishes the
//! requests in flight.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use axum::Router;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;

/// The check a server fails when it starts to shut down.
pub const SHUTDOWN_CHECK: &str = "shutdown";

/// Whether this instance should get traffic: a set of named checks that all
/// have to pass.
///
/// It's cheap to clone, and every clone shares the same checks, so the app
/// can hand one to the router, one to the server, and one to each task
/// that owns a check.
///
/// ```
/// use leptoskit_axum::Readiness;
///
/// let readiness = Readiness::new();
/// let migrations = readiness.check("migrations");
/// assert!(!readiness.is_ready());
///
/// // ...run the migrations...
/// migrations.pass();
/// assert!(readiness.is_ready());
/// ```
#[derive(Debug, Clone, Default)]
pub struct Readiness {
	checks: Arc<Mutex<BTreeMap<Cow<'static, str>, bool>>>,
}

impl Readiness {
	/// Readiness with no checks, which is ready until a check is added.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Adds a check named `name` that fails until it's passed, and returns
	/// the handle that passes it.
	///
	/// Asking for a name that's already there returns a handle to that same
	/// check, and leaves it as it was.
	pub fn check(&self, name: impl Into<Cow<'static, str>>) -> Check {
		let name = name.into();
		self.lock().entry(name.clone()).or_insert(false);
		Check {
			readiness: self.clone(),
			name,
		}
	}

	/// Whether every check passes.
	#[must_use]
	pub fn is_ready(&self) -> bool {
		self.lock().values().all(|passing| *passing)
	}

	/// The names of the checks that fail, in order.
	#[must_use]
	pub fn failing(&self) -> Vec<String> {
		self.lock()
			.iter()
			.filter(|(_, passing)| !**passing)
			.map(|(name, _)| name.to_string())
			.collect()
	}

	/// Fails readiness for good, under [`SHUTDOWN_CHECK`]. The server calls
	/// this when it starts to shut down.
	pub fn shut_down(&self) {
		self.check(SHUTDOWN_CHECK).fail();
	}

	/// `GET /healthz` and `GET /readyz`, to merge into the app's router.
	///
	/// `/healthz` answers `200 ok` while the process is up. `/readyz`
	/// answers `200` with `{"ready":true,"failing":[]}` when every check
	/// passes, and `503` naming the failing checks otherwise, such as
	/// `{"ready":false,"failing":["migrations"]}`. Neither is cached.
	///
	/// ```
	/// use leptoskit_axum::Readiness;
	/// use leptoskit_axum::axum::Router;
	///
	/// let readiness = Readiness::new();
	/// let app: Router = Router::new().merge(readiness.routes());
	/// ```
	pub fn routes<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		let readiness = self.clone();
		Router::new()
			.route("/healthz", get(|| async { no_store("ok") }))
			.route(
				"/readyz",
				get(move || {
					let readiness = readiness.clone();
					async move { readiness.response() }
				}),
			)
	}

	fn response(&self) -> Response {
		let failing = self.failing();
		let status = if failing.is_empty() {
			StatusCode::OK
		} else {
			StatusCode::SERVICE_UNAVAILABLE
		};
		let body = serde_json::json!({ "ready": failing.is_empty(), "failing": failing });
		let mut response = (status, no_store(body.to_string())).into_response();
		response.headers_mut().insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static("application/json"),
		);
		response
	}

	fn set(&self, name: &str, passing: bool) {
		if let Some(state) = self.lock().get_mut(name) {
			*state = passing;
		}
	}

	fn get(&self, name: &str) -> bool {
		self.lock().get(name).copied().unwrap_or(false)
	}

	fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Cow<'static, str>, bool>> {
		// A panic while holding the lock can't leave the map half-written:
		// every critical section is a single insert or read.
		self.checks.lock().unwrap_or_else(PoisonError::into_inner)
	}
}

fn no_store(body: impl IntoResponse) -> Response {
	(
		[(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
		body,
	)
		.into_response()
}

/// One named check in a [`Readiness`], from [`Readiness::check`].
#[derive(Debug, Clone)]
pub struct Check {
	readiness: Readiness,
	name: Cow<'static, str>,
}

impl Check {
	/// The check's name, as `/readyz` reports it.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.name
	}

	/// Marks the check as passing.
	pub fn pass(&self) {
		self.set(true);
	}

	/// Marks the check as failing.
	pub fn fail(&self) {
		self.set(false);
	}

	/// Marks the check as passing or failing.
	pub fn set(&self, passing: bool) {
		self.readiness.set(&self.name, passing);
	}

	/// Whether the check passes.
	#[must_use]
	pub fn is_passing(&self) -> bool {
		self.readiness.get(&self.name)
	}
}

/// How a server stops: on what signal, whether it fails readiness first,
/// how long it waits before it stops accepting, and how long it gives the
/// requests in flight.
///
/// Pass it to [`serve_listener_with`](crate::serve_listener_with):
///
/// ```no_run
/// use leptoskit_axum::GracefulShutdown;
/// use leptoskit_axum::Readiness;
/// use leptoskit_axum::axum::Router;
/// use leptoskit_axum::tokio;
///
/// #[tokio::main]
/// async fn main() -> Result<(), leptoskit_axum::ServeError> {
/// 	let readiness = Readiness::new();
/// 	let app = Router::new().merge(readiness.routes());
/// 	let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
/// 		.await
/// 		.expect("bind");
/// 	let shutdown = GracefulShutdown::new().readiness(&readiness);
/// 	leptoskit_axum::serve_listener_with(listener, app, shutdown).await
/// }
/// ```
pub struct GracefulShutdown {
	pub(crate) readiness: Option<Readiness>,
	pub(crate) drain_delay: Duration,
	pub(crate) grace_period: Option<Duration>,
	pub(crate) signal: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl GracefulShutdown {
	/// How long [`new`](Self::new) waits between failing readiness and
	/// closing the listener: long enough for a load balancer that probes
	/// every few seconds to notice.
	pub const DEFAULT_DRAIN_DELAY: Duration = Duration::from_secs(5);
	/// How long [`new`](Self::new) gives the requests in flight once the
	/// listener closes. With the drain delay, a server stops within 25
	/// seconds, inside the 30 most platforms wait before they kill it.
	pub const DEFAULT_GRACE_PERIOD: Duration = Duration::from_secs(20);

	/// Stops on Ctrl-C or `SIGTERM` (see
	/// [`shutdown_signal`](crate::shutdown_signal)),
	/// with [`DEFAULT_DRAIN_DELAY`](Self::DEFAULT_DRAIN_DELAY) and
	/// [`DEFAULT_GRACE_PERIOD`](Self::DEFAULT_GRACE_PERIOD).
	#[must_use]
	pub fn new() -> Self {
		Self {
			readiness: None,
			drain_delay: Self::DEFAULT_DRAIN_DELAY,
			grace_period: Some(Self::DEFAULT_GRACE_PERIOD),
			signal: Box::pin(crate::shutdown_signal()),
		}
	}

	/// What [`serve_listener`](crate::serve_listener) has always done: on
	/// the signal, stop accepting at once and wait for every request.
	pub(crate) fn immediate() -> Self {
		Self {
			drain_delay: Duration::ZERO,
			grace_period: None,
			..Self::new()
		}
	}

	/// Fails this readiness (under [`SHUTDOWN_CHECK`]) as soon as the
	/// signal arrives, before the drain delay.
	#[must_use]
	pub fn readiness(mut self, readiness: &Readiness) -> Self {
		self.readiness = Some(readiness.clone());
		self
	}

	/// How long to keep accepting after readiness fails. Load balancers
	/// probe on an interval, so for a moment they still send traffic to an
	/// instance that has started to stop.
	#[must_use]
	pub fn drain_delay(mut self, delay: Duration) -> Self {
		self.drain_delay = delay;
		self
	}

	/// How long the requests in flight get once the listener closes. Past
	/// it, [`serve_listener_with`](crate::serve_listener_with) stops waiting
	/// and returns
	/// [`ServeError::GracePeriodElapsed`](crate::ServeError::GracePeriodElapsed);
	/// the connections that remain end with the process.
	#[must_use]
	pub fn grace_period(mut self, grace_period: Duration) -> Self {
		self.grace_period = Some(grace_period);
		self
	}

	/// Waits for every request in flight, however long it takes.
	#[must_use]
	pub fn without_grace_period(mut self) -> Self {
		self.grace_period = None;
		self
	}

	/// Stops when `signal` resolves instead of on Ctrl-C or `SIGTERM`.
	#[must_use]
	pub fn signal(mut self, signal: impl Future<Output = ()> + Send + 'static) -> Self {
		self.signal = Box::pin(signal);
		self
	}
}

impl Default for GracefulShutdown {
	fn default() -> Self {
		Self::new()
	}
}

impl fmt::Debug for GracefulShutdown {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("GracefulShutdown")
			.field("readiness", &self.readiness)
			.field("drain_delay", &self.drain_delay)
			.field("grace_period", &self.grace_period)
			.finish_non_exhaustive()
	}
}
