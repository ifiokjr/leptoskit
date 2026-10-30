//! The `rpc` schema pack: `rpc.receipts` on Postgres, `rpc_receipts` on
//! SQLite.
//!
//! A mutation called with a request ID, such as an HTTP `Idempotency-Key`,
//! writes a receipt in its own transaction: who called, the ID, a hash of the
//! arguments, and the result. A repeat of the call finds the receipt and
//! answers with the stored result instead of running again, so a client
//! whose answer was lost on a flaky network can safely send the call again.
//!
//! The tables join the app's schema, and its migrations, like the `auth`
//! pack's. Add them to the crate that declares your tables with
//! `leptoskit::db::packs!(leptoskit_rpc)`.

use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;

/// The longest request ID a receipt keeps.
pub const MAX_REQUEST_ID_LEN: usize = 255;

/// One mutation call that ran with a request ID, and what it returned.
///
/// Each caller sees only their own receipts: the policy compares
/// [`owner`](Self::owner) with the [`ReceiptOwner`] the RPC adds to the
/// context it runs a mutation in.
#[table(
	name = "receipts",
	pack = "rpc",
	unique(owner, request_id),
	index(created_at)
)]
#[policy(select, insert, |receipt, cx| cx.global(|owner: &ReceiptOwner| receipt.owner.eq(owner.0.clone())))]
pub struct Receipt {
	pub id: Id<Self>,
	/// Who called: their [`Identity::caller`](super::Identity::caller), such
	/// as a session ID, or empty for anonymous callers.
	#[db(max_len = 255)]
	pub owner: String,
	/// The ID the call came with.
	#[db(max_len = 255)]
	pub request_id: String,
	/// The operation's name.
	#[db(max_len = 255)]
	pub operation: String,
	/// A SHA-256 of the operation's name and arguments, in hex: a repeat
	/// with the same request ID and other arguments is refused rather than
	/// answered with another call's result.
	#[db(max_len = 64)]
	pub arguments: String,
	/// What the call returned, as JSON.
	pub result: Json<serde_json::Value>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Whose receipts a mutation may read and write: the caller's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptOwner(pub String);

#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use running::ReceiptKey;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use running::prune;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod running {
	use std::fmt::Write;

	use leptoskit_db::Db;
	use leptoskit_db::DbError;
	use leptoskit_db::Json;
	use leptoskit_db::Read;
	use leptoskit_db::Tx;
	use sha2::Digest;
	use sha2::Sha256;

	use super::MAX_REQUEST_ID_LEN;
	use super::NewReceipt;
	use super::Receipt;
	use crate::Error;

	/// Which call a receipt is for.
	pub(crate) struct ReceiptKey {
		pub owner: String,
		pub request_id: String,
		pub operation: &'static str,
		pub arguments: String,
	}

	impl ReceiptKey {
		/// The key for `operation` called by `owner` with `arguments`.
		pub(crate) fn new(
			owner: Option<&str>,
			request_id: &str,
			operation: &'static str,
			arguments: &impl serde::Serialize,
		) -> Result<Self, Error> {
			if request_id.is_empty() || request_id.len() > MAX_REQUEST_ID_LEN {
				return Err(Error::invalid(format!(
					"an idempotency key is 1 to {MAX_REQUEST_ID_LEN} characters long"
				)));
			}
			let encoded =
				serde_json::to_vec(arguments).map_err(|error| Error::invalid(error.to_string()))?;
			let mut hash = Sha256::new();
			hash.update(operation.as_bytes());
			hash.update([0]);
			hash.update(&encoded);
			let arguments =
				hash.finalize()
					.iter()
					.fold(String::with_capacity(64), |mut hex, byte| {
						let _ = write!(hex, "{byte:02x}");
						hex
					});
			Ok(Self {
				owner: owner.unwrap_or_default().to_owned(),
				request_id: request_id.to_owned(),
				operation,
				arguments,
			})
		}

		/// The stored result of this call, if it has run.
		///
		/// # Errors
		///
		/// [`Error::Conflict`] when the request ID was used for another
		/// operation or other arguments.
		pub(crate) async fn find(
			&self,
			db: &impl Read,
		) -> Result<Option<serde_json::Value>, Error> {
			let receipt = Receipt::query()
				.filter(|receipt| {
					receipt.owner.eq(self.owner.clone())
						& receipt.request_id.eq(self.request_id.clone())
				})
				.first(db)
				.await?;
			let Some(receipt) = receipt else {
				return Ok(None);
			};
			if receipt.operation != self.operation || receipt.arguments != self.arguments {
				return Err(Error::conflict(
					"this idempotency key was already used for a different call; use a new key \
					 for each call",
				));
			}
			Ok(Some(receipt.result.0))
		}

		/// Writes the receipt in the mutation's transaction.
		pub(crate) async fn write(
			&self,
			tx: &Tx,
			result: serde_json::Value,
		) -> Result<(), DbError> {
			NewReceipt {
				owner: self.owner.clone(),
				request_id: self.request_id.clone(),
				operation: self.operation.to_owned(),
				arguments: self.arguments.clone(),
				result: Json(result),
			}
			.insert(tx)
			.await
			.map(|_| ())
		}
	}

	/// Deletes receipts older than `age`, so repeats of calls that old run
	/// again. Run it now and then, such as daily, with a handle that
	/// bypasses policies: `prune(&db.system("prune receipts"), …)`.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn prune(db: &Db, age: std::time::Duration) -> Result<u64, DbError> {
		let age = chrono::Duration::from_std(age).unwrap_or(chrono::Duration::MAX);
		let cutoff = leptoskit_db::now()
			.checked_sub_signed(age)
			.unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);
		Receipt::query()
			.filter(|receipt| receipt.created_at.lt(cutoff))
			.delete(db)
			.await
	}
}
