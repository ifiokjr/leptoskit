//! Reports a native host to the hub from two threads of its own: one keeps
//! each guest's tree and sends what changed, the other waits for commands.
//! The host hands its changes over a channel and never waits for either.
//!
//! A guest reports what only it knows (its route, queries, forms, and named
//! state) through the `DevTools` bridge plugin, which hands it to the
//! guest's [`Guest`]. The reporter keeps each guest's values beside its
//! tree, puts the shown guest's in every snapshot and sends their changes
//! between, and hands the hub's commands for them to the guest.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
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

use leptoskit_devtools_protocol::Capability;
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
use crate::registry;
use crate::registry::Reported;
use crate::registry::Slot;

/// How often the reporter sends while the tree keeps changing, so a burst of
/// commits, such as an animation's, goes out as one request.
const INTERVAL: Duration = Duration::from_millis(100);

/// How long to wait before trying an unreachable hub again.
const RETRY: Duration = Duration::from_secs(2);

/// How long the reporter sleeps with nothing to send or wait for.
const IDLE: Duration = Duration::from_secs(60);

/// How long after the first render a reporter started with
/// [`Reporter::with_guests`] waits for a guest to say what it feeds before
/// opening the session without it. A guest says so before it first renders,
/// so this only passes for a guest built without `DevTools`.
const GUEST_GRACE: Duration = Duration::from_millis(500);

/// How long such a reporter waits at most, for a guest that never renders.
const GUEST_WAIT: Duration = Duration::from_secs(3);

