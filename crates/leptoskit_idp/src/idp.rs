//! [`Idp`]: the provider, and how an app configures it.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_rpc::Scopes;
use serde_json::Value;

use crate::IdpError;
use crate::Result;
use crate::ScopeDescription;

/// Sign in with your app: the OAuth 2.1 and OpenID Connect provider, over
/// the `idp` and `auth` tables in the app's database.
///
/// Build one at startup, next to [`Auth`], and share it; clones are cheap.
///
/// ```ignore
/// let idp = Idp::builder(&db, &auth)
/// 	.issuer("https://myapp.com")
/// 	.scopes::<Scope>()
/// 	.claims::<User>()
/// 	.sign_in_page("/sign-in")
/// 	.consent_page("/connect")
/// 	.build()?;
/// ```
#[derive(Clone)]
pub struct Idp {
	pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
	/// The app's database, bypassing policies: only this crate's own
	/// queries run on it.
	pub db: Db,
	pub auth: Auth,
	pub issuer: String,
	pub audience: String,
	/// Every scope a client may ask for: the OpenID Connect ones, then the
	/// app's.
	pub scopes: Vec<ScopeDescription>,
	pub claims: ClaimsHook,
	pub sign_in_page: String,
	pub consent_page: String,
	pub signed_out_page: String,
	pub lifetimes: Lifetimes,
	/// Public keys by key ID, with when each leaves the key set: they never
	/// change, so once read they're kept.
	pub verifying_keys: Mutex<HashMap<String, (p256::ecdsa::VerifyingKey, Timestamp)>>,
}

impl Idp {
	/// Starts configuring the provider over `db`, signing people in through
	/// `auth`. Its keys derive from `auth`'s.
	pub fn builder(db: &Db, auth: &Auth) -> IdpBuilder {
		IdpBuilder {
			db: db.system("idp: sign in with this app"),
			auth: auth.clone(),
			issuer: auth.origin().map(str::to_owned),
			audience: None,
			scopes: Vec::new(),
			claims: |_, _, _, _| Box::pin(async { Ok(()) }),
			sign_in_page: "/sign-in".into(),
			consent_page: "/consent".into(),
			signed_out_page: "/".into(),
			lifetimes: Lifetimes::default(),
		}
	}

	/// The issuer: the app's origin, such as `https://myapp.com`.
	pub fn issuer(&self) -> &str {
		&self.inner.issuer
	}

	/// The scopes clients may ask for, with what each allows.
	pub fn scopes(&self) -> &[ScopeDescription] {
		&self.inner.scopes
	}

	/// The OpenID Connect discovery document, which
	/// `/.well-known/openid-configuration` serves.
	pub fn discovery(&self) -> Value {
		let issuer = &self.inner.issuer;
		let endpoint = |path: &str| format!("{issuer}{path}");
		let scopes: Vec<&str> = self
			.inner
			.scopes
			.iter()
			.map(|scope| scope.name.as_str())
			.collect();

		serde_json::json!({
			"issuer": issuer,
			"authorization_endpoint": endpoint("/oauth/authorize"),
			"token_endpoint": endpoint("/oauth/token"),
			"userinfo_endpoint": endpoint("/oauth/userinfo"),
			"jwks_uri": endpoint("/oauth/jwks"),
			"revocation_endpoint": endpoint("/oauth/revoke"),
			"introspection_endpoint": endpoint("/oauth/introspect"),
			"end_session_endpoint": endpoint("/oauth/end_session"),
			"scopes_supported": scopes,
			"response_types_supported": ["code"],
			"response_modes_supported": ["query"],
			"grant_types_supported": ["authorization_code", "refresh_token"],
			"subject_types_supported": ["pairwise", "public"],
			"id_token_signing_alg_values_supported": [crate::jwt::ALGORITHM],
			"token_endpoint_auth_methods_supported":
				["client_secret_basic", "client_secret_post", "none"],
			"revocation_endpoint_auth_methods_supported":
				["client_secret_basic", "client_secret_post", "none"],
			"introspection_endpoint_auth_methods_supported":
				["client_secret_basic", "client_secret_post"],
			"code_challenge_methods_supported": ["S256"],
			"prompt_values_supported": ["none", "login", "consent"],
			"claims_supported": [
				"sub", "iss", "aud", "exp", "iat", "auth_time", "nonce", "sid",
				"email", "email_verified", "name", "given_name", "family_name",
				"preferred_username", "picture", "locale", "zoneinfo", "updated_at",
			],
			"claims_parameter_supported": false,
			"request_parameter_supported": false,
			"request_uri_parameter_supported": false,
			"authorization_response_iss_parameter_supported": true,
		})
	}

