//! What can go wrong while another app signs someone in.

use leptoskit_auth::AuthError;
use leptoskit_db::DbError;

/// Why a step of signing in with this app failed.
///
/// [`IdpError::OAuth`] is the protocol's own error, with a code from RFC
/// 6749 or OpenID Connect that the app on the other end matches on. The
/// server's own failures never reach that app in detail.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdpError {
	/// A protocol error, such as `invalid_grant`, with a description for the
	/// app's developer.
	#[error("{code}: {description}")]
	OAuth {
		/// The error code, such as `invalid_request`.
		code: &'static str,
		/// What went wrong.
		description: String,
	},
	/// The request isn't signed in.
	#[error("sign in to continue")]
	SignedOut,
	/// The consent request, grant, or app doesn't exist, has expired, or
	/// isn't the signed-in person's.
	#[error("that no longer exists")]
	NotFound,
	/// The provider is misconfigured.
	#[error("the identity provider is misconfigured: {0}")]
	Config(String),
	/// Sign-in failed underneath.
	#[error(transparent)]
	Auth(#[from] AuthError),
	/// The database failed.
	#[error(transparent)]
	Db(#[from] DbError),
}

impl IdpError {
	pub(crate) fn oauth(code: &'static str, description: impl Into<String>) -> Self {
		Self::OAuth {
			code,
			description: description.into(),
		}
	}

	pub(crate) fn invalid_request(description: impl Into<String>) -> Self {
		Self::oauth("invalid_request", description)
	}

	pub(crate) fn invalid_client() -> Self {
		Self::oauth(
			"invalid_client",
			"the client is unknown, or didn't authenticate as it registered to",
		)
	}

	pub(crate) fn invalid_grant(description: impl Into<String>) -> Self {
		Self::oauth("invalid_grant", description)
	}

	pub(crate) fn invalid_scope(description: impl Into<String>) -> Self {
		Self::oauth("invalid_scope", description)
	}

	/// The OAuth error code: the protocol's own, or `server_error` for
	/// everything else.
	pub fn code(&self) -> &'static str {
		match self {
			Self::OAuth { code, .. } => code,
			Self::SignedOut => "login_required",
			Self::NotFound => "not_found",
			Self::Auth(AuthError::RateLimited { .. }) => "rate_limited",
			Self::Config(_) | Self::Auth(_) | Self::Db(_) => "server_error",
		}
	}

	/// How long to wait before trying again, when the answer was
	/// `rate_limited`.
	pub fn retry_after(&self) -> Option<std::time::Duration> {
		match self {
			Self::Auth(AuthError::RateLimited { retry_after }) => Some(*retry_after),
			_ => None,
		}
	}

	/// What another app hears: the description of a protocol error, and
	/// nothing about the server's own failures.
	pub fn description(&self) -> String {
		match self {
			Self::OAuth { description, .. } => description.clone(),
			Self::Auth(error @ AuthError::RateLimited { .. }) => error.to_string(),
			Self::Config(_) | Self::Auth(_) | Self::Db(_) => {
				"something went wrong; try again".into()
			}
			_ => self.to_string(),
		}
	}
}

/// The result of a step of signing in with this app.
pub type Result<T, E = IdpError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn server_failures_stay_on_the_server() {
		let error = IdpError::Config("SECRET_ENV is missing".into());
		assert_eq!(error.code(), "server_error");
		assert!(!error.description().contains("SECRET"));

		let error = IdpError::invalid_grant("that code was already used");
		assert_eq!(error.code(), "invalid_grant");
		assert_eq!(error.description(), "that code was already used");
		assert_eq!(IdpError::SignedOut.code(), "login_required");
		let limited = IdpError::Auth(AuthError::RateLimited {
			retry_after: std::time::Duration::from_secs(30),
		});
		assert_eq!(limited.code(), "rate_limited");
		assert!(limited.description().contains("30 seconds"));
		assert_eq!(limited.retry_after().unwrap().as_secs(), 30);
		assert_eq!(IdpError::NotFound.retry_after(), None);
		assert_eq!(IdpError::NotFound.description(), "that no longer exists");
	}
}
