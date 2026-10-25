//! Axum routes over a [`ReleaseStore`]: the static [`crate::layout`] files
//! that [`crate::CodePushClient`] reads, and the older dynamic check,
//! download, and delta endpoints.
//!
//! The static routes serve exactly the files a bucket holds after
//! [`crate::publish_channel`], so a client cannot tell this server from a
//! CDN. Every route, dynamic ones included, answers from `index.json` and at
//! most two more files of the layout: no request lists or parses the
//! channel's releases, so the cost of serving a channel does not grow with
//! its history and an unauthenticated flood of checks cannot turn into disk
//! and CPU work. Enabled by the `server` feature. [`codepush_router`] returns
//! the routes unprefixed (`/{app}/{channel}/...`); nest it under a prefix of
//! your choosing, such as `/api/codepush`.

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use semver::Version;
use serde::Deserialize;

use crate::client::UpdatePlan;
use crate::delta::Delta;
use crate::layout::ObjectKey;
use crate::layout::ObjectKind;
use crate::layout::parse_delta_file_name;
use crate::layout::parse_release_name;
use crate::store::ReleaseStore;
use crate::store::StoreError;

/// The origin always revalidates `index.json`, so a new release is visible on
/// the next check; a CDN in front of a bucket uses
/// [`ObjectKind::cache_control`] instead.
const INDEX_CACHE_CONTROL: &str = "no-cache";

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
		.with_state(CodePushState { store })
}

async fn index_file(
	State(state): State<CodePushState>,
	Path((app, channel)): Path<(String, String)>,
) -> Result<Response, ApiError> {
	object_response(&state.store, &ObjectKey::index(&app, &channel))
}

async fn release_file(
	State(state): State<CodePushState>,
	Path((app, channel, release)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
	let (version, build) = release_id(&release)?;
	object_response(
		&state.store,
		&ObjectKey::release(&app, &channel, &version, build),
	)
}

async fn signature_file(
	State(state): State<CodePushState>,
	Path((app, channel, release)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
	let (version, build) = release_id(&release)?;
	object_response(
		&state.store,
		&ObjectKey::signature(&app, &channel, &version, build),
	)
}

async fn delta_file(
	State(state): State<CodePushState>,
	Path((app, channel, release, delta)): Path<(String, String, String, String)>,
) -> Result<Response, ApiError> {
	let (version, build) = release_id(&release)?;
	let key = parse_delta_file_name(&delta)
		.and_then(|base_hash| ObjectKey::delta(&app, &channel, &version, build, base_hash))
		.ok_or_else(|| ApiError::not_found("no such delta"))?;
	object_response(&state.store, &key)
}

/// Parses a release directory name from a request path. Anything but the
/// canonical `<version>+<build>` names no release, so it is a 404.
fn release_id(release: &str) -> Result<(Version, u64), ApiError> {
	parse_release_name(release).ok_or_else(|| ApiError::not_found("no such release"))
}

/// Serves a file of the store's static layout, or a 404.
fn object_response(store: &ReleaseStore, key: &ObjectKey) -> Result<Response, ApiError> {
	let bytes = store
		.read_object(key)?
		.ok_or_else(|| ApiError::not_found("no such file"))?;
	let kind = key.kind();
	let cache_control = match kind {
		ObjectKind::Index => INDEX_CACHE_CONTROL,
		ObjectKind::Release | ObjectKind::Signature | ObjectKind::Delta => kind.cache_control(),
	};
	Ok((
		[
			(header::CONTENT_TYPE, kind.content_type()),
			(header::CACHE_CONTROL, cache_control),
		],
		bytes,
	)
		.into_response())
}

async fn check(
	State(state): State<CodePushState>,
	Path((app, channel)): Path<(String, String)>,
	Query(query): Query<CheckQuery>,
) -> Result<Json<UpdatePlan>, ApiError> {
	let current = Version::parse(&query.version)
		.map_err(|error| ApiError::bad_request(format!("invalid version: {error}")))?;
	let plan = state
		.store
		.plan_update(&app, &channel, &current, query.build)?;
	Ok(Json(plan))
}

async fn download(
	State(state): State<CodePushState>,
	Path((app, channel, version, build)): Path<(String, String, String, u64)>,
) -> Result<Response, ApiError> {
	let version = Version::parse(&version)
		.map_err(|error| ApiError::bad_request(format!("invalid version: {error}")))?;
	let key = ObjectKey::release(&app, &channel, &version, build);
	let bytes = state
		.store
		.read_object(&key)?
		.ok_or_else(|| ApiError::not_found("no such release"))?;
	Ok(([(header::CONTENT_TYPE, key.kind().content_type())], bytes).into_response())
}

async fn delta(
	State(state): State<CodePushState>,
	Path((app, channel, from_version, from_build)): Path<(String, String, String, u64)>,
) -> Result<Json<Delta>, ApiError> {
	let from_version = Version::parse(&from_version)
		.map_err(|error| ApiError::bad_request(format!("invalid version: {error}")))?;
	let latest = state
		.store
		.index(&app, &channel)?
		.ok_or_else(|| ApiError::not_found("no releases on channel"))?
		.manifest;
	let base = state
		.store
		.read_object(&ObjectKey::release(
			&app,
			&channel,
			&from_version,
			from_build,
		))?
		.ok_or_else(|| ApiError::not_found("no such base release"))?;
	// Deltas are addressed by the hash of the container they apply to, not by
	// version string: a rebuilt "same version" release produces different
	// bytes and must not be patched with a stale delta.
	let delta = state.store.delta(
		&app,
		&channel,
		&latest.version,
		latest.build,
		&crate::manifest::hash_bytes(&base),
	)?;
	match delta {
		Some(delta) => Ok(Json(delta)),
		None => Err(ApiError::not_found("no delta for that base")),
	}
}

/// Errors surfaced by the routes.
#[derive(Debug)]
pub struct ApiError {
	status: StatusCode,
	message: String,
}

impl ApiError {
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

impl From<StoreError> for ApiError {
	fn from(_: StoreError) -> Self {
		// Store errors quote filesystem paths and file contents; a client
		// gets the status, not the server's layout.
		Self {
			status: StatusCode::INTERNAL_SERVER_ERROR,
			message: "the release store could not be read".to_owned(),
		}
	}
}

impl IntoResponse for ApiError {
	fn into_response(self) -> Response {
		(self.status, self.message).into_response()
	}
}
