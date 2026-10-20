//! The on-device wasm runtime host.
//!
//! This is the piece that makes code push work like React Native's: the app
//! shipped through the store is a thin native shell, and the *interface* is a
//! wasm module fetched over the air. The host runs that module in
//! [`wasmi`] — a pure interpreter, so it is legal on iOS where JIT
//! compilation is not — and drives a
//! [`NativeSurface`] through a versioned ABI.
//!
//! # The ABI
//!
//! The module imports host functions from the `leptoskit` module. Ids are
//! `i64`s the module chooses; pointers, lengths, and indices are `i32`s
//! read as unsigned.
//!
//! - `node_create(id, kind)` — create a widget
//! - `node_set_props(id, ptr, len)` — set props from JSON in wasm memory
//! - `node_insert(parent, child, index)` — attach a child
//! - `node_remove(parent, child)` — detach a child
//! - `node_delete(id)` — destroy a widget
//! - `commit()` — hand the assembled tree to the renderer
//!
//! The module exports:
//!
//! - `memory` — its linear memory;
//! - `leptoskit_main()` — build and commit the initial tree;
//! - `leptoskit_event(id, kind, ptr, len) -> i32` — rebuild after an event and
//!   return `1` when the event was handled. `ptr`/`len` point at a JSON payload
//!   for change, submit, scroll, toggle, and value-change events, and are `0`
//!   otherwise. Toggle payloads are `{"checked":true}`; value-change payloads
//!   are `{"value":0.5}`, with a JSON boolean and number respectively;
//! - `leptoskit_alloc(len) -> ptr` — *optional*, but required to receive
//!   payload events: without it those dispatches fail with
//!   [`WasmError::MissingExport`]. It returns a non-zero pointer to `len`
//!   writable bytes;
//! - `leptoskit_abi_version() -> i32` — *optional*. When present, the host
//!   calls it on load and refuses a module whose answer differs from
//!   [`ABI_VERSION`]. Modules that omit it are assumed to speak the current
//!   version.
//!
//! Each entry point is fuel-metered, so a bad bundle cannot hang the device;
//! running out surfaces as [`WasmError::OutOfFuel`].
//!
//! ## Tree rules
//!
//! The host keeps the wire tree a tree, and traps the offending call
//! otherwise:
//!
//! - `node_insert` *moves* a child that already has a parent, exactly like the
//!   DOM's `insertBefore`: the child is detached first, then inserted. An
//!   `index` past the end appends.
//! - `node_insert` refuses to put a node inside itself or one of its own
//!   descendants, and refuses to nest deeper than [`MAX_TREE_DEPTH`].
//! - `node_delete` destroys one node and detaches it from its parent; its
//!   children become parentless and must be reattached or deleted.
//! - `commit` renders the single parentless node as the root. Committing with
//!   more than one parentless node traps, and committing with no nodes at all
//!   leaves the previous interface on screen.
//!
//! ## Memory ownership
//!
//! The module owns its memory. `node_set_props` copies the bytes during the
//! call, so the module may reuse them as soon as it returns. Event payloads
//! are written into a buffer the module handed out from `leptoskit_alloc`;
//! the host never frees it, so the module reclaims it once `leptoskit_event`
//! returns.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::collections::BTreeMap;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_yoga::NodeId;
use wasmi::Caller;
use wasmi::Config;
use wasmi::Engine;
use wasmi::Extern;
use wasmi::Linker;
use wasmi::Memory;
use wasmi::Module;
use wasmi::Store;
use wasmi::TrapCode;
use wasmi::Val;

mod plugins;

/// The ABI version this host speaks.
pub const ABI_VERSION: u32 = 1;

/// The wire form of a node's properties: the native props plus its style.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct WireNode {
	/// The widget properties.
	#[serde(flatten)]
	pub props: NativeProps,
	/// The layout style.
	#[serde(default)]
	pub style: leptoskit_yoga::Style,
	/// Built-in Tailwind utilities, validated before a tree is committed.
	#[serde(default)]
	pub class: String,
}

