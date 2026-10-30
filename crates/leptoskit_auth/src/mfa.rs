//! Second factors: an authenticator app (TOTP) and recovery codes. They
//! guard the second step of signing in, once an account has turned them on,
//! and step-up, which a sensitive action asks for even of someone already
//! signed in.

use std::time::Duration;

use chrono::TimeDelta;
use leptoskit_db::Connection;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;

use crate::Aal;
use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::Challenge;
use crate::EmailAddress;
use crate::EventKind;
use crate::Factor;
use crate::Mail;
use crate::MfaStatus;
use crate::NewChallenge;
use crate::NewRecoveryCode;
use crate::NewTotpSecret;
use crate::Purpose;
use crate::RecoveryCode;
use crate::RecoveryCodes;
use crate::RequestInfo;
use crate::Result;
use crate::Session;
use crate::SessionToken;
use crate::SignIn;
use crate::SignedIn;
use crate::TotpSecret;
use crate::TotpSetup;
use crate::Transport;
use crate::auth::finish;
use crate::email::refuse_unusable;
use crate::events::record;
use crate::secrets;
use crate::totp;

/// How recently someone must have used a second factor for a sensitive
/// action, such as turning their authenticator off: ten minutes.
pub const STEP_UP_WINDOW: Duration = Duration::from_mins(10);

/// How long the second step of signing in waits for its code.
const SECOND_FACTOR_LIFETIME: TimeDelta = TimeDelta::minutes(5);

/// How many recovery codes an account holds at a time.
pub(crate) const RECOVERY_CODE_COUNT: usize = 10;

/// The characters in a recovery code, before the dash.
const RECOVERY_CODE_LENGTH: usize = 10;

/// Where signing in with a first factor, such as an email code, got to.
#[derive(Debug)]
pub enum SignInStep {
	/// Signed in: the account has no second factor.
	SignedIn(Box<SignIn>),
	/// The account has a second factor, which the person must use next.
	SecondFactor(SecondFactorChallenge),
}

impl SignInStep {
	/// The new session, when no second factor was needed.
	pub fn signed_in(self) -> Option<SignIn> {
		match self {
			Self::SignedIn(sign_in) => Some(*sign_in),
			Self::SecondFactor(_) => None,
		}
	}
}

/// The second step of signing in. The device keeps `token` and sends it
/// back with the code: a browser in an `HttpOnly` cookie, a native app in
/// memory. Only its hash is stored; it lasts five minutes and allows five
/// attempts.
#[derive(Debug)]
pub struct SecondFactorChallenge {
	/// Proves the first step happened, on this device.
	pub token: SessionToken,
	/// The factors the account can use.
	pub factors: Vec<Factor>,
	/// When the step expires.
	pub expires_at: Timestamp,
}

/// An authenticator that was just turned on: the session, elevated and with
/// a new token, and the recovery codes, when the account had none.
#[derive(Debug)]
pub struct TotpConfirmed {
	/// The session, now at `aal2`.
	pub sign_in: SignIn,
	/// New recovery codes, shown once.
	pub recovery_codes: Option<RecoveryCodes>,
}

/// Which factor a code turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Used {
	Totp,
	RecoveryCode,
}

impl SignedIn {
	/// Whether this session used a second factor within `window`, as
	/// sensitive actions require.
	pub fn stepped_up_within(&self, window: Duration) -> bool {
		let Ok(window) = TimeDelta::from_std(window) else {
			return false;
		};
		self.aal == Aal::Two
			&& self
				.mfa_at
				.is_some_and(|at| leptoskit_db::now() - at <= window)
	}

	/// `Ok` when this session used a second factor within `window`, and
	/// otherwise [`AuthError::StepUpRequired`], which the client answers by
	/// asking for a code and calling step-up.
	///
	/// ```ignore
	/// let me = auth::require_signed_in()?;
	/// me.require_step_up(auth::STEP_UP_WINDOW)?;
	/// ```
	///
	/// # Errors
	///
	/// [`AuthError::StepUpRequired`].
	pub fn require_step_up(&self, window: Duration) -> Result<()> {
		if self.stepped_up_within(window) {
			Ok(())
		} else {
			Err(AuthError::StepUpRequired)
		}
	}
}

