//! The audit trail.

use leptoskit_db::Connection;
use leptoskit_db::Id;

use crate::Account;
use crate::Auth;
use crate::EventKind;
use crate::NewEvent;
use crate::RequestInfo;
use crate::Result;

/// The longest `detail` kept, in characters.
const DETAIL_LENGTH: usize = 200;

/// Records that `kind` happened to `account`, from `request`.
pub(crate) async fn record(
	connection: &impl Connection,
	account: Option<Id<Account>>,
	kind: EventKind,
	request: &RequestInfo,
) -> Result<()> {
	record_with(connection, account, kind, None, request).await
}

async fn record_with(
	connection: &impl Connection,
	account: Option<Id<Account>>,
	kind: EventKind,
	detail: Option<&str>,
	request: &RequestInfo,
) -> Result<()> {
	NewEvent {
		account_id: account,
		kind,
		detail: detail.map(|detail| detail.chars().take(DETAIL_LENGTH).collect()),
		ip: request.ip_text(),
		user_agent: request.user_agent_text(),
	}
	.insert(connection)
	.await?;
	Ok(())
}

impl Auth {
	/// Adds to the audit trail: `kind` happened to `account`, from
	/// `request`, concerning `detail`, such as the name of the app that was
	/// connected. For crates that build on sign-in, such as `leptoskit_idp`,
	/// to record their own events in `auth.events` next to sign-in's.
	///
	/// Pass the transaction that made the change, so the event and the
	/// change land together. `detail` is cut to 200 characters; never put a
	/// credential in it.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn record(
		&self,
		connection: &impl Connection,
		account: Option<Id<Account>>,
		kind: EventKind,
		detail: Option<&str>,
		request: &RequestInfo,
	) -> Result<()> {
		record_with(connection, account, kind, detail, request).await
	}

	/// Counts an attempt at `action` from `source` against `limit`, through
	/// the same rate limiter sign-in uses, for crates that build on it, such
	/// as `leptoskit_idp` counting guesses at device codes. The two are
	/// hashed together, apart from sign-in's own counters, so neither is
	/// stored.
	///
	/// # Errors
	///
	/// [`AuthError::RateLimited`](crate::AuthError::RateLimited) when the
	/// limit is used up, or when the limiter fails.
	pub async fn limit_attempts(
		&self,
		action: &str,
		source: &str,
		limit: crate::Limit,
	) -> Result<()> {
		let key = crate::secrets::digest(&[b"extension", action.as_bytes(), source.as_bytes()]);
		self.inner.limiter.hit(&key, limit).await
	}
}
