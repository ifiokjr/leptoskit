//! Running an operation: decoding its arguments, the mutation's
//! transaction, retries and receipt, and calling one from server code.

use std::hash::BuildHasher;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_query::BoxFuture;
use serde_json::Value;

use super::Identity;
use super::MutationCx;
use super::OperationContext;
use super::QueryCx;
use super::ReadSink;
use super::receipts::ReceiptKey;
use super::receipts::ReceiptOwner;
use crate::Error;
use crate::Operation;

/// One call to run: the arguments as JSON, who is calling, and the database
/// to run on, already acting for them.
pub struct Invocation {
	/// The database, acting for the caller.
	pub db: Db,
	/// Who is calling.
	pub identity: Identity,
	/// The call's request ID, such as its `Idempotency-Key`.
	pub request_id: Option<String>,
	/// The arguments, as one JSON object.
	pub arguments: Value,
	/// Where a query adds what it read, for the live hub; `None` to let
	/// the read set go.
	pub reads: Option<ReadSink>,
}

/// What a registration runs.
pub(crate) type Handler = fn(Invocation) -> BoxFuture<'static, crate::Result<Value>>;

/// Decodes the arguments of `O`, runs it, and encodes its result: the
/// handler `#[rpc::query]` and `#[rpc::mutation]` register.
#[doc(hidden)]
pub fn handle<O: Operation>(invocation: Invocation) -> BoxFuture<'static, crate::Result<Value>> {
	Box::pin(async move {
		let op: O = serde_json::from_value(invocation.arguments).map_err(|error| {
			Error::invalid(format!("the arguments of `{}` don't fit: {error}", O::NAME))
		})?;
		let output = O::Context::dispatch_recording(
			&invocation.db,
			invocation.identity,
			invocation.request_id.as_deref(),
			invocation.reads,
			op,
		)
		.await?;
		serde_json::to_value(output)
			.map_err(|_| Error::server(format!("the result of `{}` isn't JSON", O::NAME)))
	})
}

