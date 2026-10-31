//! [`Auth`]: the server's sign-in service, and how an app configures it.

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Email;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use leptoskit_db::Url;

use crate::Account;
use crate::AuthError;
use crate::DbRateLimiter;
use crate::Keys;
use crate::Limits;
use crate::Mailer;
use crate::RateLimiter;
use crate::Result;
use crate::SignedIn;
use crate::Transport;

/// Sign-in for one app: sessions, email codes, rate limits, and the audit
/// trail, over the `auth` tables in the app's database.
///
/// Build one at startup and share it; clones are cheap.
///
/// ```ignore
/// let auth = Auth::builder(&db, Keys::from_env("AUTH_ENCRYPTION_KEYS")?)
/// 	.app_name("My App")
/// 	.mailer(my_mailer)
/// 	.on_sign_up::<User>()
/// 	.build()?;
/// ```
#[derive(Clone)]
pub struct Auth {
	pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
	/// The app's database, bypassing policies: only this crate's own
	/// queries run on it.
	pub db: Db,
	pub keys: Keys,
	pub mailer: Arc<dyn Mailer>,
	pub limiter: Arc<dyn RateLimiter>,
	pub sign_up: SignUpHook,
	pub app_name: String,
	pub sessions: SessionLifetimes,
	pub codes: CodeRules,
	pub limits: Limits,
	// Only the HTTP layer reads these.
	/// The site's own origin, which state-changing requests must come from.
	#[cfg_attr(not(feature = "axum"), allow(dead_code))]
	pub origin: Option<String>,
	/// The header a trusted proxy puts the client's IP address in.
	#[cfg_attr(not(feature = "axum"), allow(dead_code))]
	pub client_ip_header: Option<String>,
	/// The mailbox `/_leptoskit/mail` shows, in development.
	#[cfg_attr(not(feature = "axum"), allow(dead_code))]
	pub dev_mailbox: Option<crate::Mailbox>,
	/// Whether `/auth/dev/sign-in` signs anyone in without a code.
	pub dev_sign_in: bool,
	/// Who must have a second factor.
	pub mfa: MfaPolicy,
	/// The app's roles for an account, for [`MfaPolicy::required_for`].
	pub roles: Option<RolesHook>,
	/// The WebAuthn relying party: the origin's host. `None` without an
	/// origin, and then passkeys answer a configuration error.
	#[cfg(feature = "passkey")]
	pub webauthn: Option<webauthn_rs::Webauthn>,
	/// The third-party sign-in providers.
	#[cfg(feature = "oauth")]
	pub providers: crate::oauth::Providers,
	/// How wallets work here.
	pub solana: crate::SolanaOptions,
	/// How accounts' data moves when they merge, when the app moves some
	/// of it itself.
	pub on_merge: Option<crate::merge::MergeHooks>,
}

impl Auth {
	/// Starts configuring sign-in over `db`, with the server's `keys`.
	pub fn builder(db: &Db, keys: Keys) -> AuthBuilder {
		AuthBuilder {
			db: db.system("auth: sign-in"),
			keys,
			mailer: None,
			limiter: None,
			sign_up: |_, _, _| Box::pin(async { Ok(()) }),
			app_name: "this app".into(),
			sessions: SessionLifetimes::default(),
			codes: CodeRules::default(),
			limits: Limits::default(),
			origin: None,
			client_ip_header: None,
			dev_mailbox: None,
			dev_sign_in: false,
			mfa: MfaPolicy::default(),
			roles: None,
			solana: crate::SolanaOptions::default(),
			on_merge: None,
			#[cfg(feature = "oauth")]
			providers: Vec::new(),
			#[cfg(feature = "oauth")]
			provider_http: None,
		}
	}

	/// The app's name, as emails say it.
	pub fn app_name(&self) -> &str {
		&self.inner.app_name
	}
}

impl std::fmt::Debug for Auth {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Auth")
			.field("app_name", &self.inner.app_name)
			.field("keys", &self.inner.keys)
			.finish_non_exhaustive()
	}
}