/// The payload delivered with a text-change or submit event.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct WireEvent {
	/// A string value, for change and submit events.
	#[serde(default)]
	pub value: String,
}

/// The event kind codes crossing the ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
	/// Activation.
	Press,
	/// Content changed.
	Change,
	/// Content submitted.
	Submit,
	/// Focus gained.
	Focus,
	/// Focus lost.
	Blur,
	/// Scrolled.
	Scroll,
	/// A boolean control proposed a checked state.
	Toggle,
	/// A numeric control proposed a finite value.
	ValueChange,
}

impl EventKind {
	/// The ABI code for the kind.
	pub fn code(self) -> i32 {
		match self {
			Self::Press => 0,
			Self::Change => 1,
			Self::Submit => 2,
			Self::Focus => 3,
			Self::Blur => 4,
			Self::Scroll => 5,
			Self::Toggle => 6,
			Self::ValueChange => 7,
		}
	}
}

/// The deepest wire tree the host accepts, counting the root as level one.
///
/// Assembly validates this bound before constructing nested elements. Later
/// rendering stages recurse through the tree, so the limit also bounds their
/// stack usage. Real interfaces sit far below it.
pub const MAX_TREE_DEPTH: usize = 256;

/// The fuel granted to each wasm entry point unless
/// [`WasmHost::set_fuel_budget`] overrides it: enough for a rebuild, small
/// enough that an infinite loop in a bad bundle gives up.
pub const DEFAULT_FUEL_BUDGET: u64 = 2_000_000_000;

/// A tree the module committed, waiting for the host loop to render it.
#[derive(Debug)]
struct PendingCommit {
	tree: Element,
	/// The wire id of every node in `tree`, in pre-order.
	wire_ids: Vec<i64>,
}

/// What the host accumulates between `node_*` calls and a `commit`.
///
/// `children` and `parents` always describe a forest: every node has at most
/// one parent and no node is its own ancestor.
#[derive(Debug, Default)]
struct HostState {
	plugins: leptoskit_bridge::Bridge,
	nodes: BTreeMap<i64, WireNode>,
	kinds: BTreeMap<i64, NativeKind>,
	children: BTreeMap<i64, Vec<i64>>,
	parents: BTreeMap<i64, i64>,
	pending: Option<PendingCommit>,
	commits: u32,
	/// Wire id by committed node, resynced after commits and environment
	/// changes.
	wire_by_node: BTreeMap<usize, i64>,
}

impl HostState {
	/// Detaches `child` from its parent, if it has one.
	fn detach(&mut self, child: i64) {
		if let Some(parent) = self.parents.remove(&child)
			&& let Some(siblings) = self.children.get_mut(&parent)
		{
			siblings.retain(|&existing| existing != child);
		}
	}

	/// Validates a pre-order traversal before building nested elements from
	/// the leaves upward. Both traversals use heap storage so even an invalid
	/// tree is rejected without recursing through its untrusted depth.
	fn assemble(&self, root: i64) -> Result<PendingCommit, wasmi::Error> {
		let mut stack = vec![(root, 1)];
		let mut wire_ids = Vec::new();
		let mut elements = Vec::new();

		while let Some((id, depth)) = stack.pop() {
			if depth > MAX_TREE_DEPTH {
				return Err(host_trap(format!(
					"the committed tree is deeper than {MAX_TREE_DEPTH} levels"
				)));
			}

			let wire = self.nodes.get(&id).cloned().unwrap_or_default();
			let mut element =
				Element::new(self.kinds.get(&id).copied().unwrap_or(NativeKind::View));
			element.props = wire.props;
			element.style = wire.style;
			element = element
				.try_class(&wire.class)
				.map_err(|error| host_trap(format!("node {id}: {error}")))?;
			let children = self.children.get(&id).map_or(&[][..], Vec::as_slice);
			wire_ids.push(id);
			elements.push((element, children.len()));
			stack.extend(children.iter().rev().map(|&child| (child, depth + 1)));
		}

		let mut assembled = Vec::new();

		for (mut element, child_count) in elements.into_iter().rev() {
			for _ in 0..child_count {
				element
					.children
					.push(assembled.pop().expect("children precede their parent"));
			}

			assembled.push(element);
		}

		let tree = assembled.pop().expect("the traversal includes its root");
		debug_assert!(assembled.is_empty(), "the traversal describes one tree");
		tree.validate()
			.map_err(|error| host_trap(error.to_string()))?;
		Ok(PendingCommit { tree, wire_ids })
	}
}

