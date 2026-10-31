//! Failures as JSON: a status, a code to match on, and a sentence.

use commonplace_shared::ApiError;
use leptoskit::db::DbError;
use leptoskit::server::axum::Json;
use leptoskit::server::axum::http::StatusCode;
use leptoskit::server::axum::response::IntoResponse;
use leptoskit::server::axum::response::Response;

/// A request that failed, and how to tell the client.
#[derive(Debug)]
pub struct Failure {
	status: StatusCode,
	code: &'static str,
	message: String,
}

impl Failure {
	/// The request asked for something it may not have, or that is not there.
	pub fn invalid(message: impl Into<String>) -> Self {
		Self {
			status: StatusCode::UNPROCESSABLE_ENTITY,
			code: "invalid",
			message: message.into(),
		}
	}

	/// Nothing with that ID is visible to this person.
	pub fn not_found(what: &str) -> Self {
		Self {
			status: StatusCode::NOT_FOUND,
			code: "not_found",
			message: format!("No such {what}."),
		}
	}
}

impl From<DbError> for Failure {
	fn from(error: DbError) -> Self {
		let (status, code) = match &error {
			DbError::Invalid(_) | DbError::Check { .. } => {
				(StatusCode::UNPROCESSABLE_ENTITY, "invalid")
			}
			DbError::NotFound { .. } => (StatusCode::NOT_FOUND, "not_found"),
			DbError::Forbidden { .. } => (StatusCode::FORBIDDEN, "forbidden"),
			DbError::Conflict { .. } | DbError::Stale { .. } => (StatusCode::CONFLICT, "conflict"),
			_ => (StatusCode::INTERNAL_SERVER_ERROR, "server"),
		};
		// A database error names the table and column, never the values, so
		// its text is safe to show; anything else stays on the server.
		let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
			eprintln!("commonplace: {error}");
			"Something went wrong on the server.".to_owned()
		} else {
			error.to_string()
		};
		Self {
			status,
			code,
			message,
		}
	}
}

impl IntoResponse for Failure {
	fn into_response(self) -> Response {
		let body = ApiError {
			error: self.code.to_owned(),
			message: self.message,
		};
		(self.status, Json(body)).into_response()
	}
}

/// What every handler returns.
pub type Answer<T> = Result<Json<T>, Failure>;
