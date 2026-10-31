//! Passwords, for apps that turn them on: off by default, since passkeys
//! and emailed codes do the same with nothing to steal.
//!
//! - **Hashing:** Argon2id with OWASP's parameters (19 MiB, two passes, one
//!   lane), in the PHC string format, which records them; a hash made with
//!   others is replaced at the next sign-in. Hashing runs off the async
//!   threads.
//! - **Rules, after NIST SP 800-63B:** a minimum length (15 by default, the
//!   floor for a password that's the only factor), at most 256 characters, no
//!   rules about kinds of characters, and no common passwords, ones close to
//!   the email address or the app's name, or, if the app asks, ones Have I Been
//!   Pwned has seen in a breach. Passwords are compared as NFKC.
//! - **Signing in** is by email address. A wrong address and a wrong password
//!   answer the same, in the same time. Each address gets a few tries every
//!   quarter hour, and after too many wrong ones a day, password sign-in for it
//!   pauses for a while; the account's other ways in keep working, so nobody
//!   can lock someone out by guessing.
//! - **A reset** is an emailed code from the sign-in flow plus a new password.
//!   It ends every session. An account with a second factor needs it too,
//!   before anything changes.

use leptoskit_db::Email;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use serde::Deserialize;
use serde::Serialize;

use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::Challenge;
use crate::EmailAddress;
use crate::EventKind;
use crate::NewChallenge;
use crate::Password;
use crate::PasswordProblem;
use crate::PasswordSettings;
use crate::Purpose;
use crate::RequestInfo;
use crate::Result;
use crate::SignInStep;
use crate::SignedIn;
use crate::auth::finish;
use crate::email::refuse_unusable;
use crate::events::record;

/// The most characters a password may have. Argon2's cost doesn't grow
/// with the length, so this only bounds the request.
pub(crate) const MAX_LENGTH: u32 = 256;

/// The most common passwords long enough that a rule could allow them.
const COMMON: &str = include_str!("passwords/common.txt");

/// How passwords work here, when they're on.
#[derive(Clone)]
pub struct PasswordOptions {
	/// The fewest characters a new password may have: at least 8. NIST SP
	/// 800-63B asks for 15 when a password is the only factor.
	pub min_length: u32,
	/// Checks new passwords against Have I Been Pwned's breached
	/// passwords, sending only the first 5 characters of their SHA-1 hash.
	pub breach_check: bool,
}

impl Default for PasswordOptions {
	fn default() -> Self {
		Self {
			min_length: 15,
			breach_check: false,
		}
	}
}

impl std::fmt::Debug for PasswordOptions {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PasswordOptions")
			.field("min_length", &self.min_length)
			.field("breach_check", &self.breach_check)
			.finish()
	}
}

/// What to do once a second step succeeds, besides signing in: sealed in
/// that step's challenge.
#[derive(Serialize, Deserialize)]
pub(crate) enum Then {
	/// Finish a password reset: replace the hash, and end every session.
	ResetPassword { hash: String },
}

impl Then {
	pub(crate) fn seal(&self) -> Result<Vec<u8>> {
		serde_json::to_vec(self).map_err(|_| AuthError::Config("a reset couldn't be stored".into()))
	}

	pub(crate) fn open(bytes: &[u8]) -> Result<Self> {
		serde_json::from_slice(bytes).map_err(|_| AuthError::SignInExpired)
	}
}

/// The password as compared: NFKC, so the same password typed on two
/// keyboards matches.
#[cfg(feature = "password")]
fn normalize(password: &str) -> String {
	use unicode_normalization::UnicodeNormalization;

	password.nfkc().collect()
}

/// Letters and digits only, lowercased, without trailing digits: what a
/// password is "close to".
fn stem(text: &str) -> String {
	let letters: String = text
		.chars()
		.filter(|character| character.is_alphanumeric())
		.flat_map(char::to_lowercase)
		.collect();
	letters
		.trim_end_matches(|character: char| character.is_ascii_digit())
		.to_owned()
}