/// Errors raised by the host.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
	/// The module could not be compiled or instantiated.
	#[error("could not compile the wasm module: {0}")]
	Compile(String),
	/// The module, or a host function it called, trapped.
	#[error("host call failed: {0}")]
	Trap(String),
	/// The module does not export what the ABI requires.
	#[error("missing export `{0}`; the module does not implement the leptoskit ABI")]
	MissingExport(String),
	/// The module declares an ABI version this host does not speak.
	#[error("the module speaks leptoskit ABI version {found}, but this host speaks {expected}")]
	AbiMismatch {
		/// [`ABI_VERSION`].
		expected: u32,
		/// What the module's `leptoskit_abi_version` returned.
		found: u32,
	},
	/// The module ran out of fuel.
	#[error("the module exhausted its fuel budget")]
	OutOfFuel,
}

impl From<wasmi::Error> for WasmError {
	fn from(error: wasmi::Error) -> Self {
		if error.as_trap_code() == Some(TrapCode::OutOfFuel) {
			Self::OutOfFuel
		} else {
			Self::Trap(error.to_string())
		}
	}
}

/// A host-side trap, for errors raised inside imported functions.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct HostTrap(String);

impl wasmi::errors::HostError for HostTrap {}

impl From<wasmi::errors::LinkerError> for WasmError {
	fn from(error: wasmi::errors::LinkerError) -> Self {
		Self::Trap(error.to_string())
	}
}

fn host_trap(message: impl Into<String>) -> wasmi::Error {
	wasmi::Error::host(HostTrap(message.into()))
}

/// Runs one wasm app against a native backend.
pub struct WasmHost<B: NativeBackend> {
	surface: NativeSurface<B>,
	/// The last committed wire tree's ids in pre-order, independent of the
	/// guest's mutable forest and of nodes recreated by responsive styles.
	wire_ids: Vec<i64>,
	store: Store<HostState>,
	instance: wasmi::Instance,
	main_fn: wasmi::Func,
	event_fn: wasmi::Func,
	alloc_fn: Option<wasmi::Func>,
	/// Fuel granted per wasm entry point.
	fuel_budget: u64,
}

impl WasmHost<HeadlessBackend> {
	/// A host over the recording backend, for tests and tooling.
	pub fn headless(wasm: &[u8]) -> Result<Self, WasmError> {
		Self::new(HeadlessBackend::new(), wasm)
	}
}

