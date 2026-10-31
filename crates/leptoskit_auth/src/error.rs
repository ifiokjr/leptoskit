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
	/// The account must have two-step verification, and this session hasn't
	/// set it up or used it: it may only do that, or sign out.
	#[error("set up two-step verification to continue")]
	MfaRequired,
	/// The passkey was refused: unknown, cancelled, for another site, or a
	/// signature that didn't check out.
	#[error("that passkey didn't work; try again")]
	PasskeyFailed,
	/// Removing it would leave the account with no way to sign in, or
	/// without the second factor it must have.
	#[error("keep at least one way to sign in, and a second factor if your account needs one")]
	LastSignInMethod,
	/// A name was empty.
	#[error("enter a name")]
	InvalidName,
	/// There's no such passkey or connected account, or it isn't theirs.
	#[error("that wasn't found")]
	NotFound,
	/// There's no provider by that name in `[auth.providers]`.
	#[error("that sign-in provider isn't set up")]
	UnknownProvider,
	/// The provider refused, or answered with something that didn't check
	/// out, such as an ID token with a bad signature. The details are for
	/// the server's logs, never the client.
	#[error("signing in with that account didn't work: {0}")]
	Provider(String),
	/// The person cancelled signing in at the provider.
	#[error("signing in was cancelled")]
	Cancelled,
	/// Where to go after signing in isn't a path on this site.
	#[error("that address to return to isn't on this site")]
	InvalidRedirect,
	/// A wallet's answer didn't check out: the message isn't the one
	/// issued, the challenge is unknown, used, or expired, or the signature
	/// is wrong.
	#[error("that wallet's signature didn't work; try again")]
	WalletFailed,
	/// The wallet is linked to another account, which it can't sign in to,
	/// so this can't prove that account is yours.
	#[error(
		"that wallet is linked to another account; sign in to it and unlink the wallet, or turn \
		 on signing in with it there to merge the two"
	)]
	WalletInUse,
	/// The wallet isn't linked to an account it may sign in to.
	#[error(
		"that wallet can't sign in here; link it from your account's settings and turn on signing \
		 in with it"
	)]
	UnknownWallet,
	/// The action needs a fresh signature from the wallet.
	#[error("sign with that wallet again to confirm it's still yours")]
	ProofRequired,
	/// Bringing the other account's data along broke one of the app's
	/// rules, such as a unique constraint; nothing changed. The table and
	/// the database's error are for the server's logs.
	#[error("the other account's data couldn't move here: {0}")]
	MergeConflict(String),
	/// The email address and password didn't match an account's. Which of
	/// the two was wrong, or whether the address has an account at all, is
	/// never said.
	#[error("that email and password didn't match")]
	InvalidPassword,
	/// A new password breaks a rule.
	#[error("{0}")]
	WeakPassword(PasswordProblem),
	/// This server or this device can't do that yet, such as passkeys
	/// without the `passkey` feature.
	#[error("{0}")]
	Unsupported(&'static str),
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

/// Why a new password was refused. There are no rules about which kinds
/// of characters it holds, as NIST SP 800-63B advises: only its length,
/// and whether it's too easy to guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PasswordProblem {
	/// It's shorter than the server's minimum, in characters.
	TooShort {
		/// The fewest characters allowed.
		min: u32,
	},
	/// It's longer than the most allowed, in characters.
	TooLong {
		/// The most characters allowed.
		max: u32,
	},
	/// It's one of the most common passwords.
	Common,
	/// It's the email address, its name, or the app's name, give or take
	/// some digits.
	Personal,
	/// It has appeared in a data breach, as Have I Been Pwned knows.
	Breached,
}

impl std::fmt::Display for PasswordProblem {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::TooShort { min } => write!(f, "use at least {min} characters"),
			Self::TooLong { max } => write!(f, "use at most {max} characters"),
			Self::Common => f.write_str("that password is too common; choose another"),
			Self::Personal => {
				f.write_str("that password is too close to your email address or this app's name")
			}
			Self::Breached => {
				f.write_str("that password has appeared in a data breach; choose another")
			}
		}
	}
}