	/// What `scope` lets an app do, for a consent screen.
	pub(crate) fn describe(&self, scopes: &[String]) -> Vec<ScopeDescription> {
		scopes
			.iter()
			.map(|name| {
				self.inner
					.scopes
					.iter()
					.find(|scope| scope.name == *name)
					.cloned()
					.unwrap_or_else(|| {
						ScopeDescription {
							name: name.clone(),
							description: String::new(),
						}
					})
			})
			.collect()
	}

	/// Whether clients may ask for `scope`.
	pub(crate) fn knows_scope(&self, scope: &str) -> bool {
		self.inner.scopes.iter().any(|known| known.name == scope)
	}

	pub(crate) fn lifetime(duration: Duration) -> Result<chrono::TimeDelta> {
		chrono::TimeDelta::from_std(duration)
			.map_err(|_| IdpError::Config("a lifetime is too long".into()))
	}
}

impl std::fmt::Debug for Idp {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Idp")
			.field("issuer", &self.inner.issuer)
			.finish_non_exhaustive()
	}
}

/// The scopes OpenID Connect defines, which every provider offers.
const STANDARD_SCOPES: [(&str, &str); 3] = [
	("openid", "Sign you in with your account."),
	("profile", "See your name and profile picture."),
	("email", "See your email address."),
];

/// Configures an [`Idp`].
#[must_use]
pub struct IdpBuilder {
	db: Db,
	auth: Auth,
	issuer: Option<String>,
	audience: Option<String>,
	scopes: Vec<leptoskit_rpc::ScopeInfo>,
	claims: ClaimsHook,
	sign_in_page: String,
	consent_page: String,
	signed_out_page: String,
	lifetimes: Lifetimes,
}

impl std::fmt::Debug for IdpBuilder {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("IdpBuilder")
			.field("issuer", &self.issuer)
			.finish_non_exhaustive()
	}
}

impl IdpBuilder {
	/// The issuer, which tokens name and apps discover the provider at: the
	/// app's own origin, such as `https://myapp.com`, with no path. Defaults
	/// to `auth`'s origin.
	pub fn issuer(mut self, issuer: impl Into<String>) -> Self {
		self.issuer = Some(issuer.into().trim_end_matches('/').to_owned());
		self
	}

	/// What access tokens name as their audience: your API. Defaults to the
	/// issuer, for an app whose API is on its own origin.
	pub fn audience(mut self, audience: impl Into<String>) -> Self {
		self.audience = Some(audience.into());
		self
	}

	/// Offers the scopes `S` declares, from
	/// [`scopes!`](leptoskit_rpc::scopes!), as well as `openid`, `profile`
	/// and `email`. Call it once per set.
	pub fn scopes<S: Scopes>(mut self) -> Self {
		self.scopes.extend_from_slice(S::ALL);
		self
	}

	/// Fills in the claims apps see about a person, beyond `sub` and their
	/// verified email, with `C::claims`.
	pub fn claims<C: Claims + 'static>(mut self) -> Self {
		self.claims = |db, account, scopes, out| Box::pin(C::claims(db, account, scopes, out));
		self
	}

	/// Your sign-in page, which `/oauth/authorize` sends signed-out people to
	/// with `?return_to=…`. Sign them in, then send them there. Defaults to
	/// `/sign-in`.
	pub fn sign_in_page(mut self, path: impl Into<String>) -> Self {
		self.sign_in_page = path.into();
		self
	}

	/// Your consent page, which `/oauth/authorize` sends people to with
	/// `?request=…`. It reads `GET /oauth/consent?request=…` and posts the
	/// answer to `POST /oauth/consent`. Defaults to `/consent`.
	pub fn consent_page(mut self, path: impl Into<String>) -> Self {
		self.consent_page = path.into();
		self
	}

	/// Where `/oauth/end_session` sends people when the app that signed them
	/// out named nowhere to go. Defaults to `/`.
	pub fn signed_out_page(mut self, path: impl Into<String>) -> Self {
		self.signed_out_page = path.into();
		self
	}

	/// How long codes, tokens and signing keys last.
	pub fn lifetimes(mut self, lifetimes: Lifetimes) -> Self {
		self.lifetimes = lifetimes;
		self
	}

	/// The configured provider.
	///
	/// # Errors
	///
	/// [`IdpError::Config`] without an issuer, with one that isn't an
	/// `https` origin (or `http` on localhost), with a page that isn't a
	/// path on this site, with a scope that isn't one or is declared twice,
	/// or with lifetimes out of bounds.
	pub fn build(self) -> Result<Idp> {
		let issuer = self.issuer.ok_or_else(|| {
			IdpError::Config("set the issuer, or `origin` on Auth, to the app's own origin".into())
		})?;
		check_issuer(&issuer)?;

		for page in [
			&self.sign_in_page,
			&self.consent_page,
			&self.signed_out_page,
		] {
			if !is_local_path(page) {
				return Err(IdpError::Config(format!(
					"`{page}` must be a path on this site, such as `/consent`"
				)));
			}
		}

		let mut scopes: Vec<ScopeDescription> = STANDARD_SCOPES
			.iter()
			.map(|(name, description)| {
				ScopeDescription {
					name: (*name).into(),
					description: (*description).into(),
				}
			})
			.collect();

		for scope in &self.scopes {
			if !leptoskit_rpc::is_scope_name(scope.name) {
				return Err(IdpError::Config(format!("`{}` isn't a scope", scope.name)));
			}
			if scopes.iter().any(|known| known.name == scope.name) {
				return Err(IdpError::Config(format!(
					"the scope `{}` is declared twice",
					scope.name
				)));
			}
			scopes.push(ScopeDescription {
				name: scope.name.into(),
				description: scope.description(),
			});
		}

		self.lifetimes.check()?;

		Ok(Idp {
			inner: Arc::new(Inner {
				db: self.db,
				auth: self.auth,
				audience: self.audience.unwrap_or_else(|| issuer.clone()),
				issuer,
				scopes,
				claims: self.claims,
				sign_in_page: self.sign_in_page,
				consent_page: self.consent_page,
				signed_out_page: self.signed_out_page,
				lifetimes: self.lifetimes,
				verifying_keys: Mutex::new(HashMap::new()),
			}),
		})
	}
}