impl<B: NativeBackend> WasmHost<B> {
	/// Compiles the module, links the host functions, and checks the module's
	/// declared ABI version when it exports one.
	pub fn new(backend: B, wasm: &[u8]) -> Result<Self, WasmError> {
		let mut config = Config::default();
		config.consume_fuel(true);
		let engine = Engine::new(&config);
		let module =
			Module::new(&engine, wasm).map_err(|error| WasmError::Compile(error.to_string()))?;
		let mut store = Store::new(&engine, HostState::default());
		store
			.set_fuel(DEFAULT_FUEL_BUDGET)
			.map_err(WasmError::from)?;

		let mut linker: Linker<HostState> = Linker::new(&engine);
		linker.func_wrap("leptoskit", "node_create", node_create)?;
		linker.func_wrap("leptoskit", "node_set_props", node_set_props)?;
		linker.func_wrap("leptoskit", "node_insert", node_insert)?;
		linker.func_wrap("leptoskit", "node_remove", node_remove)?;
		linker.func_wrap("leptoskit", "node_delete", node_delete)?;
		linker.func_wrap("leptoskit", "commit", commit)?;
		plugins::link(&mut linker)?;

		let instance = linker
			.instantiate_and_start(&mut store, &module)
			.map_err(|error| {
				match WasmError::from(error) {
					WasmError::Trap(message) => WasmError::Compile(message),
					other => other,
				}
			})?;

		let main_fn = export_func(instance, &store, "leptoskit_main")?;
		let event_fn = export_func(instance, &store, "leptoskit_event")?;
		let alloc_fn = instance
			.get_export(&store, "leptoskit_alloc")
			.and_then(Extern::into_func);
		if let Some(version_fn) = instance
			.get_export(&store, "leptoskit_abi_version")
			.and_then(Extern::into_func)
		{
			let mut result = [Val::I32(0)];
			version_fn.call(&mut store, &[], &mut result)?;
			let Val::I32(found) = result[0] else {
				return Err(WasmError::Trap(
					"leptoskit_abi_version must return an i32".to_owned(),
				));
			};
			let found = found as u32;
			if found != ABI_VERSION {
				return Err(WasmError::AbiMismatch {
					expected: ABI_VERSION,
					found,
				});
			}
		}

		Ok(Self {
			surface: NativeSurface::new(backend),
			wire_ids: Vec::new(),
			store,
			instance,
			main_fn,
			event_fn,
			alloc_fn,
			fuel_budget: DEFAULT_FUEL_BUDGET,
		})
	}

	/// The renderer surface, for layout queries and backend access.
	pub fn surface(&self) -> &NativeSurface<B> {
		&self.surface
	}

	/// Updates the viewport and re-evaluates retained responsive utilities.
	/// Any replacement widgets keep their original guest event targets.
	pub fn set_viewport(&mut self, width: f32, height: f32) {
		self.surface.set_viewport(width, height);
		self.resync_map();
	}

	/// Updates the host appearance and re-evaluates retained `dark:` utilities.
	/// Any replacement widgets keep their original guest event targets.
	pub fn set_dark_mode(&mut self, dark: bool) {
		self.surface.set_dark_mode(dark);
		self.resync_map();
	}

	/// Host-owned plugin transport. Register and grant capabilities before
	/// [`Self::start`], then drain cancellations and requests after each guest
	/// entry point.
	///
	/// Native callbacks complete calls through this transport on the owning
	/// thread. Call [`Self::wake_plugins`] to let the guest consume responses
	/// and events, avoiding an idle polling loop.
	pub fn plugins(&mut self) -> &mut leptoskit_bridge::Bridge {
		&mut self.store.data_mut().plugins
	}

	/// Notifies the guest that plugin replies or events are available.
	///
	/// Plugin-aware guests export `leptoskit_plugins_ready()`. The callback
	/// is fuel-metered and can update the UI through the usual rendering ABI.
	pub fn wake_plugins(&mut self) -> Result<(), WasmError> {
		let callback = export_func(self.instance, &self.store, "leptoskit_plugins_ready")?;
		self.refuel()?;
		callback.call(&mut self.store, &[], &mut [])?;
		self.drain_commit();
		Ok(())
	}

	/// The number of commits the module has made.
	pub fn commits(&self) -> u32 {
		self.store.data().commits
	}

	/// Sets the fuel granted to each later entry-point call, replacing
	/// [`DEFAULT_FUEL_BUDGET`].
	pub fn set_fuel_budget(&mut self, fuel: u64) {
		self.fuel_budget = fuel;
	}

