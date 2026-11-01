//! The Axum server toolkit for leptoskit projects.
//!
//! Axum is the one supported server. This crate assembles the pieces every
//! leptoskit deployment needs around it:
//!
//! - static asset serving for wasm builds, with pre-compressed brotli and gzip,
//!   through [`dev_router`];
//! - the code push release registry, mounted from
//!   [`leptoskit_codepush::codepush_router`];
//! - a ready-to-run [`serve`] for `leptoskit dev` and production static
//!   hosting, and [`serve_router`] for an app's own router;
//! - the native apps' association files, [`app_associations`], which let the
//!   iOS, macOS, and Android apps use the site's passkeys and sign-in return
//!   address;
//! - `/healthz` and `/readyz` probes from a [`Readiness`], and a
//!   [`GracefulShutdown`] that fails readiness before it stops, through
//!   [`serve_listener_with`];
//! - the crates the server is built from, re-exported as [`axum`], [`tokio`],
//!   [`tower`], and [`tower_http`], so a deployment depends on this crate alone
//!   and never has to keep their versions in step by hand.
//!
//! Server-side rendering and leptos server functions mount through `leptos`
//! and `leptos_axum` in the app itself; this crate stays leptos-free so the
//! server binary stays small and the toolkit composes with any router.
//!
//! The re-exported [`tokio`] carries the `macros` and `rt-multi-thread`
//! features, so a server binary needs only to bring it into scope for
//! `#[tokio::main]` to work:
//!
//! ```no_run
//! use leptoskit_axum::tokio;
//!
//! #[tokio::main]
//! async fn main() {
//! 	// ...
//! }
//! ```
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::future::Future;
use std::future::IntoFuture as _;
use std::net::AddrParseError;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

pub use axum;
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
pub use tokio;
pub use tower;
use tower::Layer;
pub use tower_http;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeader;
use tower_http::set_header::SetResponseHeaderLayer;

mod health;

pub use health::Check;
pub use health::GracefulShutdown;
pub use health::Readiness;
pub use health::SHUTDOWN_CHECK;

/// Configuration for [`serve`] and [`dev_router`].
#[derive(Debug, Clone)]
pub struct ServeOptions {
	/// The directory of static assets to serve.
	pub dist_dir: PathBuf,
	/// The bind address: an IPv4 or IPv6 literal such as `127.0.0.1` or `::1`.
	pub address: String,
	/// The bind port.
	pub port: u16,
	/// An optional code push release store to serve updates from.
	pub release_store: Option<leptoskit_codepush::ReleaseStore>,
}

impl ServeOptions {
	/// Options serving `dist_dir` on the given address and port.
	pub fn new(dist_dir: impl Into<PathBuf>, address: impl Into<String>, port: u16) -> Self {
		Self {
			dist_dir: dist_dir.into(),
			address: address.into(),
			port,
			release_store: None,
		}
	}

	/// Serves code push updates from a release store.
	#[must_use]
	pub fn with_release_store(mut self, store: leptoskit_codepush::ReleaseStore) -> Self {
		self.release_store = Some(store);
		self
	}

	fn socket_address(&self) -> Result<SocketAddr, ServeError> {
		let ip = self.address.parse::<IpAddr>().map_err(|source| {
			ServeError::InvalidAddress {
				address: self.address.clone(),
				source,
			}
		})?;
		Ok(SocketAddr::new(ip, self.port))
	}
}

/// The router every leptoskit server shares.
///
/// Serves static assets with pre-compressed brotli and gzip, answers health
/// checks at `/_leptoskit/health`, and mounts the code push registry under
/// `/api/codepush` when a store is configured. Unknown paths under
/// `/api/codepush` are `404 Not Found`.
///
/// Requests no static file matches fall back to `index.html` (when one
/// exists) only if they look like client-side routes: the method is `GET` or
/// `HEAD` and the last path segment has no file extension. So
/// `/settings/profile` renders the app, while a missing `/pkg/app_bg.wasm`
/// is a `404` instead of an HTML page the browser tries to compile.
pub fn dev_router(options: &ServeOptions) -> Router {
	let mut router = Router::new().route("/_leptoskit/health", get(health));
	if let Some(store) = &options.release_store {
		router = router.nest(
			"/api/codepush",
			leptoskit_codepush::codepush_router(store.clone()).fallback(not_found),
		);
	}
	router.fallback_service(static_service(&options.dist_dir))
}