/// Why `password` (normalized) can't be a new password, without asking
/// any other service.
fn problem(
	password: &str,
	min_length: u32,
	address: Option<&Email>,
	app_name: &str,
) -> Option<PasswordProblem> {
	let length = u32::try_from(password.chars().count()).unwrap_or(u32::MAX);

	if length < min_length {
		return Some(PasswordProblem::TooShort { min: min_length });
	}

	if length > MAX_LENGTH {
		return Some(PasswordProblem::TooLong { max: MAX_LENGTH });
	}

	let lowered = password.to_lowercase();

	if COMMON
		.lines()
		.filter(|line| !line.starts_with('#'))
		.any(|common| common == lowered)
	{
		return Some(PasswordProblem::Common);
	}

	let stemmed = stem(password);
	let personal = address
		.into_iter()
		.flat_map(|address| {
			let address = address.as_str();
			[address, address.split('@').next().unwrap_or_default()]
		})
		.chain([app_name])
		.map(stem)
		.any(|context| context.chars().count() >= 4 && stemmed == context);

	personal.then_some(PasswordProblem::Personal)
}

impl Auth {
	/// Whether people may use passwords here, and the rules for new ones.
	pub fn password_settings(&self) -> PasswordSettings {
		match self.password_options() {
			Ok(options) => {
				PasswordSettings {
					enabled: true,
					min_length: options.min_length,
					max_length: MAX_LENGTH,
				}
			}
			Err(_) => PasswordSettings::default(),
		}
	}

	fn password_options(&self) -> Result<&PasswordOptions> {
		self.inner.passwords.as_ref().ok_or(AuthError::Unsupported(
			"passwords are off; turn them on with `[auth.passwords] enabled` and the `password` \
			 feature",
		))
	}

	/// Checks a new password against the rules, and returns it as it's
	/// compared.
	///
	/// # Errors
	///
	/// [`AuthError::WeakPassword`] with what's wrong.
	async fn check_new_password(&self, password: &str, address: Option<&Email>) -> Result<String> {
		let options = self.password_options()?;
		let password = normalize(password);

		if let Some(problem) = problem(&password, options.min_length, address, &self.inner.app_name)
		{
			return Err(AuthError::WeakPassword(problem));
		}

		if options.breach_check && self.breached(&password).await {
			return Err(AuthError::WeakPassword(PasswordProblem::Breached));
		}

		Ok(password)
	}

	/// Signs in with an account's email address and its password. When the
	/// account has a second factor, it starts the second step instead, as
	/// an emailed code does.
	///
	/// # Errors
	///
	/// [`AuthError::InvalidPassword`] for an address without an account or
	/// password, or a wrong password, without saying which;
	/// [`AuthError::RateLimited`] after too many tries for the address or
	/// from the IP, or while password sign-in for the address is paused;
	/// [`AuthError::Disabled`] or [`AuthError::Locked`]; or
	/// [`AuthError::Unsupported`] when passwords are off.
	pub async fn sign_in_with_password(
		&self,
		address: &str,
		password: &str,
		request: &RequestInfo,
	) -> Result<SignInStep> {
		self.password_options()?;
		let address = Email::parse(address).map_err(|_| AuthError::InvalidEmail)?;
		let limits = self.inner.limits;

		if let Some(ip) = request.ip_text() {
			self.limit(&[b"password-ip", ip.as_bytes()], limits.verify_per_ip)
				.await?;
		}

		self.limit(
			&[b"password-address", address.as_bytes()],
			limits.password_per_address,
		)
		.await?;
		self.refuse_paused(&address).await?;

		let db = &self.inner.db;
		let account = EmailAddress::query()
			.filter(|email| email.address.eq(&address) & email.verified_at.is_not_null())
			.first(db)
			.await?
			.map(|email| email.account_id);
		let stored = match account {
			Some(account) => {
				Password::query()
					.filter(|password| password.account_id.eq(account))
					.first(db)
					.await?
			}
			None => None,
		};
		let password = normalize(password);
		// A missing password is checked against a stand-in hash, so it takes
		// as long to refuse as a wrong one.
		let hash = stored.as_ref().map(|stored| stored.hash.clone());
		let verified = hashing::verify(hash, &password).await? && stored.is_some();

		let (Some(account), Some(stored), true) = (account, stored, verified) else {
			self.password_failed(&address, account, request).await?;
			return Err(AuthError::InvalidPassword);
		};

		let tx = db.begin().await?;
		let result = async {
			refuse_unusable(&Account::get(&tx, account).await?)?;
			self.after_first_factor(&tx, account, false, true, request)
				.await
		}
		.await;
		let step = finish(tx, result).await?;

		if hashing::outdated(&stored.hash) {
			let rehashed = hashing::hash(&password).await?;
			Password::update()
				.filter(|row| row.id.eq(stored.id) & row.hash.eq(stored.hash.clone()))
				.set(|row| row.hash, rehashed)
				.run(db)
				.await?;
		}

		Ok(step)
	}

