//! The Axum server toolkit for leptoskit projects.
//!
//! Axum is the one supported server. This crate assembles the pieces every
//! leptoskit deployment needs around it:
//!
//! - static asset serving for wasm builds, with pre-compressed gzip support,
//!   through [`dev_router`];
//! - the code push release registry, mounted from
//!   [`leptoskit_codepush::codepush_router`];
//! - a ready-to-run [`serve`] for `leptoskit dev` and production static
//!   hosting.
//!
//! Server-side rendering and leptos server functions mount through `leptos`
//! and `leptos_axum` in the app itself; this crate stays leptos-free so the
//! server binary stays small and the toolkit composes with any router.

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use tower_http::services::ServeDir;

/// Configuration for [`serve`] and [`dev_router`].
#[derive(Debug, Clone)]
pub struct ServeOptions {
	/// The directory of static assets to serve.
	pub dist_dir: PathBuf,
	/// The bind address.
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

	fn socket_address(&self) -> SocketAddr {
		format!("{}:{}", self.address, self.port)
			.parse()
			.unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], self.port)))
	}
}

/// The router every leptoskit server shares.
///
/// Serves static assets with pre-compressed gzip support, answers health
/// checks at `/_leptoskit/health`, falls back to `index.html` for unknown
/// paths when one exists, and mounts the code push registry under
/// `/api/codepush` when a store is configured.
pub fn dev_router(options: &ServeOptions) -> Router {
	let mut router = Router::new().route("/_leptoskit/health", get(health));
	if let Some(store) = &options.release_store {
		router = router.nest(
			"/api/codepush",
			leptoskit_codepush::codepush_router(store.clone()),
		);
	}
	router.fallback_service(static_service(&options.dist_dir))
}

async fn health() -> &'static str {
	"ok"
}

fn static_service(dist_dir: &Path) -> ServeDir<spa_fallback::SpaFallback> {
	let index_dir = dist_dir.to_path_buf();
	ServeDir::new(dist_dir)
		.precompressed_gzip()
		.fallback(spa_fallback::service(index_dir))
}

/// The single-page-application fallback: `index.html` for unknown paths.
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
			Box::pin(async move { Ok(spa_response(&dir, request.uri()).await) })
		}
	}

	async fn spa_response(dir: &Path, uri: &Uri) -> Response {
		let index = dir.join("index.html");
		if !index.is_file() {
			return (StatusCode::NOT_FOUND, format!("no route for {uri}")).into_response();
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
			Err(error) => {
				(
					StatusCode::INTERNAL_SERVER_ERROR,
					format!("could not read index.html: {error}"),
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
/// Returns an error when the server fails to bind or serve.
pub async fn serve(options: ServeOptions) -> Result<(), ServeError> {
	let address = options.socket_address();
	let app = dev_router(&options);
	let listener = tokio::net::TcpListener::bind(address)
		.await
		.map_err(|source| ServeError::Bind { address, source })?;
	axum::serve(listener, app)
		.with_graceful_shutdown(shutdown_signal())
		.await
		.map_err(|source| ServeError::Serve(source.to_string()))?;
	Ok(())
}

/// Errors from the server.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
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
}

async fn shutdown_signal() {
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
