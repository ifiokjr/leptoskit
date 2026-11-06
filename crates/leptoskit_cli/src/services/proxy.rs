//! The development reverse proxy: puts each public service's paths on the
//! app's origin, as the edge does in a deployment.
//!
//! A request whose path is under a public service's prefix
//! (`/_leptoskit/services/<name>`, or one of its `routes`) goes to that
//! service unchanged; every other request goes to the app. Bodies stream in
//! both directions, so server-rendered pages still stream and server-sent
//! events still arrive as they're sent, and a WebSocket upgrade is handed
//! through to whichever side answers it. A service whose `trust` is
//! `isolated` never sees the app's cookies, and can't set them: `Cookie` is
//! removed from requests to it and `Set-Cookie` from its responses.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse as _;
use axum::response::Response;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use leptoskit_config::ServiceTrust;

use super::BuiltService;

/// One public prefix and the service behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
	/// The path prefix, such as `/_leptoskit/services/analytics`.
	pub prefix: String,
	/// The service's name.
	pub service: String,
	/// Where the service listens.
	pub address: SocketAddr,
	/// Whether the app's cookies stay away from it.
	pub isolated: bool,
}

/// Every public prefix, longest first, so the most specific wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Routes {
	routes: Vec<Route>,
}

impl Routes {
	/// The public prefixes of the services that run here. Remote services
	/// can't be public, so none of theirs appear.
	#[must_use]
	pub fn of(services: &[BuiltService]) -> Self {
		let mut routes = Vec::new();

		for built in services {
			let settings = &built.service.settings;

			// A remote service runs nothing here, and can't be public.
			let Some(port) = settings.port.filter(|_| built.binary.is_some()) else {
				continue;
			};

			for prefix in settings.public_prefixes() {
				routes.push(Route {
					prefix,
					service: settings.name.clone(),
					address: SocketAddr::from(([127, 0, 0, 1], port)),
					isolated: settings.trust == ServiceTrust::Isolated,
				});
			}
		}

		routes.sort_by_key(|route| std::cmp::Reverse(route.prefix.len()));
		Self { routes }
	}

	/// Whether no service is public, so no proxy is needed.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.routes.is_empty()
	}

	/// The route `path` falls under, or `None` for the app.
	#[must_use]
	pub fn route(&self, path: &str) -> Option<&Route> {
		self.routes.iter().find(|route| {
			path == route.prefix
				|| path
					.strip_prefix(route.prefix.as_str())
					.is_some_and(|rest| rest.starts_with('/'))
		})
	}
}

struct Proxy {
	routes: Routes,
	app: SocketAddr,
	client: Client<HttpConnector, Body>,
}

/// The proxy's router: every request goes to a service or to the app at
/// `app`.
pub fn router(routes: Routes, app: SocketAddr) -> Router {
	let client = Client::builder(TokioExecutor::new()).build_http::<Body>();
	Router::new().fallback(forward).with_state(Arc::new(Proxy {
		routes,
		app,
		client,
	}))
}

/// The headers that describe one hop, never forwarded.
const HOP_BY_HOP: &[header::HeaderName] = &[
	header::CONNECTION,
	header::PROXY_AUTHENTICATE,
	header::PROXY_AUTHORIZATION,
	header::TE,
	header::TRAILER,
	header::TRANSFER_ENCODING,
	header::UPGRADE,
];

fn is_upgrade(headers: &HeaderMap) -> bool {
	headers
		.get(header::CONNECTION)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| {
			value
				.split(',')
				.any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
		})
		&& headers.contains_key(header::UPGRADE)
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
	for name in HOP_BY_HOP {
		headers.remove(name);
	}

	headers.remove("keep-alive");
}

async fn forward(State(proxy): State<Arc<Proxy>>, mut request: Request) -> Response {
	let route = proxy.routes.route(request.uri().path()).cloned();
	let (upstream, isolated) = route
		.as_ref()
		.map_or((proxy.app, false), |route| (route.address, route.isolated));
	let upgrade = is_upgrade(request.headers());
	let downstream = upgrade.then(|| hyper::upgrade::on(&mut request));
	let path = request
		.uri()
		.path_and_query()
		.map_or("/", |path| path.as_str())
		.to_owned();
	let (mut parts, body) = request.into_parts();

	parts.uri = match format!("http://{upstream}{path}").parse() {
		Ok(uri) => uri,
		Err(error) => return bad_gateway(&format!("couldn't address {upstream}: {error}")),
	};

	if !upgrade {
		strip_hop_by_hop(&mut parts.headers);
	}

	if isolated {
		parts.headers.remove(header::COOKIE);
	}

	if let Some(host) = parts.headers.get(header::HOST).cloned() {
		parts.headers.insert("x-forwarded-host", host);
	}

	parts
		.headers
		.insert("x-forwarded-proto", HeaderValue::from_static("http"));

	let mut response = match proxy.client.request(Request::from_parts(parts, body)).await {
		Ok(response) => response,
		Err(error) => {
			let who = route.as_ref().map_or_else(
				|| "the app".to_owned(),
				|route| format!("services.{}", route.service),
			);
			return bad_gateway(&format!("{who} at {upstream} didn't answer: {error}"));
		}
	};

	if response.status() == StatusCode::SWITCHING_PROTOCOLS
		&& let Some(downstream) = downstream
	{
		let upstream_upgrade = hyper::upgrade::on(&mut response);
		tokio::spawn(async move {
			let (Ok(downstream), Ok(upstream)) = (downstream.await, upstream_upgrade.await) else {
				return;
			};
			let mut downstream = TokioIo::new(downstream);
			let mut upstream = TokioIo::new(upstream);
			let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
		});
	}

	let (mut parts, body) = response.into_parts();

	if parts.status != StatusCode::SWITCHING_PROTOCOLS {
		strip_hop_by_hop(&mut parts.headers);
	}

	if isolated {
		parts.headers.remove(header::SET_COOKIE);
	}

	Response::from_parts(parts, Body::new(body))
}

fn bad_gateway(message: &str) -> Response {
	(StatusCode::BAD_GATEWAY, message.to_owned()).into_response()
}
