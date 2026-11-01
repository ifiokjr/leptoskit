//! Passkeys: WebAuthn credentials that sign in on their own, as a second
//! step after an emailed code, or for step-up.
//!
//! Each ceremony's state, the challenge the device must sign, lives sealed in
//! `auth.challenges`, works once, and expires after five minutes. It's never
//! kept on the device: a browser holds only a random token in an `HttpOnly`
//! cookie, and a signed-in ceremony is bound to the session instead.

use chrono::TimeDelta;
use leptoskit_db::Connection;
use leptoskit_db::Id;

use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::Challenge;
use crate::EmailAddress;
use crate::EventKind;
use crate::Identity;
use crate::NewChallenge;
use crate::Passkey;
use crate::PasskeyInfo;
use crate::Purpose;
use crate::RecoveryCodes;
use crate::RequestInfo;
use crate::Result;
use crate::SessionToken;
use crate::SignIn;
use crate::SignedIn;
use crate::events::record;

/// How long a passkey ceremony waits for the device.
pub(crate) const CEREMONY_LIFETIME: TimeDelta = TimeDelta::minutes(5);

/// The longest passkey name kept, in characters.
const NAME_LENGTH: usize = 100;

/// A passkey that was just added: its row, the session, elevated and with
/// a new token, and recovery codes, when the account had none.
#[derive(Debug)]
pub struct PasskeyAdded {
	/// The passkey.
	pub passkey: PasskeyInfo,
	/// The session, now at `aal2`.
	pub sign_in: SignIn,
	/// New recovery codes, shown once.
	pub recovery_codes: Option<RecoveryCodes>,
}

/// A passkey sign-in that has started: the options for the device, and the
/// token that matches its answer to this ceremony, which a browser keeps in
/// an `HttpOnly` cookie.
#[derive(Debug)]
pub struct PasskeyChallenge {
	/// Matches the answer to the ceremony.
	pub token: SessionToken,
	/// The options for `navigator.credentials.get`.
	pub options: crate::RequestChallengeResponse,
}

impl From<Passkey> for PasskeyInfo {
	fn from(passkey: Passkey) -> Self {
		Self {
			id: passkey.id,
			name: passkey.name,
			backed_up: passkey.backed_up,
			created_at: passkey.created_at,
			last_used_at: passkey.last_used_at,
		}
	}
}