	/// Sets the signed-in person's password, or changes it, after a fresh
	/// sign-in or step-up. Changing it ends their other sessions. They sign
	/// in with it at their verified email address.
	///
	/// # Errors
	///
	/// [`AuthError::WeakPassword`]; [`AuthError::Reauthenticate`] or
	/// [`AuthError::StepUpRequired`] when the session isn't fresh;
	/// [`AuthError::Unsupported`] when passwords are off, or the account has
	/// no verified address to sign in with.
	pub async fn set_password(
		&self,
		me: &SignedIn,
		password: &str,
		request: &RequestInfo,
	) -> Result<()> {
		self.password_options()?;
		let db = &self.inner.db;
		self.require_fresh(db, me).await?;
		let address = crate::mfa::primary_email(db, me.account)
			.await?
			.and_then(|address| Email::parse(&address).ok())
			.ok_or(AuthError::Unsupported(
				"a password signs in with the account's email address, and this account has none",
			))?;
		let password = self.check_new_password(password, Some(&address)).await?;
		let hash = hashing::hash(&password).await?;
		let tx = db.begin().await?;
		let result = async {
			let changed = self.store_password(&tx, me.account, hash).await?;

			if changed {
				self.end_others(&tx, me.account, me.session, request)
					.await?;
			}

			let kind = if changed {
				EventKind::PasswordChanged
			} else {
				EventKind::PasswordSet
			};
			record(&tx, Some(me.account), kind, request).await?;
			Ok(changed)
		}
		.await;
		let changed = finish(tx, result).await?;
		let text = if changed {
			"Your password was changed, and every other device was signed out. If you didn't do \
			 this, reset your password and sign out everywhere."
		} else {
			"You can now sign in with a password. If you didn't set one, reset it and sign out \
			 everywhere."
		};
		self.notify(me.account, "Your password was set", text).await;
		Ok(())
	}

	/// Removes the signed-in person's password, after a fresh sign-in or
	/// step-up. They keep signing in with emailed codes, passkeys, and
	/// anything else they have.
	///
	/// # Errors
	///
	/// [`AuthError::NotFound`] when they have none, or as
	/// [`Auth::set_password`] when the session isn't fresh.
	pub async fn remove_password(&self, me: &SignedIn, request: &RequestInfo) -> Result<()> {
		let db = &self.inner.db;
		self.require_fresh(db, me).await?;
		let tx = db.begin().await?;
		let result = async {
			let removed = Password::query()
				.filter(|password| password.account_id.eq(me.account))
				.delete(&tx)
				.await?;

			if removed == 0 {
				return Err(AuthError::NotFound);
			}

			record(&tx, Some(me.account), EventKind::PasswordRemoved, request).await
		}
		.await;
		finish(tx, result).await?;
		self.notify(
			me.account,
			"Your password was removed",
			"Signing in with a password is off for your account. If you didn't do this, sign in \
			 and sign out everywhere.",
		)
		.await;
		Ok(())
	}