/// Configures an [`Auth`].
#[must_use]
pub struct AuthBuilder {
	db: Db,
	keys: Keys,
	mailer: Option<Arc<dyn Mailer>>,
	limiter: Option<Arc<dyn RateLimiter>>,
	sign_up: SignUpHook,
	app_name: String,
	sessions: SessionLifetimes,
	codes: CodeRules,
	limits: Limits,
	origin: Option<String>,
	client_ip_header: Option<String>,
	dev_mailbox: Option<crate::Mailbox>,
	dev_sign_in: bool,
	mfa: MfaPolicy,
	roles: Option<RolesHook>,
	solana: crate::SolanaOptions,
	on_merge: Option<crate::merge::MergeHooks>,
	#[cfg(feature = "oauth")]
	providers: Vec<crate::oauth::Provider>,
	#[cfg(feature = "oauth")]
	provider_http: Option<Arc<dyn crate::oauth::ProviderHttp>>,
}

impl std::fmt::Debug for AuthBuilder {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AuthBuilder")
			.field("app_name", &self.app_name)
			.field("keys", &self.keys)
			.finish_non_exhaustive()
	}
}

impl AuthBuilder {
	/// The app's name, as emails say it.
	pub fn app_name(mut self, name: impl Into<String>) -> Self {
		self.app_name = name.into();
		self
	}

	/// How email reaches people. [`Mailbox`](crate::Mailbox) keeps it, for
	/// development and tests.
	pub fn mailer(mut self, mailer: impl Mailer) -> Self {
		self.mailer = Some(Arc::new(mailer));
		self
	}

	/// Where attempts are counted, instead of `auth.rate_limits`.
	pub fn rate_limiter(mut self, limiter: impl RateLimiter) -> Self {
		self.limiter = Some(Arc::new(limiter));
		self
	}

	/// How often each step may be tried.
	pub fn limits(mut self, limits: Limits) -> Self {
		self.limits = limits;
		self
	}

	/// How long sessions last.
	pub fn sessions(mut self, lifetimes: SessionLifetimes) -> Self {
		self.sessions = lifetimes;
		self
	}

	/// How long emailed codes last, and how often each may be tried.
	pub fn codes(mut self, rules: CodeRules) -> Self {
		self.codes = rules;
		self
	}

	/// The site's origin, such as `https://myapp.com`: state-changing
	/// requests from anywhere else are refused, and an `https` origin keeps
	/// sessions in a `__Host-` cookie that only this origin can set. Without
	/// one, requests must come from the host they're sent to.
	pub fn origin(mut self, origin: impl Into<String>) -> Self {
		self.origin = Some(origin.into().trim_end_matches('/').to_owned());
		self
	}

	/// Reads the client's IP address from `header`, such as
	/// `fly-client-ip`, which a proxy in front of the server sets. Only name a
	/// header your proxy always overwrites: anyone can send one.
	pub fn client_ip_header(mut self, header: impl Into<String>) -> Self {
		self.client_ip_header = Some(header.into().to_ascii_lowercase());
		self
	}

	/// Development only: keeps every email in a mailbox instead of sending
	/// it, and shows them at `/_leptoskit/mail`. Release builds don't have
	/// this method.
	#[cfg(debug_assertions)]
	pub fn dev_mailbox(mut self) -> Self {
		let mailbox = crate::Mailbox::new();
		self.mailer = Some(Arc::new(mailbox.clone()));
		self.dev_mailbox = Some(mailbox);
		self
	}

	/// Development only: `POST /auth/dev/sign-in` signs in to any address
	/// without a code, creating its account if needed, so you can try the
	/// app as several people. Release builds don't have this method.
	#[cfg(debug_assertions)]
	pub fn dev_sign_in(mut self) -> Self {
		self.dev_sign_in = true;
		self
	}

	/// Who must have a second factor: everyone, or people with certain
	/// roles. Until they have one, their sessions may only set one up.
	pub fn mfa(mut self, policy: MfaPolicy) -> Self {
		self.mfa = policy;
		self
	}

