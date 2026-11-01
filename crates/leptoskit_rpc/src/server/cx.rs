//! What an operation's body receives: the database as the caller, and who
//! the caller is.

use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::ReadEntry;
use leptoskit_db::Tracked;
use leptoskit_db::Tx;
use leptoskit_query::BoxFuture;

use super::Identity;
use super::OperationContext;
use super::sealed::Sealed;
use crate::Error;
use crate::Operation;

/// What an `#[rpc::query]` body receives.
///
/// [`db`](Self::db) reads as the caller: their policies apply to every row.
/// It only reads, so a query that tries to write fails to compile, and it
/// records what it reads: the read set a live query uses to tell which
/// writes could change its result. [`reads`](Self::reads) returns it, and a
/// [`ReadSink`] collects it from a query run by name.
pub struct QueryCx {
	db: Tracked<Db>,
	identity: Identity,
}

impl QueryCx {
	/// A context reading `db` as `identity`. `db` should already act for
	/// them, as one from `Db::with_context(identity.context)` does.
	pub fn new(db: Db, identity: Identity) -> Self {
		Self {
			db: Tracked::new(db),
			identity,
		}
	}

	/// The database, read-only, as the caller.
	pub fn db(&self) -> &Tracked<Db> {
		&self.db
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

	/// The caller's global of type `G`, or [`Error::Unauthorized`] when
	/// there is none: `cx.require::<SignedIn>()?` for a query only signed-in
	/// people may run.
	///
	/// # Errors
	///
	/// [`Error::Unauthorized`] when the caller has no `G`.
	pub fn require<G: 'static>(&self) -> Result<&G, Error> {
		require(&self.identity.context)
	}

	/// Whether the caller is signed in: the request's context has an
	/// [`Actor`](leptoskit_db::Actor), which `leptoskit_auth` adds for a
	/// signed-in session.
	pub fn signed_in(&self) -> bool {
		self.identity.context.actor().is_some()
	}

	/// The signed-in person's key in the app's user table, or `None` for a
	/// caller who isn't signed in.
	pub fn user_id<U>(&self) -> Option<Id<U>> {
		self.identity.context.actor().map(|actor| actor.id())
	}

	/// The signed-in person's key in the app's user table.
	///
	/// # Errors
	///
	/// [`Error::Unauthorized`] when nobody is signed in.
	pub fn require_user<U>(&self) -> Result<Id<U>, Error> {
		self.user_id()
			.ok_or_else(|| Error::unauthorized("sign in to do this"))
	}

	/// The database with policies bypassed, for the RPC's own bookkeeping.
	/// Not for app code.
	#[doc(hidden)]
	pub fn system(&self, reason: &'static str) -> Db {
		self.db.connection().system(reason)
	}

	/// Every read the body has made so far, one entry per read: the table,
	/// the condition its rows met, its order and limit, and the keys it
	/// returned.
	pub fn reads(&self) -> Vec<ReadEntry> {
		self.db.reads()
	}

	/// The read set, once the body has finished.
	pub fn into_reads(self) -> Vec<ReadEntry> {
		self.db.into_reads()
	}
}

/// Where a query's read set goes once its body has run, when it runs by
/// name through an [`Invocation`](super::Invocation), as the live hub runs
/// its subscriptions:
///
/// ```ignore
/// let reads = ReadSink::new();
/// let value = (registration.handler)(Invocation { reads: Some(reads.clone()), .. }).await?;
/// let read_set = reads.take();
/// ```
///
/// Only queries record reads; a mutation leaves it empty.
#[derive(Clone, Debug, Default)]
pub struct ReadSink(Arc<Mutex<Vec<ReadEntry>>>);

impl ReadSink {
	/// An empty sink.
	pub fn new() -> Self {
		Self::default()
	}

	/// Everything recorded so far, emptying the sink.
	pub fn take(&self) -> Vec<ReadEntry> {
		std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
	}

	fn extend(&self, reads: Vec<ReadEntry>) {
		self.0
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.extend(reads);
	}
}

impl std::fmt::Debug for QueryCx {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("QueryCx")
			.field("identity", &self.identity)
			.finish_non_exhaustive()
	}
}

/// Work to run once a mutation has committed.
type AfterCommit = BoxFuture<'static, ()>;

/// What an `#[rpc::mutation]` body receives.
///
/// [`tx`](Self::tx) is one transaction, as the caller: `SERIALIZABLE` on
/// Postgres, and SQLite's single writer. When another transaction gets in
/// its way the whole body runs again, so it must not reach the outside world
/// itself; [`after_commit`](Self::after_commit) runs work once the
/// transaction has committed.
pub struct MutationCx {
	tx: Tx,
	identity: Identity,
	attempt: u32,
	after_commit: Mutex<Vec<AfterCommit>>,
}

impl MutationCx {
	pub(crate) fn new(tx: Tx, identity: Identity, attempt: u32) -> Self {
		Self {
			tx,
			identity,
			attempt,
			after_commit: Mutex::new(Vec::new()),
		}
	}

