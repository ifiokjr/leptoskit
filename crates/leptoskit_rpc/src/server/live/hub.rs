//! The hub: one reader of the change log for every connection on this
//! server, the subscriptions it matches each commit against, and the
//! re-runs those commits cause.
//!
//! Subscriptions that would read the same rows share a *group*: the same
//! query, the same arguments, and the same caller. A group runs once per
//! commit batch whatever its member count, and its diff is computed once
//! and sent to every member.
//!
//! A round handles one batch of commits: every commit the log had to hand
//! when the round began. It finds the groups a commit could change through
//! an index from table to group, re-runs them, and only then, holding the
//! state's lock, applies every result at once: each connection gets one
//! [`Batch`] for the round, and [`Hub::applied`] moves to the round's last
//! version after every batch is queued. A group that joins while a round
//! runs is checked against the round's commits before it applies, so no
//! commit slips between a subscription's first run and its registration.
//!
//! Commits come from the database's change log, or, when the server has a
//! [`Relay`](crate::server::Relay), from the relay's stream (see
//! [`Source`]). When the database has read replicas, a round's re-runs read
//! from one that has replayed past every commit in the round, and from the
//! primary when none has within [`Hub::replica_wait`].

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::Weak;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::ReadEntry;
use leptoskit_db::TableName;
use leptoskit_db::live::Change;
use leptoskit_db::live::ChangeLog;
use leptoskit_db::live::LogEntry;
use serde_json::Value;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::sync::watch;

use super::Identity;
use super::run_query;
use crate::live::Patch;
use crate::live::Update;
use crate::live::WireError;
use crate::live::patch;

/// How many re-runs one round runs at once.
const RERUNS: usize = 16;

/// The most commits a resuming connection may have missed before it gets
/// every result again instead of a diff of the log.
const MAX_RESUME_COMMITS: usize = 10_000;

/// How long reading the log back for a resume may take before the
/// connection gets every result again instead.
const RESUME_READ: Duration = Duration::from_secs(5);

/// What one round changed for one connection, at one version.
#[derive(Debug)]
pub(super) struct Batch {
	/// The version every result in it reflects.
	pub to: u64,
	/// Whether every subscription's whole value is here, because the log was
	/// pruned past the hub.
	pub full: bool,
	/// The connection must validate its identity before sending these values.
	pub revalidate: bool,
	/// Each changed subscription, by the connection's id for it.
	pub changes: Vec<(u32, Changed)>,
}

/// How one subscription's result changed.
#[derive(Clone, Debug)]
pub(super) enum Changed {
	/// The whole new value.
	Whole(Arc<Value>),
	/// A patch from the value the member had to `.1`.
	Patch(Arc<Patch>, Arc<Value>),
	/// The re-run failed; the member keeps its value.
	Failed(WireError),
}

impl Changed {
	/// The wire update, and the member's value after it, if it has one.
	pub(super) fn into_update(self) -> (Update, Option<Arc<Value>>) {
		match self {
			Self::Whole(value) => (Update::Ok((*value).clone()), Some(value)),
			Self::Patch(patch, value) => (Update::Patch((*patch).clone()), Some(value)),
			Self::Failed(error) => (Update::Err(error), None),
		}
	}
}

/// Which subscriptions may share one re-run.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum ShareKey {
	/// The same query, arguments and caller: the database runs the same
	/// statements for each of them, so they get the same result.
	Shared {
		query: String,
		args: String,
		who: Who,
		/// The adapter's revision of policy-relevant identity context.
		revision: Option<String>,
	},
	/// A caller the hub can't vouch for, such as one with globals but no
	/// caller ID: its subscription runs alone.
	Alone { connection: u32, id: u32 },
}

/// Who a shared subscription reads as.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Who {
	/// A caller, such as a session, and the actor its context names. Every
	/// request a caller makes gets the same context from the app's
	/// [`Identify`](crate::server::Identify), as its receipts already rely
	/// on.
	Caller(String, Option<leptoskit_db::Actor>),
	/// Nobody: no caller, and a context with no globals.
	Anonymous,
}

