//! Signing in, or up, with a code sent to an email address.
//!
//! One flow does both: proving an address that has an account signs in to
//! it, and proving one that doesn't creates the account. So the response
//! to "send me a code" is the same for every address, and says nothing
//! about who has an account.

use leptoskit_db::Connection;
use leptoskit_db::Email;
use leptoskit_db::Id;
use leptoskit_db::Tx;

use crate::Aal;
use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::Challenge;
use crate::EmailAddress;
use crate::EventKind;
use crate::Mail;
use crate::NewAccount;
use crate::NewChallenge;
use crate::NewEmailAddress;
use crate::Profile;
use crate::Purpose;
use crate::RequestInfo;
use crate::Result;
use crate::SignIn;
use crate::auth::finish;
use crate::events::record;
use crate::secrets;

impl Auth {
	/// Emails a sign-in code to `address`. Any earlier code for it stops
	/// working.
	///
	/// Every valid address gets the same answer, whether or not it has an
	/// account: the code signs in to one, or creates it.
	///
	/// # Errors
	///
	/// [`AuthError::InvalidEmail`], [`AuthError::RateLimited`] after too
	/// many requests for the address or from the IP, or
	/// [`AuthError::Mail`] when the mailer fails.
	pub async fn send_email_code(&self, address: &str, request: &RequestInfo) -> Result<()> {
		let address = Email::parse(address).map_err(|_| AuthError::InvalidEmail)?;
		let limits = self.inner.limits;

		if let Some(ip) = request.ip_text() {
			self.limit(&[b"email-code-ip", ip.as_bytes()], limits.code_per_ip)
				.await?;
		}
		self.limit(
			&[b"email-code-address", address.as_bytes()],
			limits.code_per_address,
		)
		.await?;

		let code = secrets::generate_code()?;
		let hash = self.code_hash(&address, &code);
		let ttl = self.inner.codes.lifetime;
		let expires_at = leptoskit_db::now() + crate::sessions::lifetime(ttl)?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			retire_codes(&tx, &address).await?;
			NewChallenge {
				purpose: Purpose::EmailCode,
				subject: address.to_string(),
				secret_hash: hash,
				attempts: 0,
				expires_at,
				used_at: None,
			}
			.insert(&tx)
			.await?;
			let account = account_for(&tx, &address).await?;
			record(&tx, account, EventKind::CodeSent, request).await
		}
		.await;
		finish(tx, result).await?;

		let minutes = ttl.as_secs().div_ceil(60);
		let app = &self.inner.app_name;
		self.inner
			.mailer
			.send(Mail {
				to: address.to_string(),
				subject: format!("Your {app} sign-in code: {code}"),
				text: format!(
					"Your code to sign in to {app} is {code}.\n\nIt works once, for the next \
					 {minutes} minutes. If you didn't ask for it, you can ignore this email."
				),
			})
			.await?;