/// The built site's static files, for a server-rendered app's own router.
///
/// Mount it as the fallback behind your leptos routes, so pages render on
/// the server and everything else (`pkg/`, `public/` assets) comes from the
/// built site with the same precompressed serving and `Vary` header as
/// [`dev_router`]:
///
/// ```no_run
/// # let site_root = std::path::Path::new("dist");
/// let app: axum::Router = axum::Router::new()
/// 	// .leptos_routes(&leptos_options, routes, shell)
/// 	.fallback_service(leptoskit_axum::static_files(site_root));
/// ```
///
/// A path no file matches is a `404`, except that a client route (see
/// [`dev_router`]) gets `index.html` when the site has one.
pub fn static_files(dist_dir: &Path) -> Router {
	Router::new().fallback_service(static_service(dist_dir))
}

mod associations;
pub use associations::app_associations;
pub use associations::apple_app_site_association;
pub use associations::asset_links;

async fn health() -> &'static str {
	"ok"
}

async fn not_found(uri: Uri) -> Response {
	(StatusCode::NOT_FOUND, format!("no route for {uri}")).into_response()
}

/// Static files with brotli and gzip negotiation.
///
/// `ServeDir` picks `foo.br` or `foo.gz` over `foo` from `Accept-Encoding`
/// (release builds write both) but does not say so, so every response
/// carries `Vary: Accept-Encoding` to stop shared caches from serving
/// compressed bytes to clients that never asked for them.
fn static_service(
	dist_dir: &Path,
) -> SetResponseHeader<ServeDir<spa_fallback::SpaFallback>, HeaderValue> {
	let index_dir = dist_dir.to_path_buf();
	let files = ServeDir::new(dist_dir)
		.precompressed_br()
		.precompressed_gzip()
		.fallback(spa_fallback::service(index_dir));
	SetResponseHeaderLayer::if_not_present(
		header::VARY,
		HeaderValue::from_static("accept-encoding"),
	)
	.layer(files)
}

/// The single-page-application fallback: `index.html` for client routes.
mod spa_fallback {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct SpaFallback {
		dir: PathBuf,
	}

	pub(crate) fn service(dir: PathBuf) -> SpaFallback {
		SpaFallback { dir }
	}

	impl tower::Service<Request> for SpaFallback {
		type Error = std::convert::Infallible;
		type Future = std::pin::Pin<
			Box<dyn Future<Output = Result<Response, std::convert::Infallible>> + Send>,
		>;
		type Response = Response;

		fn poll_ready(
			&mut self,
			_context: &mut std::task::Context<'_>,
		) -> std::task::Poll<Result<(), std::convert::Infallible>> {
			std::task::Poll::Ready(Ok(()))
		}

		fn call(&mut self, request: Request) -> Self::Future {
			let dir = self.dir.clone();
			let client_route = is_client_route(&request);
			let uri = request.uri().clone();
			Box::pin(async move { Ok(spa_response(&dir, client_route, uri).await) })
		}
	}

	/// Whether a request no file matched is a client-side route.
	///
	/// Only `GET` and `HEAD` navigate, and a last path segment with an
	/// extension (`app_bg.wasm`, `logo.svg`) names a file, not a route.
	fn is_client_route(request: &Request) -> bool {
		let navigates = matches!(*request.method(), Method::GET | Method::HEAD);
		let last_segment = request.uri().path().rsplit('/').next().unwrap_or_default();
		navigates && !last_segment.contains('.')
	}