impl ShareKey {
	/// The key for `connection`'s subscription `id` to `query`, as
	/// `identity`. Conservative: anything the hub can't be sure of runs
	/// alone.
	pub(super) fn new(
		query: &str,
		args: &Value,
		identity: &Identity,
		connection: u32,
		id: u32,
	) -> Self {
		let who = match &identity.caller {
			Some(caller) => Who::Caller(caller.clone(), identity.context.actor()),
			None if identity.context.is_empty() => Who::Anonymous,
			None => return Self::Alone { connection, id },
		};

		match serde_json::to_string(args) {
			Ok(args) => {
				Self::Shared {
					query: query.to_owned(),
					args,
					who,
					revision: None,
				}
			}
			Err(_) => Self::Alone { connection, id },
		}
	}

	/// Keeps distinct policy revisions out of the same shared group.
	pub(super) fn with_revision(mut self, key: Option<String>) -> Self {
		if let Self::Shared { revision, .. } = &mut self {
			*revision = key.filter(|key| !key.is_empty());
		}
		self
	}
}

/// What a subscription's first run read, for [`Hub::register`].
pub(super) struct Registration {
	pub key: ShareKey,
	pub query: String,
	pub args: Value,
	pub identity: Identity,
	pub db: Db,
	pub result: Arc<Value>,
	pub reads: Vec<ReadEntry>,
}

pub(super) struct Hub {
	core: Arc<Core>,
	/// Where commits come from instead of the database's log.
	#[cfg(feature = "relay")]
	relay: std::sync::OnceLock<crate::server::Relay>,
	started: tokio::sync::OnceCell<()>,
	stop: Arc<Notify>,
	/// Wakes the reader to run a round without a commit, for groups that
	/// must run again.
	kick: Arc<Notify>,
}

struct Core {
	db: Db,
	state: Mutex<State>,
	applied: watch::Sender<u64>,
	reruns: AtomicU64,
	commits: AtomicU64,
	/// Re-runs that read from a replica.
	replica_reruns: AtomicU64,
	/// Log entries the relay couldn't bring, read from the database.
	relay_fills: Arc<AtomicU64>,
	/// Whether re-runs may read from replicas.
	use_replicas: std::sync::atomic::AtomicBool,
	/// How long a round waits for a replica to catch up, in microseconds.
	replica_wait: AtomicU64,
	/// A write-ahead log position every result the hub has sent may
	/// reflect: a replica behind it could show a client an older state than
	/// it already has, so re-runs read only from replicas past it.
	floor: std::sync::atomic::AtomicI64,
	/// Whether something has read the primary since `floor` was measured,
	/// so it has to be measured again.
	floor_stale: std::sync::atomic::AtomicBool,
}

/// How long a round waits for a replica to replay its commits before it
/// reads from the primary.
pub(super) const REPLICA_WAIT: Duration = Duration::from_millis(20);

#[derive(Default)]
struct State {
	/// The version every group's result is valid through.
	applied: u64,
	next_group: u64,
	groups: HashMap<u64, Group>,
	by_key: HashMap<ShareKey, u64>,
	/// The groups that read each table.
	by_table: HashMap<TableName, HashSet<u64>>,
	/// Groups that read nothing the hub could see: every commit re-runs
	/// them.
	everywhere: HashSet<u64>,
	/// Each `(connection, id)`'s group.
	members: HashMap<(u32, u32), u64>,
	/// Groups made since the round in progress looked for candidates.
	fresh: Vec<u64>,
	/// Groups to re-run whatever the commits, because a member registered
	/// a value the hub can't vouch for.
	forced: HashSet<u64>,
	connections: HashMap<u32, mpsc::UnboundedSender<Batch>>,
	/// Authentication dependencies, including connections with no queries.
	authentication: HashMap<u32, Vec<ReadEntry>>,
}

