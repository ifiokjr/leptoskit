//! Axum routes over a [`ReleaseStore`]: the static [`crate::layout`] files
//! that [`crate::CodePushClient`] reads, and the older dynamic check,
//! download, and delta endpoints.
//!
//! The static routes serve exactly the files a bucket holds after
//! [`crate::publish_channel`], so a client cannot tell this server from a
//! CDN. Downloads stream in bounded chunks, and update checks read only
//! publisher metadata on blocking workers. Admission is bounded through the
//! lifetime of each response body. Enabled by the `server` feature.
//! [`codepush_router`] returns the routes unprefixed (`/{app}/{channel}/...`);
//! nest it under a prefix of your choosing, such as `/api/codepush`.

use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::Request;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header;
use axum::middleware;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use futures::StreamExt;
use semver::Version;
use serde::Deserialize;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;
use tower_http::services::ServeFile;

use crate::client::UpdatePlan;
use crate::layout::ObjectKey;
use crate::layout::ObjectKind;
use crate::layout::parse_delta_file_name;
use crate::layout::parse_release_name;
use crate::store::ReleaseStore;
use crate::store::StoreError;

/// Caches may not reuse `index.json` without asking the origin, and the
/// routes send no validators to ask with, so every check fetches it again and
/// a new release is visible on the next one; a CDN in front of a bucket uses
/// [`ObjectKind::cache_control`] instead.
const INDEX_CACHE_CONTROL: &str = "no-cache";

const MAX_ACTIVE_RESPONSES: usize = 16;
const STREAM_CHUNK_BYTES: usize = 64 * 1024;
type RequestPermit = Arc<OwnedSemaphorePermit>;

/// Shared state for the code push routes.
#[derive(Debug, Clone)]
pub struct CodePushState {
	store: ReleaseStore,
}

/// The query parameters of a check request.
#[derive(Debug, Deserialize)]
pub struct CheckQuery {
	/// The version the client currently runs.
	pub version: String,
	/// The build the client currently runs.
	pub build: u64,
}

/// The router.
///
/// The static layout: `GET /{app}/{channel}/index.json`,
/// `.../{version}+{build}/release.lkb`, `.../release.sig`, and
/// `.../deltas/{base hash}.jsondelta`.
///
/// The dynamic endpoints: `GET /{app}/{channel}/check`,
/// `.../download/{version}/{build}`, and
/// `.../delta/{from_version}/{from_build}`.
///
/// At most 16 responses are active per router, including cloned routers and
/// bodies still being downloaded. Excess requests receive 503 without queuing.
/// Files support Range and HEAD requests and stream in 64 KiB chunks. Put a
/// rate-limiting proxy or CDN in front of publicly reachable origins to limit
/// sustained traffic and slow clients.
pub fn codepush_router(store: ReleaseStore) -> Router {
	Router::new()
		.route("/{app}/{channel}/index.json", get(index_file))
		.route("/{app}/{channel}/{release}/release.lkb", get(release_file))
		.route(
			"/{app}/{channel}/{release}/release.sig",
			get(signature_file),
		)
		.route("/{app}/{channel}/{release}/deltas/{delta}", get(delta_file))
		.route("/{app}/{channel}/check", get(check))
		.route("/{app}/{channel}/download/{version}/{build}", get(download))
		.route(
			"/{app}/{channel}/delta/{from_version}/{from_build}",
			get(delta),
		)
		.layer(middleware::from_fn_with_state(
			Arc::new(Semaphore::new(MAX_ACTIVE_RESPONSES)),
			admit,
		))
		.with_state(CodePushState { store })
}

async fn admit(State(slots): State<Arc<Semaphore>>, mut request: Request, next: Next) -> Response {
	let Ok(permit) = slots.try_acquire_owned() else {
		return (StatusCode::SERVICE_UNAVAILABLE, "release server is busy").into_response();
	};
	let permit = Arc::new(permit);
	request.extensions_mut().insert(permit.clone());
	let (parts, body) = next.run(request).await.into_parts();
	// Keep admission until the body is consumed or dropped, not merely until
	// headers are ready. A slow downloader still occupies a file and a buffer.
	let stream = body.into_data_stream().map(move |chunk| {
		let _keep_permit = &permit;
		chunk
	});
	Response::from_parts(parts, Body::from_stream(stream))
}

async fn blocking<T: Send + 'static>(
	permit: RequestPermit,
	work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
	tokio::task::spawn_blocking(move || {
		// Cancellation must not release capacity while this work still runs.
		let _permit = permit;
		work()
	})
	.await
	.map_err(|_| ApiError::internal())?
}

async fn index_file(
	State(state): State<CodePushState>,
	Path((app, channel)): Path<(String, String)>,
	request: Request,
) -> Result<Response, ApiError> {
	mutable_object_response(&state.store, &ObjectKey::index(&app, &channel), request).await
}

