//! The one-shot HTTP transport: `POST /api/<name>` for every query and
//! mutation, and the same calls in process while the server renders.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::post;
use leptoskit_db::Db;
use serde_json::Value;

use super::Anonymous;
use super::Identify;
use super::Invocation;
use super::Registration;
use crate::Error;
use crate::Kind;

/// The header a mutation's request ID travels in.
pub const IDEMPOTENCY_KEY: &str = "idempotency-key";

/// The largest request body a call may send: 2 MiB.
const BODY_LIMIT: usize = 2 * 1024 * 1024;

/// The app's operations, served: the database they run on, and how to tell
/// who is calling.
///
/// ```ignore
/// let rpc = rpc::server::Server::new(db.clone()).identify(auth.clone());
/// let app = Router::new().merge(rpc.router()).layer(auth.layer());
/// ```
#[derive(Clone)]
pub struct Server {
	inner: Arc<Inner>,
}

struct Inner {
	db: Db,
	identify: Arc<dyn Identify>,
	/// The service these operations belong to; `None` for the app's own.
	service: Option<&'static str>,
}

impl Server {
	/// Operations on `db`, where every caller is anonymous until
	/// [`identify`](Self::identify) says otherwise.
	pub fn new(db: Db) -> Self {
		Self {
			inner: Arc::new(Inner {
				db,
				identify: Arc::new(Anonymous),
				service: None,
			}),
		}
	}

	/// Tells callers apart with `identify`, such as `leptoskit_auth`'s
	/// `Auth`.
	#[must_use]
	pub fn identify(self, identify: impl Identify) -> Self {
		Self {
			inner: Arc::new(Inner {
				db: self.inner.db.clone(),
				identify: Arc::new(identify),
				service: self.inner.service,
			}),
		}
	}

	/// Serves the operations of the service `name`, declared with
	/// `#[rpc::query(service = "name")]` and the like, instead of the app's
	/// own: what a service's binary builds. Every server also serves the
	/// built-in `rpc.*` operations for its own jobs and inbox.
	///
	/// Nest its [`router`](Self::router) (and its live server's) under
	/// [`service_prefix(name)`](crate::service_prefix), where the app's
	/// clients send the service's operations:
	///
	/// ```ignore
	/// let rpc = Server::new(db).service("analytics").identify(ServiceCallers::for_service(Anonymous));
	/// let live = rpc.live().origin(public_origin);
	/// let app = Router::new().nest(&rpc::service_prefix("analytics"), rpc.router().merge(live.router()));
	/// ```
	///
	/// In this process, a call to one of the service's own operations runs
	/// in process, as the app's do in the app's server.
	///
	/// # Panics
	///
	/// When `name` isn't a service name: see
	/// [`is_service_name`](crate::is_service_name).
	#[must_use]
	pub fn service(self, name: &'static str) -> Self {
		assert!(
			crate::is_service_name(name),
			"`{name}` isn't a service name: a lowercase letter, then lowercase letters, digits, \
			 and `-`"
		);
		Self {
			inner: Arc::new(Inner {
				db: self.inner.db.clone(),
				identify: Arc::clone(&self.inner.identify),
				service: Some(name),
			}),
		}
	}

