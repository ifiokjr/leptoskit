//! Reports a native host to the hub from two threads of its own: one keeps
//! each guest's tree and sends what changed, the other waits for commands.
//! The host hands its changes over a channel and never waits for either.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Commit;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Frame;
use leptoskit_devtools_protocol::HISTORY;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::Tree;
use leptoskit_devtools_protocol::TreePatch;

use super::HostAgent;
use super::SessionGone;
use super::mirror::Change;
use super::mirror::Mirror;
use super::patch::diff;
use super::snapshot_of;

/// How often the reporter sends while the tree keeps changing, so a burst of
/// commits, such as an animation's, goes out as one request.
const INTERVAL: Duration = Duration::from_millis(100);

/// How long to wait before trying an unreachable hub again.
const RETRY: Duration = Duration::from_secs(2);

/// How long the reporter sleeps with nothing to send or wait for.
const IDLE: Duration = Duration::from_secs(60);

/// One render's changes to a guest's tree, handed over by the host's render
/// path.
#[derive(Clone, Debug, Default)]
pub struct Batch {
	/// The root the renderer shows after these changes.
	pub root: Option<u64>,
	/// The changes, in the order the renderer applies them.
	pub changes: Vec<Change>,
	/// The commits the surface made since the last batch.
	pub commits: Vec<CommitRecord>,
}

/// A commit the surface made, as its commit observer saw it.
#[derive(Clone, Copy, Debug)]
pub struct CommitRecord {
	/// When it finished.
	pub at: Instant,
	/// How many mutations it applied.
	pub mutations: usize,
	/// The time spent in layout, while profiling is on.
	pub layout_us: Option<u64>,
	/// The time spent applying the backend batches, while profiling is on.
	pub apply_us: Option<u64>,
}

/// What the hub asked the app to highlight.
#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
	/// The match key the hub named.
	pub key: MatchKey,
	/// The reported tree's node with that key, and its frame in viewport
	/// space, when the tree has one.
	pub node: Option<(u64, Frame)>,
}

/// A native host's report to the local hub.
///
/// It opens a session as soon as it starts, retrying until the hub answers,
/// and opens a new one whenever the hub forgets it. Each guest the host runs
/// is a [`Source`]. The newest source that has rendered is the tree the
/// session shows, so a hot reload keeps the session and sends a fresh
/// snapshot once the new guest renders.
pub struct Reporter {
	shared: Arc<Shared>,
	sender: mpsc::Sender<Message>,
	sources: AtomicU64,
}

impl Reporter {
	/// Starts reporting to the hub at `hub` as the app `hello` describes.
	///
	/// # Errors
	///
	/// When a thread cannot start.
	pub fn start(hub: &str, hello: Hello) -> std::io::Result<Self> {
		let shared = Arc::new(Shared::default());
		let (sender, receiver) = mpsc::channel();
		let worker = Worker::new(hub.to_owned(), hello, Arc::clone(&shared));
		std::thread::Builder::new()
			.name("leptoskit-devtools".to_owned())
			.spawn(move || worker.run(&receiver))?;
		let poller = Poller {
			shared: Arc::clone(&shared),
			sender: sender.clone(),
		};

		if let Err(error) = std::thread::Builder::new()
			.name("leptoskit-devtools-commands".to_owned())
			.spawn(move || poller.run())
		{
			let _ = sender.send(Message::Stop);
			return Err(error);
		}

		Ok(Self {
			shared,
			sender,
			sources: AtomicU64::new(0),
		})
	}

	/// A source for the next guest the host runs. Sources made later win:
	/// once a newer one renders, the session shows its tree.
	#[must_use]
	pub fn source(&self) -> Source {
		Source {
			id: self.sources.fetch_add(1, Ordering::Relaxed),
			sender: self.sender.clone(),
			shared: Arc::clone(&self.shared),
		}
	}

	/// What the hub last asked the app to highlight, if anything.
	#[must_use]
	pub fn highlight(&self) -> Option<Highlight> {
		self.shared.highlight().clone()
	}

	/// Whether the hub asked to pick a node with the next tap.
	#[must_use]
	pub fn picking(&self) -> bool {
		self.shared.picking.load(Ordering::Acquire)
	}
}

impl Drop for Reporter {
	/// Stops both threads. The command thread ends once its current
	/// long-poll returns.
	fn drop(&mut self) {
		self.shared.stopped.store(true, Ordering::Release);
		self.shared.session_changed.notify_all();
		let _ = self.sender.send(Message::Stop);
	}
}