impl Auth {
	/// The account's passkeys, newest first.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn passkeys(&self, account: Id<Account>) -> Result<Vec<PasskeyInfo>> {
		let passkeys = Passkey::query()
			.filter(|passkey| passkey.account_id.eq(account))
			.order_by(|passkey| passkey.created_at.desc())
			.all(&self.inner.db)
			.await?;
		Ok(passkeys.into_iter().map(PasskeyInfo::from).collect())
	}

	/// Renames one of the signed-in person's passkeys.
	///
	/// # Errors
	///
	/// [`AuthError::NotFound`] when it isn't theirs, or
	/// [`AuthError::InvalidName`] for an empty name.
	pub async fn rename_passkey(&self, me: &SignedIn, id: Id<Passkey>, name: &str) -> Result<()> {
		let name = passkey_name(name).ok_or(AuthError::InvalidName)?;
		let renamed = Passkey::update()
			.filter(|row| row.id.eq(id) & row.account_id.eq(me.account))
			.set(|row| row.name, name)
			.run(&self.inner.db)
			.await?;

		if renamed == 0 {
			return Err(AuthError::NotFound);
		}

		Ok(())
	}

	/// Removes one of the signed-in person's passkeys, after step-up. Every
	/// other session ends, since whoever removes a factor may not be the
	/// owner. The last passkey of an account that must have a second factor,
	/// or that has no other way to sign in, stays.
	///
	/// # Errors
	///
	/// [`AuthError::StepUpRequired`], [`AuthError::NotFound`], or
	/// [`AuthError::LastSignInMethod`].
	pub async fn remove_passkey(
		&self,
		me: &SignedIn,
		id: Id<Passkey>,
		request: &RequestInfo,
	) -> Result<()> {
		me.require_step_up(crate::STEP_UP_WINDOW)?;
		// The app's roles are read before the transaction, which may hold
		// the only connection.
		let required = self.mfa_required(&self.inner.db, me.account).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let passkey = Passkey::query()
				.filter(|row| row.id.eq(id) & row.account_id.eq(me.account))
				.first(&tx)
				.await?
				.ok_or(AuthError::NotFound)?;
			passkey_row_delete(&tx, passkey.id).await?;
			let methods = self.sign_in_methods(&tx, me.account).await?;
			let factors = self.factors(&tx, me.account).await?;

			if methods.is_empty() || (required && factors.is_empty()) {
				return Err(AuthError::LastSignInMethod);
			}

			self.forget_recovery_codes_without_factors(&tx, me.account)
				.await?;
			self.end_others(&tx, me.account, me.session, request)
				.await?;
			record(&tx, Some(me.account), EventKind::PasskeyRemoved, request).await
		}
		.await;
		crate::auth::finish(tx, result).await?;
		self.notify(
			me.account,
			"A passkey was removed",
			"A passkey can no longer sign in to your account, and every other device was signed \
			 out. If you didn't do this, sign in and sign out everywhere.",
		)
		.await;
		Ok(())
	}

	/// How the account can sign in on its own: `email`, `passkey`, each
	/// connected provider, and `solana` for wallets that may sign in.
	pub(crate) async fn sign_in_methods(
		&self,
		connection: &impl Connection,
		account: Id<Account>,
	) -> Result<Vec<String>> {
		let mut methods = Vec::new();
		let emails = EmailAddress::query()
			.filter(|email| email.account_id.eq(account) & email.verified_at.is_not_null())
			.count(connection)
			.await?;

		if emails > 0 {
			methods.push("email".to_owned());
		}

		let passkeys = Passkey::query()
			.filter(|passkey| passkey.account_id.eq(account))
			.count(connection)
			.await?;

		if passkeys > 0 {
			methods.push("passkey".to_owned());
		}

		let identities = Identity::query()
			.filter(|identity| identity.account_id.eq(account))
			.all(connection)
			.await?;
		methods.extend(identities.into_iter().map(|identity| identity.provider));

		if self.wallet_settings().sign_in {
			let wallets = crate::Wallet::query()
				.filter(|wallet| wallet.account_id.eq(account) & wallet.can_sign_in.eq(true))
				.count(connection)
				.await?;

			if wallets > 0 {
				methods.push(crate::wallets::CHAIN.to_owned());
			}
		}

		Ok(methods)
	}

	/// Starts a passkey ceremony bound to `subject`, keeping `state` sealed
	/// until it finishes. Earlier unfinished ones for the same subject stop
	/// working. Returns the token that also finds it.
	pub(crate) async fn store_ceremony(
		&self,
		connection: &impl Connection,
		purpose: Purpose,
		subject: &str,
		state: &[u8],
	) -> Result<SessionToken> {
		self.store_ceremony_for(connection, purpose, subject, state, CEREMONY_LIFETIME)
			.await
	}

	/// [`Auth::store_ceremony`], lasting `lifetime`.
	pub(crate) async fn store_ceremony_for(
		&self,
		connection: &impl Connection,
		purpose: Purpose,
		subject: &str,
		state: &[u8],
		lifetime: TimeDelta,
	) -> Result<SessionToken> {
		let now = leptoskit_db::now();
		let token = SessionToken::generate()?;
		Challenge::update()
			.filter(|row| row.purpose.eq(purpose) & row.subject.eq(subject) & row.used_at.is_null())
			.set(|row| row.used_at, Some(now))
			.run(connection)
			.await?;
		let hash = token.hash();
		let sealed = self
			.inner
			.keys
			.seal(&ceremony_context(purpose, subject, &hash), state)?;
		NewChallenge {
			purpose,
			subject: subject.to_owned(),
			secret_hash: hash,
			state: Some(sealed),
			attempts: 0,
			expires_at: now + lifetime,
			used_at: None,
		}
		.insert(connection)
		.await?;
		Ok(token)
	}

	/// The state of the newest live ceremony bound to `subject`, and when it
	/// expires, without using it up.
	pub(crate) async fn peek_ceremony(
		&self,
		purpose: Purpose,
		subject: &str,
	) -> Result<Option<(Vec<u8>, leptoskit_db::Timestamp)>> {
		let now = leptoskit_db::now();
		let row = Challenge::query()
			.filter(|row| {
				row.purpose.eq(purpose)
					& row.subject.eq(subject)
					& row.used_at.is_null()
					& row.expires_at.gt(now)
			})
			.order_by(|row| row.created_at.desc())
			.first(&self.inner.db)
			.await?;
		let Some(row) = row else {
			return Ok(None);
		};
		let Some(sealed) = &row.state else {
			return Ok(None);
		};
		let context = ceremony_context(purpose, subject, &row.secret_hash);
		Ok(Some((
			self.inner.keys.open(&context, sealed)?,
			row.expires_at,
		)))
	}

	/// Uses up the ceremony bound to `subject`, or with `token` when it has
	/// one, and opens its state. It's claimed before anything is checked, so
	/// it works once even when the answer is wrong.
	pub(crate) async fn take_ceremony(
		&self,
		purpose: Purpose,
		subject: &str,
		token: Option<&SessionToken>,
	) -> Result<Option<Vec<u8>>> {
		let db = &self.inner.db;
		let now = leptoskit_db::now();
		let hash = token.map(SessionToken::hash);
		let row = Challenge::query()
			.filter(|row| {
				let mut matches = row.purpose.eq(purpose)
					& row.subject.eq(subject)
					& row.used_at.is_null()
					& row.expires_at.gt(now);

				if let Some(hash) = &hash {
					matches = matches & row.secret_hash.eq(hash.clone());
				}

				matches
			})
			.order_by(|row| row.created_at.desc())
			.first(db)
			.await?;
		let Some(row) = row else {
			return Ok(None);
		};
		let claimed = Challenge::update()
			.filter(|update| update.id.eq(row.id) & update.used_at.is_null())
			.set(|update| update.used_at, Some(now))
			.run(db)
			.await?;

		if claimed == 0 {
			return Ok(None);
		}

		let Some(sealed) = row.state else {
			return Ok(None);
		};
		let context = ceremony_context(purpose, subject, &row.secret_hash);
		Ok(Some(self.inner.keys.open(&context, &sealed)?))
	}
}

