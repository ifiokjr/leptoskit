//! The seam between the hooks an app calls and the agent that reports them.
//!
//! A hook registers a [`Source`]: something that reports values into the
//! session view (the route, the queries, a form, named state) and answers the
//! commands addressed to it. On the app's own path a hook only marks its
//! source dirty, inside the signal write or cache event that changed it. The
//! agent's debounced flush collects the dirty sources, serializes what they
//! hold, and sends only the values that changed since the hub last heard.
//! Values are coalesced by where they go in the session view, so a burst of
//! changes costs one value each, and the registry never grows past one entry
//! per value the app shows.
//!
//! The agent is a [`Sink`]. The web agent installs itself when it starts; a
//! native guest's bridge plugin can install another without changing a
//! hook.
//!
//! The registry is per thread: hooks and agents run on the app's UI thread,
//! and wasm has only that one. A callback that fires on another thread finds
//! that thread's registry, which no agent drains.

// The agent that drains the registry runs only in the browser so far; a
// native guest's will use the same calls.
#![cfg_attr(
	not(any(test, all(feature = "web", target_arch = "wasm32"))),
	allow(dead_code)
)]

use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::rc::Rc;

use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::NamedValue;
use leptoskit_devtools_protocol::Snapshot;
use leptoskit_devtools_protocol::Tree;
use leptoskit_devtools_protocol::Value;

/// Whether this build runs the web agent: a browser build that `leptoskit
/// dev` compiled a hub address into. Hooks then register even before the
/// agent installs itself, so the order of `install` and the hooks doesn't
/// matter.
const WEB_AGENT: bool =
	cfg!(all(feature = "web", target_arch = "wasm32")) && crate::hub().is_some();

/// Wakes the agent when a source changes.
pub(crate) trait Sink {
	/// A source changed: collect and report soon. This runs on the app's
	/// path, inside the write that changed the source, so it only schedules
	/// work.
	fn wake(&self);
}

/// Something a hook reports: the route, a cache, a form, or a signal.
pub(crate) trait Source: Any {
	/// Reports every value the source holds now. Only the agent's flush
	/// calls this, never the app's render path.
	fn collect(&self, report: &mut Report);

	/// Applies `command` if it addresses this source, or returns `None`.
	fn apply(&self, command: &Command) -> Option<Result<(), String>>;
}

/// A registered source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceId(u64);

/// Where a reported value goes in the session view.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Slot {
	Route,
	Query(String),
	Mutation(String),
	Form(String),
	State(String),
}

/// A value as the hub was last sent it.
#[derive(Clone, Debug, PartialEq)]
enum Reported {
	Route(String),
	Value(Value),
}

/// The values one source holds, collected in the agent's flush.
#[derive(Debug, Default)]
pub(crate) struct Report {
	items: Vec<(Slot, Reported)>,
}

impl Report {
	/// The current route, as an href.
	pub(crate) fn route(&mut self, href: String) {
		self.items.push((Slot::Route, Reported::Route(href)));
	}

	/// A query, by its key.
	#[cfg_attr(not(feature = "query"), allow(dead_code))]
	pub(crate) fn query(&mut self, key: String, value: Value) {
		self.items.push((Slot::Query(key), Reported::Value(value)));
	}

	/// A mutation, by its key.
	#[cfg_attr(not(feature = "query"), allow(dead_code))]
	pub(crate) fn mutation(&mut self, key: String, value: Value) {
		self.items
			.push((Slot::Mutation(key), Reported::Value(value)));
	}

	/// A form, by its name.
	#[cfg_attr(not(feature = "form"), allow(dead_code))]
	pub(crate) fn form(&mut self, name: String, value: Value) {
		self.items.push((Slot::Form(name), Reported::Value(value)));
	}

	/// Named state, by its name.
	pub(crate) fn state(&mut self, name: String, value: Value) {
		self.items.push((Slot::State(name), Reported::Value(value)));
	}
}

/// What changed since the last collect.
#[derive(Debug, Default)]
pub(crate) struct Changes {
	/// One event for each value that changed.
	pub(crate) events: Vec<Event>,
	/// Whether a value went away. The protocol has no event for that, so
	/// only a snapshot shows it.
	pub(crate) removed: bool,
}

struct Entry {
	id: SourceId,
	/// Registering another source under the same key replaces this one.
	key: Option<Slot>,
	source: Rc<dyn Source>,
	/// Effects and subscriptions that live as long as the source.
	guards: Vec<Box<dyn Any>>,
	/// The slots this source reported at the last collect.
	slots: BTreeSet<Slot>,
}

#[derive(Default)]
struct Registry {
	next: u64,
	entries: Vec<Entry>,
	/// What the hub was last sent, by slot.
	reported: BTreeMap<Slot, Reported>,
	/// Slots a source stopped reporting, or whose source went away, since
	/// the last collect.
	orphaned: BTreeSet<Slot>,
}

thread_local! {
	static REGISTRY: RefCell<Registry> = RefCell::default();
	/// Kept apart from the registry, so a source marked dirty from inside
	/// any registry call never finds it borrowed.
	static DIRTY: RefCell<BTreeSet<SourceId>> = const { RefCell::new(BTreeSet::new()) };
	static SINK: RefCell<Option<Rc<dyn Sink>>> = const { RefCell::new(None) };
}