	/// Runs `leptoskit_main` and commits the initial tree.
	pub fn start(&mut self, width: f32, height: f32) -> Result<(), WasmError> {
		self.set_viewport(width, height);
		self.refuel()?;
		self.main_fn
			.call(&mut self.store, &[], &mut [])
			.map_err(WasmError::from)?;
		self.drain_commit();
		Ok(())
	}

	/// Delivers an event to the module at the node the surface committed.
	///
	/// Payload events need the module's `leptoskit_alloc` export. Control
	/// proposals are checked against the committed widget kind and slider
	/// configuration; application state remains authoritative after dispatch.
	pub fn dispatch(
		&mut self,
		target: NodeId,
		event: &NativeEvent,
	) -> Result<EventOutcome, WasmError> {
		let Some(wire_id) = self.store.data().wire_by_node.get(&target.index()).copied() else {
			return Ok(EventOutcome::Unhandled);
		};
		let mut ancestor = Some(target);

		while let Some(node) = ancestor {
			let props = self.surface.props(node);

			if props.hidden == Some(true) || props.enabled == Some(false) {
				return Ok(EventOutcome::Unhandled);
			}

			ancestor = self.surface.parent(node);
		}

		let (kind, payload) = match event {
			NativeEvent::Press => (EventKind::Press, None),
			NativeEvent::Focus => (EventKind::Focus, None),
			NativeEvent::Blur => (EventKind::Blur, None),
			NativeEvent::Scroll { offset_x, offset_y } => {
				(
					EventKind::Scroll,
					Some(format!(r#"{{"x":{offset_x},"y":{offset_y}}}"#)),
				)
			}
			NativeEvent::Change { value } => (EventKind::Change, Some(serialize_value(value))),
			NativeEvent::Submit { value } => (EventKind::Submit, Some(serialize_value(value))),
			NativeEvent::Toggle { checked } => {
				let target_kind = self.surface.kind(target);

				if !matches!(
					target_kind,
					NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
				) || (target_kind == NativeKind::Radio && !checked)
				{
					return Ok(EventOutcome::Unhandled);
				}

				(
					EventKind::Toggle,
					Some(format!(r#"{{"checked":{checked}}}"#)),
				)
			}
			NativeEvent::ValueChange { value } => {
				if self.surface.kind(target) != NativeKind::Slider {
					return Ok(EventOutcome::Unhandled);
				}

				let Some(value) = self
					.surface
					.props(target)
					.slider
					.unwrap_or_default()
					.proposed_value(*value)
				else {
					return Ok(EventOutcome::Unhandled);
				};

				(
					EventKind::ValueChange,
					Some(format!(r#"{{"value":{value}}}"#)),
				)
			}
		};
		self.dispatch_wire(wire_id, kind, payload.as_deref())
	}

	/// Delivers an event by wire id, with an optional JSON payload.
	///
	/// A payload is copied into a buffer from the module's `leptoskit_alloc`
	/// export, which must exist when `payload` is `Some`.
	/// This low-level entry point calls the guest directly; [`Self::dispatch`]
	/// applies native visibility and enabled-state checks.
	pub fn dispatch_wire(
		&mut self,
		wire_id: i64,
		kind: EventKind,
		payload: Option<&str>,
	) -> Result<EventOutcome, WasmError> {
		let (ptr, len) = match payload {
			Some(payload) => self.write_into_wasm(payload.as_bytes())?,
			None => (0, 0),
		};
		self.refuel()?;
		let mut results = [Val::I32(0)];
		self.event_fn
			.call(
				&mut self.store,
				&[
					Val::I64(wire_id),
					Val::I32(kind.code()),
					Val::I32(ptr as i32),
					Val::I32(len as i32),
				],
				&mut results,
			)
			.map_err(WasmError::from)?;
		self.drain_commit();
		let handled = matches!(results[0], Val::I32(1));
		Ok(if handled {
			EventOutcome::Handled
		} else {
			EventOutcome::Unhandled
		})
	}

	fn refuel(&mut self) -> Result<(), WasmError> {
		self.store
			.set_fuel(self.fuel_budget)
			.map_err(WasmError::from)
	}

	/// Writes bytes into the module's memory through its allocator, returning
	/// the guest pointer and length.
	fn write_into_wasm(&mut self, bytes: &[u8]) -> Result<(u32, u32), WasmError> {
		let alloc = self
			.alloc_fn
			.ok_or_else(|| WasmError::MissingExport("leptoskit_alloc".to_owned()))?;
		let len = u32::try_from(bytes.len())
			.map_err(|_| WasmError::Trap("event payload exceeds 4 GiB".to_owned()))?;
		let mut pointer = [Val::I32(0)];
		self.refuel()?;
		alloc
			.call(&mut self.store, &[Val::I32(len as i32)], &mut pointer)
			.map_err(WasmError::from)?;
		let Val::I32(ptr) = pointer[0] else {
			return Err(WasmError::Trap("allocator returned no pointer".to_owned()));
		};
		let ptr = ptr as u32;
		if ptr == 0 {
			return Err(WasmError::Trap("allocator refused the request".to_owned()));
		}
		let memory = export_memory(self.instance, &self.store)?;
		memory
			.write(&mut self.store, ptr as usize, bytes)
			.map_err(|error| WasmError::Trap(error.to_string()))?;
		Ok((ptr, len))
	}

	/// Hands a pending committed tree to the surface and resyncs the id map.
	fn drain_commit(&mut self) {
		if let Some(pending) = self.store.data_mut().pending.take() {
			self.surface.set_root(&pending.tree);
			self.wire_ids = pending.wire_ids;
			self.resync_map();
		}
	}

	/// Pairs the committed surface nodes with the wire ids they were built
	/// from. The surface mirrors the committed element tree exactly, so a
	/// pre-order walk of each lines up one to one.
	fn resync_map(&mut self) {
		let mut map = BTreeMap::new();
		let mut wire_ids = self.wire_ids.iter().copied();
		let mut stack: Vec<NodeId> = self.surface.root().into_iter().collect();
		while let Some(node) = stack.pop() {
			let wire_id = wire_ids.next().expect("surface and wire tree diverged");
			map.insert(node.index(), wire_id);
			stack.extend(self.surface.children(node).iter().rev());
		}
		debug_assert!(wire_ids.next().is_none(), "surface and wire tree diverged");
		self.store.data_mut().wire_by_node = map;
	}
}

fn export_func(
	instance: wasmi::Instance,
	store: &Store<HostState>,
	name: &str,
) -> Result<wasmi::Func, WasmError> {
	instance
		.get_export(store, name)
		.and_then(Extern::into_func)
		.ok_or_else(|| WasmError::MissingExport(name.to_owned()))
}

fn export_memory(instance: wasmi::Instance, store: &Store<HostState>) -> Result<Memory, WasmError> {
	instance
		.get_export(store, "memory")
		.and_then(Extern::into_memory)
		.ok_or_else(|| WasmError::MissingExport("memory".to_owned()))
}

fn node_create(mut caller: Caller<'_, HostState>, id: i64, kind: i32) {
	let kind = match kind {
		1 => NativeKind::Text,
		2 => NativeKind::TextRun,
		3 => NativeKind::Image,
		4 => NativeKind::ScrollView,
		5 => NativeKind::TextInput,
		6 => NativeKind::Button,
		7 => NativeKind::Switch,
		8 => NativeKind::Checkbox,
		9 => NativeKind::Radio,
		10 => NativeKind::Slider,
		11 => NativeKind::LinearProgressIndicator,
		12 => NativeKind::ActivityIndicator,
		13 => NativeKind::PasswordInput,
		_ => NativeKind::View,
	};
	caller.data_mut().nodes.entry(id).or_default();
	caller.data_mut().kinds.insert(id, kind);
}

fn node_set_props(
	mut caller: Caller<'_, HostState>,
	id: i64,
	ptr: i32,
	len: i32,
) -> Result<(), wasmi::Error> {
	let bytes = read_memory(&mut caller, ptr, len)?;
	let wire: WireNode =
		serde_json::from_slice(&bytes).map_err(|error| host_trap(error.to_string()))?;
	caller.data_mut().nodes.insert(id, wire);
	Ok(())
}

fn node_insert(
	mut caller: Caller<'_, HostState>,
	parent: i64,
	child: i64,
	index: i32,
) -> Result<(), wasmi::Error> {
	let state = caller.data_mut();
	// Walk up from the parent: finding the child means the insert would close
	// a cycle. The walk is bounded, so a hostile chain cannot stall the host.
	let mut ancestor = Some(parent);
	let mut depth = 0usize;
	while let Some(id) = ancestor {
		if id == child {
			return Err(host_trap(format!(
				"node_insert({parent}, {child}) would make node {child} its own ancestor"
			)));
		}
		depth += 1;
		if depth >= MAX_TREE_DEPTH {
			return Err(host_trap(format!(
				"node_insert({parent}, {child}) would nest deeper than {MAX_TREE_DEPTH} levels"
			)));
		}
		ancestor = state.parents.get(&id).copied();
	}

	state.detach(child);
	state.nodes.entry(parent).or_default();
	state.nodes.entry(child).or_default();
	let siblings = state.children.entry(parent).or_default();
	let index = (index as u32 as usize).min(siblings.len());
	siblings.insert(index, child);
	state.parents.insert(child, parent);
	Ok(())
}

fn node_remove(mut caller: Caller<'_, HostState>, parent: i64, child: i64) {
	let state = caller.data_mut();
	if state.parents.get(&child) == Some(&parent) {
		state.detach(child);
	}
}

fn node_delete(mut caller: Caller<'_, HostState>, id: i64) {
	let state = caller.data_mut();
	state.detach(id);
	for child in state.children.remove(&id).into_iter().flatten() {
		state.parents.remove(&child);
	}
	state.nodes.remove(&id);
	state.kinds.remove(&id);
}

fn commit(mut caller: Caller<'_, HostState>) -> Result<(), wasmi::Error> {
	let state = caller.data();
	let parentless: Vec<i64> = state
		.nodes
		.keys()
		.copied()
		.filter(|id| !state.parents.contains_key(id))
		.collect();
	let root = match parentless.as_slice() {
		[] => return Ok(()),
		[root] => *root,
		many => {
			let shown: Vec<String> = many.iter().take(8).map(i64::to_string).collect();
			let more = if many.len() > shown.len() {
				", …"
			} else {
				""
			};
			return Err(host_trap(format!(
				"commit found {} parentless nodes ({}{more}); exactly one root may be committed, \
				 so attach or delete the others first",
				many.len(),
				shown.join(", ")
			)));
		}
	};
	let pending = state.assemble(root)?;
	let state = caller.data_mut();
	state.pending = Some(pending);
	state.commits += 1;
	Ok(())
}

/// Copies `len` bytes at `ptr` out of the module's memory, trapping when the
/// range is out of bounds. Both values are unsigned.
fn read_memory(
	caller: &mut Caller<'_, HostState>,
	ptr: i32,
	len: i32,
) -> Result<Vec<u8>, wasmi::Error> {
	let start = ptr as u32 as usize;
	let len = len as u32 as usize;
	if len == 0 {
		return Ok(Vec::new());
	}
	let memory = caller
		.get_export("memory")
		.and_then(Extern::into_memory)
		.ok_or_else(|| host_trap("the module does not export memory"))?;
	start
		.checked_add(len)
		.and_then(|end| memory.data(&*caller).get(start..end))
		.map(<[u8]>::to_vec)
		.ok_or_else(|| host_trap(format!("props range {start}+{len} is out of bounds")))
}

fn serialize_value(value: &str) -> String {
	format!(
		r#"{{"value":{}}}"#,
		serde_json::Value::String(value.to_owned())
	)
}