		Ok(())
	}

	/// Signs in with a code from [`Auth::send_email_code`], creating the
	/// account if the address has none, and starts a session.
	///
	/// # Errors
	///
	/// [`AuthError::InvalidCode`] for a wrong, expired, used, or
	/// over-guessed code, without saying which; [`AuthError::RateLimited`],
	/// [`AuthError::Disabled`], or [`AuthError::Locked`]; or whatever the
	/// app's sign-up hook returns, in which case nothing is created.
	pub async fn verify_email_code(
		&self,
		address: &str,
		code: &str,
		request: &RequestInfo,
	) -> Result<SignIn> {
		let address = Email::parse(address).map_err(|_| AuthError::InvalidEmail)?;

		if let Some(ip) = request.ip_text() {
			self.limit(
				&[b"email-code-verify-ip", ip.as_bytes()],
				self.inner.limits.verify_per_ip,
			)
			.await?;
		}

		let db = &self.inner.db;
		let now = leptoskit_db::now();
		let challenge = Challenge::query()
			.filter(|challenge| {
				challenge.purpose.eq(Purpose::EmailCode)
					& challenge.subject.eq(address.to_string())
					& challenge.used_at.is_null()
					& challenge.expires_at.gt(now)
			})
			.order_by(|challenge| challenge.created_at.desc())
			.first(db)
			.await?;
		let code = secrets::normalize_code(code).unwrap_or_default();
		// The hash is computed even without a challenge, so a missing code
		// takes as long to refuse as a wrong one.
		let matches = challenge.as_ref().is_some_and(|challenge| {
			self.inner
				.keys
				.verify(&challenge.secret_hash, &code_parts(&address, &code))
		});
		let Some(challenge) = challenge else {
			self.fail(&address, request).await?;
			return Err(AuthError::InvalidCode);
		};

		// Count the attempt before looking at the answer, so guesses can't
		// outrun the count. A code that has had its tries stops working.
		let attempts = challenge.attempts + 1;
		let allowed = i32::try_from(self.inner.codes.attempts).unwrap_or(i32::MAX);
		let counted = Challenge::update()
			.filter(|row| {
				row.id.eq(challenge.id)
					& row.attempts.eq(challenge.attempts)
					& row.used_at.is_null()
			})
			.set(|row| row.attempts, attempts)
			.set(
				|row| row.used_at,
				(attempts >= allowed && !matches).then_some(now),
			)
			.run(db)
			.await?;

		if counted == 0 || !matches || attempts > allowed {
			self.fail(&address, request).await?;
			return Err(AuthError::InvalidCode);
		}

		let tx = db.begin().await?;
		let result = self
			.sign_in_with_email(&tx, &challenge, &address, request)
			.await;
		finish(tx, result).await
	}

	/// Uses up the challenge and signs in to the address's account, or
	/// creates one.
	async fn sign_in_with_email(
		&self,
		tx: &Tx,
		challenge: &Challenge,
		address: &Email,
		request: &RequestInfo,
	) -> Result<SignIn> {
		let now = leptoskit_db::now();
		let claimed = Challenge::update()
			.filter(|row| row.id.eq(challenge.id) & row.used_at.is_null())
			.set(|row| row.used_at, Some(now))
			.run(tx)
			.await?;

		if claimed == 0 {
			return Err(AuthError::InvalidCode);
		}

		self.sign_in_to_address(tx, address, request).await
	}

	/// Signs in to the account of an address that was just proven, or
	/// creates it.
	async fn sign_in_to_address(
		&self,
		tx: &Tx,
		address: &Email,
		request: &RequestInfo,
	) -> Result<SignIn> {
		let now = leptoskit_db::now();
		let existing = EmailAddress::query()
			.filter(|email| email.address.eq(address))
			.first(tx)
			.await?;
		let (account, signed_up) = match existing {
			Some(email) => {
				let account = Account::get(tx, email.account_id).await?;
				refuse_unusable(&account)?;
				EmailAddress::update()
					.filter(|row| row.id.eq(email.id))
					.set(|row| row.verified_at, Some(now))
					.run(tx)
					.await?;
				(account.id, false)
			}
			None => (self.sign_up(tx, address, request).await?, true),
		};

		let mut sign_in = self
			.start_session(tx, account, Aal::One, now, None, request)
			.await?;
		record(tx, Some(account), EventKind::SignedIn, request).await?;
		sign_in.signed_up = signed_up;
		Ok(sign_in)
	}

	/// Development only: signs in to `address` without a code, creating its
	/// account if needed. Refused unless the builder enabled
	/// [`AuthBuilder::dev_sign_in`](crate::AuthBuilder::dev_sign_in).
	///
	/// # Errors
	///
	/// [`AuthError::Config`] when dev sign-in is off, or as
	/// [`Auth::verify_email_code`] after the code is accepted.
	pub async fn dev_sign_in(&self, address: &str, request: &RequestInfo) -> Result<SignIn> {
		if !cfg!(debug_assertions) || !self.inner.dev_sign_in {
			return Err(AuthError::Config("dev sign-in is off".into()));
		}

		let address = Email::parse(address).map_err(|_| AuthError::InvalidEmail)?;
		let tx = self.inner.db.begin().await?;
		let result = self.sign_in_to_address(&tx, &address, request).await;
		finish(tx, result).await
	}

	/// Creates an account for a proven address, and runs the app's hook.
	async fn sign_up(
		&self,
		tx: &Tx,
		address: &Email,
		request: &RequestInfo,
	) -> Result<Id<Account>> {
		let account = NewAccount {
			disabled_at: None,
			locked_until: None,
		}
		.insert(tx)
		.await?;
		NewEmailAddress {
			account_id: account.id,
			address: address.clone(),
			verified_at: Some(leptoskit_db::now()),
			is_primary: true,
		}
		.insert(tx)
		.await?;
		(self.inner.sign_up)(tx, account.id, &Profile::from_email(address.clone())).await?;
		record(tx, Some(account.id), EventKind::SignedUp, request).await?;
		Ok(account.id)
	}

	/// Records a failed attempt against the address's account, if it has
	/// one.
	async fn fail(&self, address: &Email, request: &RequestInfo) -> Result<()> {
		let db = &self.inner.db;
		let account = account_for(db, address).await?;
		record(db, account, EventKind::SignInFailed, request).await
	}

	async fn limit(&self, parts: &[&[u8]], limit: crate::Limit) -> Result<()> {
		self.inner.limiter.hit(&secrets::digest(parts), limit).await
	}

	fn code_hash(&self, address: &Email, code: &str) -> Vec<u8> {
		let code = secrets::normalize_code(code).unwrap_or_default();
		self.inner.keys.hash(&code_parts(address, &code))
	}
}

