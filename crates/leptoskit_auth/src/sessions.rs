//! Sessions: starting them, recognizing their tokens, and ending them.

use chrono::TimeDelta;
use leptoskit_db::Connection;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;

use crate::Aal;
use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::EventKind;
use crate::NewSession;
use crate::RequestInfo;
use crate::Result;
use crate::Session;
use crate::SessionToken;
use crate::SignedIn;
use crate::auth::finish;
use crate::events::record;

/// How stale `last_seen_at` may get before a request refreshes it, so that
/// a busy session isn't a write on every request.
const SEEN_PRECISION: TimeDelta = TimeDelta::minutes(1);

/// A session that just started: the token for the device, and its row.
#[derive(Debug)]
pub struct SignIn {
	/// The token the device keeps and sends back.
	pub token: SessionToken,
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
}

impl Auth {
	/// Who `token` signs in, if its session is live: not revoked, not
	/// expired, and its account isn't disabled. Each use extends the idle
	/// lifetime, up to the absolute one.
	///
	/// # Errors
	///
	/// When the database fails. An unknown or dead token is `Ok(None)`.
	pub async fn resolve(&self, token: &SessionToken) -> Result<Option<SignedIn>> {
		let db = &self.inner.db;
		let now = leptoskit_db::now();
		let session = Session::query()
			.filter(|session| {
				session.token_hash.eq(token.hash())
					& session.revoked_at.is_null()
					& session.expires_at.gt(now)
					& session.idle_expires_at.gt(now)
			})
			.first(db)
			.await?;
		let Some(session) = session else {
			return Ok(None);
		};
		let account = Account::find(db, session.account_id).await?;

		if account.is_none_or(|account| account.disabled_at.is_some()) {
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

		Ok(Some(signed_in(&session)))
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
		let session = NewSession {
			account_id: account,
			token_hash: token.hash(),
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
			let ended = Session::update()
				.filter(|row| row.account_id.eq(account) & row.revoked_at.is_null())
				.set(|row| row.revoked_at, Some(now))
				.run(&tx)
				.await?;

			if ended > 0 {
				record(&tx, Some(account), EventKind::SessionRevoked, request).await?;
			}

			Ok(())
		}
		.await;

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
}
