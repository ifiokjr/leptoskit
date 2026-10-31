//! Sessions: starting them, recognizing their tokens, and ending them.

use chrono::TimeDelta;
use leptoskit_db::Connection;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;

use crate::Aal;
use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::Device;
use crate::EventKind;
use crate::NewSession;
use crate::PublicSession;
use crate::RequestInfo;
use crate::Result;
use crate::Session;
use crate::SessionToken;
use crate::SignedIn;
use crate::Tokens;
use crate::Transport;
use crate::auth::finish;
use crate::events::record;

/// How stale `last_seen_at` may get before a request refreshes it, so that
/// a busy session isn't a write on every request.
const SEEN_PRECISION: TimeDelta = TimeDelta::minutes(1);

/// How long a refresh token still works after it was traded, for an app
/// whose refresh response was lost on the way back. Presented again after
/// this, it means someone else holds a copy, and the session ends.
const REFRESH_GRACE: TimeDelta = TimeDelta::seconds(30);

/// A live session, as [`Auth::resolve_session`] found it.
pub(crate) struct Resolved {
	/// Who it signs in.
	pub signed_in: SignedIn,
	/// Whether its account must have a second factor, and the session
	/// hasn't used one, so it may only set one up.
	pub enrolling: bool,
}

/// A session that just started: the token for the device, and its row.
#[derive(Debug)]
pub struct SignIn {
	/// The token the device sends back: the cookie, or a native app's
	/// access token.
	pub token: SessionToken,
	/// A native app's refresh token.
	pub refresh: Option<SessionToken>,
	/// The session.
	pub session: Session,
	/// Whether the account was created by this sign-in.
	pub signed_up: bool,
}

impl SignIn {
	/// Who the new session signs in, as policies see them.
	pub fn signed_in(&self) -> SignedIn {
		signed_in(&self.session)
	}

	/// A native app's tokens, to send it; `None` for a browser's cookie.
	pub fn tokens(&self) -> Option<Tokens> {
		let refresh = self.refresh.as_ref()?;
		let expires_at = self.session.access_expires_at?;
		let left = (expires_at - leptoskit_db::now()).num_seconds();
		Some(Tokens {
			access_token: self.token.expose().to_owned(),
			refresh_token: refresh.expose().to_owned(),
			expires_in: u64::try_from(left).unwrap_or_default(),
		})
	}
}

impl Auth {
	/// Who `token` signs in, if its session is live: not revoked, not
	/// expired, and its account isn't disabled. A native app's access token
	/// also stops working after its own short lifetime. Each use extends the
	/// session's idle lifetime, up to the absolute one.
	///
	/// A session of an account that must have two-step verification, which
	/// hasn't used it, signs no one in here: it may only set it up.
	///
	/// # Errors
	///
	/// When the database fails. An unknown or dead token is `Ok(None)`.
	pub async fn resolve(&self, token: &SessionToken) -> Result<Option<SignedIn>> {
		Ok(self
			.resolve_session(token)
			.await?
			.filter(|resolved| !resolved.enrolling)
			.map(|resolved| resolved.signed_in))
	}

	/// The live session `token` belongs to, and whether it may only set up
	/// a second factor.
	pub(crate) async fn resolve_session(&self, token: &SessionToken) -> Result<Option<Resolved>> {
		let db = &self.inner.db;
		let now = leptoskit_db::now();
		let session = Session::query()
			.filter(|session| {
				session.token_hash.eq(token.hash())
					& session.revoked_at.is_null()
					& session.expires_at.gt(now)
					& session.idle_expires_at.gt(now)
					& (session.access_expires_at.is_null() | session.access_expires_at.gt(now))
			})
			.first(db)
			.await?;
		let Some(session) = session else {
			return Ok(None);
		};

		if !usable(db, session.account_id).await? {
			return Ok(None);
		}

		if now - session.last_seen_at >= SEEN_PRECISION {
			let idle_expires_at = (now + self.idle()?).min(session.expires_at);
			Session::update()
				.filter(|row| row.id.eq(session.id))
				.set(|row| row.last_seen_at, now)
				.set(|row| row.idle_expires_at, idle_expires_at)
				.run(db)
				.await?;
		}

		let enrolling = session.aal == Aal::One
			&& !self.inner.mfa.optional()
			&& self.mfa_required(db, session.account_id).await?;
		Ok(Some(Resolved {
			signed_in: signed_in(&session),
			enrolling,
		}))
	}