/// Somewhere server code can run an operation of a kind whose body takes
/// `C`: a [`Db`] runs any, a [`QueryCx`] runs queries, and a [`MutationCx`]
/// runs mutations.
pub trait Target<C> {
	/// Runs `op` here.
	fn invoke<O: Operation<Context = C>>(
		&self,
		op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send;
}

impl<C: OperationContext> Target<C> for Db {
	fn invoke<O: Operation<Context = C>>(
		&self,
		op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send {
		C::dispatch(self, Identity::of(self), None, op)
	}
}

/// A query inside a query reads through the same connection, so its reads
/// join the caller's read set.
impl Target<QueryCx> for QueryCx {
	fn invoke<O: Operation<Context = QueryCx>>(
		&self,
		op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send {
		op.run(self)
	}
}

/// A mutation inside a mutation runs in the caller's transaction: it commits
/// with it, or not at all.
impl Target<MutationCx> for MutationCx {
	fn invoke<O: Operation<Context = MutationCx>>(
		&self,
		op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send {
		op.run(self)
	}
}

/// Runs an operation from server code: a server function, a job, a test, or
/// another operation.
///
/// ```ignore
/// // As whoever `db` acts for: a mutation gets its transaction and retries.
/// let id = rpc::call(&db, SendMessage { channel, body }).await?;
/// // Inside a mutation: in its transaction.
/// rpc::call(cx, MarkRead { channel }).await?;
/// ```
///
/// # Errors
///
/// Whatever the operation fails with.
pub async fn call<O, T>(target: &T, op: O) -> crate::Result<O::Output>
where
	O: Operation,
	T: Target<O::Context>,
{
	target.invoke(op).await
}

/// Runs an operation as `identity` on `db`, which this acts for, with an
/// optional request ID that makes a mutation run once however often it is
/// called: what the HTTP routes and server rendering do.
///
/// # Errors
///
/// Whatever the operation fails with, and [`Error::Conflict`] when the
/// request ID was used for another call.
pub async fn call_as<O: Operation>(
	db: &Db,
	identity: Identity,
	request_id: Option<&str>,
	op: O,
) -> crate::Result<O::Output> {
	let db = db.with_context(identity.context.clone());
	O::Context::dispatch(&db, identity, request_id, op).await
}

/// The first delay before a mutation runs again, and the longest.
const FIRST_RETRY: Duration = Duration::from_millis(10);
const LONGEST_RETRY: Duration = Duration::from_millis(500);

/// Runs a mutation: in a serializable transaction, again when another one
/// gets in its way, and once only for a request ID.
pub(crate) async fn mutate<M: Operation<Context = MutationCx>>(
	db: &Db,
	identity: Identity,
	request_id: Option<&str>,
	op: M,
) -> crate::Result<M::Output> {
	let receipt = request_id
		.map(|request_id| ReceiptKey::new(identity.caller.as_deref(), request_id, M::NAME, &op))
		.transpose()?;
	// The receipts' policy shows each caller their own.
	let db = match (&receipt, db.context()) {
		(Some(key), Some(context)) => {
			db.with_context(context.clone().with(ReceiptOwner(key.owner.clone())))
		}
		_ => db.clone(),
	};
	if let Some(key) = &receipt
		&& let Some(stored) = key.find(&db).await?
	{
		return decode::<M>(stored);
	}

	let mut attempt = 0;
	loop {
		let tx = db.begin_serializable().await?;
		let cx = MutationCx::new(tx, identity.clone(), attempt);
		let outcome = op.clone().run(&cx).await;
		let (tx, after_commit) = cx.into_parts();

		let failure = match outcome {
			Ok(output) => {
				match finish(tx, receipt.as_ref(), &output).await {
					Ok(()) => {
						for work in after_commit {
							tokio::spawn(work);
						}
						return Ok(output);
					}
					Err(Finish::Repeated) => {
						// Another call with this request ID committed first.
						let stored = match &receipt {
							Some(key) => key.find(&db).await?,
							None => None,
						};
						return match stored {
							Some(stored) => decode::<M>(stored),
							None => Err(Error::Contended),
						};
					}
					Err(Finish::Failed(error)) => error,
				}
			}
			Err(error) => {
				// The transaction may already be over; rolling back is
				// cleanup either way.
				let _ = tx.rollback().await;
				error
			}
		};

		if !failure.is_contended() {
			return Err(failure);
		}
		if attempt >= M::RETRIES {
			return Err(Error::Contended);
		}
		leptoskit_query::sleep(backoff(attempt)).await;
		attempt += 1;
	}
}

/// How committing a mutation's transaction went wrong.
enum Finish {
	/// A call with the same request ID committed first.
	Repeated,
	Failed(Error),
}

/// Writes the receipt, if any, and commits.
async fn finish(
	tx: leptoskit_db::Tx,
	receipt: Option<&ReceiptKey>,
	output: &impl serde::Serialize,
) -> Result<(), Finish> {
	if let Some(key) = receipt {
		let result = serde_json::to_value(output)
			.map_err(|_| Finish::Failed(Error::server("the result isn't JSON")))?;
		if let Err(error) = key.write(&tx, result).await {
			let _ = tx.rollback().await;
			return Err(if error.is_conflict() {
				Finish::Repeated
			} else {
				Finish::Failed(error.into())
			});
		}
	}
	tx.commit().await.map_err(|error| {
		if receipt.is_some() && error.is_conflict() {
			Finish::Repeated
		} else {
			Finish::Failed(error.into())
		}
	})
}

fn decode<M: Operation>(stored: Value) -> crate::Result<M::Output> {
	serde_json::from_value(stored).map_err(|_| {
		Error::conflict(format!(
			"the stored result of this call no longer fits `{}`; use a new idempotency key",
			M::NAME
		))
	})
}

/// How long to wait before run `attempt + 1`: doubling from 10 ms up to half
/// a second, each wait a random part of that, so two mutations that keep
/// meeting drift apart.
fn backoff(attempt: u32) -> Duration {
	let ceiling = FIRST_RETRY
		.saturating_mul(1 << attempt.min(16))
		.min(LONGEST_RETRY);
	let random = std::collections::hash_map::RandomState::new().hash_one(attempt);
	let millis = u64::try_from(ceiling.as_millis())
		.unwrap_or(u64::MAX)
		.max(1);
	Duration::from_millis(millis / 2 + random % (millis / 2 + 1))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn backoff_grows_with_jitter_up_to_a_ceiling() {
		for attempt in 0..20 {
			let wait = backoff(attempt);
			let ceiling = FIRST_RETRY
				.saturating_mul(1 << attempt.min(16))
				.min(LONGEST_RETRY);
			assert!(wait <= ceiling, "{wait:?} > {ceiling:?}");
			assert!(wait >= ceiling / 2, "{wait:?} < half of {ceiling:?}");
		}
	}
}