	/// Reads an account's roles with `R::roles`, for
	/// [`MfaPolicy::required_for`].
	pub fn roles<R: Roles + 'static>(mut self) -> Self {
		self.roles = Some(|db, account| Box::pin(R::roles(db, account)));
		self
	}

	/// How Solana wallets work: whether one may sign in once its owner
	/// turns that on, and the cluster messages name. Linking needs the
	/// `solana` feature and an [`origin`](Self::origin).
	pub fn solana(mut self, options: crate::SolanaOptions) -> Self {
		self.solana = options;
		self
	}

	/// Lets a merge bring the other account's data along with `U`'s
	/// [`OnMerge`](crate::OnMerge): its summary for the preview, and the
	/// tables marked `on_account_merge = custom`, which it moves itself.
	pub fn on_merge<U: crate::OnMerge + 'static>(mut self) -> Self {
		self.on_merge = Some(crate::merge::MergeHooks::of::<U>());
		self
	}

	/// Adds a third-party sign-in provider, such as
	/// [`Provider::google`](crate::Provider::google).
	#[cfg(feature = "oauth")]
	pub fn provider(mut self, provider: crate::oauth::Provider) -> Self {
		self.providers.retain(|existing| existing.id != provider.id);
		self.providers.push(provider);
		self
	}

	/// Sends the providers' requests through `http` instead of the network,
	/// for tests.
	#[cfg(feature = "oauth")]
	pub fn provider_http(mut self, http: impl crate::oauth::ProviderHttp) -> Self {
		self.provider_http = Some(Arc::new(http));
		self
	}

	/// Runs `U::on_sign_up` whenever an account is created, so the app's
	/// user row exists from the first request.
	pub fn on_sign_up<U: OnSignUp + 'static>(mut self) -> Self {
		self.sign_up = |tx, account, profile| Box::pin(U::on_sign_up(tx, account, profile));
		self
	}

	/// The configured service.
	///
	/// # Errors
	///
	/// [`AuthError::Config`] without a mailer, when roles require a second
	/// factor but nothing reads them, or when the origin can't be a passkey
	/// relying party.
	pub fn build(self) -> Result<Auth> {
		let mailer = self.mailer.ok_or_else(|| {
			AuthError::Config("set a mailer, such as `Mailbox::new()` in development".into())
		})?;

		if !self.mfa.required_for.is_empty() && self.roles.is_none() {
			return Err(AuthError::Config(
				"`required_for` names roles; tell sign-in how to read them with \
				 `AuthBuilder::roles`"
					.into(),
			));
		}

		#[cfg(feature = "passkey")]
		let webauthn = self
			.origin
			.as_deref()
			.map(|origin| crate::passkeys::relying_party(origin, &self.app_name))
			.transpose()?;
		#[cfg(feature = "oauth")]
		let providers = crate::oauth::Providers::new(self.providers, self.provider_http);
		let limiter = self
			.limiter
			.unwrap_or_else(|| Arc::new(DbRateLimiter::new(&self.db)));

		Ok(Auth {
			inner: Arc::new(Inner {
				db: self.db,
				keys: self.keys,
				mailer,
				limiter,
				sign_up: self.sign_up,
				app_name: self.app_name,
				sessions: self.sessions,
				codes: self.codes,
				limits: self.limits,
				origin: self.origin,
				client_ip_header: self.client_ip_header,
				dev_mailbox: self.dev_mailbox,
				dev_sign_in: self.dev_sign_in,
				mfa: self.mfa,
				roles: self.roles,
				solana: self.solana,
				on_merge: self.on_merge,
				#[cfg(feature = "passkey")]
				webauthn,
				#[cfg(feature = "oauth")]
				providers,
			}),
		})
	}
}

/// How long sessions last. The defaults follow NIST SP 800-63B for a
/// single factor: 14 days unused, and 90 days at most. A native app's
/// access token lasts 15 minutes before it refreshes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionLifetimes {
	/// How long a session lasts without being used.
	pub idle: Duration,
	/// How long a session lasts however often it's used.
	pub absolute: Duration,
	/// How long a native app's access token works before the app trades its
	/// refresh token for a new one.
	pub access: Duration,
}

impl Default for SessionLifetimes {
	fn default() -> Self {
		Self {
			idle: Duration::from_hours(14 * 24),
			absolute: Duration::from_hours(90 * 24),
			access: Duration::from_mins(15),
		}
	}
}

/// How long an emailed code lasts, and how often it may be tried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeRules {
	/// How long a code works.
	pub lifetime: Duration,
	/// Wrong guesses before the code stops working.
	pub attempts: u32,
}

impl Default for CodeRules {
	fn default() -> Self {
		Self {
			lifetime: Duration::from_mins(10),
			attempts: 5,
		}
	}
}