/// One guest's tree as its host renders it. Dropping it tells the reporter
/// the guest is gone.
pub struct Source {
	id: u64,
	sender: mpsc::Sender<Message>,
	shared: Arc<Shared>,
}

impl Source {
	/// Hands over one render's changes. It never blocks: the reporter's
	/// thread applies them.
	pub fn send(&self, batch: Batch) {
		let _ = self.sender.send(Message::Batch(self.id, batch));
	}

	/// Reports `node` as picked when the hub asked to pick a node with the
	/// next tap, and turns picking off. Returns whether it did, in which case
	/// the tap belongs to the hub rather than the app.
	pub fn pick(&self, node: u64) -> bool {
		if !self.shared.picking.load(Ordering::Acquire)
			|| !self.shared.picking.swap(false, Ordering::AcqRel)
		{
			return false;
		}

		let _ = self.sender.send(Message::Picked(self.id, node));
		true
	}

	/// Whether the hub asked for commit timings.
	#[must_use]
	pub fn profiling(&self) -> bool {
		self.shared.profiling.load(Ordering::Acquire)
	}
}

impl Drop for Source {
	fn drop(&mut self) {
		let _ = self.sender.send(Message::Closed(self.id));
	}
}

/// What the reporter's threads and the host share.
#[derive(Default)]
struct Shared {
	highlight: Mutex<Option<Highlight>>,
	picking: AtomicBool,
	profiling: AtomicBool,
	/// The open session, for the command thread.
	session: Mutex<Option<HostAgent>>,
	session_changed: Condvar,
	stopped: AtomicBool,
}

impl Shared {
	/// Each lock is held only to move a value in or out, so a panic while
	/// one is held cannot leave it half changed.
	fn highlight(&self) -> MutexGuard<'_, Option<Highlight>> {
		self.highlight
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn session(&self) -> MutexGuard<'_, Option<HostAgent>> {
		self.session.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn publish(&self, session: Option<HostAgent>) {
		*self.session() = session;
		self.session_changed.notify_all();
	}
}

enum Message {
	/// A source's render.
	Batch(u64, Batch),
	/// A source is gone.
	Closed(u64),
	/// A source's node was picked.
	Picked(u64, u64),
	/// The hub sent a command.
	Command(Command),
	/// The hub forgot this session.
	Gone(SessionId),
	Stop,
}

/// The reporting thread's state.
struct Worker {
	hub: String,
	hello: Hello,
	shared: Arc<Shared>,
	/// The open session.
	agent: Option<HostAgent>,
	sources: BTreeMap<u64, Mirror>,
	/// The source the session shows.
	current: Option<u64>,
	/// The tree the hub has.
	sent: Option<Tree>,
	/// Whether the next report replaces the hub's tree: a new session, a new
	/// guest, or the hub asked.
	snapshot: bool,
	/// Whether the shown tree changed since the last report.
	changed: bool,
	/// Commits waiting to be sent, the newest [`HISTORY`] of them.
	commits: VecDeque<CommitRecord>,
	picked: Vec<Event>,
	/// The session's next commit number.
	seq: u64,
	/// When the session opened.
	opened: Instant,
	next_report: Instant,
	next_connect: Instant,
	/// Whether the last attempt to reach the hub failed, so a hub that stays
	/// away is reported once.
	unreachable: bool,
}

impl Worker {
	fn new(hub: String, hello: Hello, shared: Arc<Shared>) -> Self {
		let now = Instant::now();
		Self {
			hub,
			hello,
			shared,
			agent: None,
			sources: BTreeMap::new(),
			current: None,
			sent: None,
			snapshot: true,
			changed: false,
			commits: VecDeque::new(),
			picked: Vec::new(),
			seq: 0,
			opened: now,
			next_report: now,
			next_connect: now,
			unreachable: false,
		}
	}

	fn run(mut self, receiver: &mpsc::Receiver<Message>) {
		loop {
			let wait = self.next_wake().saturating_duration_since(Instant::now());
			let first = match receiver.recv_timeout(wait) {
				Ok(message) => Some(message),
				Err(mpsc::RecvTimeoutError::Timeout) => None,
				Err(mpsc::RecvTimeoutError::Disconnected) => return,
			};

			for message in first.into_iter().chain(receiver.try_iter()) {
				if !self.handle(message) {
					self.shared.publish(None);
					return;
				}
			}

			self.connect();
			self.report();
		}
	}