async fn release_file(
	State(state): State<CodePushState>,
	Path((app, channel, release)): Path<(String, String, String)>,
	request: Request,
) -> Result<Response, ApiError> {
	let (version, build) = release_id(&release)?;
	object_response(
		&state.store,
		&ObjectKey::release(&app, &channel, &version, build),
		request,
	)
	.await
}

async fn signature_file(
	State(state): State<CodePushState>,
	Path((app, channel, release)): Path<(String, String, String)>,
	request: Request,
) -> Result<Response, ApiError> {
	let (version, build) = release_id(&release)?;
	object_response(
		&state.store,
		&ObjectKey::signature(&app, &channel, &version, build),
		request,
	)
	.await
}

async fn delta_file(
	State(state): State<CodePushState>,
	Path((app, channel, release, delta)): Path<(String, String, String, String)>,
	request: Request,
) -> Result<Response, ApiError> {
	let (version, build) = release_id(&release)?;
	let key = parse_delta_file_name(&delta)
		.and_then(|base_hash| ObjectKey::delta(&app, &channel, &version, build, base_hash))
		.ok_or_else(|| ApiError::not_found("no such delta"))?;
	object_response(&state.store, &key, request).await
}

/// Parses a release directory name from a request path. Anything but the
/// canonical `<version>+<build>` names no release, so it is a 404.
fn release_id(release: &str) -> Result<(Version, u64), ApiError> {
	parse_release_name(release).ok_or_else(|| ApiError::not_found("no such release"))
}

/// Serves a file of the store's static layout, or a 404.
async fn object_response(
	store: &ReleaseStore,
	key: &ObjectKey,
	request: Request,
) -> Result<Response, ApiError> {
	let path = store.root().join(key.as_str());
	let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
		if error.kind() == std::io::ErrorKind::NotFound {
			ApiError::not_found("no such file")
		} else {
			ApiError::from(StoreError::Io(error))
		}
	})?;

	if !metadata.is_file() {
		return Err(StoreError::Io(std::io::Error::other("expected a release file")).into());
	}

	let mut response = ServeFile::new(path)
		.with_buf_chunk_size(STREAM_CHUNK_BYTES)
		.try_call(request)
		.await
		.map_err(StoreError::from)?
		.map(Body::new);
	let kind = key.kind();

	let cache_control = match kind {
		ObjectKind::Index => INDEX_CACHE_CONTROL,
		ObjectKind::Release | ObjectKind::Signature | ObjectKind::Delta => kind.cache_control(),
	};

	response.headers_mut().insert(
		header::CONTENT_TYPE,
		kind.content_type().parse().expect("static header"),
	);
	response.headers_mut().insert(
		header::CACHE_CONTROL,
		cache_control.parse().expect("static header"),
	);
	Ok(response)
}

/// Mutable URLs cannot use file dates as validators: two publications in
/// the same second can otherwise hide a new release behind a stale 304.
async fn mutable_object_response(
	store: &ReleaseStore,
	key: &ObjectKey,
	mut request: Request,
) -> Result<Response, ApiError> {
	let headers = request.headers_mut();
	headers.remove(header::IF_MODIFIED_SINCE);
	headers.remove(header::IF_UNMODIFIED_SINCE);

	if headers.remove(header::IF_RANGE).is_some() {
		headers.remove(header::RANGE);
	}

	let mut response = object_response(store, key, request).await?;
	response.headers_mut().remove(header::LAST_MODIFIED);
	response.headers_mut().insert(
		header::CACHE_CONTROL,
		INDEX_CACHE_CONTROL.parse().expect("static header"),
	);
	Ok(response)
}

async fn check(
	State(state): State<CodePushState>,
	Path((app, channel)): Path<(String, String)>,
	Query(query): Query<CheckQuery>,
	Extension(permit): Extension<RequestPermit>,
) -> Result<Json<UpdatePlan>, ApiError> {
	let current = Version::parse(&query.version)
		.map_err(|error| ApiError::bad_request(format!("invalid version: {error}")))?;
	let plan = blocking(permit, move || {
		Ok(state
			.store
			.plan_update(&app, &channel, &current, query.build)?)
	})
	.await?;
	Ok(Json(plan))
}

async fn download(
	State(state): State<CodePushState>,
	Path((app, channel, version, build)): Path<(String, String, String, u64)>,
	request: Request,
) -> Result<Response, ApiError> {
	let version = Version::parse(&version)
		.map_err(|error| ApiError::bad_request(format!("invalid version: {error}")))?;
	let key = ObjectKey::release(&app, &channel, &version, build);
	object_response(&state.store, &key, request).await
}