struct Group {
	key: ShareKey,
	query: String,
	args: Value,
	identity: Identity,
	db: Db,
	result: Arc<Value>,
	reads: Vec<ReadEntry>,
	tables: HashSet<TableName>,
	/// Each member and the value it has, when the hub knows it: a resumed
	/// subscription keeps the client's value, which the hub never saw.
	members: HashMap<(u32, u32), Option<Arc<Value>>>,
}

/// A group to re-run.
struct Job {
	group: u64,
	query: String,
	args: Value,
	identity: Identity,
	db: Db,
}

type Outcome = Result<(Value, Vec<ReadEntry>), WireError>;

impl Hub {
	/// A hub reading `db`, a system handle, once started.
	pub(super) fn new(db: Db) -> Self {
		Self {
			core: Arc::new(Core {
				db,
				state: Mutex::new(State::default()),
				applied: watch::channel(0).0,
				reruns: AtomicU64::new(0),
				commits: AtomicU64::new(0),
				replica_reruns: AtomicU64::new(0),
				relay_fills: Arc::new(AtomicU64::new(0)),
				use_replicas: std::sync::atomic::AtomicBool::new(true),
				replica_wait: AtomicU64::new(
					u64::try_from(REPLICA_WAIT.as_micros()).unwrap_or(u64::MAX),
				),
				floor: std::sync::atomic::AtomicI64::new(i64::MIN),
				floor_stale: std::sync::atomic::AtomicBool::new(true),
			}),
			#[cfg(feature = "relay")]
			relay: std::sync::OnceLock::new(),
			started: tokio::sync::OnceCell::new(),
			stop: Arc::new(Notify::new()),
			kick: Arc::new(Notify::new()),
		}
	}

	/// Takes commits from `relay` instead of the database's log, when it
	/// starts.
	#[cfg(feature = "relay")]
	pub(super) fn use_relay(&self, relay: crate::server::Relay) {
		let _ = self.relay.set(relay);
	}

	/// Whether re-runs may read from the database's replicas, and how long a
	/// round waits for one to catch up.
	pub(super) fn use_replicas(&self, enabled: bool, wait: Duration) {
		self.core.use_replicas.store(enabled, Ordering::Relaxed);
		self.core.replica_wait.store(
			u64::try_from(wait.as_micros()).unwrap_or(u64::MAX),
			Ordering::Relaxed,
		);
	}

	/// Whether the hub has started reading commits.
	pub(super) fn is_started(&self) -> bool {
		self.started.initialized()
	}

	/// Starts reading commits from the newest version, once.
	pub(super) async fn start(&self) -> Result<(), WireError> {
		self.started
			.get_or_try_init(|| {
				async {
					let unreadable = |error: &dyn std::fmt::Display| {
						WireError {
							status: 503,
							message: format!("the change log can't be read: {error}"),
						}
					};
					let source = self.source().await.map_err(|error| unreadable(&error))?;
					let version = u64::try_from(source.version()).unwrap_or(0);
					self.core.state().applied = version;
					self.core.applied.send_replace(version);
					tokio::spawn(read(
						Arc::downgrade(&self.core),
						source,
						Arc::clone(&self.stop),
						Arc::clone(&self.kick),
					));
					Ok(())
				}
			})
			.await
			.map(drop)
	}

	/// Where commits come from: the relay, or the database's log.
	async fn source(&self) -> Result<Source, Box<dyn std::error::Error + Send + Sync>> {
		#[cfg(feature = "relay")]
		if let Some(relay) = self.relay.get() {
			let version = self.core.db.live_version().await?;
			let feed = crate::server::relay::Feed::start(
				relay.clone(),
				self.core.db.clone(),
				version,
				Arc::clone(&self.core.relay_fills),
			)
			.await?;
			return Ok(Source::Relay(Box::new(feed)));
		}

		Ok(Source::Log(self.core.db.change_log(None).await?))
	}