/// An issuer is an `https` origin, or `http` on localhost for development,
/// with no path, query or fragment: discovery lives at its root.
fn check_issuer(issuer: &str) -> Result<()> {
	let url = url::Url::parse(issuer)
		.map_err(|_| IdpError::Config(format!("the issuer `{issuer}` isn't a URL")))?;
	let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
	let secure = url.scheme() == "https" || (url.scheme() == "http" && local);
	let bare = url.path() == "/" && url.query().is_none() && url.fragment().is_none();

	if secure && bare && url.username().is_empty() && url.password().is_none() {
		Ok(())
	} else {
		Err(IdpError::Config(format!(
			"the issuer `{issuer}` must be an https origin with no path, such as \
			 `https://myapp.com`"
		)))
	}
}

/// A path on this site, which can't be read as another host's address.
pub(crate) fn is_local_path(path: &str) -> bool {
	path.starts_with('/') && !path.starts_with("//") && !path.contains('\\')
}

/// How long the provider's codes, tokens and keys last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lifetimes {
	/// An access token: 10 minutes, at most an hour. Revocation reaches
	/// resource servers that only check signatures after this long.
	pub access: Duration,
	/// An ID token: 10 minutes.
	pub id_token: Duration,
	/// An authorization code: one minute, at most ten.
	pub code: Duration,
	/// How long a consent screen may wait for an answer: 10 minutes.
	pub consent: Duration,
	/// A refresh-token family, however often it's used: 90 days.
	pub refresh: Duration,
	/// How long a signing key signs before the next one takes over: 30
	/// days.
	pub signing_key: Duration,
}

impl Default for Lifetimes {
	fn default() -> Self {
		Self {
			access: Duration::from_mins(10),
			id_token: Duration::from_mins(10),
			code: Duration::from_secs(60),
			consent: Duration::from_mins(10),
			refresh: Duration::from_hours(90 * 24),
			signing_key: Duration::from_hours(30 * 24),
		}
	}
}

impl Lifetimes {
	fn check(&self) -> Result<()> {
		let within = |value: Duration, max: Duration| !value.is_zero() && value <= max;

		if within(self.access, Duration::from_hours(1))
			&& within(self.id_token, Duration::from_hours(24))
			&& within(self.code, Duration::from_mins(10))
			&& within(self.consent, Duration::from_hours(1))
			&& within(self.refresh, Duration::from_hours(400 * 24))
			&& within(self.signing_key, Duration::from_hours(400 * 24))
		{
			Ok(())
		} else {
			Err(IdpError::Config(
				"lifetimes must be positive; access tokens last at most an hour, and codes at \
				 most ten minutes"
					.into(),
			))
		}
	}
}

/// The scopes a person granted the app asking about them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantedScopes {
	scopes: Vec<String>,
}

impl GrantedScopes {
	pub(crate) fn new(scopes: Vec<String>) -> Self {
		Self { scopes }
	}

	/// Whether `scope` was granted, such as `"profile"` or
	/// `Scope::PostsRead.as_str()`.
	pub fn has(&self, scope: &str) -> bool {
		self.scopes.iter().any(|granted| granted == scope)
	}

	/// Every granted scope.
	pub fn iter(&self) -> impl Iterator<Item = &str> {
		self.scopes.iter().map(String::as_str)
	}
}