impl Auth {
	/// The second factors `account` has turned on.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn mfa_status(&self, account: Id<Account>) -> Result<MfaStatus> {
		let db = &self.inner.db;
		let totp = confirmed_totp(db, account).await?.is_some();
		let left = RecoveryCode::query()
			.filter(|code| code.account_id.eq(account) & code.used_at.is_null())
			.count(db)
			.await?;
		Ok(MfaStatus {
			totp,
			recovery_codes: u32::try_from(left).unwrap_or(u32::MAX),
		})
	}

	/// The factors `account` can use as a second step, best first.
	pub(crate) async fn factors(
		&self,
		connection: &impl Connection,
		account: Id<Account>,
	) -> Result<Vec<Factor>> {
		let mut factors = Vec::new();

		if confirmed_totp(connection, account).await?.is_some() {
			factors.push(Factor::Totp);

			let left = RecoveryCode::query()
				.filter(|code| code.account_id.eq(account) & code.used_at.is_null())
				.count(connection)
				.await?;

			if left > 0 {
				factors.push(Factor::RecoveryCode);
			}
		}

		Ok(factors)
	}

	/// Starts the second step of signing in to `account`, retiring any
	/// earlier one.
	pub(crate) async fn second_factor_challenge(
		&self,
		tx: &Tx,
		account: Id<Account>,
		factors: Vec<Factor>,
	) -> Result<SecondFactorChallenge> {
		let now = leptoskit_db::now();
		let token = SessionToken::generate()?;
		let expires_at = now + SECOND_FACTOR_LIFETIME;
		Challenge::update()
			.filter(|row| {
				row.purpose.eq(Purpose::SecondFactor)
					& row.subject.eq(account.to_string())
					& row.used_at.is_null()
			})
			.set(|row| row.used_at, Some(now))
			.run(tx)
			.await?;
		NewChallenge {
			purpose: Purpose::SecondFactor,
			subject: account.to_string(),
			secret_hash: token.hash(),
			attempts: 0,
			expires_at,
			used_at: None,
		}
		.insert(tx)
		.await?;

		Ok(SecondFactorChallenge {
			token,
			factors,
			expires_at,
		})
	}

	/// Finishes signing in: checks a code from the account's authenticator,
	/// or one of its recovery codes, against the second step `token` began,
	/// and starts an `aal2` session.
	///
	/// Using a recovery code ends the account's other sessions and emails
	/// the account, since it often means a lost phone.
	///
	/// # Errors
	///
	/// [`AuthError::SignInExpired`] when the step is unknown, used, expired,
	/// or out of attempts; [`AuthError::InvalidCode`] for a wrong code;
	/// [`AuthError::RateLimited`]; or [`AuthError::Disabled`] or
	/// [`AuthError::Locked`].
	pub async fn complete_sign_in(
		&self,
		token: &SessionToken,
		code: &str,
		request: &RequestInfo,
	) -> Result<SignIn> {
		if let Some(ip) = request.ip_text() {
			self.limit(
				&[b"second-factor-ip", ip.as_bytes()],
				self.inner.limits.verify_per_ip,
			)
			.await?;
		}

		let db = &self.inner.db;
		let now = leptoskit_db::now();
		let challenge = Challenge::query()
			.filter(|challenge| {
				challenge.purpose.eq(Purpose::SecondFactor)
					& challenge.secret_hash.eq(token.hash())
					& challenge.used_at.is_null()
					& challenge.expires_at.gt(now)
			})
			.first(db)
			.await?
			.ok_or(AuthError::SignInExpired)?;
		let account: Id<Account> = challenge
			.subject
			.parse()
			.map_err(|_| AuthError::SignInExpired)?;
		self.limit_second_factor(account).await?;

		// Count the attempt before looking at the code, so guesses can't
		// outrun the count.
		let attempts = challenge.attempts + 1;
		let allowed = i32::try_from(self.inner.codes.attempts).unwrap_or(i32::MAX);
		let counted = Challenge::update()
			.filter(|row| {
				row.id.eq(challenge.id)
					& row.attempts.eq(challenge.attempts)
					& row.used_at.is_null()
			})
			.set(|row| row.attempts, attempts)
			.run(db)
			.await?;

		if counted == 0 || attempts > allowed {
			return Err(AuthError::SignInExpired);
		}

		let tx = db.begin().await?;
		let result = self
			.finish_second_factor(&tx, &challenge, account, code, request)
			.await;
		let result = finish(tx, result).await;

		if let Ok((_, Used::RecoveryCode)) = &result {
			self.notify(
				account,
				"A recovery code was used",
				"Someone signed in to your account with one of its recovery codes, and every \
				 other device was signed out. If it wasn't you, sign in, turn two-step \
				 verification off and on again, and make new recovery codes.",
			)
			.await;
		} else if let Err(AuthError::InvalidCode) = &result {
			// The last attempt retires the step.
			if attempts >= allowed {
				Challenge::update()
					.filter(|row| row.id.eq(challenge.id))
					.set(|row| row.used_at, Some(now))
					.run(db)
					.await?;
			}
			record(db, Some(account), EventKind::SecondFactorFailed, request).await?;
		}

		result.map(|(sign_in, _)| sign_in)
	}

	/// The inside of [`Auth::complete_sign_in`], in its transaction.
	async fn finish_second_factor(
		&self,
		tx: &Tx,
		challenge: &Challenge,
		account: Id<Account>,
		code: &str,
		request: &RequestInfo,
	) -> Result<(SignIn, Used)> {
		let claimed = Challenge::update()
			.filter(|row| row.id.eq(challenge.id) & row.used_at.is_null())
			.set(|row| row.used_at, Some(leptoskit_db::now()))
			.run(tx)
			.await?;

		if claimed == 0 {
			return Err(AuthError::SignInExpired);
		}

		refuse_unusable(&Account::get(tx, account).await?)?;
		let used = self
			.use_factor(tx, account, code)
			.await?
			.ok_or(AuthError::InvalidCode)?;
		let now = leptoskit_db::now();
		let sign_in = self
			.start_session(
				tx,
				account,
				Aal::Two,
				challenge.created_at,
				Some(now),
				request,
			)
			.await?;
		record(tx, Some(account), EventKind::SignedIn, request).await?;

		if used == Used::RecoveryCode {
			self.after_recovery_code(tx, account, sign_in.session.id, request)
				.await?;
		}

		Ok((sign_in, used))
	}

	/// Proves the signed-in person is still here with a code from their
	/// authenticator or a recovery code, for an action that asked for
	/// step-up. The session moves to `aal2` with a fresh `mfa_at`, and gets
	/// a new token, so one captured before stops working.
	///
	/// # Errors
	///
	/// [`AuthError::InvalidCode`], [`AuthError::RateLimited`], or
	/// [`AuthError::SignedOut`] when the session has ended.
	pub async fn step_up(
		&self,
		me: &SignedIn,
		code: &str,
		request: &RequestInfo,
	) -> Result<SignIn> {
		if let Some(ip) = request.ip_text() {
			self.limit(
				&[b"second-factor-ip", ip.as_bytes()],
				self.inner.limits.verify_per_ip,
			)
			.await?;
		}
		self.limit_second_factor(me.account).await?;

		let tx = self.inner.db.begin().await?;
		let result = async {
			let used = self
				.use_factor(&tx, me.account, code)
				.await?
				.ok_or(AuthError::InvalidCode)?;
			let sign_in = self.elevate(&tx, me, request).await?;
			record(&tx, Some(me.account), EventKind::SteppedUp, request).await?;

			if used == Used::RecoveryCode {
				self.after_recovery_code(&tx, me.account, me.session, request)
					.await?;
			}

			Ok((sign_in, used))
		}
		.await;
		let result = finish(tx, result).await;

		match &result {
			Ok((_, Used::RecoveryCode)) => {
				self.notify(
					me.account,
					"A recovery code was used",
					"One of your account's recovery codes was used to confirm it's you, and every \
					 other device was signed out. If it wasn't you, turn two-step verification \
					 off and on again, and make new recovery codes.",
				)
				.await;
			}
			Err(AuthError::InvalidCode) => {
				record(
					&self.inner.db,
					Some(me.account),
					EventKind::SecondFactorFailed,
					request,
				)
				.await?;
			}
			_ => {}
		}

		result.map(|(sign_in, _)| sign_in)
	}

	/// Starts turning on an authenticator app: a new secret, kept encrypted
	/// and unconfirmed until [`Auth::confirm_totp`] sees a code from it. Show
	/// the person the [`TotpSetup`]'s link as a QR code, or its secret to
	/// type in.
	///
	/// It needs a fresh sign-in: within the last ten minutes, or step-up
	/// when the account already has a second factor. Starting again
	/// replaces an unconfirmed secret.
	///
	/// # Errors
	///
	/// [`AuthError::Reauthenticate`] or [`AuthError::StepUpRequired`] when
	/// the session isn't fresh, and [`AuthError::AlreadyEnrolled`] when an
	/// authenticator is already on.
	pub async fn begin_totp(&self, me: &SignedIn) -> Result<TotpSetup> {
		let db = &self.inner.db;
		self.require_fresh(db, me).await?;
		let existing = TotpSecret::query()
			.filter(|row| row.account_id.eq(me.account))
			.first(db)
			.await?;

		if existing
			.as_ref()
			.is_some_and(|row| row.confirmed_at.is_some())
		{
			return Err(AuthError::AlreadyEnrolled);
		}

		let secret = totp::Secret(secrets::random::<{ totp::SECRET_LENGTH }>()?.to_vec());
		let account = me.account.uuid();
		let sealed = self
			.inner
			.keys
			.seal(&totp_context(account.as_bytes()), &secret.0)?;

		match existing {
			Some(row) => {
				TotpSecret::update()
					.filter(|update| update.id.eq(row.id) & update.confirmed_at.is_null())
					.set(|update| update.secret, sealed)
					.set(|update| update.last_used_step, None)
					.run(db)
					.await?;
			}
			None => {
				NewTotpSecret {
					account_id: me.account,
					secret: sealed,
					confirmed_at: None,
					last_used_step: None,
				}
				.insert(db)
				.await?;
			}
		}

		let label = primary_email(db, me.account)
			.await?
			.unwrap_or_else(|| self.inner.app_name.clone());
		Ok(TotpSetup {
			secret: secret.base32(),
			uri: secret.uri(&self.inner.app_name, &label),
		})
	}

	/// Turns the authenticator on once it shows a code that matches the
	/// secret [`Auth::begin_totp`] gave. The session moves to `aal2` with a
	/// new token, and an account without recovery codes gets ten, shown
	/// once.
	///
	/// # Errors
	///
	/// [`AuthError::NotEnrolled`] without a secret to confirm,
	/// [`AuthError::InvalidCode`], [`AuthError::RateLimited`], or as
	/// [`Auth::begin_totp`] when the session isn't fresh.
	pub async fn confirm_totp(
		&self,
		me: &SignedIn,
		code: &str,
		request: &RequestInfo,
	) -> Result<TotpConfirmed> {
		self.require_fresh(&self.inner.db, me).await?;
		self.limit_second_factor(me.account).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let row = TotpSecret::query()
				.filter(|row| row.account_id.eq(me.account) & row.confirmed_at.is_null())
				.first(&tx)
				.await?
				.ok_or(AuthError::NotEnrolled)?;
			let account = me.account.uuid();
			let secret = totp::Secret(
				self.inner
					.keys
					.open(&totp_context(account.as_bytes()), &row.secret)?,
			);
			let now = leptoskit_db::now();
			let step = secret
				.verify(code, totp::Secret::step(now.timestamp()), None)
				.ok_or(AuthError::InvalidCode)?;
			let confirmed = TotpSecret::update()
				.filter(|update| update.id.eq(row.id) & update.confirmed_at.is_null())
				.set(|update| update.confirmed_at, Some(now))
				.set(|update| update.last_used_step, Some(step))
				.run(&tx)
				.await?;

			if confirmed == 0 {
				return Err(AuthError::NotEnrolled);
			}

			let left = RecoveryCode::query()
				.filter(|code| code.account_id.eq(me.account) & code.used_at.is_null())
				.count(&tx)
				.await?;
			let recovery_codes = if left == 0 {
				Some(replace_recovery_codes(self, &tx, me.account).await?)
			} else {
				None
			};
			let sign_in = self.elevate(&tx, me, request).await?;
			record(&tx, Some(me.account), EventKind::TotpEnabled, request).await?;
			Ok(TotpConfirmed {
				sign_in,
				recovery_codes,
			})
		}
		.await;
		let result = finish(tx, result).await;

		match &result {
			Ok(_) => {
				self.notify(
					me.account,
					"Two-step verification is on",
					"An authenticator app now guards your account: signing in asks for a code \
					 from it after the emailed one. If you didn't turn this on, sign in and turn \
					 it off, then sign out everywhere.",
				)
				.await;
			}
			Err(AuthError::InvalidCode) => {
				record(
					&self.inner.db,
					Some(me.account),
					EventKind::SecondFactorFailed,
					request,
				)
				.await?;
			}
			Err(_) => {}
		}

		result
	}

	/// Turns the authenticator off and deletes the recovery codes, after
	/// step-up. Every other session ends, since whoever removes a factor may
	/// not be the owner.
	///
	/// # Errors
	///
	/// [`AuthError::StepUpRequired`], or [`AuthError::NotEnrolled`] when
	/// there's no authenticator.
	pub async fn disable_totp(&self, me: &SignedIn, request: &RequestInfo) -> Result<()> {
		me.require_step_up(STEP_UP_WINDOW)?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let removed = TotpSecret::query()
				.filter(|row| row.account_id.eq(me.account))
				.delete(&tx)
				.await?;

			if removed == 0 {
				return Err(AuthError::NotEnrolled);
			}

			RecoveryCode::query()
				.filter(|code| code.account_id.eq(me.account))
				.delete(&tx)
				.await?;
			self.end_others(&tx, me.account, me.session, request)
				.await?;
			record(&tx, Some(me.account), EventKind::TotpDisabled, request).await
		}
		.await;
		finish(tx, result).await?;
		self.notify(
			me.account,
			"Two-step verification is off",
			"Your account no longer asks for a code from an authenticator app, and every other \
			 device was signed out. If you didn't do this, sign in and turn it on again.",
		)
		.await;
		Ok(())
	}

	/// Replaces the account's recovery codes with ten new ones, after
	/// step-up. The old ones stop working.
	///
	/// # Errors
	///
	/// [`AuthError::StepUpRequired`], or [`AuthError::NotEnrolled`] without
	/// an authenticator, since recovery codes back one up.
	pub async fn regenerate_recovery_codes(
		&self,
		me: &SignedIn,
		request: &RequestInfo,
	) -> Result<RecoveryCodes> {
		me.require_step_up(STEP_UP_WINDOW)?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			if confirmed_totp(&tx, me.account).await?.is_none() {
				return Err(AuthError::NotEnrolled);
			}

			let codes = replace_recovery_codes(self, &tx, me.account).await?;
			record(
				&tx,
				Some(me.account),
				EventKind::RecoveryCodesRegenerated,
				request,
			)
			.await?;
			Ok(codes)
		}
		.await;
		finish(tx, result).await
	}

	/// Whether `me` may change their sign-in methods: step-up when the
	/// account has a second factor, and otherwise a sign-in within the
	/// step-up window.
	async fn require_fresh(&self, connection: &impl Connection, me: &SignedIn) -> Result<()> {
		if !self.factors(connection, me.account).await?.is_empty() {
			return me.require_step_up(STEP_UP_WINDOW);
		}

		let window = TimeDelta::from_std(STEP_UP_WINDOW).unwrap_or_default();

		if me.stepped_up_within(STEP_UP_WINDOW)
			|| leptoskit_db::now() - me.authenticated_at <= window
		{
			Ok(())
		} else {
			Err(AuthError::Reauthenticate)
		}
	}

	/// Checks `input` against the account's authenticator, or, when it
	/// looks like one, its recovery codes, and uses up what matched: the
	/// authenticator's time step, so its code works once, or the recovery
	/// code.
	async fn use_factor(
		&self,
		connection: &impl Connection,
		account: Id<Account>,
		input: &str,
	) -> Result<Option<Used>> {
		let now = leptoskit_db::now();

		if totp::normalize(input).is_some() {
			let Some(row) = confirmed_totp(connection, account).await? else {
				return Ok(None);
			};
			let uuid = account.uuid();
			let secret = totp::Secret(
				self.inner
					.keys
					.open(&totp_context(uuid.as_bytes()), &row.secret)?,
			);
			let Some(step) = secret.verify(
				input,
				totp::Secret::step(now.timestamp()),
				row.last_used_step,
			) else {
				return Ok(None);
			};
			// Claiming the step is what makes the code single use, even when
			// two requests race with it.
			let claimed = TotpSecret::update()
				.filter(|update| {
					update.id.eq(row.id)
						& (update.last_used_step.is_null() | update.last_used_step.lt(step))
				})
				.set(|update| update.last_used_step, Some(step))
				.run(connection)
				.await?;
			return Ok((claimed == 1).then_some(Used::Totp));
		}

		let Some(code) = normalize_recovery_code(input) else {
			return Ok(None);
		};
		let codes = RecoveryCode::query()
			.filter(|row| row.account_id.eq(account) & row.used_at.is_null())
			.all(connection)
			.await?;
		let uuid = account.uuid();
		let parts = recovery_parts(uuid.as_bytes(), &code);
		// Every unused code is checked, in constant time, so how long this
		// takes doesn't say which one matched.
		let mut matched = None;

		for row in &codes {
			if self.inner.keys.verify(&row.code_hash, &parts) {
				matched = Some(row.id);
			}
		}

		let Some(id) = matched else {
			return Ok(None);
		};
		let claimed = RecoveryCode::update()
			.filter(|row| row.id.eq(id) & row.used_at.is_null())
			.set(|row| row.used_at, Some(now))
			.run(connection)
			.await?;
		Ok((claimed == 1).then_some(Used::RecoveryCode))
	}

	/// Moves `me`'s session to `aal2` now, with a new token in place of the
	/// old one, or new tokens for a native app.
	async fn elevate(
		&self,
		connection: &impl Connection,
		me: &SignedIn,
		request: &RequestInfo,
	) -> Result<SignIn> {
		let now = leptoskit_db::now();
		let session = Session::get(connection, me.session).await?;
		let token = SessionToken::generate()?;
		let (refresh, access_expires_at) = match request.transport {
			Transport::Cookie => (None, None),
			Transport::Bearer => {
				(
					Some(SessionToken::generate()?),
					Some((now + self.access()?).min(session.expires_at)),
				)
			}
		};
		let changed = Session::update()
			.filter(|row| row.id.eq(me.session) & row.revoked_at.is_null())
			.set(|row| row.token_hash, token.hash())
			.set(|row| row.access_expires_at, access_expires_at)
			.set(
				|row| row.refresh_hash,
				refresh.as_ref().map(SessionToken::hash),
			)
			.set(|row| row.previous_refresh_hash, None)
			.set(|row| row.refreshed_at, None)
			.set(|row| row.aal, Aal::Two)
			.set(|row| row.mfa_at, Some(now))
			.set(|row| row.last_seen_at, now)
			.run(connection)
			.await?;

		if changed == 0 {
			return Err(AuthError::SignedOut);
		}

		Ok(SignIn {
			token,
			refresh,
			session: Session::get(connection, me.session).await?,
			signed_up: false,
		})
	}

	/// After a recovery code: every other session ends, and the event is
	/// recorded.
	async fn after_recovery_code(
		&self,
		tx: &Tx,
		account: Id<Account>,
		keep: Id<Session>,
		request: &RequestInfo,
	) -> Result<()> {
		self.end_others(tx, account, keep, request).await?;
		record(tx, Some(account), EventKind::RecoveryCodeUsed, request).await
	}

	/// Ends every live session of `account`'s but `keep`.
	async fn end_others(
		&self,
		tx: &Tx,
		account: Id<Account>,
		keep: Id<Session>,
		request: &RequestInfo,
	) -> Result<()> {
		let ended = Session::update()
			.filter(|row| row.account_id.eq(account) & row.id.ne(keep) & row.revoked_at.is_null())
			.set(|row| row.revoked_at, Some(leptoskit_db::now()))
			.run(tx)
			.await?;

		if ended > 0 {
			record(tx, Some(account), EventKind::SessionRevoked, request).await?;
		}

		Ok(())
	}

	/// Second-factor attempts for one account, from anywhere.
	async fn limit_second_factor(&self, account: Id<Account>) -> Result<()> {
		self.limit(
			&[b"second-factor-account", account.uuid().as_bytes()],
			self.inner.limits.second_factor_per_account,
		)
		.await
	}

	/// Emails the account's primary address about a change to how it signs
	/// in. Best effort: the change has already happened and been recorded in
	/// the audit trail, so a mailer failure doesn't undo or hide it.
	async fn notify(&self, account: Id<Account>, subject: &str, text: &str) {
		let Ok(Some(to)) = primary_email(&self.inner.db, account).await else {
			return;
		};
		let app = &self.inner.app_name;
		let mail = Mail {
			to,
			subject: format!("{app}: {subject}"),
			text: text.to_owned(),
		};
		// Nothing more can be done: the audit trail already holds the event.
		let _ = self.inner.mailer.send(mail).await;
	}
}