	/// The version every result the hub holds is valid through, which it
	/// has queued every connection's changes up to.
	pub(super) fn applied(&self) -> u64 {
		*self.core.applied.borrow()
	}

	/// Follows [`applied`](Self::applied).
	pub(super) fn watch(&self) -> watch::Receiver<u64> {
		self.core.applied.subscribe()
	}

	/// Groups re-run, commits read, re-runs that read from a replica, and
	/// log entries the relay couldn't bring.
	pub(super) fn counts(&self) -> (u64, u64, u64, u64) {
		(
			self.core.reruns.load(Ordering::Relaxed),
			self.core.commits.load(Ordering::Relaxed),
			self.core.replica_reruns.load(Ordering::Relaxed),
			self.core.relay_fills.load(Ordering::Relaxed),
		)
	}

	/// Starts sending `connection`'s batches to `events`.
	pub(super) fn connect(&self, connection: u32, events: mpsc::UnboundedSender<Batch>) {
		self.core.state().connections.insert(connection, events);
	}

	/// Installs a connection's authentication read set. A round that landed
	/// during validation requires another check before any work is served.
	pub(super) fn watch_identity(
		&self,
		connection: u32,
		reads: Vec<ReadEntry>,
		since: u64,
	) -> bool {
		let mut state = self.core.state();
		if since != state.applied {
			return false;
		}
		state.authentication.insert(connection, reads);
		true
	}

	/// Forgets `connection` and its subscriptions `ids`.
	pub(super) fn disconnect(&self, connection: u32, ids: impl IntoIterator<Item = u32>) {
		let mut state = self.core.state();
		state.connections.remove(&connection);
		state.authentication.remove(&connection);

		for id in ids {
			state.leave(connection, id);
		}
	}

	/// Ends `connection`'s subscription `id`.
	pub(super) fn leave(&self, connection: u32, id: u32) {
		self.core.state().leave(connection, id);
	}

	/// The result and read set of the group `key` names, if there is one.
	pub(super) fn peek(&self, key: &ShareKey) -> Option<(Arc<Value>, Vec<ReadEntry>)> {
		let state = self.core.state();
		let group = state.groups.get(state.by_key.get(key)?)?;
		Some((Arc::clone(&group.result), group.reads.clone()))
	}

	/// Makes `connection`'s subscription `id` a member of the existing group
	/// `key`, and returns the group's result. `known` says whether the
	/// client will have that result; a member whose value the hub doesn't
	/// know gets the whole value at the group's next change.
	pub(super) fn join(
		&self,
		connection: u32,
		id: u32,
		key: &ShareKey,
		known: bool,
	) -> Option<Arc<Value>> {
		let mut state = self.core.state();
		let group_id = *state.by_key.get(key)?;
		state.leave(connection, id);
		let group = state.groups.get_mut(&group_id)?;
		let result = Arc::clone(&group.result);
		group
			.members
			.insert((connection, id), known.then(|| Arc::clone(&result)));
		state.members.insert((connection, id), group_id);
		Some(result)
	}