	/// Trades a native app's refresh token for new tokens. Each refresh token
	/// works once: the old access token stops working, and the refresh token
	/// becomes the session's previous one.
	///
	/// The previous refresh token still works for a few seconds, in case the
	/// app never got the response. After that, presenting it again means
	/// someone copied it, so the session ends and the event is recorded.
	/// Apps must refresh one request at a time; the second of two at once
	/// is refused.
	///
	/// # Errors
	///
	/// [`AuthError::SignedOut`] when the token is unknown, already used, or
	/// its session has ended.
	pub async fn refresh(&self, refresh: &SessionToken, request: &RequestInfo) -> Result<SignIn> {
		let tx = self.inner.db.begin().await?;
		let result = self.trade(&tx, &refresh.hash(), request).await;

		// A reused token's session ends whatever the caller hears, so commit
		// before refusing it.
		finish(tx, result).await?.ok_or(AuthError::SignedOut)
	}

	/// The inside of [`Auth::refresh`]: `Ok(None)` when the token was reused
	/// and its session has just ended.
	async fn trade(
		&self,
		tx: &Tx,
		presented: &[u8],
		request: &RequestInfo,
	) -> Result<Option<SignIn>> {
		let now = leptoskit_db::now();
		let session = Session::query()
			.filter(|session| {
				(session.refresh_hash.eq(presented.to_vec())
					| session.previous_refresh_hash.eq(presented.to_vec()))
					& session.revoked_at.is_null()
					& session.expires_at.gt(now)
					& session.idle_expires_at.gt(now)
			})
			.first(tx)
			.await?;
		let Some(session) = session else {
			return Err(AuthError::SignedOut);
		};

		if !usable(tx, session.account_id).await? {
			return Err(AuthError::SignedOut);
		}

		let current = session.refresh_hash.as_deref() == Some(presented);
		let within_grace = session
			.refreshed_at
			.is_some_and(|at| now - at < REFRESH_GRACE);

		if !current && !within_grace {
			end(tx, session.id).await?;
			record(
				tx,
				Some(session.account_id),
				EventKind::RefreshReused,
				request,
			)
			.await?;
			return Ok(None);
		}

		let access = SessionToken::generate()?;
		let refresh = SessionToken::generate()?;
		let mut update = Session::update()
			// Only if no other refresh got here first.
			.filter(|row| row.id.eq(session.id) & row.refresh_hash.eq(session.refresh_hash.clone()))
			.set(|row| row.token_hash, access.hash())
			.set(
				|row| row.access_expires_at,
				Some((now + self.access()?).min(session.expires_at)),
			)
			.set(|row| row.refresh_hash, Some(refresh.hash()))
			.set(|row| row.last_seen_at, now)
			.set(
				|row| row.idle_expires_at,
				(now + self.idle()?).min(session.expires_at),
			);

		// Within the grace, the token presented stays the previous one, so the
		// grace doesn't stretch; the replacement the app never got stops
		// working.
		if current {
			update = update
				.set(|row| row.previous_refresh_hash, Some(presented.to_vec()))
				.set(|row| row.refreshed_at, Some(now));
		}

		if update.run(tx).await? == 0 {
			return Err(AuthError::SignedOut);
		}

		Ok(Some(SignIn {
			token: access,
			refresh: Some(refresh),
			session: Session::get(tx, session.id).await?,
			signed_up: false,
		}))
	}

	/// Ends the session `token` belongs to, from its own device.
	///
	/// # Errors
	///
	/// When the database fails. An unknown token is already signed out.
	pub async fn sign_out(&self, token: &SessionToken, request: &RequestInfo) -> Result<()> {
		let db = &self.inner.db;
		let session = Session::query()
			.filter(|session| session.token_hash.eq(token.hash()) & session.revoked_at.is_null())
			.first(db)
			.await?;

		if let Some(session) = session {
			let tx = db.begin().await?;
			let result = async {
				end(&tx, session.id).await?;
				record(&tx, Some(session.account_id), EventKind::SignedOut, request).await
			}
			.await;
			finish(tx, result).await?;
		}

		Ok(())
	}

	/// The signed-in person's live sessions, one per device, most recently
	/// used first, with the one `me` came from marked current.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn devices(&self, me: &SignedIn) -> Result<Vec<Device>> {
		let now = leptoskit_db::now();
		let sessions = Session::query()
			.filter(|session| {
				session.account_id.eq(me.account)
					& session.revoked_at.is_null()
					& session.expires_at.gt(now)
					& session.idle_expires_at.gt(now)
			})
			.order_by(|session| session.last_seen_at.desc())
			.all(&self.inner.db)
			.await?;