/// What the server knows about where a request came from, for rate
/// limits, the audit trail, and the list of signed-in devices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestInfo {
	/// The client's IP address.
	pub ip: Option<IpAddr>,
	/// The `User-Agent` header.
	pub user_agent: Option<String>,
	/// How the device keeps a session: a cookie, or access and refresh
	/// tokens.
	pub transport: Transport,
	/// What a native app calls its device, such as "Ada's iPhone".
	pub device_name: Option<String>,
}

/// The longest device name kept, in characters.
const DEVICE_NAME_LENGTH: usize = 100;

impl RequestInfo {
	/// A short label for the device: the name the app gave it, or one read
	/// from the user agent, such as "Firefox on macOS".
	pub fn device(&self) -> String {
		let name = self
			.device_name
			.as_deref()
			.map(str::trim)
			.filter(|name| !name.is_empty());

		if let Some(name) = name {
			return name.chars().take(DEVICE_NAME_LENGTH).collect();
		}

		let agent = self.user_agent.as_deref().unwrap_or_default();
		let browser = [
			("Edg/", "Edge"),
			("Firefox/", "Firefox"),
			("Chrome/", "Chrome"),
			("Safari/", "Safari"),
		]
		.into_iter()
		.find(|(marker, _)| agent.contains(marker))
		.map(|(_, name)| name);
		let system = [
			("iPhone", "iOS"),
			("iPad", "iPadOS"),
			("Android", "Android"),
			("Mac OS X", "macOS"),
			("Windows", "Windows"),
			("Linux", "Linux"),
		]
		.into_iter()
		.find(|(marker, _)| agent.contains(marker))
		.map(|(_, name)| name);

		match (browser, system) {
			(Some(browser), Some(system)) => format!("{browser} on {system}"),
			(Some(name), None) | (None, Some(name)) => name.to_owned(),
			(None, None) => "Unknown device".to_owned(),
		}
	}

	pub(crate) fn ip_text(&self) -> Option<String> {
		self.ip.map(|ip| ip.to_string())
	}

	pub(crate) fn user_agent_text(&self) -> Option<String> {
		self.user_agent
			.as_deref()
			.map(|agent| agent.chars().take(512).collect())
	}
}

/// What a sign-in method learned about a new person, for
/// [`OnSignUp::on_sign_up`] to fill in their profile from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Profile {
	/// Their verified email address.
	pub email: Option<Email>,
	/// Their name, as the provider has it.
	pub name: Option<String>,
	/// The username they'd like.
	pub preferred_username: Option<String>,
	/// A picture of them.
	pub picture: Option<Url>,
}

impl Profile {
	/// A profile that only knows an email address.
	pub fn from_email(email: Email) -> Self {
		Self {
			email: Some(email),
			..Self::default()
		}
	}
}

/// Creates the app's own row for a new account, such as its `User`. Runs
/// once per account, in the transaction that creates it: if it fails,
/// nothing is created.
///
/// ```ignore
/// impl auth::OnSignUp for User {
/// 	async fn on_sign_up(tx: &Tx, account: Id<auth::Account>, profile: &auth::Profile) -> auth::Result<()> {
/// 		NewUser { id: account.into(), name: profile.name.clone().unwrap_or_default() }
/// 			.insert(tx)
/// 			.await?;
/// 		Ok(())
/// 	}
/// }
/// ```
///
/// The transaction bypasses policies: nobody is signed in yet.
pub trait OnSignUp {
	/// Creates the app's rows for `account`.
	fn on_sign_up(
		tx: &Tx,
		account: Id<Account>,
		profile: &Profile,
	) -> impl Future<Output = Result<()>> + Send;
}

/// Who must have a second factor. The default asks no one: people turn
/// it on themselves, and sensitive actions ask for step-up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MfaPolicy {
	/// Everyone must.
	pub required: bool,
	/// People with any of these roles must, as [`Roles`] reads them.
	pub required_for: Vec<String>,
}

impl MfaPolicy {
	/// Everyone must have a second factor.
	pub fn required() -> Self {
		Self {
			required: true,
			required_for: Vec::new(),
		}
	}

	/// People with any of `roles` must have a second factor.
	pub fn required_for<R: Into<String>>(roles: impl IntoIterator<Item = R>) -> Self {
		Self {
			required: false,
			required_for: roles.into_iter().map(Into::into).collect(),
		}
	}

	/// Whether nobody has to.
	pub(crate) fn optional(&self) -> bool {
		!self.required && self.required_for.is_empty()
	}
}

