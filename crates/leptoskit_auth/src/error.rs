//! What can go wrong while signing in.

use std::time::Duration;

use leptoskit_db::Timestamp;

/// Why a sign-in step failed. The messages are safe to show: none says
/// whether an address has an account.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
	/// The email address isn't one.
	#[error("enter a valid email address")]
	InvalidEmail,
	/// The code is wrong, expired, used, or tried too often. Which one is
	/// never said.
	#[error("that code didn't work; ask for a new one")]
	InvalidCode,
	/// Too many attempts; try again after `retry_after`.
	#[error("too many attempts; try again in {} seconds", retry_after.as_secs().max(1))]
	RateLimited {
		/// How long until the next attempt counts.
		retry_after: Duration,
	},
	/// The account was disabled.
	#[error("this account is disabled")]
	Disabled,
	/// The account is locked after too many failures.
	#[error("this account is locked; try again later")]
	Locked {
		/// When it unlocks.
		until: Timestamp,
	},
	/// The request isn't signed in.
	#[error("sign in to continue")]
	SignedOut,
	/// The email couldn't be sent.
	#[error("the email couldn't be sent: {0}")]
	Mail(#[from] crate::MailError),
	/// The app's sign-up hook refused or failed. Nothing was created.
	#[error("sign-up failed: {0}")]
	SignUp(Box<dyn std::error::Error + Send + Sync>),
	/// The server is misconfigured, such as missing keys.
	#[error("auth is misconfigured: {0}")]
	Config(String),
	/// The database failed.
	#[error(transparent)]
	Db(#[from] leptoskit_db::DbError),
}

impl AuthError {
	/// Wraps an error from the app's sign-up hook.
	pub fn sign_up(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
		Self::SignUp(error.into())
	}
}

/// The result of a sign-in step.
pub type Result<T, E = AuthError> = std::result::Result<T, E>;