	/// Registers `connection`'s subscription `id`, whose first run saw the
	/// database at `since` or later. Returns `false`, registering nothing,
	/// when a round has applied since then: the run may have missed a
	/// commit the hub won't show it again, so run it again. With no
	/// `since`, it registers anyway, and the hub runs the group again at
	/// once. `known` says whether the client will have the result.
	pub(super) fn register(
		&self,
		connection: u32,
		id: u32,
		since: Option<u64>,
		known: bool,
		registration: Registration,
	) -> bool {
		let mut state = self.core.state();

		if since.is_some_and(|since| state.applied != since) {
			return false;
		}

		// Its first run read the primary, at a position the hub hasn't
		// measured.
		self.core.floor_stale.store(true, Ordering::Relaxed);

		state.leave(connection, id);
		let base = known.then(|| Arc::clone(&registration.result));

		let group_id = if let Some(&group_id) = state.by_key.get(&registration.key)
			&& let Some(group) = state.groups.get_mut(&group_id)
		{
			group.members.insert((connection, id), base);
			state.members.insert((connection, id), group_id);
			group_id
		} else {
			state.next_group += 1;
			let group_id = state.next_group;
			let tables = tables(&registration.reads);
			let mut members = HashMap::new();
			members.insert((connection, id), base);
			state.index(
				group_id,
				&HashSet::new(),
				&tables,
				registration.reads.is_empty(),
			);
			state.by_key.insert(registration.key.clone(), group_id);
			state.members.insert((connection, id), group_id);
			state.groups.insert(
				group_id,
				Group {
					key: registration.key,
					query: registration.query,
					args: registration.args,
					identity: registration.identity,
					db: registration.db,
					result: registration.result,
					reads: registration.reads,
					tables,
					members,
				},
			);
			state.fresh.push(group_id);
			group_id
		};

		if since.is_none() {
			state.forced.insert(group_id);
			drop(state);
			self.kick.notify_one();
		}

		true
	}

	/// Notes that the client now has `value` for `connection`'s
	/// subscription `id`, if that's still its group's result.
	pub(super) fn known(&self, connection: u32, id: u32, value: &Arc<Value>) {
		let mut state = self.core.state();
		let Some(&group_id) = state.members.get(&(connection, id)) else {
			return;
		};
		let Some(group) = state.groups.get_mut(&group_id) else {
			return;
		};

		if Arc::ptr_eq(&group.result, value)
			&& let Some(base @ None) = group.members.get_mut(&(connection, id))
		{
			*base = Some(Arc::clone(value));
		}
	}

	/// Every change committed after `from` up to `to`, or `None` when the
	/// log no longer reaches back that far, or holds too many to be worth
	/// reading: then a resuming client gets every result again.
	pub(super) async fn changes_between(&self, from: u64, to: u64) -> Option<Vec<Change>> {
		if from == to {
			return Some(Vec::new());
		}

		if from > to {
			return None;
		}

		let deadline = tokio::time::Instant::now() + RESUME_READ;
		let mut after = i64::try_from(from).ok()?;
		let to = i64::try_from(to).ok()?;
		let mut changes = Vec::new();
		let mut commits = 0;

		// Page by page, without listening, so a server reading commits from
		// a relay holds no connection open for it.
		while after < to {
			let page = tokio::time::timeout_at(deadline, self.core.db.change_log_after(after))
				.await
				.ok()?
				.ok()?;

			if page.is_empty() {
				return None;
			}

			for entry in page {
				match entry {
					LogEntry::Commit(commit) if commit.version <= to => {
						commits += 1;

						if commits > MAX_RESUME_COMMITS {
							return None;
						}

						after = commit.version;
						changes.extend(commit.changes);
					}
					LogEntry::Commit(_) => return Some(changes),
					LogEntry::Pruned { .. } => return None,
				}
			}
		}

		Some(changes)
	}

	/// The database's dialect, for matching changes against reads.
	pub(super) fn dialect(&self) -> Dialect {
		self.core.db.dialect()
	}
}

impl Drop for Hub {
	fn drop(&mut self) {
		self.stop.notify_one();
	}
}