		Ok(sessions
			.into_iter()
			.map(|session| {
				Device {
					current: session.id == me.session,
					session: PublicSession::from(session),
				}
			})
			.collect())
	}

	/// Ends one of the signed-in person's sessions, such as a lost phone's.
	/// Returns whether it was theirs and still live.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn revoke(
		&self,
		me: &SignedIn,
		session: Id<Session>,
		request: &RequestInfo,
	) -> Result<bool> {
		let tx = self.inner.db.begin().await?;
		let result = async {
			let ended = Session::update()
				.filter(|row| {
					row.id.eq(session) & row.account_id.eq(me.account) & row.revoked_at.is_null()
				})
				.set(|row| row.revoked_at, Some(leptoskit_db::now()))
				.run(&tx)
				.await?;

			if ended > 0 {
				record(&tx, Some(me.account), EventKind::SessionRevoked, request).await?;
			}

			Ok(ended > 0)
		}
		.await;

		finish(tx, result).await
	}

	/// Ends every session of the signed-in person's except the current one,
	/// and returns how many ended.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn revoke_others(&self, me: &SignedIn, request: &RequestInfo) -> Result<u64> {
		let tx = self.inner.db.begin().await?;
		let result = async {
			let ended = Session::update()
				.filter(|row| {
					row.account_id.eq(me.account) & row.id.ne(me.session) & row.revoked_at.is_null()
				})
				.set(|row| row.revoked_at, Some(leptoskit_db::now()))
				.run(&tx)
				.await?;

			if ended > 0 {
				record(&tx, Some(me.account), EventKind::SessionRevoked, request).await?;
			}

			Ok(ended)
		}
		.await;

		finish(tx, result).await
	}

	/// Replaces the current session's token with a new one, as every
	/// change in privilege should, so a token captured earlier stops
	/// working. The new session keeps when its owner signed in.
	///
	/// # Errors
	///
	/// [`AuthError::SignedOut`] when the session already ended.
	pub async fn rotate(&self, me: &SignedIn, request: &RequestInfo) -> Result<SignIn> {
		let tx = self.inner.db.begin().await?;
		let result = async {
			let ended = Session::update()
				.filter(|row| row.id.eq(me.session) & row.revoked_at.is_null())
				.set(|row| row.revoked_at, Some(leptoskit_db::now()))
				.run(&tx)
				.await?;

			if ended == 0 {
				return Err(AuthError::SignedOut);
			}

			self.start_session(
				&tx,
				me.account,
				me.aal,
				me.authenticated_at,
				me.mfa_at,
				request,
			)
			.await
		}
		.await;

		finish(tx, result).await
	}

	/// Starts a session for `account`, inside the transaction that signed
	/// it in.
	pub(crate) async fn start_session(
		&self,
		connection: &impl Connection,
		account: Id<Account>,
		aal: Aal,
		authenticated_at: Timestamp,
		mfa_at: Option<Timestamp>,
		request: &RequestInfo,
	) -> Result<SignIn> {
		let now = leptoskit_db::now();
		let token = SessionToken::generate()?;
		let expires_at = now + self.absolute()?;
		let (refresh, access_expires_at) = match request.transport {
			Transport::Cookie => (None, None),
			Transport::Bearer => {
				(
					Some(SessionToken::generate()?),
					Some((now + self.access()?).min(expires_at)),
				)
			}
		};
		let session = NewSession {
			account_id: account,
			token_hash: token.hash(),
			access_expires_at,
			refresh_hash: refresh.as_ref().map(SessionToken::hash),
			previous_refresh_hash: None,
			refreshed_at: None,
			aal,
			authenticated_at,
			mfa_at,
			last_seen_at: now,
			idle_expires_at: (now + self.idle()?).min(expires_at),
			expires_at,
			device: request.device(),
			revoked_at: None,
		}
		.insert(connection)
		.await?;

		Ok(SignIn {
			token,
			refresh,
			session,
			signed_up: false,
		})
	}

	/// Disables `account`: it can't sign in, and every session it has ends
	/// now. For moderators and administrators; check that the caller may.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn disable(&self, account: Id<Account>, request: &RequestInfo) -> Result<()> {
		let tx = self.inner.db.begin().await?;
		let result = async {
			let now = leptoskit_db::now();
			Account::update()
				.filter(|row| row.id.eq(account) & row.disabled_at.is_null())
				.set(|row| row.disabled_at, Some(now))
				.run(&tx)
				.await?;
			end_all(&tx, account, request).await?;
			Ok(())
		}
		.await;

		finish(tx, result).await
	}

	/// Ends every session `account` has, the current device's included, and
	/// returns how many ended: "sign out everywhere", or an administrator's
	/// answer to a stolen account.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn sign_out_everywhere(
		&self,
		account: Id<Account>,
		request: &RequestInfo,
	) -> Result<u64> {
		let tx = self.inner.db.begin().await?;
		let result = end_all(&tx, account, request).await;
		finish(tx, result).await
	}

	/// Lets a disabled `account` sign in again. Its old sessions stay ended.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn enable(&self, account: Id<Account>) -> Result<()> {
		Account::update()
			.filter(|row| row.id.eq(account))
			.set(|row| row.disabled_at, None)
			.run(&self.inner.db)
			.await?;
		Ok(())
	}

	fn idle(&self) -> Result<TimeDelta> {
		lifetime(self.inner.sessions.idle)
	}

	fn absolute(&self) -> Result<TimeDelta> {
		lifetime(self.inner.sessions.absolute)
	}

	pub(crate) fn access(&self) -> Result<TimeDelta> {
		lifetime(self.inner.sessions.access)
	}
}