	fn next_wake(&self) -> Instant {
		if self.agent.is_none() {
			self.next_connect
		} else if self.pending() {
			self.next_report
		} else {
			Instant::now() + IDLE
		}
	}

	/// Whether a report has something to send.
	fn pending(&self) -> bool {
		(self.snapshot && self.current.is_some())
			|| self.changed
			|| !self.commits.is_empty()
			|| !self.picked.is_empty()
	}

	/// Applies one message; false when the reporter stops.
	fn handle(&mut self, message: Message) -> bool {
		match message {
			Message::Batch(id, batch) => self.apply(id, batch),
			Message::Closed(id) => self.close(id),
			Message::Picked(id, node) => self.pick(id, node),
			Message::Command(command) => self.command(command),
			Message::Gone(session) => {
				if self
					.agent
					.as_ref()
					.is_some_and(|agent| *agent.session() == session)
				{
					self.disconnect();
				}
			}
			Message::Stop => return false,
		}

		true
	}

	fn apply(&mut self, id: u64, batch: Batch) {
		let mirror = self.sources.entry(id).or_default();
		let root = mirror.root();
		let edited = !batch.changes.is_empty();

		for change in batch.changes {
			mirror.apply(change);
		}

		mirror.set_root(batch.root);
		let rendered = mirror.root().is_some();
		let moved = mirror.root() != root;

		// A newer guest's first render replaces the tree the session shows.
		if self.current != Some(id) && rendered && self.current.is_none_or(|current| id > current) {
			self.current = Some(id);
			self.snapshot = true;
		}

		if self.current == Some(id) {
			self.changed |= edited || moved;

			for record in batch.commits {
				if self.commits.len() == HISTORY {
					self.commits.pop_front();
				}

				self.commits.push_back(record);
			}
		}
	}

	/// Forgets a guest. When it was the one shown, the newest guest left
	/// that has rendered is shown instead; with none, the hub keeps the last
	/// tree until the next guest renders.
	fn close(&mut self, id: u64) {
		self.sources.remove(&id);

		if self.current == Some(id) {
			self.current = self
				.sources
				.iter()
				.rev()
				.find(|(_, mirror)| mirror.root().is_some())
				.map(|(id, _)| *id);
			self.snapshot = true;
		}
	}

	fn pick(&mut self, id: u64, node: u64) {
		let key = self
			.sources
			.get(&id)
			.and_then(Mirror::tree)
			.and_then(|tree| {
				tree.nodes
					.into_iter()
					.find(|candidate| candidate.id == NodeRef::Native(node))
			})
			.map(|node| node.key);

		if let Some(key) = key {
			self.picked.push(Event::Picked {
				node: NodeRef::Native(node),
				key: key.clone(),
			});
			self.highlight(key);
		}
	}

	fn command(&mut self, command: Command) {
		match command {
			Command::RequestSnapshot => self.snapshot = true,
			Command::Highlight { key } | Command::Select { key, .. } => self.highlight(key),
			Command::Pick { on } => self.shared.picking.store(on, Ordering::Release),
			Command::StartProfiling => self.shared.profiling.store(true, Ordering::Release),
			Command::StopProfiling => self.shared.profiling.store(false, Ordering::Release),
			// Classes, routes, queries, forms, and state are the guest's,
			// which reports them through its own agent.
			_ => {}
		}
	}

	/// Highlights the node with `key`; an empty key clears the highlight.
	fn highlight(&mut self, key: MatchKey) {
		let highlight = (!key.as_str().is_empty()).then(|| {
			Highlight {
				node: self.current_tree().and_then(|tree| locate(&tree, &key)),
				key,
			}
		});
		*self.shared.highlight() = highlight;
	}

	fn current_tree(&self) -> Option<Tree> {
		self.current
			.and_then(|id| self.sources.get(&id))
			.and_then(Mirror::tree)
	}

	fn connect(&mut self) {
		if self.agent.is_some() || Instant::now() < self.next_connect {
			return;
		}

		match HostAgent::connect(&self.hub, &self.hello) {
			Ok(agent) => {
				self.unreachable = false;
				self.opened = Instant::now();
				self.seq = 0;
				self.sent = None;
				self.snapshot = true;
				self.next_report = self.opened;
				self.shared.publish(Some(agent.clone()));
				self.agent = Some(agent);
			}
			Err(error) => {
				if !self.unreachable {
					eprintln!(
						"leptoskit devtools: cannot reach the hub at {}, retrying: {error}",
						self.hub
					);
					self.unreachable = true;
				}

				self.next_connect = Instant::now() + RETRY;
			}
		}
	}