impl Core {
	fn state(&self) -> MutexGuard<'_, State> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// Handles one batch of log entries: finds what they change, re-runs it,
	/// and applies every result at once.
	async fn round(&self, entries: Vec<LogEntry>) {
		let dialect = self.db.dialect();
		let mut to = self.state().applied;
		let mut full = false;
		let mut changes: Vec<Change> = Vec::new();
		// Where a replica must have replayed to for every commit here: the
		// furthest of their positions, or none when one has none.
		let mut fence = Some(i64::MIN);

		for entry in entries {
			match entry {
				LogEntry::Commit(commit) => {
					to = to.max(u64::try_from(commit.version).unwrap_or(0));
					fence = fence.zip(commit.lsn).map(|(a, b)| a.max(b));
					changes.extend(commit.changes);
				}
				LogEntry::Pruned { through } => {
					to = to.max(u64::try_from(through).unwrap_or(0));
					full = true;
					fence = None;
				}
			}
		}

		// A round with no commits, run for groups that must run again, has
		// nothing to wait for but reads the primary, as their first runs did.
		let fence = fence.filter(|fence| *fence != i64::MIN);
		let mut replica: Option<Option<Db>> = None;
		let mut first = true;
		let mut results: HashMap<u64, Outcome> = HashMap::new();

		loop {
			let jobs = {
				let mut state = self.state();
				let jobs = state.jobs(&changes, full, first, dialect, &results);
				first = false;

				if jobs.is_empty() {
					state.apply(results, to, full, &changes, dialect);
					self.applied.send_replace(to);
					return;
				}

				jobs
			};

			self.reruns.fetch_add(jobs.len() as u64, Ordering::Relaxed);

			// Groups that joined while the round ran read the primary when
			// they first ran, at a position the replica was never checked
			// against, so only the round's first pass reads a replica.
			let first_pass = replica.is_none();

			if first_pass {
				replica = Some(self.replica(fence).await);
			}

			let pass = if first_pass {
				replica.as_ref().and_then(Option::as_ref)
			} else {
				None
			};

			if pass.is_none() {
				self.floor_stale.store(true, Ordering::Relaxed);
			}

			let jobs: Vec<Job> = match pass {
				Some(replica) => {
					self.replica_reruns
						.fetch_add(jobs.len() as u64, Ordering::Relaxed);
					jobs.into_iter()
						.map(|job| {
							Job {
								db: job.db.reading_like(replica),
								..job
							}
						})
						.collect()
				}
				None => jobs,
			};
			let ran: Vec<(u64, Outcome)> = futures::stream::iter(jobs)
				.map(|job| {
					async move {
						(
							job.group,
							run_query(&job.query, job.args, job.db, job.identity).await,
						)
					}
				})
				.buffer_unordered(RERUNS)
				.collect()
				.await;
			results.extend(ran);

			// With several replicas, the next round may pick another, which
			// must not be behind what this one showed.
			if let Some(replica) = pass
				&& self.db.replica_count() > 1
				&& let Some(position) = replica.replica_position().await
			{
				self.floor.fetch_max(position, Ordering::Relaxed);
			}
		}
	}
}

impl Core {
	/// A replica that has replayed past `fence` and the hub's floor, if
	/// re-runs may use one and one gets there in time.
	async fn replica(&self, fence: Option<i64>) -> Option<Db> {
		let fence = fence?;

		if self.db.replica_count() == 0 || !self.use_replicas.load(Ordering::Relaxed) {
			return None;
		}

		// Something read the primary since the floor was measured: what the
		// primary has now is at least what that read saw.
		if self.floor_stale.swap(false, Ordering::Relaxed) {
			if let Ok(Some(position)) = self.db.wal_position().await {
				self.floor.fetch_max(position, Ordering::Relaxed);
			} else {
				self.floor_stale.store(true, Ordering::Relaxed);
				return None;
			}
		}

		let wait = Duration::from_micros(self.replica_wait.load(Ordering::Relaxed));
		let needed = fence.max(self.floor.load(Ordering::Relaxed));
		self.db.on_replica_at(needed, wait).await
	}
}