	async fn spa_response(dir: &Path, client_route: bool, uri: Uri) -> Response {
		let index = dir.join("index.html");
		if !client_route || !index.is_file() {
			return not_found(uri).await;
		}
		match tokio::fs::read(&index).await {
			Ok(body) => {
				let mut response = Response::new(Body::from(body));
				response.headers_mut().insert(
					header::CONTENT_TYPE,
					HeaderValue::from_static("text/html; charset=utf-8"),
				);
				response
			}
			Err(_) => {
				(
					StatusCode::INTERNAL_SERVER_ERROR,
					"could not read index.html",
				)
					.into_response()
			}
		}
	}
}

/// Runs the server until shutdown.
///
/// # Errors
///
/// Returns an error when the address is not an IP literal, or the server
/// fails to bind or serve.
pub async fn serve(options: ServeOptions) -> Result<(), ServeError> {
	let address = options.socket_address()?;
	serve_router(address, dev_router(&options)).await
}

/// Serves an app's own router on `address` until `SIGINT` or `SIGTERM`,
/// finishing the requests in flight first.
///
/// This is what [`serve`] does for the toolkit's router, for a server that
/// composes its own: leptos routes, server functions, and
/// [`static_files`] behind them. Containers stop a process with `SIGTERM`,
/// so a server that only waited for Ctrl-C would cut off whatever was in
/// flight on every deploy.
///
/// ```no_run
/// use leptoskit_axum::axum::Router;
/// use leptoskit_axum::axum::routing::get;
/// use leptoskit_axum::tokio;
///
/// #[tokio::main]
/// async fn main() -> Result<(), leptoskit_axum::ServeError> {
/// 	let app = Router::new().route("/", get(|| async { "hello" }));
/// 	leptoskit_axum::serve_router("127.0.0.1:3000".parse().expect("address"), app).await
/// }
/// ```
///
/// # Errors
///
/// Returns an error when the address cannot be bound or serving fails.
pub async fn serve_router(address: SocketAddr, router: Router) -> Result<(), ServeError> {
	serve_router_with(address, router, GracefulShutdown::immediate()).await
}

/// [`serve_router`] that stops the way `shutdown` says: see
/// [`serve_listener_with`].
///
/// # Errors
///
/// Returns an error when the address cannot be bound, serving fails, or the
/// grace period runs out.
pub async fn serve_router_with(
	address: SocketAddr,
	router: Router,
	shutdown: GracefulShutdown,
) -> Result<(), ServeError> {
	let listener = tokio::net::TcpListener::bind(address)
		.await
		.map_err(|source| ServeError::Bind { address, source })?;
	serve_listener_with(listener, router, shutdown).await
}

/// [`serve_router`] for a listener the caller has already bound: to report
/// the address before serving, to learn the port the OS picked for `:0`, or
/// to take a socket handed over by the service manager.
///
/// # Errors
///
/// Returns an error when serving fails.
pub async fn serve_listener(
	listener: tokio::net::TcpListener,
	router: Router,
) -> Result<(), ServeError> {
	serve_listener_with(listener, router, GracefulShutdown::immediate()).await
}