/// Installs the agent that reports the registry, and wakes it if a source
/// changed before it arrived.
pub(crate) fn install(sink: Rc<dyn Sink>) {
	SINK.with_borrow_mut(|installed| *installed = Some(sink));
	if DIRTY.with_borrow(|dirty| !dirty.is_empty()) {
		wake();
	}
}

/// Whether hooks should register anything: an agent is installed, or this
/// build will start one. Without either, every hook does nothing.
pub(crate) fn active() -> bool {
	WEB_AGENT || SINK.with_borrow(Option::is_some)
}

/// What a session with these hooks can feed and accept, whether or not the
/// app calls them yet.
pub(crate) fn capabilities() -> Vec<Capability> {
	let mut capabilities = vec![
		Capability::Navigation,
		Capability::State,
		Capability::Editing,
	];
	if cfg!(feature = "form") {
		capabilities.push(Capability::Forms);
	}
	if cfg!(feature = "query") {
		capabilities.extend([Capability::Queries, Capability::Mutations]);
	}
	capabilities
}

/// Registers `source`, replacing the source registered under the same
/// `key`, and marks it dirty so the next flush reports it.
pub(crate) fn register(key: Option<Slot>, source: Rc<dyn Source>) -> SourceId {
	let (id, replaced) = REGISTRY.with_borrow_mut(|registry| {
		registry.next += 1;
		let id = SourceId(registry.next);
		let replaced = key
			.as_ref()
			.and_then(|key| registry.remove(|entry| entry.key.as_ref() == Some(key)));
		registry.entries.push(Entry {
			id,
			key,
			source,
			guards: Vec::new(),
			slots: BTreeSet::new(),
		});
		(id, replaced)
	});
	// Effects and subscriptions drop outside the registry.
	drop(replaced);
	mark_dirty(id);
	id
}

/// Keeps `guard`, such as an effect or a subscription, alive as long as the
/// source.
pub(crate) fn keep(id: SourceId, guard: impl Any) {
	let guard: Box<dyn Any> = Box::new(guard);
	let orphan = REGISTRY.with_borrow_mut(|registry| {
		match registry.entries.iter_mut().find(|entry| entry.id == id) {
			Some(entry) => {
				entry.guards.push(guard);
				None
			}
			None => Some(guard),
		}
	});
	drop(orphan);
}

/// Removes a source, such as one whose component unmounted. Its values
/// leave the session view with the next snapshot.
pub(crate) fn unregister(id: SourceId) {
	let removed = REGISTRY
		.try_with(|registry| registry.borrow_mut().remove(|entry| entry.id == id))
		.ok()
		.flatten();
	if removed.is_some() {
		drop(removed);
		wake();
	}
}

/// Marks a source dirty and wakes the agent. This runs on the app's path,
/// so it does nothing else.
pub(crate) fn mark_dirty(id: SourceId) {
	let fresh = DIRTY
		.try_with(|dirty| dirty.borrow_mut().insert(id))
		.unwrap_or(false);
	if fresh {
		wake();
	}
}

fn wake() {
	let sink = SINK.try_with(|sink| sink.borrow().clone()).ok().flatten();
	if let Some(sink) = sink {
		sink.wake();
	}
}

/// The source registered under a key matching `key`, if it is an `S`.
#[cfg_attr(not(feature = "form"), allow(dead_code))]
pub(crate) fn find<S: Source>(key: impl Fn(&Slot) -> bool) -> Option<(SourceId, Rc<S>)> {
	let (id, source) = REGISTRY.with_borrow(|registry| {
		registry
			.entries
			.iter()
			.find(|entry| entry.key.as_ref().is_some_and(&key))
			.map(|entry| (entry.id, Rc::clone(&entry.source)))
	})?;
	let source: Rc<dyn Any> = source;
	source.downcast::<S>().ok().map(|source| (id, source))
}

/// Collects every dirty source and returns what changed since the last
/// collect. Only the agent's flush calls this.
pub(crate) fn collect() -> Changes {
	let dirty = DIRTY.with_borrow_mut(std::mem::take);
	let sources: Vec<(SourceId, Rc<dyn Source>)> = REGISTRY.with_borrow(|registry| {
		registry
			.entries
			.iter()
			.filter(|entry| dirty.contains(&entry.id))
			.map(|entry| (entry.id, Rc::clone(&entry.source)))
			.collect()
	});
	// Sources read signals and caches, so they run outside the registry.
	let reports: Vec<(SourceId, Report)> = sources
		.into_iter()
		.map(|(id, source)| {
			let mut report = Report::default();
			source.collect(&mut report);
			(id, report)
		})
		.collect();
	REGISTRY.with_borrow_mut(|registry| registry.merge(reports))
}