	fn disconnect(&mut self) {
		self.agent = None;
		self.shared.publish(None);
		self.next_connect = Instant::now();
	}

	/// Sends what changed since the last report: a snapshot or a patch of
	/// the shown tree, then the commits and picks waiting.
	fn report(&mut self) {
		let now = Instant::now();

		if self.agent.is_none() || !self.pending() || now < self.next_report {
			return;
		}

		let mut events = Vec::new();

		if let Some(tree) = self.current_tree() {
			self.refresh_highlight(&tree);
			let patch = match &self.sent {
				Some(sent) if !self.snapshot && self.changed => diff(sent, &tree),
				Some(_) if !self.snapshot => Some(Vec::new()),
				_ => None,
			};

			match patch {
				Some(ops) if ops.is_empty() => {}
				Some(ops) => {
					events.push(Event::TreePatch {
						patch: TreePatch { ops },
					});
				}
				None => {
					events.push(Event::Snapshot {
						snapshot: snapshot_of(tree.clone()),
					});
				}
			}

			self.sent = Some(tree);
			self.snapshot = false;
		}

		self.changed = false;

		for record in self.commits.drain(..) {
			events.push(Event::Commit {
				commit: Commit {
					seq: self.seq,
					at_ms: u64::try_from(
						record.at.saturating_duration_since(self.opened).as_millis(),
					)
					.unwrap_or(u64::MAX),
					mutations: record.mutations,
					layout_us: record.layout_us,
					apply_us: record.apply_us,
				},
			});
			self.seq += 1;
		}

		events.append(&mut self.picked);

		if events.is_empty() {
			return;
		}

		let result = self
			.agent
			.as_ref()
			.map_or(Ok(()), |agent| agent.report(&events));

		match result {
			Ok(()) => self.next_report = now + INTERVAL,
			Err(error) if error.is::<SessionGone>() => self.disconnect(),
			Err(error) => {
				eprintln!(
					"leptoskit devtools: the hub refused a report, resending the tree: {error}"
				);
				// The hub may have missed these events, so the next report
				// replaces its tree rather than patching a copy it may not
				// have.
				self.snapshot = true;
				self.next_report = now + RETRY;
			}
		}
	}

	/// Moves the highlight to where its node is in the new tree.
	fn refresh_highlight(&self, tree: &Tree) {
		if let Some(highlight) = self.shared.highlight().as_mut() {
			highlight.node = locate(tree, &highlight.key);
		}
	}
}

/// The node with `key` in `tree`, and its frame.
fn locate(tree: &Tree, key: &MatchKey) -> Option<(u64, Frame)> {
	tree.nodes
		.iter()
		.find(|node| &node.key == key)
		.and_then(|node| {
			match (&node.id, node.frame) {
				(NodeRef::Native(id), Some(frame)) => Some((*id, frame)),
				_ => None,
			}
		})
}

/// The command thread: long-polls the open session and hands each command to
/// the reporting thread.
struct Poller {
	shared: Arc<Shared>,
	sender: mpsc::Sender<Message>,
}

impl Poller {
	fn run(self) {
		let mut polling: Option<HostAgent> = None;
		let mut gone = false;

		while self.next_session(&mut polling, &mut gone) {
			let Some(agent) = polling.as_mut() else {
				continue;
			};

			match agent.commands() {
				Ok(commands) => {
					for command in commands {
						if self.sender.send(Message::Command(command)).is_err() {
							return;
						}
					}
				}
				Err(error) if error.is::<SessionGone>() => {
					gone = true;

					if self
						.sender
						.send(Message::Gone(agent.session().clone()))
						.is_err()
					{
						return;
					}
				}
				Err(_) => std::thread::sleep(RETRY),
			}
		}
	}

	/// Waits for a session to poll: the open one, or once the hub has
	/// forgotten it, the next. False once the reporter stops.
	fn next_session(&self, polling: &mut Option<HostAgent>, gone: &mut bool) -> bool {
		let mut session = self.shared.session();

		loop {
			if self.shared.stopped.load(Ordering::Acquire) {
				return false;
			}

			if let Some(open) = session.as_ref() {
				let ours = polling
					.as_ref()
					.is_some_and(|agent| agent.session() == open.session());

				if !ours {
					*polling = Some(open.clone());
					*gone = false;
					return true;
				}

				if !*gone {
					return true;
				}
			}

			session = self
				.shared
				.session_changed
				.wait(session)
				.unwrap_or_else(PoisonError::into_inner);
		}
	}
}