/// The most values the reporter keeps for one guest.
const MAX_GUEST_VALUES: usize = 1024;

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
	/// Where a platform draws the outline: the nearest view it draws, the
	/// node's own or an ancestor's, and the node's frame in the coordinates
	/// that view lays its children out in (see [`Mirror::anchor`]).
	pub anchor: Option<(u64, Frame)>,
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
		Self::launch(hub, hello, false)
	}

	/// Starts reporting a host whose guests report their own values through
	/// [`Guest`]. The session opens once the first guest has said what it
	/// feeds, which `hello`'s capabilities then include, or shortly after
	/// it first renders without saying.
	///
	/// # Errors
	///
	/// When a thread cannot start.
	pub fn with_guests(hub: &str, hello: Hello) -> std::io::Result<Self> {
		Self::launch(hub, hello, true)
	}

	fn launch(hub: &str, hello: Hello, await_guest: bool) -> std::io::Result<Self> {
		let shared = Arc::new(Shared::default());
		let (sender, receiver) = mpsc::channel();
		let mut worker = Worker::new(hub.to_owned(), hello, Arc::clone(&shared));
		if await_guest {
			worker.fed = Fed::Awaited;
		}
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

	/// Picks what a tap at `(x, y)` lands on, when the hub asked to pick a
	/// node with the next tap, and turns picking off. The point is in the
	/// coordinates the drawn view `view` lays its children out in, so the
	/// platform need not know where its scroll views are; the shown tree
	/// finds the deepest node under it, including layout-only containers the
	/// platform never draws (see [`Mirror::hit`]). Returns whether it did,
	/// in which case the tap belongs to the hub rather than the app.
	pub fn tap(&self, view: u64, x: f32, y: f32) -> bool {
		if !self.shared.picking.load(Ordering::Acquire)
			|| !self.shared.picking.swap(false, Ordering::AcqRel)
		{
			return false;
		}

		let _ = self.sender.send(Message::Tap(view, x, y));
		true
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

	/// The guest's half, for the `DevTools` bridge plugin.
	#[must_use]
	pub fn guest(&self) -> Guest {
		Guest {
			id: self.id,
			sender: self.sender.clone(),
		}
	}
}

/// What a guest reports through the `DevTools` bridge plugin, and where the
/// hub's commands for it go. Clones share it, and any thread may use it; it
/// ends with its [`Source`].
#[derive(Clone)]
pub struct Guest {
	id: u64,
	sender: mpsc::Sender<Message>,
}

impl Guest {
	/// The guest said what it can feed: navigation, state, and the rest.
	pub fn hello(&self, capabilities: BTreeSet<Capability>) {
		let _ = self.sender.send(Message::GuestHello(capabilities));
	}

	/// The guest reported values: with `reset`, every value it holds, which
	/// replace the ones it reported before; otherwise only those that
	/// changed. Events other than values are ignored.
	pub fn report(&self, reset: bool, events: Vec<Event>) {
		let _ = self
			.sender
			.send(Message::GuestReport(self.id, reset, events));
	}

	/// Hands the hub's commands for this guest to `deliver`: navigating,
	/// setting state, refetching, invalidating, and editing queries, and a
	/// snapshot request, which the guest answers with every value.
	pub fn on_command(&self, deliver: impl Fn(Command) + Send + 'static) {
		let _ = self
			.sender
			.send(Message::GuestCommands(self.id, Box::new(deliver)));
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
	/// A tap picked whatever the shown tree has under it.
	Tap(u64, f32, f32),
	/// The hub sent a command.
	Command(Command),
	/// The hub forgot this session.
	Gone(SessionId),
	/// A guest said what it feeds.
	GuestHello(BTreeSet<Capability>),
	/// A guest reported values.
	GuestReport(u64, bool, Vec<Event>),
	/// Where a guest's commands go.
	GuestCommands(u64, Box<dyn Fn(Command) + Send>),
	Stop,
}

/// What the guests feed, as far as the reporter knows.
enum Fed {
	/// Nothing yet, and the session doesn't wait for it.
	Unawaited,
	/// Nothing yet; the session waits for a guest to say.
	Awaited,
	/// What the guests said, which the session's hello lists.
	Heard(BTreeSet<Capability>),
}

/// What a guest reported, and where its commands go.
#[derive(Default)]
struct GuestState {
	values: BTreeMap<Slot, Reported>,
	deliver: Option<Box<dyn Fn(Command) + Send>>,
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
	/// Each guest's values, by source.
	guests: BTreeMap<u64, GuestState>,
	/// The shown guest's values the hub hasn't been sent.
	dirty: BTreeSet<Slot>,
	/// What the guests said they feed, and whether the session waits to
	/// hear it.
	fed: Fed,
	started: Instant,
	first_render: Option<Instant>,
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
			guests: BTreeMap::new(),
			dirty: BTreeSet::new(),
			fed: Fed::Unawaited,
			started: now,
			first_render: None,
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
			self.held()
				.map_or(self.next_connect, |until| until.max(self.next_connect))
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
			|| (self.current.is_some() && !self.dirty.is_empty())
	}

	/// Until when the session waits for a guest to say what it feeds, if it
	/// still does.
	fn held(&self) -> Option<Instant> {
		if !matches!(self.fed, Fed::Awaited) {
			return None;
		}
		let until = self
			.first_render
			.map_or(self.started + GUEST_WAIT, |rendered| {
				(rendered + GUEST_GRACE).min(self.started + GUEST_WAIT)
			});
		(Instant::now() < until).then_some(until)
	}

	/// Applies one message; false when the reporter stops.
	fn handle(&mut self, message: Message) -> bool {
		match message {
			Message::Batch(id, batch) => self.apply(id, batch),
			Message::Closed(id) => self.close(id),
			Message::Picked(id, node) => self.pick(id, node),
			Message::Tap(view, x, y) => {
				if let Some(current) = self.current
					&& let Some(node) = self
						.sources
						.get(&current)
						.and_then(|mirror| mirror.hit(view, x, y))
				{
					self.pick(current, node);
				}
			}
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
			Message::GuestHello(capabilities) => {
				match &mut self.fed {
					Fed::Heard(fed) => fed.extend(capabilities),
					fed => *fed = Fed::Heard(capabilities),
				}
			}
			Message::GuestReport(id, reset, events) => self.guest_report(id, reset, events),
			Message::GuestCommands(id, deliver) => {
				self.guests.entry(id).or_default().deliver = Some(deliver);
			}
			Message::Stop => return false,
		}

		true
	}

	/// Keeps a guest's values. The shown guest's changes are sent with the
	/// next report; a reset that leaves a value out needs a snapshot, since
	/// the protocol has no removal.
	fn guest_report(&mut self, id: u64, reset: bool, events: Vec<Event>) {
		let shown = self.current == Some(id);
		let guest = self.guests.entry(id).or_default();
		let previous = reset.then(|| std::mem::take(&mut guest.values));

		for event in events {
			let Some((slot, value)) = registry::slot_of(event) else {
				continue;
			};
			if !guest.values.contains_key(&slot) && guest.values.len() >= MAX_GUEST_VALUES {
				continue;
			}
			let before = previous.as_ref().unwrap_or(&guest.values).get(&slot);
			if shown && before != Some(&value) {
				self.dirty.insert(slot.clone());
			}
			guest.values.insert(slot, value);
		}

		if shown
			&& let Some(previous) = previous
			&& previous.keys().any(|slot| !guest.values.contains_key(slot))
		{
			self.snapshot = true;
		}
	}

	/// The hub's command for the guest, to the one shown, or before any has
	/// rendered, the newest.
	fn to_guest(&self, command: Command) {
		let deliver = self
			.current
			.or_else(|| self.guests.keys().next_back().copied())
			.and_then(|id| self.guests.get(&id))
			.and_then(|guest| guest.deliver.as_ref());
		if let Some(deliver) = deliver {
			deliver(command);
		}
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

		if rendered && self.first_render.is_none() {
			self.first_render = Some(Instant::now());
		}

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
		self.guests.remove(&id);

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
			Command::RequestSnapshot => {
				self.snapshot = true;
				// The guest resends its values too, in case the reporter
				// missed some.
				self.to_guest(Command::RequestSnapshot);
			}
			Command::Highlight { key } | Command::Select { key, .. } => self.highlight(key),
			Command::Pick { on } => self.shared.picking.store(on, Ordering::Release),
			Command::StartProfiling => self.shared.profiling.store(true, Ordering::Release),
			Command::StopProfiling => self.shared.profiling.store(false, Ordering::Release),
			// Routes, queries, forms, and state are the guest's, which
			// applies these through its own agent.
			command @ (Command::Navigate { .. }
			| Command::InvalidateQuery { .. }
			| Command::RefetchQuery { .. }
			| Command::SetQueryData { .. }
			| Command::SetState { .. }) => self.to_guest(command),
			Command::SetClasses { .. } => {}
		}
	}

	/// Highlights the node with `key`; an empty key clears the highlight.
	fn highlight(&mut self, key: MatchKey) {
		let highlight = (!key.as_str().is_empty()).then(|| {
			let mut highlight = Highlight {
				key,
				node: None,
				anchor: None,
			};

			if let Some(tree) = self.current_tree() {
				self.locate(&tree, &mut highlight);
			}

			highlight
		});
		*self.shared.highlight() = highlight;
	}

	/// Finds the highlighted node in `tree`, the shown tree, and where a
	/// platform draws it.
	fn locate(&self, tree: &Tree, highlight: &mut Highlight) {
		highlight.node = locate(tree, &highlight.key);
		highlight.anchor = highlight.node.and_then(|(id, _)| {
			self.current
				.and_then(|current| self.sources.get(&current))
				.and_then(|mirror| mirror.anchor(id))
		});
	}

	fn current_tree(&self) -> Option<Tree> {
		self.current
			.and_then(|id| self.sources.get(&id))
			.and_then(Mirror::tree)
	}

	fn connect(&mut self) {
		if self.agent.is_some() || Instant::now() < self.next_connect || self.held().is_some() {
			return;
		}

		let mut hello = self.hello.clone();
		if let Fed::Heard(fed) = &self.fed {
			hello.capabilities.extend(fed.iter().copied());
		}

		match HostAgent::connect(&self.hub, &hello) {
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
					let mut snapshot = snapshot_of(tree.clone());
					if let Some(guest) = self.current.and_then(|id| self.guests.get(&id)) {
						registry::fill(&mut snapshot, &guest.values);
					}
					self.dirty.clear();
					events.push(Event::Snapshot { snapshot });
				}
			}

			self.sent = Some(tree);
			self.snapshot = false;
		}

		self.changed = false;

		if !self.snapshot {
			let values = self
				.current
				.and_then(|id| self.guests.get(&id))
				.map(|guest| &guest.values);
			for slot in std::mem::take(&mut self.dirty) {
				let value = values.and_then(|values| values.get(&slot));
				if let Some(event) = value.and_then(|value| registry::event(&slot, value)) {
					events.push(event);
				}
			}
		}

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
			self.locate(tree, highlight);
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