/// [`serve_listener`] that stops the way `shutdown` says, for a server
/// behind a load balancer.
///
/// When the signal arrives, the server:
///
/// 1. fails `shutdown`'s [`Readiness`], so `/readyz` answers `503`;
/// 2. keeps serving for the drain delay, while the load balancer notices;
/// 3. stops accepting connections, and finishes the requests in flight;
/// 4. after the grace period, returns an error without waiting for the rest,
///    which end with the process when `main` returns it.
///
/// ```no_run
/// use std::time::Duration;
///
/// use leptoskit_axum::GracefulShutdown;
/// use leptoskit_axum::Readiness;
/// use leptoskit_axum::axum::Router;
/// use leptoskit_axum::tokio;
///
/// #[tokio::main]
/// async fn main() -> Result<(), leptoskit_axum::ServeError> {
/// 	let readiness = Readiness::new();
/// 	let migrations = readiness.check("migrations");
/// 	let app = Router::new().merge(readiness.routes());
/// 	let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
/// 		.await
/// 		.expect("bind");
/// 	// ...run the migrations, then:
/// 	migrations.pass();
/// 	let shutdown = GracefulShutdown::new()
/// 		.readiness(&readiness)
/// 		.drain_delay(Duration::from_secs(5))
/// 		.grace_period(Duration::from_secs(20));
/// 	leptoskit_axum::serve_listener_with(listener, app, shutdown).await
/// }
/// ```
///
/// # Errors
///
/// Returns an error when serving fails, or
/// [`ServeError::GracePeriodElapsed`] when requests were still in flight
/// at the end of the grace period.
pub async fn serve_listener_with(
	listener: tokio::net::TcpListener,
	router: Router,
	shutdown: GracefulShutdown,
) -> Result<(), ServeError> {
	let GracefulShutdown {
		readiness,
		drain_delay,
		grace_period,
		signal,
	} = shutdown;
	let (closing, closed) = tokio::sync::oneshot::channel::<()>();
	let stop = async move {
		signal.await;
		if let Some(readiness) = &readiness {
			readiness.shut_down();
		}
		tokio::time::sleep(drain_delay).await;
		let _ = closing.send(());
	};
	// `ConnectInfo` is attached so handlers can see the peer address: the
	// auth layer's per-IP rate limits read it, and they silently do nothing
	// when it is absent.
	let server = axum::serve(
		listener,
		router.into_make_service_with_connect_info::<SocketAddr>(),
	)
	.with_graceful_shutdown(stop)
	.into_future();
	// The grace period starts when the listener closes. The sender only
	// drops unsent when the server has already finished, which wins the
	// race below anyway.
	let deadline = async move {
		match (closed.await, grace_period) {
			(Ok(()), Some(grace_period)) => tokio::time::sleep(grace_period).await,
			_ => std::future::pending().await,
		}
	};
	tokio::select! {
		result = server => result.map_err(|source| ServeError::Serve(source.to_string())),
		() = deadline => {
			Err(ServeError::GracePeriodElapsed {
				grace_period: grace_period.unwrap_or_default(),
			})
		}
	}
}

/// Errors from the server.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
	/// The configured bind address is not an IP address.
	#[error("invalid bind address `{address}`: expected an IP address such as 127.0.0.1")]
	InvalidAddress {
		/// The address as configured.
		address: String,
		/// The parse failure.
		source: AddrParseError,
	},
	/// The address could not be bound.
	#[error("could not bind {address}")]
	Bind {
		/// The address that failed.
		address: SocketAddr,
		/// The underlying IO error.
		source: std::io::Error,
	},
	/// Serving failed.
	#[error("serve failed: {0}")]
	Serve(String),
	/// Requests were still in flight when the grace period ran out, so the
	/// server stopped waiting for them.
	#[error(
		"stopped with requests still in flight: the {}s grace period ran out",
		grace_period.as_secs_f32()
	)]
	GracePeriodElapsed {
		/// The grace period that ran out.
		grace_period: Duration,
	},
}

/// Resolves on Ctrl-C or, on Unix, `SIGTERM`: the signals [`serve_listener`]
/// and [`GracefulShutdown::new`] stop on. Code that runs its own loop beside a
/// server can await it to stop the same way.
///
/// # Panics
///
/// Panics when the `SIGTERM` handler cannot be installed.
pub async fn shutdown_signal() {
	let ctrl_c = async {
		let _ = tokio::signal::ctrl_c().await;
	};
	#[cfg(unix)]
	let terminate = async {
		let _ = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
			.expect("install SIGTERM handler")
			.recv()
			.await;
	};
	#[cfg(not(unix))]
	let terminate = std::future::pending::<()>();
	tokio::select! {
		() = ctrl_c => {},
		() = terminate => {},
	}
}