/// Deletes a passkey row.
async fn passkey_row_delete(connection: &impl Connection, id: Id<Passkey>) -> Result<()> {
	Passkey::query()
		.filter(|row| row.id.eq(id))
		.delete(connection)
		.await?;
	Ok(())
}

/// What a ceremony's sealed state is bound to: its purpose, subject, and
/// row, so it opens nowhere else.
pub(crate) fn ceremony_context<'a>(
	purpose: Purpose,
	subject: &'a str,
	hash: &'a [u8],
) -> [&'a [u8]; 4] {
	let purpose: &'static [u8] = match purpose {
		Purpose::PasskeyRegistration => b"passkey-registration",
		Purpose::PasskeyAuthentication => b"passkey-authentication",
		Purpose::Provider => b"provider",
		Purpose::ProviderSignUp => b"provider-sign-up",
		Purpose::Merge => b"merge",
		Purpose::WalletLink => b"wallet-link",
		Purpose::WalletProof => b"wallet-proof",
		Purpose::WalletSignIn => b"wallet-sign-in",
		Purpose::PasswordPause => b"password-pause",
		Purpose::EmailCode | Purpose::SecondFactor => b"code",
	};

	[b"ceremony", purpose, subject.as_bytes(), hash]
}

/// A passkey name as typed: trimmed, and at most 100 characters. `None`
/// when it's empty.
pub(crate) fn passkey_name(name: &str) -> Option<String> {
	let name = name.trim();
	(!name.is_empty()).then(|| name.chars().take(NAME_LENGTH).collect())
}

/// The subject of the passkey ceremony for the second step `challenge`.
pub(crate) fn second_factor_subject(challenge: Id<Challenge>) -> String {
	format!("second-factor:{challenge}")
}

/// The subject of a step-up passkey ceremony for `session`.
pub(crate) fn step_up_subject(session: Id<crate::Session>) -> String {
	format!("step-up:{session}")
}

