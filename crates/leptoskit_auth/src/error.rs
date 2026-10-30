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
	/// The second step of signing in is unknown, used, expired, or out of
	/// attempts: start signing in again.
	#[error("signing in took too long; start again")]
	SignInExpired,
	/// The action needs a second factor used within the last few minutes:
	/// a code from the authenticator app, or a recovery code.
	#[error("confirm it's you with a code from your authenticator app")]
	StepUpRequired,
	/// The action needs a recent sign-in, and the account has no second
	/// factor to step up with: sign in again.
	#[error("sign in again to change how you sign in")]
	Reauthenticate,
	/// An authenticator app is already on; turn it off first.
	#[error("an authenticator app is already set up")]
	AlreadyEnrolled,
	/// There's no authenticator app to confirm, turn off, or back up.
	#[error("there's no authenticator app set up")]
	NotEnrolled,
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
