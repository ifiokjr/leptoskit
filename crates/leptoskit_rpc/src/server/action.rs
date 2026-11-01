//! What an `#[rpc::action]` body receives: who is calling, and the queries
//! and mutations it reaches the database through.

use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;

use super::Identity;
use super::MutationCx;
use super::OperationContext;
use super::QueryCx;
use super::sealed::Sealed;
use crate::Error;
use crate::Job;
use crate::Operation;
use crate::Result;
use crate::inbox::Notice;
use crate::server::inbox::Recipient;
use crate::server::jobs::Enqueue;

/// What an `#[rpc::action]` body receives.
///
/// An action isn't a transaction and never runs again by itself, so it may
/// call anything: a model, a payment provider, a slow API. It reaches the
/// database only through [`query`](Self::query) and
/// [`mutate`](Self::mutate), which run as the caller, so their policies and
/// transactions still apply.
///
/// ```ignore
/// #[rpc::action]
/// pub async fn summarize_channel(cx: &rpc::ActionCx, channel: Id<Channel>) -> rpc::Result<String> {
/// 	let messages = cx.query(Messages { channel, limit: 200 }).await?;
/// 	let summary = my_llm::summarize(&messages).await?;
/// 	cx.mutate(PinSummary { channel, summary: summary.clone() }).await?;
/// 	Ok(summary)
/// }
/// ```
pub struct ActionCx {
	db: Db,
	identity: Identity,
}

impl Sealed for ActionCx {}

impl ActionCx {
	/// A context acting for `identity` on `db`, which should already act for
	/// them.
	pub fn new(db: Db, identity: Identity) -> Self {
		Self { db, identity }
	}

	/// Who is calling.
	pub fn identity(&self) -> &Identity {
		&self.identity
	}

	/// What policies read.
	pub fn context(&self) -> &Context {
		&self.identity.context
	}

	/// The caller's global of type `G`, such as the signed-in person.
	pub fn get<G: 'static>(&self) -> Option<&G> {
		self.identity.context.get()
	}

	/// Whether the caller is signed in.
	pub fn signed_in(&self) -> bool {
		self.identity.context.actor().is_some()
	}

	/// The signed-in person's key in the app's user table.
	pub fn user_id<U>(&self) -> Option<Id<U>> {
		self.identity.context.actor().map(|actor| actor.id())
	}

	/// The signed-in person's key in the app's user table.
	///
	/// # Errors
	///
	/// [`Error::Unauthorized`] when nobody is signed in.
	pub fn require_user<U>(&self) -> Result<Id<U>> {
		self.user_id()
			.ok_or_else(|| Error::unauthorized("sign in to do this"))
	}

	/// Runs a query as the caller.
	///
	/// # Errors
	///
	/// What the query fails with.
	pub async fn query<Q: Operation<Context = QueryCx>>(&self, op: Q) -> Result<Q::Output> {
		QueryCx::dispatch(&self.db, self.identity.clone(), None, op).await
	}

	/// Runs a mutation as the caller, in its own transaction with its
	/// retries.
	///
	/// # Errors
	///
	/// What the mutation fails with.
	pub async fn mutate<M: Operation<Context = MutationCx>>(&self, op: M) -> Result<M::Output> {
		super::run::mutate(&self.db, self.identity.clone(), None, op).await
	}

	/// Enqueues a job owned by the caller.
	pub fn enqueue<J: Job>(&self, job: J) -> Enqueue<'static, J> {
		Enqueue::on(self.db.clone(), job, self.identity.context.actor())
	}

	/// Puts a notice in `account`'s inbox, and hands it to the app's
	/// delivery channels.
	///
	/// # Errors
	///
	/// When the notice can't be written.
	pub async fn notify(&self, account: impl Recipient, notice: Notice) -> Result<Option<String>> {
		let Some(delivery) = super::inbox::insert(&self.db, &account, notice).await? else {
			return Ok(None);
		};
		let id = delivery.id.clone();
		super::inbox::deliver(delivery).await;
		Ok(Some(id))
	}
}

impl std::fmt::Debug for ActionCx {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ActionCx")
			.field("identity", &self.identity)
			.finish_non_exhaustive()
	}
}

impl OperationContext for ActionCx {
	async fn dispatch<O: Operation<Context = Self>>(
		db: &Db,
		identity: Identity,
		_request_id: Option<&str>,
		op: O,
	) -> Result<O::Output> {
		let cx = Self::new(db.clone(), identity);
		op.run(&cx).await
	}
}
