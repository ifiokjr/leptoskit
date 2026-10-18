//! Axum routes over a [`ReleaseStore`]: the check, download, and delta
//! endpoints an app shell talks to.
//!
//! Enabled by the `server` feature. The routes are mounted under a prefix of
//! your choosing; [`codepush_router`] returns them rooted at `/api/codepush`.

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;

use crate::client::UpdatePlan;
use crate::store::ReleaseStore;
use crate::store::StoreError;

/// Shared state for the code push routes.
#[derive(Debug, Clone)]
pub struct CodePushState {
	store: ReleaseStore,
	/// The public base URL downloads are served from, when it differs from
	/// the request host.
	pub download_base: Option<String>,
}

/// The query parameters of a check request.
#[derive(Debug, Deserialize)]
pub struct CheckQuery {
	/// The version the client currently runs.
	pub version: String,
	/// The build the client currently runs.
	pub build: u64,
}

/// A delta descriptor in a check response.
#[derive(Debug, Serialize)]
pub struct DeltaOffer {
	/// The version the delta applies from.
	pub from_version: String,
	/// The delta's base hash, verified client side before applying.
	pub base_hash: String,
}

/// The router: `GET /:app/:channel/check`, `.../download/:version/:build`,
/// and `.../delta/:from_version/:from_build`.
pub fn codepush_router(store: ReleaseStore) -> Router {
	Router::new()
		.route("/{app}/{channel}/check", get(check))
		.route("/{app}/{channel}/download/{version}/{build}", get(download))
		.route(
			"/{app}/{channel}/delta/{from_version}/{from_build}",
			get(delta),
		)
		.with_state(CodePushState {
			store,
			download_base: None,
		})
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
) -> Result<Vec<u8>, ApiError> {
	let version = Version::parse(&version)
		.map_err(|error| ApiError::bad_request(format!("invalid version: {error}")))?;
	let record = state
		.store
		.find(&app, &channel, &version, build)?
		.ok_or_else(|| ApiError::not_found("no such release"))?;
	Ok(record.container)
}

async fn delta(
	State(state): State<CodePushState>,
	Path((app, channel, from_version, from_build)): Path<(String, String, String, u64)>,
) -> Result<Vec<u8>, ApiError> {
	let from_version = Version::parse(&from_version)
		.map_err(|error| ApiError::bad_request(format!("invalid version: {error}")))?;
	let latest = state
		.store
		.latest(&app, &channel)?
		.ok_or_else(|| ApiError::not_found("no releases on channel"))?;
	let delta = latest.delta_from_previous.filter(|delta| {
		state
			.store
			.find(&app, &channel, &from_version, from_build)
			.ok()
			.flatten()
			.is_some_and(|base| {
				base.manifest.bundle_hash() == crate::manifest::hash_bytes(&base.container)
					&& delta.base_hash == crate::manifest::hash_bytes(&base.container)
			})
	});
	match delta {
		Some(delta) => Ok(serde_json::to_vec(&delta)?),
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
	fn from(error: StoreError) -> Self {
		Self {
			status: StatusCode::INTERNAL_SERVER_ERROR,
			message: error.to_string(),
		}
	}
}

impl From<serde_json::Error> for ApiError {
	fn from(error: serde_json::Error) -> Self {
		Self {
			status: StatusCode::INTERNAL_SERVER_ERROR,
			message: error.to_string(),
		}
	}
}

impl IntoResponse for ApiError {
	fn into_response(self) -> axum::response::Response {
		(self.status, self.message).into_response()
	}
}
