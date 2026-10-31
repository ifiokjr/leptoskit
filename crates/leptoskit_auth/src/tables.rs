//! The `auth` schema pack: the tables sign-in keeps, in the `auth` namespace
//! (`auth.sessions` on Postgres, `auth_sessions` on SQLite).
//!
//! Apps read them through policies that show each signed-in person only
//! their own rows, and link to [`Account`] with `Id<auth::Account>`. Only this
//! crate writes them.

use leptoskit_db::Enum;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;

/// One per person. Everything else about them, from their emails to their
/// sessions, points here; the app's own user table shares its key.
#[table(name = "accounts", pack = "auth", public)]
#[policy(select, |account, cx| account.id.eq(cx.setting::<crate::SignedIn>()))]
pub struct Account {
	pub id: Id<Self>,
	#[db(created_at)]
	pub created_at: Timestamp,
	/// When the account was disabled. A disabled account can't sign in, and
	/// its sessions stop working.
	pub disabled_at: Option<Timestamp>,
	/// Until when sign-in is refused, after too many failed attempts.
	pub locked_until: Option<Timestamp>,
}

/// An email address an account has proved it receives mail at.
#[table(name = "emails", pack = "auth")]
#[policy(select, |email, cx| email.account_id.eq(cx.setting::<crate::SignedIn>()))]
pub struct EmailAddress {
	pub id: Id<Self>,
	#[db(on_delete = cascade, backlink = emails)]
	pub account_id: Id<Account>,
	/// The address, trimmed and lowercased. Each belongs to one account.
	#[db(unique, private)]
	pub address: leptoskit_db::Email,
	/// When the account last proved it receives mail here.
	pub verified_at: Option<Timestamp>,
	/// Whether this is the address the account's mail goes to.
	pub is_primary: bool,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// How strongly a session's owner proved who they are, following NIST SP
/// 800-63B's authenticator assurance levels.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Aal {
	/// One factor, such as an email code.
	#[db(rename = "aal1")]
	One,
	/// Two factors, or a passkey that verified its user.
	#[db(rename = "aal2")]
	Two,
}

/// A signed-in device. The tokens that prove it live only on the device:
/// this row keeps their SHA-256 hashes, so a copy of the database signs no
/// one in.
///
/// A browser holds one token, in a cookie. A native app holds two: a
/// short-lived access token it sends with each request, and a refresh token
/// that trades for a new pair and works once.
#[table(name = "sessions", pack = "auth", public)]
#[policy(select, |session, cx| session.account_id.eq(cx.setting::<crate::SignedIn>()))]
pub struct Session {
	pub id: Id<Self>,
	#[db(on_delete = cascade, backlink = sessions)]
	pub account_id: Id<Account>,
	/// The cookie's token, or a native app's access token.
	#[db(unique, secret)]
	pub token_hash: Vec<u8>,
	/// When a native app's access token stops working, so it must refresh.
	/// `None` for a browser's cookie.
	#[db(private)]
	pub access_expires_at: Option<Timestamp>,
	/// A native app's refresh token.
	#[db(unique, secret)]
	pub refresh_hash: Option<Vec<u8>>,
	/// The refresh token this one replaced. Seen again after a short grace,
	/// it means someone else has a copy, and the session ends.
	#[db(unique, secret)]
	pub previous_refresh_hash: Option<Vec<u8>>,
	/// When the refresh token was last replaced.
	#[db(private)]
	pub refreshed_at: Option<Timestamp>,
	pub aal: Aal,
	/// When its owner last proved who they are, which carries over when the
	/// session's token is replaced.
	pub authenticated_at: Timestamp,
	/// When its owner last completed a second factor.
	pub mfa_at: Option<Timestamp>,
	/// When the session was last used, to the minute.
	pub last_seen_at: Timestamp,
	/// When it ends unless it's used before then.
	pub idle_expires_at: Timestamp,
	/// When it ends however often it's used.
	pub expires_at: Timestamp,
	/// What signed in, such as "Firefox on macOS", for the list of devices.
	#[db(max_len = 200)]
	pub device: String,
	/// When it was signed out or revoked.
	pub revoked_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// What a [`Challenge`] is for.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Purpose {
	/// A code sent to an email address, to sign in or sign up with.
	EmailCode,
}

/// A short-lived secret a sign-in method sent or expects, such as an email
/// code. Only its keyed hash is stored, it expires, and it works once.
#[table(name = "challenges", pack = "auth", index(purpose, subject))]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct Challenge {
	pub id: Id<Self>,
	pub purpose: Purpose,
	/// Who it was issued to, such as an email address.
	#[db(max_len = 320, private)]
	pub subject: String,
	#[db(secret)]
	pub secret_hash: Vec<u8>,
	/// Failed attempts so far.
	#[db(min = 0)]
	pub attempts: i32,
	pub expires_at: Timestamp,
	pub used_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// What an [`Event`] records.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
	/// A new account was created.
	SignedUp,
	/// A session started.
	SignedIn,
	/// A sign-in attempt failed.
	SignInFailed,
	/// A session was signed out from its own device.
	SignedOut,
	/// A session was revoked from another device.
	SessionRevoked,
	/// A refresh token that had already been used was presented again, so
	/// its session was ended.
	RefreshReused,
	/// A sign-in code was emailed.
	CodeSent,
}

/// The audit trail: what happened to an account, and from where. Never
/// credentials.
#[table(name = "events", pack = "auth", index(created_at))]
#[policy(select, |event, cx| event.account_id.eq(cx.setting::<crate::SignedIn>()))]
pub struct Event {
	pub id: Id<Self>,
	#[db(on_delete = cascade, backlink = events)]
	pub account_id: Option<Id<Account>>,
	pub kind: EventKind,
	#[db(max_len = 64, private)]
	pub ip: Option<String>,
	#[db(max_len = 512, private)]
	pub user_agent: Option<String>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A fixed-window counter for one action from one source, such as code
/// requests for one address. The key is a hash, so the table holds no
/// addresses.
#[table(name = "rate_limits", pack = "auth")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct RateLimit {
	pub id: Id<Self>,
	#[db(unique, secret)]
	pub key: Vec<u8>,
	pub window_start: Timestamp,
	#[db(min = 0)]
	pub count: i32,
}