/// What a code's hash covers: the code, bound to its address and purpose,
/// so it works nowhere else.
fn code_parts<'a>(address: &'a Email, code: &'a str) -> [&'a [u8]; 3] {
	[b"email-code", address.as_bytes(), code.as_bytes()]
}

/// Retires every outstanding code for `address`.
async fn retire_codes(connection: &impl Connection, address: &Email) -> Result<()> {
	Challenge::update()
		.filter(|row| {
			row.purpose.eq(Purpose::EmailCode)
				& row.subject.eq(address.to_string())
				& row.used_at.is_null()
		})
		.set(|row| row.used_at, Some(leptoskit_db::now()))
		.run(connection)
		.await?;
	Ok(())
}

async fn account_for(connection: &impl Connection, address: &Email) -> Result<Option<Id<Account>>> {
	let email = EmailAddress::query()
		.filter(|email| email.address.eq(address))
		.first(connection)
		.await?;
	Ok(email.map(|email| email.account_id))
}

/// Refuses an account that may not sign in now.
pub(crate) fn refuse_unusable(account: &Account) -> Result<()> {
	if account.disabled_at.is_some() {
		return Err(AuthError::Disabled);
	}

	match account.locked_until {
		Some(until) if until > leptoskit_db::now() => Err(AuthError::Locked { until }),
		_ => Ok(()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn account(disabled: bool, locked_for: Option<i64>) -> Account {
		let now = leptoskit_db::now();
		Account {
			id: Id::new(),
			created_at: now,
			disabled_at: disabled.then_some(now),
			locked_until: locked_for.map(|minutes| now + chrono::TimeDelta::minutes(minutes)),
		}
	}

	#[test]
	fn disabled_and_locked_accounts_cannot_sign_in() {
		assert!(refuse_unusable(&account(false, None)).is_ok());
		assert!(refuse_unusable(&account(false, Some(-5))).is_ok());
		assert!(matches!(
			refuse_unusable(&account(true, None)),
			Err(AuthError::Disabled)
		));
		assert!(matches!(
			refuse_unusable(&account(false, Some(5))),
			Err(AuthError::Locked { .. })
		));
	}
}