	/// Resets the password of `address`'s account with a code from
	/// [`Auth::send_email_code`]: the code proves the inbox, as signing in
	/// with it would. Every session ends, and a new one starts. An account
	/// with a second factor gets the second step first, and nothing changes
	/// until it succeeds. An address with no account gets one, as with an
	/// emailed code.
	///
	/// The new password is checked before the code is used, so a refused
	/// password leaves the code working.
	///
	/// # Errors
	///
	/// [`AuthError::WeakPassword`], or as [`Auth::verify_email_code`].
	pub async fn reset_password(
		&self,
		address: &str,
		code: &str,
		password: &str,
		request: &RequestInfo,
	) -> Result<SignInStep> {
		self.password_options()?;
		let address = Email::parse(address).map_err(|_| AuthError::InvalidEmail)?;
		let password = self.check_new_password(password, Some(&address)).await?;
		let hash = hashing::hash(&password).await?;
		let challenge = self.check_email_code(&address, code, request).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			self.claim_code(&tx, &challenge).await?;
			let (account, signed_up) = self.proven_account(&tx, &address, request).await?;
			let factors = self.factors(&tx, account).await?;

			if !signed_up && !factors.is_empty() {
				let then = Then::ResetPassword { hash };
				let challenge = self
					.second_factor_challenge(&tx, account, factors, Some(&then))
					.await?;
				return Ok((SignInStep::SecondFactor(challenge), account, false));
			}

			self.then(&tx, account, Then::ResetPassword { hash }, request)
				.await?;
			let step = self
				.after_first_factor(&tx, account, signed_up, false, request)
				.await?;
			Ok((step, account, true))
		}
		.await;
		let (step, account, reset) = finish(tx, result).await?;

		if reset {
			self.notify_password_reset(account).await;
		}

		Ok(step)
	}

	/// Does what a second step waited for, in its transaction.
	pub(crate) async fn then(
		&self,
		tx: &Tx,
		account: Id<Account>,
		then: Then,
		request: &RequestInfo,
	) -> Result<()> {
		match then {
			Then::ResetPassword { hash } => {
				self.store_password(tx, account, hash).await?;
				crate::sessions::end_all(tx, account, request).await?;
				record(tx, Some(account), EventKind::PasswordReset, request).await
			}
		}
	}

	pub(crate) async fn notify_password_reset(&self, account: Id<Account>) {
		self.notify(
			account,
			"Your password was reset",
			"Your password was reset with a code sent to this address, and every device was \
			 signed out. If you didn't do this, reset it again and secure your email.",
		)
		.await;
	}

	/// Keeps `hash` as `account`'s password: whether it replaced one.
	async fn store_password(&self, tx: &Tx, account: Id<Account>, hash: String) -> Result<bool> {
		let now = leptoskit_db::now();
		let changed = Password::update()
			.filter(|row| row.account_id.eq(account))
			.set(|row| row.hash, hash.clone())
			.set(|row| row.changed_at, now)
			.run(tx)
			.await?;

		if changed == 0 {
			crate::NewPassword {
				account_id: account,
				hash,
				changed_at: now,
			}
			.insert(tx)
			.await?;
		}

		Ok(changed > 0)
	}

	/// Refuses password sign-in for an address whose sign-in is paused.
	async fn refuse_paused(&self, address: &Email) -> Result<()> {
		let now = leptoskit_db::now();
		let pause = Challenge::query()
			.filter(|row| {
				row.purpose.eq(Purpose::PasswordPause)
					& row.subject.eq(address.to_string())
					& row.expires_at.gt(now)
			})
			.order_by(|row| row.expires_at.desc())
			.first(&self.inner.db)
			.await?;

		match pause {
			Some(pause) => {
				Err(AuthError::RateLimited {
					retry_after: (pause.expires_at - now).to_std().unwrap_or_default(),
				})
			}
			None => Ok(()),
		}
	}

	/// After a wrong password for `address`: records it, and once the
	/// address has had too many today, pauses password sign-in for it and
	/// emails its account, if it has one. The pause is the same for an
	/// address without an account, so it says nothing about who has one.
	async fn password_failed(
		&self,
		address: &Email,
		account: Option<Id<Account>>,
		request: &RequestInfo,
	) -> Result<()> {
		let db = &self.inner.db;
		record(db, account, EventKind::SignInFailed, request).await?;
		let limits = self.inner.limits;
		let counted = self
			.limit(
				&[b"password-failures", address.as_bytes()],
				limits.failures_before_lock,
			)
			.await;

		match counted {
			Ok(()) => return Ok(()),
			Err(AuthError::RateLimited { .. }) => {}
			Err(error) => return Err(error),
		}

		let now = leptoskit_db::now();
		let until = now + crate::sessions::lifetime(limits.lock_for)?;
		let secret = crate::secrets::random::<32>()?;
		NewChallenge {
			purpose: Purpose::PasswordPause,
			subject: address.to_string(),
			secret_hash: crate::secrets::digest_plain(&secret),
			state: None,
			attempts: 0,
			expires_at: until,
			used_at: None,
		}
		.insert(db)
		.await?;

		if let Some(account) = account {
			record(db, Some(account), EventKind::PasswordPaused, request).await?;
			self.notify(
				account,
				"Signing in with a password is paused",
				&format!(
					"Someone entered too many wrong passwords for your account, so signing in \
					 with a password is paused until {} UTC. Emailed codes, passkeys, and your \
					 other ways to sign in still work. If it wasn't you, consider a longer \
					 password.",
					until.format("%Y-%m-%d %H:%M")
				),
			)
			.await;
		}

		Ok(())
	}

	/// Whether Have I Been Pwned has seen `password` in a breach. Only the
	/// first 5 characters of its SHA-1 hash leave the server. When the
	/// service can't be reached, the password passes: the other rules still
	/// applied.
	#[cfg(feature = "password")]
	async fn breached(&self, password: &str) -> bool {
		use sha1::Digest;

		let digest = sha1::Sha1::digest(password.as_bytes());
		let hex = digest.iter().fold(String::new(), |mut hex, byte| {
			use std::fmt::Write as _;
			let _ = write!(hex, "{byte:02X}");
			hex
		});
		let (prefix, suffix) = hex.split_at(5);
		let Some(breaches) = &self.inner.breaches else {
			return false;
		};
		let Ok(range) = breaches.range(prefix).await else {
			return false;
		};
		range.lines().any(|line| {
			line.split_once(':').is_some_and(|(candidate, count)| {
				candidate.trim().eq_ignore_ascii_case(suffix) && count.trim() != "0"
			})
		})
	}

	#[cfg(not(feature = "password"))]
	#[allow(
		clippy::unused_async,
		clippy::unused_self,
		reason = "without the feature, passwords are off before this is asked"
	)]
	async fn breached(&self, _password: &str) -> bool {
		false
	}
}

