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
#[policy(select, |account, cx| cx.global(|me: &crate::SignedIn| account.id.eq(me.account)))]
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
#[policy(select, |email, cx| cx.global(|me: &crate::SignedIn| email.account_id.eq(me.account)))]
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
#[policy(select, |session, cx| cx.global(|me: &crate::SignedIn| session.account_id.eq(me.account)))]
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
	/// The second step of signing in, after the first factor, to an account
	/// with a second factor. Its subject is the account.
	SecondFactor,
	/// Adding a passkey to a signed-in account. Its subject is the session,
	/// and its state is the sealed WebAuthn registration.
	PasskeyRegistration,
	/// Signing in with a passkey, or using one as a second factor or for
	/// step-up. Its state is the sealed WebAuthn authentication.
	PasskeyAuthentication,
	/// Signing in with, or connecting, a third-party account. Its secret is
	/// the hash of the `state` sent to the provider, and its state the sealed
	/// PKCE verifier, nonce, and what the flow is for.
	Provider,
	/// A third-party sign-in that found an email address another account
	/// already has, waiting for the person to choose a separate account.
	ProviderSignUp,
	/// An offer to merge another account into the signed-in one, after the
	/// person proved they control it.
	Merge,
	/// Linking a wallet to the signed-in account. Its subject is the
	/// session, and its state the Sign-In With Solana message's fields.
	WalletLink,
	/// A linked wallet proving it's still the person's. Its subject is the
	/// session.
	WalletProof,
	/// Signing in with a wallet. Its token finds it.
	WalletSignIn,
}

/// A short-lived secret a sign-in method sent or expects, such as an email
/// code or the second step of signing in. Only its hash is stored, it
/// expires, and it works once.
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
	/// What the ceremony needs to finish, such as a WebAuthn challenge or a
	/// PKCE verifier: sealed with the server's keys and bound to this row.
	#[db(secret)]
	pub state: Option<Vec<u8>>,
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
	/// An authenticator app was turned on.
	TotpEnabled,
	/// The authenticator app was turned off, and the recovery codes deleted.
	TotpDisabled,
	/// A code from an authenticator app or a recovery code was wrong.
	SecondFactorFailed,
	/// A signed-in session confirmed it's still its owner with a second
	/// factor.
	SteppedUp,
	/// A recovery code was used, and the account's other sessions ended.
	RecoveryCodeUsed,
	/// New recovery codes replaced the old ones.
	RecoveryCodesRegenerated,
	/// A passkey was added.
	PasskeyAdded,
	/// A passkey was removed.
	PasskeyRemoved,
	/// A passkey was refused: an unknown key, a bad signature, or a
	/// counter that went backwards, which can mean a cloned key.
	PasskeyFailed,
	/// Too many wrong second factors: the account was locked for a while,
	/// and its owner emailed.
	Locked,
	/// An administrator lifted a lock.
	Unlocked,
	/// An administrator removed every second factor and passkey, and ended
	/// every session.
	MfaReset,
	/// A third-party account was connected.
	IdentityLinked,
	/// A third-party account was disconnected.
	IdentityUnlinked,
	/// Another account was merged into this one.
	AccountMerged,
	/// A wallet was linked, with a signed proof.
	WalletLinked,
	/// A wallet was unlinked.
	WalletUnlinked,
	/// A linked wallet signed a fresh proof.
	WalletProved,
	/// Signing in with a wallet was turned on or off.
	WalletSignInChanged,
	/// A wallet's signature was refused.
	WalletFailed,
}

/// The audit trail: what happened to an account, and from where. Never
/// credentials.
#[table(name = "events", pack = "auth", index(created_at))]
#[policy(select, |event, cx| cx.global(|me: &crate::SignedIn| event.account_id.eq(me.account)))]
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

/// An account's authenticator app: the TOTP secret it shares with the
/// server. The secret is encrypted with the server's keys, so a copy of the
/// database can't produce codes, and bound to its account.
#[table(name = "totp", pack = "auth")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct TotpSecret {
	pub id: Id<Self>,
	/// One authenticator per account.
	#[db(unique, on_delete = cascade)]
	pub account_id: Id<Account>,
	/// AES-256-GCM-SIV under the newest key, with the key's ID.
	#[db(secret)]
	pub secret: Vec<u8>,
	/// When a code from the app first matched. Until then the app isn't
	/// trusted to sign in with.
	pub confirmed_at: Option<Timestamp>,
	/// The last 30-second step whose code was used, so no code works twice.
	pub last_used_step: Option<i64>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// One of an account's recovery codes, for when its authenticator is lost.