/// Whether `account` may still be signed in: it exists, and isn't disabled.
async fn usable(connection: &impl Connection, account: Id<Account>) -> Result<bool> {
	let account = Account::find(connection, account).await?;
	Ok(account.is_some_and(|account| account.disabled_at.is_none()))
}

/// Ends every live session of `account`'s, and records it if there were
/// any.
pub(crate) async fn end_all(tx: &Tx, account: Id<Account>, request: &RequestInfo) -> Result<u64> {
	let ended = Session::update()
		.filter(|row| row.account_id.eq(account) & row.revoked_at.is_null())
		.set(|row| row.revoked_at, Some(leptoskit_db::now()))
		.run(tx)
		.await?;

	if ended > 0 {
		record(tx, Some(account), EventKind::SessionRevoked, request).await?;
	}

	Ok(ended)
}

/// Revokes a session.
async fn end(connection: &impl Connection, session: Id<Session>) -> Result<()> {
	Session::update()
		.filter(|row| row.id.eq(session))
		.set(|row| row.revoked_at, Some(leptoskit_db::now()))
		.run(connection)
		.await?;
	Ok(())
}

fn signed_in(session: &Session) -> SignedIn {
	SignedIn {
		account: session.account_id,
		session: session.id,
		aal: session.aal,
		authenticated_at: session.authenticated_at,
		mfa_at: session.mfa_at,
	}
}