impl State {
	/// The groups to re-run for `changes`: on the round's first pass, the
	/// ones the index finds; after that, groups made while the round ran.
	fn jobs(
		&mut self,
		changes: &[Change],
		full: bool,
		first: bool,
		dialect: Dialect,
		done: &HashMap<u64, Outcome>,
	) -> Vec<Job> {
		let forced: Vec<u64> = self.forced.drain().collect();
		let candidates: Vec<u64> = if first {
			self.fresh.clear();

			if full {
				self.groups.keys().copied().collect()
			} else {
				let mut candidates: HashSet<u64> = self.everywhere.clone();

				for change in changes {
					if let Some(groups) = self.by_table.get(&change.table) {
						candidates.extend(groups);
					}
				}

				candidates.into_iter().collect()
			}
		} else {
			std::mem::take(&mut self.fresh)
		};

		let candidates: HashSet<u64> = candidates
			.into_iter()
			.chain(forced.iter().copied())
			.collect();

		candidates
			.into_iter()
			.filter(|group| forced.contains(group) || !done.contains_key(group))
			.filter_map(|id| {
				let group = self.groups.get(&id)?;
				let dirty = full
					|| forced.contains(&id)
					|| group.reads.is_empty()
					|| group.reads.iter().any(|read| {
						changes
							.iter()
							.any(|change| read.could_change(change, dialect))
					});

				dirty.then(|| {
					Job {
						group: id,
						query: group.query.clone(),
						args: group.args.clone(),
						identity: group.identity.clone(),
						db: group.db.clone(),
					}
				})
			})
			.collect()
	}

	/// Applies a round's results, queues each connection's batch, and moves
	/// to `to`.
	fn apply(
		&mut self,
		results: HashMap<u64, Outcome>,
		to: u64,
		full: bool,
		changes: &[Change],
		dialect: Dialect,
	) {
		let mut batches: HashMap<u32, Vec<(u32, Changed)>> = HashMap::new();
		let mut reindex = Vec::new();

		for (id, outcome) in results {
			let Some(group) = self.groups.get_mut(&id) else {
				continue;
			};

			match outcome {
				Ok((value, reads)) => {
					let new = Arc::new(value);
					let shared =
						(*new != *group.result).then(|| shared_change(&group.result, &new));

					for (&(connection, member), base) in &mut group.members {
						let change = if full {
							Some(Changed::Whole(Arc::clone(&new)))
						} else {
							match base {
								Some(base) if Arc::ptr_eq(base, &group.result) => shared.clone(),
								Some(base) if **base == *new => None,
								Some(base) => Some(shared_change(base, &new)),
								None => Some(Changed::Whole(Arc::clone(&new))),
							}
						};

						if let Some(change) = change {
							batches
								.entry(connection)
								.or_default()
								.push((member, change));
						}

						*base = Some(Arc::clone(&new));
					}

					group.result = new;
					let tables = tables(&reads);

					if tables != group.tables || reads.is_empty() != group.reads.is_empty() {
						reindex.push((
							id,
							std::mem::replace(&mut group.tables, tables.clone()),
							tables,
							reads.is_empty(),
						));
					}

					group.reads = reads;
				}
				Err(error) => {
					for &(connection, member) in group.members.keys() {
						batches
							.entry(connection)
							.or_default()
							.push((member, Changed::Failed(error.clone())));
					}
				}
			}
		}

		for (id, old, new, everywhere) in reindex {
			self.index(id, &old, &new, everywhere);
		}

		let revalidate: HashSet<u32> = self
			.authentication
			.iter()
			.filter(|(_, reads)| {
				!reads.is_empty()
					&& (full
						|| reads.iter().any(|read| {
							changes
								.iter()
								.any(|change| read.could_change(change, dialect))
						}))
			})
			.map(|(&connection, _)| connection)
			.collect();
		for connection in &revalidate {
			batches.entry(*connection).or_default();
		}

		for (connection, changes) in batches {
			if let Some(events) = self.connections.get(&connection) {
				let _ = events.send(Batch {
					to,
					full,
					changes,
					revalidate: revalidate.contains(&connection),
				});
			}
		}

		self.applied = to;
	}

