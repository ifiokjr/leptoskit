//! Axum adapter. Mount once, after adding application routes. The read API
//! requires a server-only bearer key. Browser ingestion is public, bounded,
//! origin-checked and always tagged as untrusted browser evidence.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::extract::MatchedPath;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use serde::Deserialize;
use sha2::Digest as _;
use subtle::ConstantTimeEq as _;

use crate::aggregate::valid_operation;
use crate::*;

const PREFIX: &str = "/_leptoskit/analytics";
/// A dependency-free browser module, served by [`mount`]. No cookies,
/// persistent identifiers, arbitrary event properties or exception messages.
pub const BROWSER_SDK: &str = include_str!("browser.mjs");

/// Server-only settings. The key is never sent in browser configuration.
pub struct ServerConfig {
	pub export: Option<export::Config>,
	pub resource: Resource,
	pub limits: Limits,
	pub read_key: String,
	pub public_origin: String,
	pub browser_routes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
	#[error("invalid analytics export configuration: {0}")]
	Export(&'static str),
	#[error(transparent)]
	Analytics(#[from] AnalyticsError),
	#[error(
		"analytics read key must contain at least 32 and at most 256 printable non-space ASCII \
		 characters"
	)]
	Key,
	#[error("analytics public origin must be an HTTPS origin (HTTP is allowed on localhost)")]
	Origin,
	#[error("analytics browser routes need at most 32 distinct operation templates")]
	Routes,
	#[error("missing or invalid analytics environment setting: {0}")]
	Environment(&'static str),
}

#[derive(Clone)]
struct Api {
	export: Option<Arc<export::Driver>>,
	analytics: Analytics,
	key_hash: [u8; 32],
	origin: String,
	routes: BTreeSet<String>,
	budget: Arc<Mutex<(Instant, u32)>>,
}

/// Installs request instrumentation and analytics routes. Returns the same
/// collector used by the API for explicit database/RPC/live instrumentation.
///
/// # Errors
/// Rejects invalid secrets, origins, route allowlists, identities or limits.
pub fn mount(router: Router, config: ServerConfig) -> Result<(Router, Analytics), ServerError> {
	if !(32..=256).contains(&config.read_key.len())
		|| !config.read_key.bytes().all(|b| (33..127).contains(&b))
	{
		return Err(ServerError::Key);
	}
	let origin = url::Url::parse(&config.public_origin).map_err(|_| ServerError::Origin)?;
	let local = matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
	if !(origin.scheme() == "https" || (origin.scheme() == "http" && local))
		|| origin.host_str().is_none()
		|| origin.path() != "/"
		|| origin.query().is_some()
		|| origin.fragment().is_some()
		|| !origin.username().is_empty()
		|| origin.password().is_some()
	{
		return Err(ServerError::Origin);
	}
	if config.browser_routes.len() > 32 || config.browser_routes.iter().any(|r| !valid_operation(r))
	{
		return Err(ServerError::Routes);
	}
	let allowlist: BTreeSet<_> = config.browser_routes.into_iter().collect();
	let analytics = Analytics::new(config.resource, config.limits, unix_seconds)?;
	let exporter = config
		.export
		.map(|config| export::start(analytics.clone(), config))
		.transpose()
		.map_err(ServerError::Export)?;
	let state = Api {
		export: exporter,
		analytics: analytics.clone(),
		key_hash: sha2::Sha256::digest(config.read_key.as_bytes()).into(),
		origin: origin.origin().ascii_serialization(),
		routes: allowlist,
		budget: Arc::new(Mutex::new((Instant::now(), 30))),
	};
	let api = Router::new()
		.route(PREFIX, get(evidence))
		.route(&format!("{PREFIX}/browser"), post(ingest))
		.route(&format!("{PREFIX}.mjs"), get(sdk))
		.layer(DefaultBodyLimit::max(16 * 1024))
		.with_state(state);
	let router = router
		.merge(api)
		.layer(axum::middleware::from_fn_with_state(
			analytics.clone(),
			observe,
		));
	Ok((router, analytics))
}

/// Reads the managed plugin's bindings. Disabled unless
/// `LEPTOSKIT_ANALYTICS=1`; an enabled but invalid setup fails startup.
///
/// # Errors
/// Reports only setting names, never values or bearer credentials.
pub fn from_env() -> Result<Option<ServerConfig>, ServerError> {
	let enabled = match std::env::var("LEPTOSKIT_ANALYTICS") {
		Ok(value) => value,
		Err(std::env::VarError::NotPresent) => return Ok(None),
		Err(_) => return Err(ServerError::Environment("LEPTOSKIT_ANALYTICS")),
	};
	if enabled != "1" {
		return Err(ServerError::Environment("LEPTOSKIT_ANALYTICS"));
	}
	let required = |name| std::env::var(name).map_err(|_| ServerError::Environment(name));
	let parse = |name| {
		required(name)?
			.parse::<usize>()
			.map_err(|_| ServerError::Environment(name))
	};
	let routes = required("LEPTOSKIT_ANALYTICS_ROUTES")?;
	Ok(Some(ServerConfig {
		export: export_from_env()?,

		resource: Resource {
			service: required("LEPTOSKIT_ANALYTICS_SERVICE")?,
			environment: required("LEPTOSKIT_ANALYTICS_ENVIRONMENT")?,
			release: required("LEPTOSKIT_RELEASE")?,
			instance: std::env::var("HOSTNAME").unwrap_or_else(|_| "local".into()),
		},
		limits: Limits {
			retained_windows: parse("LEPTOSKIT_ANALYTICS_WINDOWS")?,
			max_series: parse("LEPTOSKIT_ANALYTICS_MAX_SERIES")?,
			..Limits::default()
		},
		read_key: required("LEPTOSKIT_ANALYTICS_READ_KEY")?,
		public_origin: required("LEPTOSKIT_ANALYTICS_ORIGIN")?,
		browser_routes: route_names(&routes),
	}))
}

fn export_from_env() -> Result<Option<export::Config>, ServerError> {
	let optional = |name| {
		match std::env::var(name) {
			Ok(value) => Ok(Some(value)),
			Err(std::env::VarError::NotPresent) => Ok(None),
			Err(_) => Err(ServerError::Environment(name)),
		}
	};
	let Some(endpoint) = optional("LEPTOSKIT_ANALYTICS_EXPORT_ENDPOINT")? else {
		return Ok(None);
	};
	let protocol = match optional("LEPTOSKIT_ANALYTICS_EXPORT_PROTOCOL")?.as_deref() {
		None | Some("leptoskit") => export::Protocol::Leptoskit,
		Some("otlp") => export::Protocol::OtlpMetrics,
		_ => {
			return Err(ServerError::Environment(
				"LEPTOSKIT_ANALYTICS_EXPORT_PROTOCOL",
			));
		}
	};
	Ok(Some(export::Config {
		endpoint,
		protocol,
		key: optional("LEPTOSKIT_ANALYTICS_WRITE_KEY")?.filter(|key| !key.is_empty()),
	}))
}

fn route_names(value: &str) -> Vec<String> {
	// Bindings use a delimiter excluded by operation validation. No general
	// JSON parser or TOML parser is needed at application startup.
	if value.is_empty() {
		Vec::new()
	} else {
		value.split(',').map(str::to_owned).collect()
	}
}

fn unix_seconds() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

async fn evidence(State(api): State<Api>, headers: HeaderMap) -> Response {
	let authorized = headers
		.get(axum::http::header::AUTHORIZATION)
		.and_then(|h| h.to_str().ok())
		.and_then(|s| s.strip_prefix("Bearer "))
		.filter(|s| s.len() <= 256)
		.is_some_and(|s| bool::from(api.key_hash.ct_eq(&sha2::Sha256::digest(s.as_bytes()))));
	let mut response = if authorized {
		let mut evidence = api.analytics.evidence();
		evidence.snapshot.host = Some(host::sample());
		if let Some(exporter) = &api.export {
			evidence.snapshot.export = exporter.health.lock().ok().map(|health| health.clone());
		}
		evidence
			.snapshot
			.limitations
			.retain(|text| !text.starts_with("No CPU, memory, disk"));
		evidence.snapshot.limitations.push(
			"Host fields are platform-dependent; cgroup and filesystem values do not prove \
			 database-pool or relay saturation."
				.into(),
		);
		Json(evidence).into_response()
	} else {
		StatusCode::UNAUTHORIZED.into_response()
	};
	response
		.headers_mut()
		.insert("cache-control", HeaderValue::from_static("no-store"));
	response
		.headers_mut()
		.insert("vary", HeaderValue::from_static("Authorization"));
	response
}

async fn sdk() -> impl IntoResponse {
	(
		[
			("content-type", "text/javascript; charset=utf-8"),
			("cache-control", "no-store"),
		],
		BROWSER_SDK,
	)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserBatch {
	events: Vec<BrowserEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserEvent {
	kind: BrowserKind,
	route: String,
	duration_ms: f64,
	traceparent: Option<TraceContext>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BrowserKind {
	PageView,
	Navigation,
	Error,
	Request,
	RequestFailed,
}

async fn ingest(
	State(api): State<Api>,
	headers: HeaderMap,
	batch: Result<Json<BrowserBatch>, axum::extract::rejection::JsonRejection>,
) -> StatusCode {
	let batch = match batch {
		Ok(Json(batch)) => batch,
		Err(rejection) => {
			api.analytics.reject_browser_batch();
			return rejection.status();
		}
	};
	let origin_ok =
		headers.get("origin").and_then(|v| v.to_str().ok()) == Some(api.origin.as_str());
	if !origin_ok
		|| batch.events.is_empty()
		|| batch.events.len() > 32
		|| batch.events.iter().any(|e| {
			!api.routes.contains(&e.route)
				|| !e.duration_ms.is_finite()
				|| !(0.0..=86_400_000.0).contains(&e.duration_ms)
		}) {
		api.analytics.reject_browser_batch();
		return StatusCode::BAD_REQUEST;
	}
	let allowed = {
		let mut budget = api
			.budget
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		if budget.0.elapsed().as_secs() >= 1 {
			*budget = (Instant::now(), 30);
		}
		if budget.1 == 0 {
			false
		} else {
			budget.1 -= 1;
			true
		}
	};
	if !allowed {
		api.analytics.reject_browser_batch();
		return StatusCode::TOO_MANY_REQUESTS;
	}
	for event in batch.events {
		let (signal, outcome) = match event.kind {
			BrowserKind::PageView => (Signal::PageView, Outcome::Ok),
			BrowserKind::Navigation => (Signal::Navigation, Outcome::Ok),
			BrowserKind::Error => (Signal::BrowserError, Outcome::ClientError),
			BrowserKind::Request => (Signal::Http, Outcome::Ok),
			BrowserKind::RequestFailed => (Signal::Http, Outcome::ClientError),
		};
		api.analytics.record(
			SeriesKey {
				source: Source::Browser,
				signal,
				operation: event.route,
				outcome,
			},
			event.duration_ms,
			event.traceparent,
		);
	}
	StatusCode::NO_CONTENT
}

async fn observe(State(analytics): State<Analytics>, mut request: Request, next: Next) -> Response {
	if request.uri().path().starts_with(PREFIX)
		|| matches!(
			request.uri().path(),
			"/healthz" | "/readyz" | "/_leptoskit/health"
		) {
		return next.run(request).await;
	}
	let operation = request
		.extensions()
		.get::<MatchedPath>()
		.map_or("unmatched", MatchedPath::as_str)
		.to_owned();
	let parent = request
		.headers()
		.get("traceparent")
		.and_then(|h| h.to_str().ok())
		.and_then(TraceContext::parse);
	let context = TraceContext::child(parent.as_ref()).ok();
	if let Some(context) = &context {
		request.extensions_mut().insert(context.clone());
	}
	request.extensions_mut().insert(analytics.clone());
	let timer = Timer::new(analytics, Signal::Http, operation, context.clone());
	let mut response = next.run(request).await;
	let outcome = if response.status().is_server_error() {
		Outcome::ServerError
	} else if response.status().is_client_error() {
		Outcome::ClientError
	} else {
		Outcome::Ok
	};
	timer.finish(outcome);
	if let Some(context) = context
		&& let Ok(value) = HeaderValue::from_str(context.as_str())
	{
		response.headers_mut().insert("traceparent", value);
	}
	response
}

/// An explicit backend operation timer for database, RPC, jobs or live work.
/// Finishing or dropping records once; dropped operations are cancelled.
pub struct Timer {
	analytics: Analytics,
	key: Option<SeriesKey>,
	context: Option<TraceContext>,
	started: Instant,
}

impl Timer {
	#[must_use]
	pub fn new(
		analytics: Analytics,
		signal: Signal,
		operation: impl Into<String>,
		context: Option<TraceContext>,
	) -> Self {
		Self {
			analytics,
			key: Some(SeriesKey {
				source: Source::Backend,
				signal,
				operation: operation.into(),
				outcome: Outcome::Cancelled,
			}),
			context,
			started: Instant::now(),
		}
	}

	pub fn finish(mut self, outcome: Outcome) {
		if let Some(key) = &mut self.key {
			key.outcome = outcome;
		}
		self.record();
	}

	fn record(&mut self) {
		if let Some(key) = self.key.take() {
			self.analytics.record(
				key,
				self.started.elapsed().as_secs_f64() * 1000.0,
				self.context.take(),
			);
		}
	}
}

impl Drop for Timer {
	fn drop(&mut self) {
		self.record();
	}
}