/// A snapshot of `tree` with every value the hub was last sent.
pub(crate) fn snapshot(tree: Tree) -> Snapshot {
	let mut snapshot = Snapshot {
		tree,
		route: None,
		queries: Vec::new(),
		mutations: Vec::new(),
		forms: Vec::new(),
		state: Vec::new(),
		calls: Vec::new(),
		rpc: Vec::new(),
	};
	REGISTRY.with_borrow(|registry| {
		for (slot, reported) in &registry.reported {
			let named = |name: &String, value: &Value| {
				NamedValue {
					name: name.clone(),
					value: value.clone(),
				}
			};
			match (slot, reported) {
				(Slot::Route, Reported::Route(route)) => snapshot.route = Some(route.clone()),
				(Slot::Query(key), Reported::Value(value)) => {
					snapshot.queries.push(named(key, value));
				}
				(Slot::Mutation(key), Reported::Value(value)) => {
					snapshot.mutations.push(named(key, value));
				}
				(Slot::Form(name), Reported::Value(value)) => {
					snapshot.forms.push(named(name, value));
				}
				(Slot::State(name), Reported::Value(value)) => {
					snapshot.state.push(named(name, value));
				}
				_ => {}
			}
		}
	});
	snapshot
}

/// Applies a command to the source it addresses, and marks that source
/// dirty so the next flush reports the result.
///
/// # Errors
///
/// When no source handles the command, or the one that does refuses it,
/// with a message that says why.
pub(crate) fn apply(command: &Command) -> Result<(), String> {
	let sources: Vec<(SourceId, Rc<dyn Source>)> = REGISTRY.with_borrow(|registry| {
		registry
			.entries
			.iter()
			.map(|entry| (entry.id, Rc::clone(&entry.source)))
			.collect()
	});
	// Commands set signals, which run effects, so they run outside the
	// registry.
	for (id, source) in sources {
		if let Some(result) = source.apply(command) {
			mark_dirty(id);
			return result;
		}
	}
	Err(unhandled(command))
}

fn unhandled(command: &Command) -> String {
	match command {
		Command::Navigate { .. } => {
			"no router is tracked, so the DevTools can't navigate; call `devtools::track_router` \
			 with the app's router"
				.to_owned()
		}
		Command::SetState { name, .. } => {
			format!("no state named {name:?} is exposed; name a signal with `devtools::expose`")
		}
		Command::InvalidateQuery { key }
		| Command::RefetchQuery { key }
		| Command::SetQueryData { key, .. } => {
			format!(
				"no tracked cache has a query {key:?}; call `devtools::track_queries` with the \
				 app's `QueryClient`"
			)
		}
		other => format!("this session doesn't handle {other:?}"),
	}
}

impl Registry {
	/// Takes out the first entry that matches, and orphans its slots.
	fn remove(&mut self, matches: impl Fn(&Entry) -> bool) -> Option<Entry> {
		let index = self.entries.iter().position(matches)?;
		let entry = self.entries.remove(index);
		self.orphaned.extend(entry.slots.iter().cloned());
		Some(entry)
	}

	fn merge(&mut self, reports: Vec<(SourceId, Report)>) -> Changes {
		let mut changes = Changes::default();

		for (id, report) in reports {
			let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
				continue;
			};
			let slots: BTreeSet<Slot> = report.items.iter().map(|(slot, _)| slot.clone()).collect();
			self.orphaned
				.extend(entry.slots.difference(&slots).cloned());
			entry.slots = slots;

			for (slot, value) in report.items {
				if self.reported.get(&slot) == Some(&value) {
					continue;
				}
				if let Some(event) = event(&slot, &value) {
					changes.events.push(event);
				}
				self.reported.insert(slot, value);
			}
		}

		for slot in std::mem::take(&mut self.orphaned) {
			let owned = self.entries.iter().any(|entry| entry.slots.contains(&slot));
			if !owned && self.reported.remove(&slot).is_some() {
				changes.removed = true;
			}
		}

		changes
	}
}

fn event(slot: &Slot, value: &Reported) -> Option<Event> {
	let event = match (slot, value) {
		(Slot::Route, Reported::Route(route)) => {
			Event::RouteChanged {
				route: route.clone(),
			}
		}
		(Slot::Query(key), Reported::Value(value)) => {
			Event::QueryChanged {
				key: key.clone(),
				value: value.clone(),
			}
		}
		(Slot::Mutation(key), Reported::Value(value)) => {
			Event::MutationChanged {
				key: key.clone(),
				value: value.clone(),
			}
		}
		(Slot::Form(name), Reported::Value(value)) => {
			Event::FormChanged {
				name: name.clone(),
				value: value.clone(),
			}
		}
		(Slot::State(name), Reported::Value(value)) => {
			Event::StateChanged {
				name: name.clone(),
				value: value.clone(),
			}
		}
		_ => return None,
	};
	Some(event)
}

/// Empties this thread's registry and removes its agent, for tests.
#[cfg(test)]
pub(crate) fn reset() {
	let registry = REGISTRY.with_borrow_mut(std::mem::take);
	DIRTY.with_borrow_mut(std::mem::take);
	let sink = SINK.with_borrow_mut(Option::take);
	drop((registry, sink));
}

/// How many sources are registered on this thread, for tests.
#[cfg(test)]
pub(crate) fn len() -> usize {
	REGISTRY.with_borrow(|registry| registry.entries.len())
}