/// The subject of every passkey sign-in ceremony; its token finds it.
pub(crate) const SIGN_IN_SUBJECT: &str = "sign-in";

/// What happened when a passkey ceremony was checked, for the second
/// step and step-up.
#[derive(Debug)]
pub(crate) enum Checked {
	/// The account's passkey signed it, and verified its user.
	Verified,
	/// It didn't check out.
	Refused,
}

#[cfg(feature = "passkey")]
pub(crate) use ceremonies::relying_party;

/// The WebAuthn ceremonies, with `webauthn-rs`.
#[cfg(feature = "passkey")]
mod ceremonies {
	use leptoskit_db::Connection;
	use leptoskit_db::Id;
	use leptoskit_db::Tx;
	use webauthn_rs::Webauthn;
	use webauthn_rs::WebauthnBuilder;
	use webauthn_rs::prelude::AuthenticationResult;
	use webauthn_rs::prelude::Credential;
	use webauthn_rs::prelude::DiscoverableAuthentication;
	use webauthn_rs::prelude::DiscoverableKey;
	use webauthn_rs::prelude::Passkey as Key;
	use webauthn_rs::prelude::PasskeyAuthentication;
	use webauthn_rs::prelude::PasskeyRegistration;
	use webauthn_rs::prelude::Url;
	use webauthn_rs_proto::ResidentKeyRequirement;

	use super::Checked;
	use super::PasskeyAdded;
	use super::PasskeyChallenge;
	use super::SIGN_IN_SUBJECT;
	use super::passkey_name;
	use super::second_factor_subject;
	use super::step_up_subject;
	use crate::Aal;
	use crate::Account;
	use crate::Auth;
	use crate::AuthError;
	use crate::Challenge;
	use crate::CreationChallengeResponse;
	use crate::EventKind;
	use crate::NewPasskey;
	use crate::Passkey;
	use crate::PasskeyInfo;
	use crate::PublicKeyCredential;
	use crate::Purpose;
	use crate::RecoveryCode;
	use crate::RegisterPublicKeyCredential;
	use crate::RequestChallengeResponse;
	use crate::RequestInfo;
	use crate::Result;
	use crate::SessionToken;
	use crate::SignIn;
	use crate::SignedIn;
	use crate::auth::finish;
	use crate::email::refuse_unusable;
	use crate::events::record;

	/// The relying party for `origin`: its host is the RP ID, so passkeys
	/// work on that host and its subdomains' pages, and nowhere else.
	pub(crate) fn relying_party(
		origin: &str,
		app_name: &str,
		native: &crate::NativeApps,
	) -> Result<Webauthn> {
		let refused = || {
			AuthError::Config(format!(
				"`[auth] origin` `{origin}` can't be a passkey relying party; use the site's \
				 own origin, such as https://myapp.com"
			))
		};
		let url = Url::parse(origin).map_err(|_| refused())?;
		let host = url.host_str().ok_or_else(refused)?.to_owned();
		let mut builder = WebauthnBuilder::new(&host, &url)
			.map_err(|_| refused())?
			.rp_name(app_name);

		// Apple's apps sign with the relying party's own `https` origin,
		// which differs from the site's only when it has a port.
		if native.apple_team_id.is_some() {
			let apple = Url::parse(&format!("https://{host}")).map_err(|_| refused())?;

			if apple != url {
				builder = builder.append_allowed_origin(&apple);
			}
		}

		for origin in native.android_origins() {
			let origin = Url::parse(&origin).map_err(|_| refused())?;
			builder = builder.append_allowed_origin(&origin);
		}

		builder.build().map_err(|_| refused())
	}

	impl Auth {
		fn webauthn(&self) -> Result<&Webauthn> {
			self.inner.webauthn.as_ref().ok_or_else(|| {
				AuthError::Config(
					"passkeys need `[auth] origin`, whose host is the relying party".into(),
				)
			})
		}

