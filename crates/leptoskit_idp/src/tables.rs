//! The `idp` schema pack: what signing in with your app keeps, in the `idp`
//! namespace (`idp.grants` on Postgres, `idp_grants` on SQLite).
//!
//! It builds on the `auth` pack: grants and tokens belong to an
//! [`Account`], and go when it does. Apps read their own rows through
//! policies; only this crate writes them.

// Without `server`, the tables are markers for `Id<…>`, and their column
// types go unused.
#![cfg_attr(not(feature = "server"), allow(unused_imports))]

use leptoskit_auth::Account;
use leptoskit_db::Enum;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;

/// How an app proves it's the client it says, at the token endpoint.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClientAuth {
	/// A public client, such as a native or single-page app, which can't
	/// keep a secret. PKCE is what protects its codes.
	#[db(rename = "none")]
	None,
	/// A secret sent with HTTP Basic authentication: the default for apps
	/// with a server.
	#[db(rename = "client_secret_basic")]
	SecretBasic,
	/// A secret sent in the form body, for clients that can't send Basic
	/// authentication.
	#[db(rename = "client_secret_post")]
	SecretPost,
}

/// An app that may ask people to sign in with yours. Operators register
/// them with `lk auth idp client add`.
#[table(name = "clients", pack = "idp")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct Client {
	pub id: Id<Self>,
	/// The public identifier the app sends as `client_id`.
	#[db(unique, max_len = 64)]
	pub client_id: String,
	/// The app's name, as the consent screen shows it.
	#[db(min_len = 1, max_len = 100)]
	pub name: String,
	pub auth_method: ClientAuth,
	/// The SHA-256 of the client secret, for a confidential client.
	#[db(secret)]
	pub secret_hash: Option<Vec<u8>>,
	/// Where codes may be sent, compared exactly.
	pub redirect_uris: Json<Vec<String>>,
	/// Where people may be sent after signing out, compared exactly.
	pub post_logout_redirect_uris: Json<Vec<String>>,
	/// The scopes the app may ask for.
	pub scopes: Json<Vec<String>>,
	/// One of your own apps: it sees the account ID as `sub` instead of a
	/// pairwise one, and needs no consent.
	pub first_party: bool,
	/// Whether it may introspect tokens issued to other apps, as a resource
	/// server does.
	pub introspection: bool,
	/// Mixed into the pairwise `sub` each account gets for this app, so two
	/// apps can't match up their users.
	#[db(secret)]
	pub subject_salt: Vec<u8>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// What a person let an app do: the scopes they consented to. One per
/// account and app; revoking it ends every token the app holds for them.
#[table(name = "grants", pack = "idp", unique(account_id, client_id))]
#[policy(select, |grant, cx| cx.global(|me: &leptoskit_auth::SignedIn| grant.account_id.eq(me.account)))]
pub struct Grant {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub account_id: Id<Account>,
	#[db(on_delete = cascade)]
	pub client_id: Id<Client>,
	/// Every scope they consented to.
	pub scopes: Json<Vec<String>>,
	#[db(created_at)]
	pub created_at: Timestamp,
	/// When they last consented to more.
	pub updated_at: Timestamp,
	/// When the app last refreshed its tokens.
	pub last_used_at: Option<Timestamp>,
}

/// One sign-in of an app: the refresh tokens that descend from one
/// authorization code. A refresh token presented twice ends the whole
/// family, since one of the two holders stole it.
#[table(name = "families", pack = "idp", index(account_id))]
#[policy(select, |family, cx| cx.global(|me: &leptoskit_auth::SignedIn| family.account_id.eq(me.account)))]
pub struct Family {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub grant_id: Id<Grant>,
	#[db(on_delete = cascade)]
	pub account_id: Id<Account>,
	#[db(on_delete = cascade)]
	pub client_id: Id<Client>,
	/// The scopes its tokens may carry.
	pub scopes: Json<Vec<String>>,
	/// When the person last proved who they are, as `auth_time` says.
	pub auth_time: Timestamp,
	/// When its refresh tokens stop working however often they're used.
	pub expires_at: Timestamp,
	/// When it was revoked, signed out, or ended by a reused token.
	pub revoked_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A refresh token. Only its SHA-256 hash is kept, and it works once.
#[table(name = "refresh_tokens", pack = "idp")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct RefreshToken {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub family_id: Id<Family>,
	#[db(unique, secret)]
	pub token_hash: Vec<u8>,
	/// When it was traded for new tokens.
	pub used_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// An authorization request: waiting for consent, then an authorization
/// code, then used. Codes last a minute and work once.
#[table(name = "authorizations", pack = "idp", index(expires_at))]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct Authorization {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub client_id: Id<Client>,
	#[db(on_delete = cascade)]
	pub account_id: Id<Account>,
	#[db(max_len = 2048)]
	pub redirect_uri: String,
	pub scopes: Json<Vec<String>>,
	#[db(max_len = 2048, private)]
	pub state: Option<String>,
	#[db(max_len = 2048, private)]
	pub nonce: Option<String>,
	/// The PKCE `S256` challenge.
	#[db(max_len = 128)]
	pub code_challenge: String,
	pub auth_time: Timestamp,
	/// The handle the consent screen reads it by, while it waits.
	#[db(unique, secret)]
	pub consent_hash: Option<Vec<u8>>,
	/// The authorization code, once consent is given.
	#[db(unique, secret)]
	pub code_hash: Option<Vec<u8>>,
	pub expires_at: Timestamp,
	/// When it was answered or its code traded.
	pub used_at: Option<Timestamp>,
	/// The family its code started, which a replayed code ends.
	#[db(on_delete = cascade)]
	pub family_id: Option<Id<Family>>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A key that signs ID and access tokens. The database keeps only its
/// public half: the private key is derived from one of the server's
/// `AUTH_ENCRYPTION_KEYS` whenever it's needed.
#[table(name = "signing_keys", pack = "idp")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct SigningKey {
	pub id: Id<Self>,
	/// The key ID tokens name in their header.
	#[db(unique, max_len = 64)]
	pub kid: String,
	/// The ID of the server key it's derived from.
	#[db(max_len = 64)]
	pub derived_from: String,
	/// The P-256 public key's coordinates, in base64url.
	#[db(max_len = 64)]
	pub x: String,
	#[db(max_len = 64)]
	pub y: String,
	/// When it stops signing new tokens.
	pub retires_at: Timestamp,
	/// When it leaves the published key set, after every token it signed
	/// has expired.
	pub expires_at: Timestamp,
	#[db(created_at)]
	pub created_at: Timestamp,
}
