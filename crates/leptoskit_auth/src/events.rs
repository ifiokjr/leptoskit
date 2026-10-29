//! The audit trail.

use leptoskit_db::Connection;
use leptoskit_db::Id;

use crate::Account;
use crate::EventKind;
use crate::NewEvent;
use crate::RequestInfo;
use crate::Result;

/// Records that `kind` happened to `account`, from `request`.
pub(crate) async fn record(
	connection: &impl Connection,
	account: Option<Id<Account>>,
	kind: EventKind,
	request: &RequestInfo,
) -> Result<()> {
	NewEvent {
		account_id: account,
		kind,
		ip: request.ip_text(),
		user_agent: request.user_agent_text(),
	}
	.insert(connection)
	.await?;
	Ok(())
}