/// Claims about a person, as `userinfo` releases them.
///
/// The provider sets `sub` and, with the `email` scope, the person's
/// verified primary email itself; those names, and the token claims (`iss`,
/// `aud`, `exp`, …), are ignored here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClaimSet {
	claims: BTreeMap<String, Value>,
}

/// Claims only the provider sets.
const RESERVED_CLAIMS: [&str; 11] = [
	"sub",
	"iss",
	"aud",
	"exp",
	"iat",
	"nbf",
	"jti",
	"auth_time",
	"nonce",
	"email",
	"email_verified",
];

impl ClaimSet {
	/// Sets the claim `name`, unless only the provider may set it.
	pub fn insert(&mut self, name: &str, value: impl Into<Value>) -> &mut Self {
		if !RESERVED_CLAIMS.contains(&name) {
			self.claims.insert(name.to_owned(), value.into());
		}
		self
	}

	/// `name`: their full name.
	pub fn name(&mut self, name: &str) -> &mut Self {
		self.insert("name", name)
	}

	/// `preferred_username`: the handle they go by.
	pub fn preferred_username(&mut self, username: &str) -> &mut Self {
		self.insert("preferred_username", username)
	}

	/// `picture`: the address of their profile picture.
	pub fn picture(&mut self, url: &str) -> &mut Self {
		self.insert("picture", url)
	}

	/// The claims set so far.
	pub fn get(&self, name: &str) -> Option<&Value> {
		self.claims.get(name)
	}

	pub(crate) fn into_inner(self) -> BTreeMap<String, Value> {
		self.claims
	}
}

/// Releases claims about a person to the apps they connected, such as their
/// name, from the app's own tables.
///
/// ```ignore
/// impl idp::Claims for User {
/// 	async fn claims(db: &Db, account: Id<auth::Account>, scopes: &idp::GrantedScopes, out: &mut idp::ClaimSet) -> idp::Result<()> {
/// 		let Some(user) = User::find(db, account).await? else { return Ok(()) };
/// 		if scopes.has("profile") {
/// 			out.name(&user.display_name).preferred_username(&user.handle);
/// 		}
/// 		Ok(())
/// 	}
/// }
/// ```
///
/// `db` acts as the person, so their own policies apply.
pub trait Claims {
	/// Adds the claims `scopes` allow about `account` to `out`.
	fn claims(
		db: &Db,
		account: Id<Account>,
		scopes: &GrantedScopes,
		out: &mut ClaimSet,
	) -> impl Future<Output = Result<()>> + Send;
}

pub(crate) type ClaimsHook = for<'a> fn(
	&'a Db,
	Id<Account>,
	&'a GrantedScopes,
	&'a mut ClaimSet,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn issuers_are_bare_secure_origins() {
		for issuer in [
			"https://myapp.com",
			"http://localhost:3000",
			"http://127.0.0.1:8080",
		] {
			assert!(check_issuer(issuer).is_ok(), "{issuer}");
		}
		for issuer in [
			"http://myapp.com",
			"https://myapp.com/idp",
			"https://myapp.com?x=1",
			"https://user@myapp.com",
			"myapp.com",
		] {
			assert!(check_issuer(issuer).is_err(), "{issuer}");
		}
	}

	#[test]
	fn pages_are_local_paths() {
		assert!(is_local_path("/consent"));
		assert!(!is_local_path("//evil.example"));
		assert!(!is_local_path("/\\evil.example"));
		assert!(!is_local_path("https://evil.example"));
	}

	#[test]
	fn lifetimes_are_bounded() {
		assert!(Lifetimes::default().check().is_ok());
		let long = Lifetimes {
			access: Duration::from_hours(2),
			..Lifetimes::default()
		};
		assert!(long.check().is_err());
		let zero = Lifetimes {
			code: Duration::ZERO,
			..Lifetimes::default()
		};
		assert!(zero.check().is_err());
	}

	#[test]
	fn claims_the_provider_owns_are_ignored() {
		let mut claims = ClaimSet::default();
		claims
			.name("Ada")
			.preferred_username("ada")
			.picture("https://img.example/ada")
			.insert("sub", "forged")
			.insert("email", "forged@example.com")
			.insert("locale", "en");

		assert_eq!(claims.get("name"), Some(&Value::from("Ada")));
		assert_eq!(claims.get("sub"), None);
		assert_eq!(claims.get("email"), None);
		assert_eq!(claims.into_inner().len(), 4);

		let scopes = GrantedScopes::new(vec!["openid".into(), "profile".into()]);
		assert!(scopes.has("profile"));
		assert!(!scopes.has("email"));
		assert_eq!(scopes.iter().count(), 2);
	}
}