	/// The transaction, as the caller.
	pub fn tx(&self) -> &Tx {
		&self.tx
	}

	/// Who is calling.
	pub fn identity(&self) -> &Identity {
		&self.identity
	}

	/// What policies read.
	pub fn context(&self) -> &Context {
		&self.identity.context
	}

	/// Enqueues `job` in this mutation's transaction: it exists exactly when
	/// the mutation commits, and never when it rolls back.
	///
	/// For a job that should wait, enqueue it in
	/// [`after_commit`](Self::after_commit) with [`crate::enqueue`].
	///
	/// # Errors
	///
	/// When the job can't be written, such as a taken key.
	pub async fn enqueue<J: crate::Job>(
		&self,
		job: J,
	) -> crate::Result<Id<crate::server::jobs::JobRow>> {
		crate::server::jobs::insert(
			&self.tx,
			&job,
			self.identity.caller.as_deref().unwrap_or_default(),
			None,
			None,
			None,
			0,
			None,
		)
		.await
	}

	/// The caller's global of type `G`, such as the signed-in person.
	pub fn get<G: 'static>(&self) -> Option<&G> {
		self.identity.context.get()
	}

	/// The caller's global of type `G`, or [`Error::Unauthorized`] when
	/// there is none.
	///
	/// # Errors
	///
	/// [`Error::Unauthorized`] when the caller has no `G`.
	pub fn require<G: 'static>(&self) -> Result<&G, Error> {
		require(&self.identity.context)
	}

	/// Whether the caller is signed in: the request's context has an
	/// [`Actor`](leptoskit_db::Actor), which `leptoskit_auth` adds for a
	/// signed-in session.
	pub fn signed_in(&self) -> bool {
		self.identity.context.actor().is_some()
	}

	/// The signed-in person's key in the app's user table, or `None` for a
	/// caller who isn't signed in.
	pub fn user_id<U>(&self) -> Option<Id<U>> {
		self.identity.context.actor().map(|actor| actor.id())
	}

	/// The signed-in person's key in the app's user table, the common first
	/// line of a mutation:
	///
	/// ```ignore
	/// let author = cx.require_user::<User>()?;
	/// ```
	///
	/// # Errors
	///
	/// [`Error::Unauthorized`] when nobody is signed in.
	pub fn require_user<U>(&self) -> Result<Id<U>, Error> {
		self.user_id()
			.ok_or_else(|| Error::unauthorized("sign in to do this"))
	}

	/// Which run of the body this is: `0` the first time, `1` after the
	/// first retry, and so on.
	pub fn attempt(&self) -> u32 {
		self.attempt
	}

	/// Runs `work` once, after the transaction commits: sending an email,
	/// telling another service. Nothing runs if the mutation fails or rolls
	/// back, and work queued by a run that was retried is dropped with it.
	/// It runs in the background, best effort: the caller's answer doesn't
	/// wait for it, and a failure inside it is the work's own to report.
	pub fn after_commit(&self, work: impl Future<Output = ()> + Send + 'static) {
		self.after_commit
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.push(Box::pin(work));
	}

	/// The transaction, and the work to run once it commits.
	pub(crate) fn into_parts(self) -> (Tx, Vec<AfterCommit>) {
		(
			self.tx,
			self.after_commit
				.into_inner()
				.unwrap_or_else(PoisonError::into_inner),
		)
	}
}

impl std::fmt::Debug for MutationCx {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("MutationCx")
			.field("identity", &self.identity)
			.field("attempt", &self.attempt)
			.finish_non_exhaustive()
	}
}

fn require<G: 'static>(context: &Context) -> Result<&G, Error> {
	context
		.get()
		.ok_or_else(|| Error::unauthorized("sign in to do this"))
}

impl Sealed for QueryCx {}
impl Sealed for MutationCx {}

impl OperationContext for crate::JobCx {
	fn dispatch<O: Operation<Context = Self>>(
		db: &Db,
		identity: Identity,
		_request_id: Option<&str>,
		op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send {
		let cx = crate::JobCx::new(db.clone(), identity, String::new(), 0);
		async move { op.run(&cx).await }
	}
}

impl OperationContext for QueryCx {
	fn dispatch<O: Operation<Context = Self>>(
		db: &Db,
		identity: Identity,
		request_id: Option<&str>,
		op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send {
		Self::dispatch_recording(db, identity, request_id, None, op)
	}

	async fn dispatch_recording<O: Operation<Context = Self>>(
		db: &Db,
		identity: Identity,
		_request_id: Option<&str>,
		reads: Option<ReadSink>,
		op: O,
	) -> crate::Result<O::Output> {
		let cx = Self::new(db.clone(), identity);
		let output = op.run(&cx).await;

		if let Some(reads) = reads {
			reads.extend(cx.into_reads());
		}

		output
	}
}

impl OperationContext for MutationCx {
	async fn dispatch<O: Operation<Context = Self>>(
		db: &Db,
		identity: Identity,
		request_id: Option<&str>,
		op: O,
	) -> crate::Result<O::Output> {
		super::run::mutate(db, identity, request_id, op).await
	}
}