		/// Starts adding a passkey to the signed-in person's account: the
		/// options for `navigator.credentials.create`. It asks for a
		/// discoverable passkey that verifies its user, and refuses the
		/// account's existing ones.
		///
		/// It needs a fresh sign-in, as turning on an authenticator does.
		///
		/// # Errors
		///
		/// [`AuthError::Reauthenticate`] or [`AuthError::StepUpRequired`] when
		/// the session isn't fresh.
		pub async fn begin_passkey_registration(
			&self,
			me: &SignedIn,
		) -> Result<CreationChallengeResponse> {
			let db = &self.inner.db;
			self.require_fresh(db, me).await?;
			let existing = keys(db, me.account).await?;
			let exclude = existing
				.iter()
				.map(|(_, key)| key.cred_id().clone())
				.collect::<Vec<_>>();
			let label = crate::mfa::primary_email(db, me.account)
				.await?
				.unwrap_or_else(|| self.inner.app_name.clone());
			let (mut options, state) = self
				.webauthn()?
				.start_passkey_registration(
					me.account.uuid(),
					&label,
					&label,
					(!exclude.is_empty()).then_some(exclude),
				)
				.map_err(|_| AuthError::Config("a passkey couldn't be started".into()))?;
			// A discoverable passkey signs in without a username, from the
			// browser's autofill.
			if let Some(selection) = options.public_key.authenticator_selection.as_mut() {
				selection.resident_key = Some(ResidentKeyRequirement::Required);
				selection.require_resident_key = true;
			}

			let state = serde_json::to_vec(&state)
				.map_err(|_| AuthError::Config("a passkey couldn't be started".into()))?;
			self.store_ceremony(
				db,
				Purpose::PasskeyRegistration,
				&me.session.to_string(),
				&state,
			)
			.await?;
			Ok(options)
		}

		/// Adds the passkey the device made for
		/// [`Auth::begin_passkey_registration`]'s options. Like confirming an
		/// authenticator, the session moves to `aal2` with a new token, and
		/// an account without recovery codes gets ten, shown once.
		///
		/// # Errors
		///
		/// [`AuthError::PasskeyFailed`] when there's no ceremony to finish, or
		/// the device's answer doesn't check out, or the passkey is already
		/// someone's; or as [`Auth::begin_passkey_registration`].
		pub async fn finish_passkey_registration(
			&self,
			me: &SignedIn,
			credential: &RegisterPublicKeyCredential,
			name: Option<&str>,
			request: &RequestInfo,
		) -> Result<PasskeyAdded> {
			let db = &self.inner.db;
			self.require_fresh(db, me).await?;
			let state = self
				.take_ceremony(Purpose::PasskeyRegistration, &me.session.to_string(), None)
				.await?
				.ok_or(AuthError::PasskeyFailed)?;
			let state: PasskeyRegistration =
				serde_json::from_slice(&state).map_err(|_| AuthError::PasskeyFailed)?;
			let Ok(key) = self
				.webauthn()?
				.finish_passkey_registration(credential, &state)
			else {
				record(db, Some(me.account), EventKind::PasskeyFailed, request).await?;

				return Err(AuthError::PasskeyFailed);
			};
			let tx = db.begin().await?;
			let result = async {
				let had_codes = RecoveryCode::query()
					.filter(|code| code.account_id.eq(me.account) & code.used_at.is_null())
					.count(&tx)
					.await?
					> 0;
				let name = name
					.and_then(passkey_name)
					.unwrap_or_else(|| format!("Passkey on {}", request.device()));
				let row = NewPasskey {
					account_id: me.account,
					credential_id: key.cred_id().to_vec(),
					credential: encode(&key)?,
					name: passkey_name(&name).unwrap_or_else(|| "Passkey".to_owned()),
					backed_up: Credential::from(key.clone()).backup_state,
					last_used_at: None,
				}
				.insert(&tx)
				.await
				.map_err(|error| {
					match error {
						// The device made a passkey it already registered,
						// here or to someone else.
						leptoskit_db::DbError::Conflict { .. } => AuthError::PasskeyFailed,
						error => error.into(),
					}
				})?;
				let recovery_codes = if had_codes {
					None
				} else {
					Some(crate::mfa::replace_recovery_codes(self, &tx, me.account).await?)
				};

				let sign_in = self.elevate(&tx, me, request).await?;
				record(&tx, Some(me.account), EventKind::PasskeyAdded, request).await?;
				Ok(PasskeyAdded {
					passkey: PasskeyInfo::from(row),
					sign_in,
					recovery_codes,
				})
			}
			.await;
			let added = finish(tx, result).await?;
			self.notify(
				me.account,
				"A passkey was added",
				"A new passkey can now sign in to your account. If you didn't add it, sign in, \
				 remove it, and sign out everywhere.",
			)
			.await;
			Ok(added)
		}