	/// The service these operations belong to, or `None` for the app's.
	#[must_use]
	pub fn service_name(&self) -> Option<&'static str> {
		self.inner.service
	}

	/// The database operations run on, before it acts for anyone.
	pub fn db(&self) -> &Db {
		&self.inner.db
	}

	/// The live server for these operations: the same database and the same
	/// way of telling callers apart. Configure it, then merge its
	/// [`router`](super::Live::router).
	#[cfg(feature = "live")]
	pub fn live(&self) -> super::Live {
		super::Live::new(self.inner.db.clone())
			.identify_arc(Arc::clone(&self.inner.identify))
			.for_service(self.inner.service)
	}

	/// Makes this the server that in-process calls use, such as
	/// `client::call` while a page renders. [`router`](Self::router) does it
	/// too.
	pub fn install(&self) {
		*INSTALLED.write().unwrap_or_else(PoisonError::into_inner) = Some(self.clone());
	}

	/// `POST /api/<name>` for every registered query and mutation, relative
	/// to wherever the router is nested, and [installs](Self::install) this
	/// server for in-process calls.
	///
	/// A call sends its arguments as one JSON object, and gets back its
	/// result as JSON with status `200`, or a `Kind|message` error as
	/// `text/plain` with the status [`Error::status`] gives: the same as a
	/// `#[server]` function with the `Json` encoding, so every generated
	/// client speaks it. A mutation's `Idempotency-Key` header becomes its
	/// request ID.
	///
	/// # Panics
	///
	/// When two operations share a name; see
	/// [`registrations`](super::registrations).
	///
	/// Only this server's own operations get routes: the app's for a
	/// default server, or one service's after [`service`](Self::service).
	/// A service's operation linked into the app's build, as in a
	/// workspace-wide build, is never served by the app.
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		let registrations =
			super::registrations_in(self.inner.service).unwrap_or_else(|error| panic!("{error}"));
		self.install();
		let mut router = Router::new();

		for registration in registrations {
			// Jobs run in the background; a worker claims them, not a route.
			if registration.kind == Kind::Job {
				continue;
			}

			let server = self.clone();
			router = router.route(
				&format!("/api/{}", registration.name),
				post(move |request: Request| {
					let server = server.clone();
					async move { server.serve(registration, request).await }
				}),
			);
		}

		router
	}

	async fn serve(&self, registration: &'static Registration, request: Request) -> Response {
		let (parts, body) = request.into_parts();
		let Ok(bytes) = axum::body::to_bytes(body, BODY_LIMIT).await else {
			return failure(&Error::invalid(
				"the request body is too large, or didn't arrive",
			));
		};
		let arguments = if bytes.iter().all(u8::is_ascii_whitespace) {
			Value::Object(serde_json::Map::new())
		} else {
			match serde_json::from_slice::<Value>(&bytes) {
				Ok(value @ Value::Object(_)) => value,
				Ok(_) | Err(_) => {
					return failure(&Error::invalid(
						"send the arguments as one JSON object, such as `{\"limit\": 10}`",
					));
				}
			}
		};

		let request_id = (registration.kind == Kind::Mutation)
			.then(|| {
				parts
					.headers
					.get(IDEMPOTENCY_KEY)
					.map(|value| value.to_str().map(str::to_owned))
			})
			.flatten()
			.transpose();

		let Ok(request_id) = request_id else {
			return failure(&Error::invalid(
				"the idempotency key must be printable ASCII",
			));
		};

		match self.call(&parts, registration, arguments, request_id).await {
			Ok(value) => {
				(
					[(
						header::CONTENT_TYPE,
						HeaderValue::from_static("application/json"),
					)],
					value.to_string(),
				)
					.into_response()
			}
			Err(error) => failure(&error),
		}
	}

	/// Runs `registration` for the request `parts` came with.
	pub(crate) async fn call(
		&self,
		parts: &http::request::Parts,
		registration: &'static Registration,
		arguments: Value,
		request_id: Option<String>,
	) -> crate::Result<Value> {
		let identity = self.inner.identify.identify(parts).await?;
		let db = self.inner.db.with_context(identity.context.clone());
		(registration.handler)(Invocation {
			db,
			identity,
			request_id,
			arguments,
			reads: None,
		})
		.await
	}
}

impl std::fmt::Debug for Server {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Server").finish_non_exhaustive()
	}
}

static INSTALLED: RwLock<Option<Server>> = RwLock::new(None);

/// The server in-process calls use, once one is installed.
pub(crate) fn installed() -> Option<Server> {
	INSTALLED
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.clone()
}

/// The answer to a failed call: the status, and `Kind|message`.
pub(crate) fn failure(error: &Error) -> Response {
	let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
	let mut response = Response::new(Body::from(error.to_wire()));
	*response.status_mut() = status;
	response
		.headers_mut()
		.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
	response
}
