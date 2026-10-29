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
	/// [`AuthError::Config`] without a mailer.
	pub fn build(self) -> Result<Auth> {
		let mailer = self.mailer.ok_or_else(|| {
			AuthError::Config("set a mailer, such as `Mailbox::new()` in development".into())
		})?;
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
			}),
		})
	}
}

/// How long sessions last. The defaults follow NIST SP 800-63B for a
/// single factor: 14 days unused, and 90 days at most.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionLifetimes {
	/// How long a session lasts without being used.
	pub idle: Duration,
	/// How long a session lasts however often it's used.
	pub absolute: Duration,
}

impl Default for SessionLifetimes {
	fn default() -> Self {
		Self {
			idle: Duration::from_secs(14 * 24 * 60 * 60),
			absolute: Duration::from_secs(90 * 24 * 60 * 60),
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
			lifetime: Duration::from_secs(10 * 60),
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
}

impl RequestInfo {
	/// A short label for the device, such as "Firefox on macOS".
	pub fn device(&self) -> String {
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

pub(crate) type SignUpHook = for<'a> fn(
	&'a Tx,
	Id<Account>,
	&'a Profile,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// The context a request's policies read: [`SignedIn`] when it has a live
/// session, and nothing when it doesn't.
pub fn context(signed_in: Option<SignedIn>) -> Context {
	match signed_in {
		Some(signed_in) => Context::new().with(signed_in),
		None => Context::new(),
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
		};

		assert_eq!(request.ip_text().as_deref(), Some("203.0.113.9"));
		assert_eq!(request.user_agent_text().unwrap().len(), 512);
	}
}