		/// Starts signing in with a passkey, without a username: the device
		/// offers the person's passkeys for this site, in a dialog or in
		/// the username field's autofill. Keep the [`PasskeyChallenge`]'s
		/// token on the device until it answers.
		///
		/// # Errors
		///
		/// [`AuthError::RateLimited`] after too many starts from the IP.
		pub async fn begin_passkey_sign_in(
			&self,
			request: &RequestInfo,
		) -> Result<PasskeyChallenge> {
			self.limit_ceremony(request).await?;
			let (options, state) = self
				.webauthn()?
				.start_discoverable_authentication()
				.map_err(|_| AuthError::Config("a passkey couldn't be started".into()))?;
			let state = serde_json::to_vec(&state)
				.map_err(|_| AuthError::Config("a passkey couldn't be started".into()))?;
			let token = self
				.store_ceremony(
					&self.inner.db,
					Purpose::PasskeyAuthentication,
					SIGN_IN_SUBJECT,
					&state,
				)
				.await?;
			Ok(PasskeyChallenge { token, options })
		}

		/// Signs in with the passkey that answered
		/// [`Auth::begin_passkey_sign_in`]. A passkey that verifies its user is
		/// two factors in one, so the session starts at `aal2`.
		///
		/// # Errors
		///
		/// [`AuthError::PasskeyFailed`] when the ceremony is unknown, used, or
		/// expired, or the answer doesn't check out; [`AuthError::Disabled`]
		/// or [`AuthError::Locked`]; or [`AuthError::RateLimited`].
		pub async fn finish_passkey_sign_in(
			&self,
			token: &SessionToken,
			credential: &PublicKeyCredential,
			request: &RequestInfo,
		) -> Result<SignIn> {
			if let Some(ip) = request.ip_text() {
				self.limit(
					&[b"passkey-ip", ip.as_bytes()],
					self.inner.limits.verify_per_ip,
				)
				.await?;
			}

			let db = &self.inner.db;
			let state = self
				.take_ceremony(Purpose::PasskeyAuthentication, SIGN_IN_SUBJECT, Some(token))
				.await?
				.ok_or(AuthError::PasskeyFailed)?;
			let state: DiscoverableAuthentication =
				serde_json::from_slice(&state).map_err(|_| AuthError::PasskeyFailed)?;
			let webauthn = self.webauthn()?;
			let (user, credential_id) = webauthn
				.identify_discoverable_authentication(credential)
				.map_err(|_| AuthError::PasskeyFailed)?;
			let row = Passkey::query()
				.filter(|row| row.credential_id.eq(credential_id.to_vec()))
				.first(db)
				.await?
				.ok_or(AuthError::PasskeyFailed)?;

			// The device says whose passkey it is; it must be the account the
			// passkey was registered to.
			if row.account_id.uuid() != user {
				return Err(AuthError::PasskeyFailed);
			}

			refuse_unusable(&Account::get(db, row.account_id).await?)?;
			let key = decode(&row.credential)?;
			let Ok(result) = webauthn.finish_discoverable_authentication(
				credential,
				state,
				&[DiscoverableKey::from(&key)],
			) else {
				record(db, Some(row.account_id), EventKind::PasskeyFailed, request).await?;

				return Err(AuthError::PasskeyFailed);
			};
			let tx = db.begin().await?;
			let result = async {
				used(&tx, &row, key, &result).await?;
				let now = leptoskit_db::now();
				let sign_in = self
					.start_session(&tx, row.account_id, Aal::Two, now, Some(now), request)
					.await?;
				record(&tx, Some(row.account_id), EventKind::SignedIn, request).await?;
				Ok(sign_in)
			}
			.await;
			finish(tx, result).await
		}

