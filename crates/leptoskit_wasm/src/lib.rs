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
use wasmi::TypedFunc;
use wasmi::Val;

mod imports;
mod plugins;
mod shims;

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
	memory: Option<Memory>,
	nodes: BTreeMap<i64, WireNode>,
	kinds: BTreeMap<i64, NativeKind>,
	children: BTreeMap<i64, Vec<i64>>,
	parents: BTreeMap<i64, i64>,
	pending: Option<PendingCommit>,
	commits: u32,
	/// The notional length of the wasm-bindgen externref table, grown by
	/// [`shims`] when a guest links wasm-bindgen's runtime.
	externref_table: i32,
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
	identity: HostIdentity,
	surface: NativeSurface<B>,
	/// The last committed wire tree's ids in pre-order, independent of the
	/// guest's mutable forest and of nodes recreated by responsive styles.
	wire_ids: Vec<i64>,
	store: Store<HostState>,
	main_fn: wasmi::Func,
	event_fn: wasmi::Func,
	alloc_fn: Option<wasmi::Func>,
	// Keep signature errors until wake: non-plugin modules need not export
	// callbacks, and optional poll errors must follow the readiness callback.
	plugins_ready: Option<Result<TypedFunc<(), ()>, wasmi::Error>>,
	plugins_poll: Option<Result<TypedFunc<(), ()>, wasmi::Error>>,
	/// Fuel granted per wasm entry point.
	fuel_budget: u64,
}

/// Opaque interpreter identity, stable across moves and retained completions.
///
/// Cloning a handle retains only its identity token, never the interpreter.
/// Compare this identity and the current Bridge identity when binding work:
/// [`WasmHost::plugins`] permits replacing or moving the entire transport.
///
/// ```compile_fail,E0423
/// use leptoskit_wasm::HostIdentity;
/// let identity = HostIdentity(std::sync::Arc::new(()));
/// ```
#[derive(Debug, Clone)]
pub struct HostIdentity(std::sync::Arc<()>);

impl PartialEq for HostIdentity {
	fn eq(&self, other: &Self) -> bool {
		std::sync::Arc::ptr_eq(&self.0, &other.0)
	}
}

impl Eq for HostIdentity {}

impl WasmHost<HeadlessBackend> {
	/// A host over the recording backend, for tests and tooling.
	pub fn headless(wasm: &[u8]) -> Result<Self, WasmError> {
		Self::new(HeadlessBackend::new(), wasm)
	}

	/// Drains committed changes for an embedding platform's widget adapter.
	pub fn take_mutations(&mut self) -> Vec<leptoskit_native::Mutation> {
		self.surface.backend_mut().take_log()
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
		imports::link(&mut linker)?;
		plugins::link(&mut linker)?;
		shims::link(&mut linker, &module)?;

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
		let plugins_ready = instance
			.get_export(&store, "leptoskit_plugins_ready")
			.and_then(Extern::into_func)
			.map(|function| function.typed(&store));
		let plugins_poll = instance
			.get_export(&store, "leptoskit_poll")
			.and_then(Extern::into_func)
			.map(|function| function.typed(&store));
		store.data_mut().memory = instance
			.get_export(&store, "memory")
			.and_then(Extern::into_memory);

		validate_abi(instance, &mut store)?;

		Ok(Self {
			identity: HostIdentity(std::sync::Arc::new(())),
			surface: NativeSurface::new(backend),
			wire_ids: Vec::new(),
			store,
			main_fn,
			event_fn,
			alloc_fn,
			plugins_ready,
			plugins_poll,
			fuel_budget: DEFAULT_FUEL_BUDGET,
		})
	}

	/// Identifies this interpreter without retaining its guest state or a
	/// borrow.
	pub fn identity(&self) -> HostIdentity {
		self.identity.clone()
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

	/// Refreshes native font metrics after the host's font or DPI changes.
	/// Any replacement widgets retain their original guest event targets.
	pub fn refresh_text_measurer(&mut self) {
		self.surface.refresh_text_measurer();
		self.resync_map();
	}

	/// Changes native backend settings and refreshes layout and event targets.
	/// Hosts use this when a window moves to a display with a different DPI.
	pub fn update_backend(&mut self, update: impl FnOnce(&mut B)) {
		update(self.surface.backend_mut());
		self.refresh_text_measurer();
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
		let callback = *self
			.plugins_ready
			.as_ref()
			.ok_or_else(|| WasmError::MissingExport("leptoskit_plugins_ready".to_owned()))?
			.as_ref()
			.map_err(|error| WasmError::Trap(error.to_string()))?;
		self.refuel()?;
		callback.call(&mut self.store, ())?;

		if let Some(poll) = &self.plugins_poll {
			poll.as_ref()
				.map_err(|error| WasmError::Trap(error.to_string()))?
				.call(&mut self.store, ())?;
		}

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

			if props.hidden == Some(true)
				|| props.enabled == Some(false)
				|| (self.surface.kind(node) == NativeKind::ActivityIndicator
					&& props.running == Some(false))
			{
				return Ok(EventOutcome::Unhandled);
			}

			ancestor = self.surface.parent(node);
		}

		let Some((kind, payload)) = self.event_payload(target, event) else {
			return Ok(EventOutcome::Unhandled);
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

	fn event_payload(
		&self,
		target: NodeId,
		event: &NativeEvent,
	) -> Option<(EventKind, Option<String>)> {
		Some(match event {
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
				(
					EventKind::Toggle,
					Some(self.toggle_payload(target, *checked)?),
				)
			}
			NativeEvent::ValueChange { value } => {
				(
					EventKind::ValueChange,
					Some(self.value_payload(target, *value)?),
				)
			}
		})
	}

	fn toggle_payload(&self, target: NodeId, checked: bool) -> Option<String> {
		let kind = self.surface.kind(target);

		if !matches!(
			kind,
			NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
		) || (kind == NativeKind::Radio && !checked)
		{
			return None;
		}

		Some(format!(r#"{{"checked":{checked}}}"#))
	}

	fn value_payload(&self, target: NodeId, value: f64) -> Option<String> {
		if self.surface.kind(target) != NativeKind::Slider {
			return None;
		}

		let value = self
			.surface
			.props(target)
			.slider
			.unwrap_or_default()
			.proposed_value(value)?;

		Some(format!(r#"{{"value":{value}}}"#))
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

		let memory = self
			.store
			.data()
			.memory
			.ok_or_else(|| WasmError::MissingExport("memory".to_owned()))?;
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

fn caller_memory(caller: &mut Caller<'_, HostState>) -> Option<Memory> {
	if let Some(memory) = caller.data().memory {
		return Some(memory);
	}

	// A module's start function can call imports before instantiation returns.
	// Cache its stable memory handle then, never a pointer invalidated by grow.
	let memory = caller.get_export("memory").and_then(Extern::into_memory)?;
	caller.data_mut().memory = Some(memory);
	Some(memory)
}

fn serialize_value(value: &str) -> String {
	format!(
		r#"{{"value":{}}}"#,
		serde_json::Value::String(value.to_owned())
	)
}

fn validate_abi(instance: wasmi::Instance, store: &mut Store<HostState>) -> Result<(), WasmError> {
	if let Some(version_fn) = instance
		.get_export(&*store, "leptoskit_abi_version")
		.and_then(Extern::into_func)
	{
		let mut result = [Val::I32(0)];
		version_fn.call(&mut *store, &[], &mut result)?;
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
	Ok(())
}