/// The account's authenticator, once confirmed.
async fn confirmed_totp(
	connection: &impl Connection,
	account: Id<Account>,
) -> Result<Option<TotpSecret>> {
	Ok(TotpSecret::query()
		.filter(|row| row.account_id.eq(account) & row.confirmed_at.is_not_null())
		.first(connection)
		.await?)
}

/// The account's primary, verified address.
async fn primary_email(
	connection: &impl Connection,
	account: Id<Account>,
) -> Result<Option<String>> {
	Ok(EmailAddress::query()
		.filter(|email| {
			email.account_id.eq(account)
				& email.is_primary.eq(true)
				& email.verified_at.is_not_null()
		})
		.first(connection)
		.await?
		.map(|email| email.address.to_string()))
}

/// Deletes the account's recovery codes and stores ten new ones, returning
/// them as the person writes them down.
async fn replace_recovery_codes(
	auth: &Auth,
	tx: &Tx,
	account: Id<Account>,
) -> Result<RecoveryCodes> {
	RecoveryCode::query()
		.filter(|row| row.account_id.eq(account))
		.delete(tx)
		.await?;
	let uuid = account.uuid();
	let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);

	for _ in 0..RECOVERY_CODE_COUNT {
		let code = generate_recovery_code()?;
		let normalized = normalize_recovery_code(&code).unwrap_or_default();
		NewRecoveryCode {
			account_id: account,
			code_hash: auth
				.inner
				.keys
				.hash(&recovery_parts(uuid.as_bytes(), &normalized)),
			used_at: None,
		}
		.insert(tx)
		.await?;
		codes.push(code);
	}

	Ok(RecoveryCodes { codes })
}