#[cfg(not(feature = "password"))]
fn normalize(password: &str) -> String {
	password.to_owned()
}

/// A future a [`BreachedPasswords`] returns.
#[cfg(feature = "password")]
pub type BreachFuture<'a> =
	std::pin::Pin<Box<dyn Future<Output = std::result::Result<String, String>> + Send + 'a>>;

/// Asks which breached passwords' SHA-1 hashes start with a prefix, as Have
/// I Been Pwned's range API answers: one `SUFFIX:COUNT` a line, where a
/// count of 0 is padding. The default asks `api.pwnedpasswords.com`; tests
/// answer from memory.
#[cfg(feature = "password")]
pub trait BreachedPasswords: Send + Sync + 'static {
	/// The suffixes for `prefix`, five uppercase hex digits. An `Err` is a
	/// failure, described for the server's logs.
	fn range(&self, prefix: &str) -> BreachFuture<'_>;
}

/// Have I Been Pwned's range API, with padding on, so the answer's size
/// says nothing about the prefix.
#[cfg(feature = "password")]
pub(crate) struct PwnedPasswords(reqwest::Client);

#[cfg(feature = "password")]
impl PwnedPasswords {
	pub(crate) fn new() -> Self {
		Self(
			reqwest::Client::builder()
				.timeout(std::time::Duration::from_secs(10))
				.redirect(reqwest::redirect::Policy::none())
				.user_agent(concat!("leptoskit-auth/", env!("CARGO_PKG_VERSION")))
				.build()
				.unwrap_or_default(),
		)
	}
}