		/// Asks the account's passkeys to sign the second step of signing in,
		/// which `token` began: the options for `navigator.credentials.get`.
		/// [`Auth::complete_sign_in_with_passkey`] finishes.
		///
		/// # Errors
		///
		/// [`AuthError::SignInExpired`] when the step is unknown, used, or
		/// expired, and [`AuthError::NotEnrolled`] when the account has no
		/// passkeys.
		pub async fn begin_passkey_second_factor(
			&self,
			token: &SessionToken,
			request: &RequestInfo,
		) -> Result<RequestChallengeResponse> {
			self.limit_ceremony(request).await?;
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
			self.passkey_assertion(db, account, &second_factor_subject(challenge.id))
				.await
		}

		/// Asks the signed-in person's passkeys to confirm it's them, for
		/// step-up: the options for `navigator.credentials.get`.
		/// [`Auth::step_up_with_passkey`] finishes.
		///
		/// # Errors
		///
		/// [`AuthError::NotEnrolled`] when the account has no passkeys.
		pub async fn begin_passkey_step_up(
			&self,
			me: &SignedIn,
			request: &RequestInfo,
		) -> Result<RequestChallengeResponse> {
			self.limit_ceremony(request).await?;
			self.passkey_assertion(&self.inner.db, me.account, &step_up_subject(me.session))
				.await
		}

		/// Starts a ceremony that only `account`'s passkeys can answer,
		/// bound to `subject`.
		pub(crate) async fn passkey_assertion(
			&self,
			connection: &impl Connection,
			account: Id<Account>,
			subject: &str,
		) -> Result<RequestChallengeResponse> {
			let keys = keys(connection, account).await?;

			if keys.is_empty() {
				return Err(AuthError::NotEnrolled);
			}

			let keys = keys.into_iter().map(|(_, key)| key).collect::<Vec<_>>();
			let (options, state) = self
				.webauthn()?
				.start_passkey_authentication(&keys)
				.map_err(|_| AuthError::Config("a passkey couldn't be started".into()))?;
			let state = serde_json::to_vec(&state)
				.map_err(|_| AuthError::Config("a passkey couldn't be started".into()))?;
			self.store_ceremony(connection, Purpose::PasskeyAuthentication, subject, &state)
				.await?;
			Ok(options)
		}

		/// Checks a passkey's answer to a ceremony, whose opened `state`
		/// [`Auth::take_ceremony`] returned, against `account`'s passkeys, and
		/// records the use.
		pub(crate) async fn check_passkey(
			&self,
			tx: &Tx,
			account: Id<Account>,
			state: &[u8],
			credential: &PublicKeyCredential,
		) -> Result<Checked> {
			let Ok(state) = serde_json::from_slice::<PasskeyAuthentication>(state) else {
				return Ok(Checked::Refused);
			};
			let Ok(result) = self
				.webauthn()?
				.finish_passkey_authentication(credential, &state)
			else {
				return Ok(Checked::Refused);
			};
			let credential_id = result.cred_id().to_vec();
			let row = Passkey::query()
				.filter(|row| row.account_id.eq(account) & row.credential_id.eq(credential_id))
				.first(tx)
				.await?;
			let Some(row) = row else {
				return Ok(Checked::Refused);
			};

			if !result.user_verified() {
				return Ok(Checked::Refused);
			}

			let key = decode(&row.credential)?;
			used(tx, &row, key, &result).await?;
			Ok(Checked::Verified)
		}

		async fn limit_ceremony(&self, request: &RequestInfo) -> Result<()> {
			match request.ip_text() {
				Some(ip) => {
					self.limit(
						&[b"ceremony-ip", ip.as_bytes()],
						self.inner.limits.ceremony_per_ip,
					)
					.await
				}
				None => Ok(()),
			}
		}
	}

	/// The account's passkeys, as `webauthn-rs` checks them.
	async fn keys(
		connection: &impl Connection,
		account: Id<Account>,
	) -> Result<Vec<(Passkey, Key)>> {
		let rows = Passkey::query()
			.filter(|row| row.account_id.eq(account))
			.all(connection)
			.await?;
		rows.into_iter()
			.map(|row| {
				let key = decode(&row.credential)?;
				Ok((row, key))
			})
			.collect()
	}