/// Each works once; only its keyed hash is stored.
#[table(name = "recovery_codes", pack = "auth", index(account_id))]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct RecoveryCode {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub account_id: Id<Account>,
	#[db(secret)]
	pub code_hash: Vec<u8>,
	pub used_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A passkey: a key pair on the person's device, or synced through their
/// password manager, whose public half signs them in. Only public keys are
/// stored.
#[table(name = "passkeys", pack = "auth", index(account_id))]
#[policy(select, |passkey, cx| cx.global(|me: &crate::SignedIn| passkey.account_id.eq(me.account)))]
pub struct Passkey {
	pub id: Id<Self>,
	#[db(on_delete = cascade, backlink = passkeys)]
	pub account_id: Id<Account>,
	/// The credential ID the device names the key by. Each belongs to one
	/// account.
	#[db(unique, private)]
	pub credential_id: Vec<u8>,
	/// The public key, its algorithm, and its signature counter, as
	/// `webauthn-rs` keeps them, in JSON.
	#[db(private)]
	pub credential: Vec<u8>,
	/// What the person calls it, such as "iCloud Keychain".
	#[db(min_len = 1, max_len = 100)]
	pub name: String,
	/// Whether it's synced to other devices, such as through a password
	/// manager, rather than bound to one.
	pub backed_up: bool,
	/// When it last signed in or confirmed it's them.
	pub last_used_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A third-party account someone signs in with, such as their Google
/// account. Who they are there is the provider and the provider's subject,
/// never the email address, which some providers let people change to
/// anything.
#[table(
	name = "identities",
	pack = "auth",
	unique(provider, subject),
	index(account_id)
)]
#[policy(select, |identity, cx| cx.global(|me: &crate::SignedIn| identity.account_id.eq(me.account)))]
pub struct Identity {
	pub id: Id<Self>,
	#[db(on_delete = cascade, backlink = identities)]
	pub account_id: Id<Account>,
	/// The provider's ID in `[auth.providers]`, such as `google`.
	#[db(max_len = 64)]
	pub provider: String,
	/// The provider's stable ID for the person.
	#[db(max_len = 255, private)]
	pub subject: String,
	/// The address the provider gave, as it gave it.
	#[db(max_len = 320, private)]
	pub email: Option<String>,
	/// Whether the provider vouched for that address. Even then, it never
	/// connects accounts by itself.
	pub email_verified: bool,
	/// Their name or username there, to tell connected accounts apart.
	#[db(max_len = 200, private)]
	pub name: Option<String>,
	/// When it last signed in.
	pub last_used_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A wallet linked to an account, with proof that the account's owner
/// controls it: a Sign-In With Solana message signed by its key. A wallet
/// belongs to one account. It's private unless its owner shows it, and
/// signs in only when its owner turns that on.
#[table(
	name = "wallets",
	pack = "auth",
	unique(chain, address),
	index(account_id)
)]
#[policy(select, |wallet, cx| cx.global(|me: &crate::SignedIn| wallet.account_id.eq(me.account)))]
pub struct Wallet {
	pub id: Id<Self>,
	#[db(on_delete = cascade, backlink = wallets)]
	pub account_id: Id<Account>,
	/// The chain: `solana`.
	#[db(max_len = 32)]
	pub chain: String,
	/// The address.
	#[db(private)]
	pub address: leptoskit_db::Pubkey,
	/// What the person calls it, such as "Treasury".
	#[db(max_len = 100)]
	pub label: Option<String>,
	/// Whether it may sign in to the account. Off until its owner turns it
	/// on.
	pub can_sign_in: bool,
	/// Whether it shows on its owner's public profile.
	pub is_public: bool,
	/// When it was linked.
	pub verified_at: Timestamp,
	/// When it last signed a proof.
	pub last_proved_at: Timestamp,
	/// The latest signed message, kept for audit.
	#[db(private, max_len = 4096)]
	pub proof: String,
	/// The message's signature.
	#[db(private)]
	pub proof_signature: leptoskit_db::Signature,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// An account that was merged into another. Old references to it, such as
/// profile links, resolve to the account that absorbed it.
#[table(name = "merged_accounts", pack = "auth")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct MergedAccount {
	pub id: Id<Self>,
	/// The account that no longer exists.
	#[db(unique, no_foreign_key)]
	pub absorbed_id: Id<Account>,
	/// The account it became part of.
	#[db(on_delete = cascade)]
	pub survivor_id: Id<Account>,
	#[db(created_at)]
	pub merged_at: Timestamp,
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