#[cfg(feature = "password")]
impl BreachedPasswords for PwnedPasswords {
	fn range(&self, prefix: &str) -> BreachFuture<'_> {
		let url = format!("https://api.pwnedpasswords.com/range/{prefix}");
		Box::pin(async move {
			let response = self
				.0
				.get(url)
				.header("add-padding", "true")
				.send()
				.await
				.map_err(|error| error.to_string())?;

			if !response.status().is_success() {
				return Err(format!("answered {}", response.status()));
			}

			response.text().await.map_err(|error| error.to_string())
		})
	}
}

/// The breach check `options` asks for: `breaches` when given, and Have I
/// Been Pwned otherwise.
#[cfg(feature = "password")]
pub(crate) fn breaches(
	options: &PasswordOptions,
	breaches: Option<std::sync::Arc<dyn BreachedPasswords>>,
) -> Option<std::sync::Arc<dyn BreachedPasswords>> {
	options
		.breach_check
		.then(|| breaches.unwrap_or_else(|| std::sync::Arc::new(PwnedPasswords::new())))
}

/// Argon2id with OWASP's parameters, off the async threads.
#[cfg(feature = "password")]
mod hashing {
	use std::sync::OnceLock;

	use argon2::Argon2;
	use argon2::PasswordHash;
	use argon2::PasswordHasher;
	use argon2::PasswordVerifier;
	use argon2::password_hash::SaltString;
	use argon2::password_hash::rand_core::OsRng;

	use crate::AuthError;
	use crate::Result;

	/// `RustCrypto`'s defaults are OWASP's: Argon2id, version 19, 19 MiB,
	/// two passes, one lane.
	fn argon2() -> Argon2<'static> {
		Argon2::default()
	}

	async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
		tokio::task::spawn_blocking(work)
			.await
			.map_err(|_| AuthError::Config("hashing a password failed".into()))
	}

	fn hash_now(password: &str) -> Result<String> {
		let salt = SaltString::generate(&mut OsRng);
		argon2()
			.hash_password(password.as_bytes(), &salt)
			.map(|hash| hash.to_string())
			.map_err(|_| AuthError::Config("hashing a password failed".into()))
	}

	pub(crate) async fn hash(password: &str) -> Result<String> {
		let password = password.to_owned();
		blocking(move || hash_now(&password)).await?
	}

	/// A hash of nothing anyone knows, to check against when there's no
	/// password, so that takes as long as a wrong one.
	fn stand_in() -> &'static str {
		static STAND_IN: OnceLock<String> = OnceLock::new();
		STAND_IN.get_or_init(|| hash_now("leptoskit: no password here").unwrap_or_default())
	}

	/// Whether `password` matches `hash`, or the stand-in when there's no
	/// hash (which never matches anything typed).
	pub(crate) async fn verify(hash: Option<String>, password: &str) -> Result<bool> {
		let password = password.to_owned();
		blocking(move || {
			let hash = hash.unwrap_or_else(|| stand_in().to_owned());
			PasswordHash::new(&hash).is_ok_and(|parsed| {
				argon2()
					.verify_password(password.as_bytes(), &parsed)
					.is_ok()
			})
		})
		.await
	}

	/// Whether `hash` was made with other parameters than today's.
	pub(crate) fn outdated(hash: &str) -> bool {
		let Ok(parsed) = PasswordHash::new(hash) else {
			return true;
		};
		let current = argon2::Params::default();
		let params = argon2::Params::try_from(&parsed);
		parsed.algorithm != argon2::Algorithm::Argon2id.ident()
			|| parsed.version != Some(argon2::Version::V0x13.into())
			|| params.is_err()
			|| params.is_ok_and(|params| {
				params.m_cost() != current.m_cost()
					|| params.t_cost() != current.t_cost()
					|| params.p_cost() != current.p_cost()
			})
	}
}

/// Without the `password` feature, passwords are off before any of this is
/// asked.
#[cfg(not(feature = "password"))]
mod hashing {
	use crate::AuthError;
	use crate::Result;

