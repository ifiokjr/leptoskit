//! An app's part of the `DevTools` report: each render's mutations as the
//! platform receives them, the classes and source locations only the guest
//! knows, and the surface's commits.
//!
//! Everything here runs on the render path, so it only copies what the
//! mirror needs and hands it to the reporter's thread, which mirrors,
//! serializes, and sends it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

use leptoskit_devtools::Batch;
use leptoskit_devtools::Change;
use leptoskit_devtools::CommitRecord;
use leptoskit_devtools::Source;
use leptoskit_devtools_protocol::HISTORY;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::NodeId;

/// One app's report.
pub(super) struct Devtools {
	source: Source,
	/// The surface's commits since the last render, from its observer.
	commits: Rc<RefCell<VecDeque<CommitRecord>>>,
	/// The guest's commit count when its wire nodes were last read.
	guest_commits: Option<u32>,
	/// The classes and source last sent for each node.
	details: HashMap<usize, (String, Option<String>)>,
	profiling: bool,
}

impl Devtools {
	/// Starts reporting `host`'s tree, when this process reports to a hub.
	pub(super) fn attach(host: &mut WasmHost<HeadlessBackend>) -> Option<Self> {
		let source = crate::devtools::reporter()?.source();
		let commits = Rc::new(RefCell::new(VecDeque::new()));
		let observed = Rc::clone(&commits);
		host.surface_mut()
			.set_commit_observer(Box::new(move |commit| {
				let mut commits = observed.borrow_mut();

				if commits.len() == HISTORY {
					commits.pop_front();
				}

				commits.push_back(CommitRecord {
					at: Instant::now(),
					mutations: commit.mutations,
					layout_us: commit.layout_us,
					apply_us: commit.apply_us,
				});
			}));

		Some(Self {
			source,
			commits,
			guest_commits: None,
			details: HashMap::new(),
			profiling: false,
		})
	}

	/// Hands over one render: `mutations`, before flattening, so the report
	/// keeps the layout-only containers the platform never draws, then the
	/// classes and source of every node whose guest description changed, and
	/// the commits since the last render.
	pub(super) fn render(&mut self, host: &mut WasmHost<HeadlessBackend>, mutations: &[Mutation]) {
		self.profile(host);
		let mut changes: Vec<Change> = mutations.iter().map(Change::of).collect();
		let mut created = false;

		for mutation in mutations {
			match mutation {
				Mutation::Create { .. } => created = true,
				Mutation::Delete { id } => {
					self.details.remove(&id.index());
				}
				_ => {}
			}
		}

		// Only a guest commit changes what the guest said about a node, and
		// only a created node needs it said; anything else leaves it as sent.
		if created || self.guest_commits != Some(host.commits()) {
			self.guest_commits = Some(host.commits());
			self.describe(host, &mut changes);
		}

		self.send(host, changes);
	}

	/// Hands over the commits made since the last render when there is
	/// nothing to render, such as a guest commit that changed nothing.
	pub(super) fn idle(&self, host: &WasmHost<HeadlessBackend>) {
		if !self.commits.borrow().is_empty() {
			self.send(host, Vec::new());
		}
	}

	/// Whether a press on `node` belongs to the hub, which asked to pick a
	/// node with the next tap. The hub then hears which node it was.
	pub(super) fn pick(&self, node: NodeId) -> bool {
		self.source.pick(node.index() as u64)
	}

	fn send(&self, host: &WasmHost<HeadlessBackend>, changes: Vec<Change>) {
		let commits: Vec<CommitRecord> = self.commits.borrow_mut().drain(..).collect();

		if changes.is_empty() && commits.is_empty() {
			return;
		}

		self.source.send(Batch {
			root: host.surface().root().map(|root| root.index() as u64),
			changes,
			commits,
		});
	}

	/// Adds the classes and source of each committed node whose wire node
	/// says something other than what was last sent.
	fn describe(&mut self, host: &WasmHost<HeadlessBackend>, changes: &mut Vec<Change>) {
		let surface = host.surface();
		let mut stack: Vec<NodeId> = surface.root().into_iter().collect();

		while let Some(node) = stack.pop() {
			stack.extend_from_slice(surface.children(node));
			let (classes, source) = host.wire_node(node).map_or(("", None), |wire| {
				(wire.class.as_str(), wire.source.as_deref())
			});
			let unchanged = self
				.details
				.get(&node.index())
				.is_some_and(|(sent, sent_source)| {
					sent == classes && sent_source.as_deref() == source
				});

			if unchanged {
				continue;
			}

			let source = source.map(str::to_owned);
			self.details
				.insert(node.index(), (classes.to_owned(), source.clone()));
			changes.push(Change::Details {
				id: node.index() as u64,
				classes: classes.to_owned(),
				source,
			});
		}
	}

	/// Times commits while the hub asks for it.
	fn profile(&mut self, host: &mut WasmHost<HeadlessBackend>) {
		let profiling = self.source.profiling();

		if profiling != self.profiling {
			self.profiling = profiling;
			host.surface_mut().set_profiling(profiling);
		}
	}
}