	/// Records that a passkey signed: when, and its new signature counter
	/// and backup state. `webauthn-rs` has already refused a counter that
	/// went backwards.
	async fn used(
		connection: &impl Connection,
		row: &Passkey,
		mut key: Key,
		result: &AuthenticationResult,
	) -> Result<()> {
		key.update_credential(result);
		Passkey::update()
			.filter(|update| update.id.eq(row.id))
			.set(|update| update.credential, encode(&key)?)
			.set(|update| update.backed_up, result.backup_state())
			.set(|update| update.last_used_at, Some(leptoskit_db::now()))
			.run(connection)
			.await?;
		Ok(())
	}

	fn encode(key: &Key) -> Result<Vec<u8>> {
		serde_json::to_vec(key)
			.map_err(|_| AuthError::Config("a passkey couldn't be stored".into()))
	}

	fn decode(bytes: &[u8]) -> Result<Key> {
		serde_json::from_slice(bytes)
			.map_err(|_| AuthError::Config("a stored passkey couldn't be read".into()))
	}
}

/// Without the `passkey` feature, every ceremony says the server can't do
/// it, so the routes and the client stay the same.
#[cfg(not(feature = "passkey"))]
mod unsupported {
	#![allow(
		clippy::unused_async,
		reason = "they stand in for the ceremonies with the feature"
	)]

	use leptoskit_db::Id;
	use leptoskit_db::Tx;

	use super::Checked;
	use super::PasskeyAdded;
	use super::PasskeyChallenge;
	use crate::Account;
	use crate::Auth;
	use crate::AuthError;
	use crate::CreationChallengeResponse;
	use crate::PublicKeyCredential;
	use crate::RegisterPublicKeyCredential;
	use crate::RequestChallengeResponse;
	use crate::RequestInfo;
	use crate::Result;
	use crate::SessionToken;
	use crate::SignIn;
	use crate::SignedIn;

	const OFF: AuthError = AuthError::Unsupported("this server doesn't have passkeys turned on");

	#[allow(
		missing_docs,
		clippy::unused_async,
		clippy::unused_async_trait_impl,
		reason = "the fallbacks share the enabled methods' async signatures"
	)]
	impl Auth {
		pub async fn begin_passkey_registration(
			&self,
			_me: &SignedIn,
		) -> Result<CreationChallengeResponse> {
			Err(OFF)
		}

		pub async fn finish_passkey_registration(
			&self,
			_me: &SignedIn,
			_credential: &RegisterPublicKeyCredential,
			_name: Option<&str>,
			_request: &RequestInfo,
		) -> Result<PasskeyAdded> {
			Err(OFF)
		}

		pub async fn begin_passkey_sign_in(
			&self,
			_request: &RequestInfo,
		) -> Result<PasskeyChallenge> {
			Err(OFF)
		}

		pub async fn finish_passkey_sign_in(
			&self,
			_token: &SessionToken,
			_credential: &PublicKeyCredential,
			_request: &RequestInfo,
		) -> Result<SignIn> {
			Err(OFF)
		}

		pub async fn begin_passkey_second_factor(
			&self,
			_token: &SessionToken,
			_request: &RequestInfo,
		) -> Result<RequestChallengeResponse> {
			Err(OFF)
		}

		pub async fn begin_passkey_step_up(
			&self,
			_me: &SignedIn,
			_request: &RequestInfo,
		) -> Result<RequestChallengeResponse> {
			Err(OFF)
		}

		pub(crate) async fn passkey_assertion(
			&self,
			_connection: &impl leptoskit_db::Connection,
			_account: Id<Account>,
			_subject: &str,
		) -> Result<RequestChallengeResponse> {
			Err(OFF)
		}

		pub(crate) async fn check_passkey(
			&self,
			_tx: &Tx,
			_account: Id<Account>,
			_state: &[u8],
			_credential: &PublicKeyCredential,
		) -> Result<Checked> {
			Err(OFF)
		}
	}
}
