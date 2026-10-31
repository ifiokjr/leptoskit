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
use serde_json::Value;

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
	/// A short-lived JWT the app signs with its own private key, whose
	/// public half it registered: no shared secret exists to leak.
	#[db(rename = "private_key_jwt")]
	PrivateKeyJwt,
}

/// How an app gets tokens.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GrantType {
	/// Sends people to sign in and consent in a browser, then trades the code
	/// it gets back. Its tokens refresh.
	#[db(rename = "authorization_code")]
	AuthorizationCode,
	/// Shows a short code that people enter on another device, for CLIs and
	/// TVs (RFC 8628). Its tokens refresh.
	#[db(rename = "urn:ietf:params:oauth:grant-type:device_code")]
	DeviceCode,
	/// Acts as itself, with no person, for one server calling another. Needs
	/// a secret or a key.
	#[db(rename = "client_credentials")]
	ClientCredentials,
}

/// How the provider signs a token.
#[derive(Enum, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SigningAlgorithm {
	/// ECDSA over P-256 with SHA-256: small, fast, and the default.
	#[default]
	#[db(rename = "ES256")]
	Es256,
	/// RSA PKCS#1 v1.5 with SHA-256, for apps whose libraries only verify
	/// RSA. It needs an RSA key configured on the server.
	#[db(rename = "RS256")]
	Rs256,
}

/// An app that may ask people to sign in with yours. Operators register
/// them with `lk auth idp client add`; with the developer portal on, people
/// register their own, which they alone see.
#[table(name = "clients", pack = "idp", index(owner_id))]
#[policy(select, |client, cx| cx.global(|me: &leptoskit_auth::SignedIn| client.owner_id.eq(me.account)))]
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
	/// The public keys a `private_key_jwt` client signs with, as a JWK set.
	pub jwks: Option<Json<Value>>,
	/// How it gets tokens.
	pub grant_types: Json<Vec<GrantType>>,
	/// How its ID tokens are signed.
	pub id_token_alg: SigningAlgorithm,
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
	/// Who registered it in the developer portal; `None` for an app the
	/// operator registered.
	#[db(on_delete = cascade)]
	pub owner_id: Option<Id<Account>>,
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
/// authorization code or device code. A refresh token presented twice ends
/// the whole family, since one of the two holders stole it.
///
/// An app acting as itself (`client_credentials`) gets a family with no
/// person, grant or refresh token, so its access tokens can be revoked
/// too.
#[table(name = "families", pack = "idp", index(account_id), index(expires_at))]
#[policy(select, |family, cx| cx.global(|me: &leptoskit_auth::SignedIn| family.account_id.eq(me.account)))]
pub struct Family {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub grant_id: Option<Id<Grant>>,
	#[db(on_delete = cascade)]
	pub account_id: Option<Id<Account>>,
	#[db(on_delete = cascade)]
	pub client_id: Id<Client>,
	/// The scopes its tokens may carry.
	pub scopes: Json<Vec<String>>,
	/// Claims the app asked to find in its ID tokens with the `claims`
	/// parameter, such as `email`. Only claims its scopes release appear.
	pub id_token_claims: Json<Vec<String>>,
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
	/// Claims asked for in the ID token with the `claims` parameter.
	pub id_token_claims: Json<Vec<String>>,
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
/// public half. An ES256 key's private half is derived from one of the
/// server's `AUTH_ENCRYPTION_KEYS` whenever it's needed; an RS256 key's is
/// the RSA key the server was given.
#[table(name = "signing_keys", pack = "idp")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct SigningKey {
	pub id: Id<Self>,
	/// The key ID tokens name in their header.
	#[db(unique, max_len = 64)]
	pub kid: String,
	pub algorithm: SigningAlgorithm,
	/// The ID of the server key an ES256 key is derived from.
	#[db(max_len = 64)]
	pub derived_from: Option<String>,
	/// An ES256 key's P-256 coordinates, in base64url.
	#[db(max_len = 64)]
	pub x: Option<String>,
	#[db(max_len = 64)]
	pub y: Option<String>,
	/// An RS256 key's modulus and exponent, in base64url.
	#[db(max_len = 1400)]
	pub n: Option<String>,
	#[db(max_len = 16)]
	pub e: Option<String>,
	/// When it stops signing new tokens.
	pub retires_at: Timestamp,
	/// When it leaves the published key set, after every token it signed
	/// has expired.
	pub expires_at: Timestamp,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// An authorization request an app pushed ahead of sending the browser
/// (RFC 9126), so its parameters travel server to server rather than
/// through the address bar. It works once, and soon expires.
#[table(name = "pushed_requests", pack = "idp", index(expires_at))]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct PushedRequest {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub client_id: Id<Client>,
	/// The `request_uri` handle, hashed.
	#[db(unique, secret)]
	pub handle_hash: Vec<u8>,
	/// The request's parameters.
	#[db(private)]
	pub request: Json<Value>,
	pub expires_at: Timestamp,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A sign-in started on a device with no browser, such as a CLI or a TV
/// (RFC 8628): it shows a short code, someone enters it on their phone or
/// computer and approves, and the device's polling then gets tokens.
#[table(name = "device_codes", pack = "idp", index(expires_at))]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct DeviceCode {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub client_id: Id<Client>,
	/// The secret the device polls with, hashed.
	#[db(unique, secret)]
	pub device_hash: Vec<u8>,
	/// The code people type, hashed.
	#[db(unique, secret)]
	pub user_code_hash: Vec<u8>,
	/// What the device asked to do.
	pub scopes: Json<Vec<String>>,
	/// Who approved or refused it.
	#[db(on_delete = cascade)]
	pub account_id: Option<Id<Account>>,
	/// When they last proved who they are, once they approved.
	pub auth_time: Option<Timestamp>,
	/// Whether they approved, once they answered.
	pub approved: Option<bool>,
	/// Seconds the device must wait between polls.
	#[db(min = 1)]
	pub interval: i32,
	/// When the device last polled.
	pub polled_at: Option<Timestamp>,
	pub expires_at: Timestamp,
	/// When the device traded it for tokens.
	pub used_at: Option<Timestamp>,
	/// The family it started, which a replayed device code ends.
	#[db(on_delete = cascade)]
	pub family_id: Option<Id<Family>>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A `private_key_jwt` assertion an app authenticated with, kept until it
/// expires so it can't be used twice.
#[table(name = "assertions", pack = "idp", index(expires_at))]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct Assertion {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub client_id: Id<Client>,
	/// The assertion's `jti`, hashed with the client's ID.
	#[db(unique, secret)]
	pub jti_hash: Vec<u8>,
	pub expires_at: Timestamp,
}