pub(crate) fn lifetime(duration: std::time::Duration) -> Result<TimeDelta> {
	TimeDelta::from_std(duration).map_err(|_| AuthError::Config("a lifetime is too long".into()))
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
	use super::*;
	use crate::Keys;
	use crate::Mailbox;
	use crate::NewAccount;

	#[tokio::test]
	async fn using_a_session_extends_it_at_most_once_a_minute() {
		let db = leptoskit_db::testing::sqlite().await;
		let auth = Auth::builder(&db, Keys::single("test", [1; 32]))
			.mailer(Mailbox::new())
			.build()
			.unwrap();
		let system = db.system("test");
		let account = NewAccount {
			disabled_at: None,
			locked_until: None,
		}
		.insert(&system)
		.await
		.unwrap();
		let now = leptoskit_db::now();
		let sign_in = auth
			.start_session(
				&system,
				account.id,
				Aal::One,
				now,
				None,
				&RequestInfo::default(),
			)
			.await
			.unwrap();
		let an_hour_ago = now - TimeDelta::hours(1);
		Session::update()
			.filter(|row| row.id.eq(sign_in.session.id))
			.set(|row| row.last_seen_at, an_hour_ago)
			.set(|row| row.idle_expires_at, now + TimeDelta::minutes(5))
			.run(&system)
			.await
			.unwrap();

		auth.resolve(&sign_in.token).await.unwrap().unwrap();
		let seen = Session::get(&system, sign_in.session.id).await.unwrap();
		assert!(seen.last_seen_at >= now);
		assert!(seen.idle_expires_at > now + TimeDelta::days(13));

		// A minute hasn't passed, so the next use writes nothing.
		auth.resolve(&sign_in.token).await.unwrap().unwrap();
		let again = Session::get(&system, sign_in.session.id).await.unwrap();
		assert_eq!(again.last_seen_at, seen.last_seen_at);
	}

	/// A native app signed in to a fresh account, and its service.
	async fn native() -> (Auth, leptoskit_db::Db, SignIn) {
		let db = leptoskit_db::testing::sqlite().await;
		let auth = Auth::builder(&db, Keys::single("test", [1; 32]))
			.mailer(Mailbox::new())
			.build()
			.unwrap();
		let system = db.system("test");
		let account = NewAccount {
			disabled_at: None,
			locked_until: None,
		}
		.insert(&system)
		.await
		.unwrap();
		let request = RequestInfo {
			transport: Transport::Bearer,
			..RequestInfo::default()
		};
		let sign_in = auth
			.start_session(
				&system,
				account.id,
				Aal::One,
				leptoskit_db::now(),
				None,
				&request,
			)
			.await
			.unwrap();
		(auth, db, sign_in)
	}

	fn refresh_token(sign_in: &SignIn) -> &SessionToken {
		sign_in.refresh.as_ref().unwrap()
	}

	#[tokio::test]
	async fn an_expired_access_token_stops_working_until_the_app_refreshes() {
		let (auth, db, sign_in) = native().await;
		Session::update()
			.filter(|row| row.id.eq(sign_in.session.id))
			.set(
				|row| row.access_expires_at,
				Some(leptoskit_db::now() - TimeDelta::seconds(1)),
			)
			.run(&db.system("test"))
			.await
			.unwrap();

		assert_eq!(auth.resolve(&sign_in.token).await.unwrap(), None);
		let refreshed = auth
			.refresh(refresh_token(&sign_in), &RequestInfo::default())
			.await
			.unwrap();
		assert!(auth.resolve(&refreshed.token).await.unwrap().is_some());
		assert_eq!(refreshed.session.id, sign_in.session.id);
	}

	#[tokio::test]
	async fn a_refresh_token_reused_after_the_grace_ends_the_session() {
		let (auth, db, sign_in) = native().await;
		let request = RequestInfo::default();
		let stolen = refresh_token(&sign_in);
		let thief = auth.refresh(stolen, &request).await.unwrap();
		Session::update()
			.filter(|row| row.id.eq(sign_in.session.id))
			.set(
				|row| row.refreshed_at,
				Some(leptoskit_db::now() - REFRESH_GRACE),
			)
			.run(&db.system("test"))
			.await
			.unwrap();

		assert!(matches!(
			auth.refresh(stolen, &request).await,
			Err(AuthError::SignedOut)
		));
		assert_eq!(auth.resolve(&thief.token).await.unwrap(), None);
		assert!(matches!(
			auth.refresh(refresh_token(&thief), &request).await,
			Err(AuthError::SignedOut)
		));
		let session = Session::get(&db.system("test"), sign_in.session.id)
			.await
			.unwrap();
		assert!(session.revoked_at.is_some());
		let events = crate::Event::query().all(&db.system("test")).await.unwrap();
		assert_eq!(
			events.last().map(|event| event.kind),
			Some(EventKind::RefreshReused)
		);
	}

	#[tokio::test]
	async fn a_disabled_account_cant_refresh() {
		let (auth, db, sign_in) = native().await;
		// Disabled behind the service's back, so its sessions weren't ended.
		Account::update()
			.filter(|row| row.id.eq(sign_in.session.account_id))
			.set(|row| row.disabled_at, Some(leptoskit_db::now()))
			.run(&db.system("test"))
			.await
			.unwrap();

		assert!(matches!(
			auth.refresh(refresh_token(&sign_in), &RequestInfo::default())
				.await,
			Err(AuthError::SignedOut)
		));
	}

	#[tokio::test]
	async fn a_browser_session_has_no_tokens_to_refresh() {
		let (auth, db, native) = native().await;
		let browser = auth
			.start_session(
				&db.system("test"),
				native.session.account_id,
				Aal::One,
				leptoskit_db::now(),
				None,
				&RequestInfo::default(),
			)
			.await
			.unwrap();

		assert!(native.tokens().is_some());
		assert_eq!(browser.tokens(), None);
		assert_eq!(browser.session.access_expires_at, None);
		assert!(matches!(
			auth.refresh(&browser.token, &RequestInfo::default()).await,
			Err(AuthError::SignedOut)
		));
	}
}