async fn delta(
	State(state): State<CodePushState>,
	Path((app, channel, from_version, from_build)): Path<(String, String, String, u64)>,
	Extension(permit): Extension<RequestPermit>,
	request: Request,
) -> Result<Response, ApiError> {
	let from_version = Version::parse(&from_version)
		.map_err(|error| ApiError::bad_request(format!("invalid version: {error}")))?;
	let store = state.store.clone();
	let key = blocking(permit, move || {
		let latest = store
			.index(&app, &channel)?
			.ok_or_else(|| ApiError::not_found("no releases on channel"))?
			.manifest;
		let base_hash = store
			.container_hash(&app, &channel, &from_version, from_build)?
			.ok_or_else(|| ApiError::not_found("no such base release"))?;
		// Deltas are addressed by the hash of the container they apply to, not
		// by version string: a rebuilt "same version" release produces
		// different bytes and must not be patched with a stale delta.
		ObjectKey::delta(&app, &channel, &latest.version, latest.build, &base_hash)
			.ok_or_else(|| ApiError::not_found("no delta for that base"))
	})
	.await?;
	// This legacy URL follows the channel's latest release; only the static
	// delta URL includes the immutable target release identity.
	mutable_object_response(&state.store, &key, request).await
}

/// Errors surfaced by the routes.
#[derive(Debug)]
pub struct ApiError {
	status: StatusCode,
	message: String,
}

impl ApiError {
	fn internal() -> Self {
		Self {
			status: StatusCode::INTERNAL_SERVER_ERROR,
			message: STORE_FAILURE.to_owned(),
		}
	}

	fn bad_request(message: impl Into<String>) -> Self {
		Self {
			status: StatusCode::BAD_REQUEST,
			message: message.into(),
		}
	}

	fn not_found(message: impl Into<String>) -> Self {
		Self {
			status: StatusCode::NOT_FOUND,
			message: message.into(),
		}
	}
}

/// The body of every `500`.
const STORE_FAILURE: &str = "the release store could not be read";

impl From<StoreError> for ApiError {
	fn from(error: StoreError) -> Self {
		// Store errors quote filesystem paths and file contents: the operator
		// gets them in the server's log, and a client gets the status, not the
		// server's layout.
		tracing::error!(%error, "a code push request failed reading the release store");
		Self::internal()
	}
}

impl IntoResponse for ApiError {
	fn into_response(self) -> Response {
		(self.status, self.message).into_response()
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;
	use std::sync::Mutex;

	use tracing::Event;
	use tracing::Level;
	use tracing::Metadata;
	use tracing::Subscriber;
	use tracing::field::Field;
	use tracing::field::Visit;
	use tracing::span;

	use super::*;

	/// Records every event as its level and its fields, `name=value`.
	#[derive(Clone, Default)]
	struct Recorder(Arc<Mutex<Vec<(Level, String)>>>);

	struct Fields(String);

	impl Visit for Fields {
		fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
			use std::fmt::Write;
			write!(self.0, "{}={value:?} ", field.name()).expect("writing to a string");
		}
	}

	impl Subscriber for Recorder {
		fn enabled(&self, _: &Metadata<'_>) -> bool {
			true
		}

		fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
			span::Id::from_u64(1)
		}

		fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

		fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

		fn event(&self, event: &Event<'_>) {
			let mut fields = Fields(String::new());
			event.record(&mut fields);
			self.0
				.lock()
				.expect("recorder")
				.push((*event.metadata().level(), fields.0));
		}

		fn enter(&self, _: &span::Id) {}

		fn exit(&self, _: &span::Id) {}
	}

	#[test]
	fn store_failures_reach_the_log_but_not_the_client() {
		let recorder = Recorder::default();
		let failure = StoreError::Io(std::io::Error::other("/srv/releases/app is unreadable"));
		let response =
			tracing::subscriber::with_default(recorder.clone(), || ApiError::from(failure));

		assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
		assert_eq!(response.message, STORE_FAILURE);
		let events = recorder.0.lock().expect("recorder");
		assert_eq!(events.len(), 1, "{events:?}");
		let (level, fields) = &events[0];
		assert_eq!(*level, Level::ERROR);
		assert!(
			fields.contains("/srv/releases/app is unreadable"),
			"the operator sees the cause: {fields}"
		);
	}

	#[tokio::test]
	async fn cancelled_requests_keep_blocking_work_budgeted_until_it_finishes() {
		let slots = Arc::new(Semaphore::new(1));
		let permit = Arc::new(slots.clone().try_acquire_owned().expect("capacity"));
		let (started, running) = tokio::sync::oneshot::channel();
		let (finish, wait) = std::sync::mpsc::channel();
		let task = tokio::spawn(blocking(permit, move || {
			started.send(()).expect("started");
			wait.recv().expect("finish");
			Ok(())
		}));
		running.await.expect("running");
		task.abort();
		assert!(task.await.expect_err("cancelled").is_cancelled());

		assert!(slots.clone().try_acquire_owned().is_err());
		finish.send(()).expect("release worker");
		let _recovered = slots.acquire_owned().await.expect("capacity returned");
	}
}