	const OFF: AuthError = AuthError::Unsupported("passwords need the `password` feature");

	#[allow(clippy::unused_async, reason = "it stands in for the real hashing")]
	pub(crate) async fn hash(_password: &str) -> Result<String> {
		Err(OFF)
	}

	#[allow(clippy::unused_async, reason = "it stands in for the real hashing")]
	pub(crate) async fn verify(_hash: Option<String>, _password: &str) -> Result<bool> {
		Err(OFF)
	}

	pub(crate) fn outdated(_hash: &str) -> bool {
		false
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn email(address: &str) -> Email {
		Email::parse(address).unwrap()
	}

	#[test]
	fn only_length_and_guessability_count() {
		let ada = email("ada.lovelace@example.com");
		let check = |password: &str| problem(password, 15, Some(&ada), "Notes");

		assert_eq!(check("correct horse battery"), None);
		assert_eq!(
			check("all lowercase and long"),
			None,
			"no composition rules"
		);
		assert_eq!(check("short"), Some(PasswordProblem::TooShort { min: 15 }));
		assert_eq!(
			check(&"a".repeat(257)),
			Some(PasswordProblem::TooLong { max: 256 })
		);
		assert_eq!(check(&"é".repeat(15)), None, "characters, not bytes");

		let common = |password: &str| problem(password, 8, None, "Notes");
		assert_eq!(common("password"), Some(PasswordProblem::Common));
		assert_eq!(common("PassWord"), Some(PasswordProblem::Common));
		assert_eq!(common("iloveyou"), Some(PasswordProblem::Common));
		assert_eq!(
			common("# The most"),
			None,
			"the list's header isn't a password"
		);

		let personal = |password: &str| problem(password, 8, Some(&ada), "Notes App");
		assert_eq!(
			personal("Ada.Lovelace2026"),
			Some(PasswordProblem::Personal)
		);
		assert_eq!(
			personal("ada.lovelace@example.com"),
			Some(PasswordProblem::Personal)
		);
		assert_eq!(personal("notes-app-1234"), Some(PasswordProblem::Personal));
		assert_eq!(personal("ada lovelace's notes"), None);
	}

	#[test]
	fn the_list_is_long_enough_to_matter() {
		let entries = COMMON.lines().filter(|line| !line.starts_with('#')).count();
		assert!(entries > 2000, "{entries}");
		assert!(
			COMMON
				.lines()
				.filter(|line| !line.starts_with('#'))
				.all(|line| line.chars().count() >= 8 && line == line.to_lowercase())
		);
	}

	#[cfg(feature = "password")]
	#[test]
	fn passwords_compare_as_nfkc() {
		// A full-width "Ａ" and the ligature "ﬁ" fold to "A" and "fi".
		assert_eq!(normalize("Ａﬁ"), "Afi");
		assert_eq!(normalize("e\u{301}"), "é");
	}

	#[cfg(feature = "password")]
	#[tokio::test]
	async fn hashes_are_argon2id_with_owasps_parameters() {
		let hash = hashing::hash("correct horse battery").await.unwrap();
		assert!(
			hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
			"{hash}"
		);
		assert!(
			hashing::verify(Some(hash.clone()), "correct horse battery")
				.await
				.unwrap()
		);
		assert!(
			!hashing::verify(Some(hash.clone()), "correct horse battery!")
				.await
				.unwrap()
		);
		assert!(!hashing::outdated(&hash));

		// Weaker or other parameters are replaced at the next sign-in.
		let weaker = argon2::Argon2::new(
			argon2::Algorithm::Argon2id,
			argon2::Version::V0x13,
			argon2::Params::new(4096, 1, 1, None).unwrap(),
		);
		let salt = argon2::password_hash::SaltString::generate(
			&mut argon2::password_hash::rand_core::OsRng,
		);
		let old = argon2::PasswordHasher::hash_password(&weaker, b"x", &salt)
			.unwrap()
			.to_string();
		assert!(hashing::outdated(&old));
		assert!(hashing::outdated(
			"$argon2i$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNo"
		));
		assert!(hashing::outdated("not a hash"));
	}
}