/// What an authenticator's encrypted secret is bound to: its purpose and
/// account, so a secret copied to another row doesn't decrypt.
fn totp_context(account: &[u8]) -> [&[u8]; 2] {
	[b"totp", account]
}

/// What a recovery code's hash covers: the code, bound to its account and
/// purpose.
fn recovery_parts<'a>(account: &'a [u8], code: &'a str) -> [&'a [u8]; 3] {
	[b"recovery-code", account, code.as_bytes()]
}

/// A new recovery code, such as `K7QMR-2XDPA`: 50 random bits from the
/// unmistakable alphabet emailed codes use.
fn generate_recovery_code() -> Result<String> {
	let bytes = secrets::random::<RECOVERY_CODE_LENGTH>()?;
	let mut code = String::with_capacity(RECOVERY_CODE_LENGTH + 1);

	for (index, byte) in bytes.into_iter().enumerate() {
		if index == RECOVERY_CODE_LENGTH / 2 {
			code.push('-');
		}
		code.push(char::from(secrets::CODE_ALPHABET[usize::from(byte % 32)]));
	}

	Ok(code)
}

/// A recovery code as typed, in the form it was hashed in. `None` when it
/// can't be one.
fn normalize_recovery_code(input: &str) -> Option<String> {
	let code: String = input
		.chars()
		.filter(|character| !character.is_whitespace() && *character != '-')
		.map(|character| character.to_ascii_uppercase())
		.collect();
	let valid = code.len() == RECOVERY_CODE_LENGTH
		&& code
			.bytes()
			.all(|byte| secrets::CODE_ALPHABET.contains(&byte));

	valid.then_some(code)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn recovery_codes_are_readable_and_normalize_as_typed() {
		let code = generate_recovery_code().unwrap();
		assert_eq!(code.len(), RECOVERY_CODE_LENGTH + 1);
		assert_eq!(code.as_bytes()[5], b'-');

		let normalized = normalize_recovery_code(&code).unwrap();
		assert_eq!(
			normalize_recovery_code(&normalized.to_lowercase()),
			Some(normalized)
		);
		assert_eq!(
			normalize_recovery_code(" k7qmr 2xdpa "),
			Some("K7QMR2XDPA".into())
		);
		// An authenticator's code isn't a recovery code, nor is a short one.
		assert_eq!(normalize_recovery_code("123456"), None);
		assert_eq!(normalize_recovery_code("K7QMR-2XDP0"), None);
		assert_ne!(generate_recovery_code().unwrap(), code);
	}

	#[test]
	fn step_up_needs_a_recent_second_factor() {
		let now = leptoskit_db::now();
		let me = |aal, minutes: Option<i64>| {
			SignedIn {
				account: Id::new(),
				session: Id::new(),
				aal,
				authenticated_at: now,
				mfa_at: minutes.map(|minutes| now - TimeDelta::minutes(minutes)),
			}
		};

		assert!(me(Aal::Two, Some(1)).stepped_up_within(STEP_UP_WINDOW));
		assert!(
			me(Aal::Two, Some(1))
				.require_step_up(STEP_UP_WINDOW)
				.is_ok()
		);
		assert!(!me(Aal::Two, Some(11)).stepped_up_within(STEP_UP_WINDOW));
		assert!(!me(Aal::One, Some(1)).stepped_up_within(STEP_UP_WINDOW));
		assert!(!me(Aal::Two, None).stepped_up_within(STEP_UP_WINDOW));
		assert!(!me(Aal::Two, Some(1)).stepped_up_within(Duration::MAX));
		assert!(matches!(
			me(Aal::One, None).require_step_up(STEP_UP_WINDOW),
			Err(AuthError::StepUpRequired)
		));
	}
}