/// Reads an account's roles, such as `"Admin"`, for
/// [`MfaPolicy::required_for`]. Sign-in calls it for sessions that haven't
/// used a second factor, so it should be one indexed query.
///
/// ```ignore
/// impl auth::Roles for User {
/// 	async fn roles(db: &Db, account: Id<auth::Account>) -> auth::Result<Vec<String>> {
/// 		let user = User::get(db, account.into()).await?;
/// 		Ok(vec![user.role.to_string()])
/// 	}
/// }
/// ```
///
/// The `Db` bypasses policies.
pub trait Roles {
	/// The roles `account` has.
	fn roles(db: &Db, account: Id<Account>) -> impl Future<Output = Result<Vec<String>>> + Send;
}

pub(crate) type RolesHook =
	for<'a> fn(
		&'a Db,
		Id<Account>,
	) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>>;

pub(crate) type SignUpHook = for<'a> fn(
	&'a Tx,
	Id<Account>,
	&'a Profile,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// The context a request's policies read: [`SignedIn`] when it has a live
/// session, and nothing when it doesn't. [`Auth::request_context`] adds the
/// person's wallets too.
pub fn context(signed_in: Option<SignedIn>) -> Context {
	match signed_in {
		Some(signed_in) => Context::new().with(signed_in),
		None => Context::new(),
	}
}

impl Auth {
	/// The context a request's policies read: [`SignedIn`] and the person's
	/// linked [`Wallets`](crate::Wallets) when it has a live session, and
	/// nothing when it doesn't. The layer builds one per request. Reading
	/// the wallets is one indexed query, made only with the `solana` feature.
	///
	/// # Errors
	///
	/// When the database fails.
	#[cfg_attr(
		not(feature = "solana"),
		allow(
			clippy::unused_async,
			reason = "the `solana` feature reads the wallets"
		)
	)]
	pub async fn request_context(&self, signed_in: Option<SignedIn>) -> Result<Context> {
		let Some(signed_in) = signed_in else {
			return Ok(Context::new());
		};
		#[cfg(feature = "solana")]
		let wallets = self.wallet_addresses(signed_in.account).await?;
		#[cfg(not(feature = "solana"))]
		let wallets: Vec<leptoskit_db::Pubkey> = Vec::new();
		let context = Context::new().with(signed_in);

		Ok(if wallets.is_empty() {
			context
		} else {
			context.with(crate::Wallets(wallets))
		})
	}
}

/// Ends a transaction: commits it when `result` is `Ok`, and rolls it back
/// when it isn't.
pub(crate) async fn finish<T>(tx: Tx, result: Result<T>) -> Result<T> {
	match result {
		Ok(value) => {
			tx.commit().await?;
			Ok(value)
		}
		Err(error) => {
			tx.rollback().await?;
			Err(error)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn devices_are_named_from_the_user_agent() {
		let device = |agent: Option<&str>| {
			RequestInfo {
				ip: None,
				user_agent: agent.map(Into::into),
				..RequestInfo::default()
			}
			.device()
		};

		assert_eq!(
			device(Some(
				"Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5; rv:131.0) Gecko/20100101 \
				 Firefox/131.0"
			)),
			"Firefox on macOS"
		);
		assert_eq!(
			device(Some(
				"Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 \
				 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"
			)),
			"Safari on iOS"
		);
		assert_eq!(device(Some("curl/8.0")), "Unknown device");
		assert_eq!(device(Some("Windows")), "Windows");
		assert_eq!(device(None), "Unknown device");
	}

	#[test]
	fn request_details_are_bounded() {
		let request = RequestInfo {
			ip: Some("203.0.113.9".parse().unwrap()),
			user_agent: Some("a".repeat(600)),
			..RequestInfo::default()
		};

		assert_eq!(request.ip_text().as_deref(), Some("203.0.113.9"));
		assert_eq!(request.user_agent_text().unwrap().len(), 512);
	}

	#[test]
	fn an_apps_device_name_wins_and_is_bounded() {
		let named = |name: &str| {
			RequestInfo {
				user_agent: Some("Firefox/131.0".into()),
				device_name: Some(name.into()),
				..RequestInfo::default()
			}
			.device()
		};

		assert_eq!(named("  Ada's iPhone "), "Ada's iPhone");
		assert_eq!(named("   "), "Firefox");
		assert_eq!(named(&"é".repeat(300)).chars().count(), 100);
	}
}