	/// Moves group `id` in the index from the tables `old` to `new`.
	fn index(
		&mut self,
		id: u64,
		old: &HashSet<TableName>,
		new: &HashSet<TableName>,
		everywhere: bool,
	) {
		for table in old.difference(new) {
			if let Some(groups) = self.by_table.get_mut(table) {
				groups.remove(&id);

				if groups.is_empty() {
					self.by_table.remove(table);
				}
			}
		}

		for table in new {
			self.by_table.entry(table.clone()).or_default().insert(id);
		}

		if everywhere {
			self.everywhere.insert(id);
		} else {
			self.everywhere.remove(&id);
		}
	}

	/// Ends `connection`'s subscription `id`, and its group with its last
	/// member.
	fn leave(&mut self, connection: u32, id: u32) {
		let Some(group_id) = self.members.remove(&(connection, id)) else {
			return;
		};
		let Some(group) = self.groups.get_mut(&group_id) else {
			return;
		};
		group.members.remove(&(connection, id));

		if !group.members.is_empty() {
			return;
		}

		let Some(group) = self.groups.remove(&group_id) else {
			return;
		};
		self.by_key.remove(&group.key);
		self.index(group_id, &group.tables, &HashSet::new(), false);
	}
}

/// The tables a read set reads.
fn tables(reads: &[ReadEntry]) -> HashSet<TableName> {
	reads.iter().map(|read| read.table.clone()).collect()
}

/// The change from `old` to `new`, computed once for every member that had
/// `old`.
fn shared_change(old: &Value, new: &Arc<Value>) -> Changed {
	match patch(old, new) {
		Update::Patch(patch) => Changed::Patch(Arc::new(patch), Arc::clone(new)),
		_ => Changed::Whole(Arc::clone(new)),
	}
}

/// Where the hub's commits come from.
enum Source {
	/// The database's change log.
	Log(ChangeLog),
	/// The relay's stream, and the database's log for its gaps.
	#[cfg(feature = "relay")]
	Relay(Box<crate::server::relay::Feed>),
}

impl Source {
	fn version(&self) -> i64 {
		match self {
			Self::Log(log) => log.version(),
			#[cfg(feature = "relay")]
			Self::Relay(feed) => feed.version(),
		}
	}

	async fn next(&mut self) -> leptoskit_db::Result<LogEntry> {
		match self {
			Self::Log(log) => log.next().await,
			#[cfg(feature = "relay")]
			Self::Relay(feed) => feed.next().await,
		}
	}

	fn next_ready(&mut self) -> Option<leptoskit_db::Result<LogEntry>> {
		match self {
			Self::Log(log) => log.next_ready(),
			#[cfg(feature = "relay")]
			Self::Relay(feed) => feed.next_ready().map(Ok),
		}
	}
}

/// Reads commits until the hub is gone, handing each batch of them to a
/// round.
async fn read(core: Weak<Core>, mut log: Source, stop: Arc<Notify>, kick: Arc<Notify>) {
	loop {
		let next = tokio::select! {
			next = log.next() => Some(next),
			() = kick.notified() => None,
			() = stop.notified() => return,
		};
		let Some(core) = core.upgrade() else {
			return;
		};

		let mut entries = match next {
			// Groups to run again, with no commit.
			None => Vec::new(),
			Some(Ok(entry)) => vec![entry],
			Some(Err(_)) => {
				// The database is unreachable, or a commit doesn't decode;
				// the log reads on from the same place, or past it.
				drop(core);
				tokio::time::sleep(Duration::from_millis(500)).await;
				continue;
			}
		};

		// Whatever the same read of the log fetched goes in the same round.
		while let Some(Ok(entry)) = log.next_ready() {
			entries.push(entry);
		}

		core.commits
			.fetch_add(entries.len() as u64, Ordering::Relaxed);
		core.round(entries).await;
	}
}
