//! The on-device wasm runtime host.
//!
//! This is the piece that makes code push work like React Native's: the app
//! shipped through the store is a thin native shell, and the *interface* is a
//! wasm module fetched over the air. The host runs that module in
//! [`wasmi`] — a pure interpreter, so it is legal on iOS where JIT
//! compilation is not — and drives a
//! [`NativeSurface`](leptoskit_native::NativeSurface) through a versioned ABI.
//!
//! # The ABI
//!
//! The module imports host functions from the `leptoskit` module:
//!
//! - `node_create(id, kind)` — create a widget
//! - `node_set_props(id, ptr, len)` — set props from JSON in wasm memory
//! - `node_insert(parent, child, index)` — attach a child
//! - `node_remove(parent, child)` — detach a child
//! - `node_delete(id)` — destroy a widget
//! - `commit()` — hand the assembled tree to the renderer
//!
//! The module exports `memory`, `leptoskit_main()` (build and commit the
//! initial tree), and `leptoskit_event(id, kind, ptr, len)` (rebuild after an
//! event; `ptr`/`len` point at a JSON payload for text events). Each call is
//! fuel-metered so a bad bundle cannot hang the device.

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
use wasmi::Val;

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
}

/// The payload delivered with an event.
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
		}
	}
}

/// What the host accumulates between `node_*` calls and a `commit`.
#[derive(Debug, Default)]
struct HostState {
	nodes: BTreeMap<i64, WireNode>,
	kinds: BTreeMap<i64, NativeKind>,
	children: BTreeMap<i64, Vec<i64>>,
	root: Option<i64>,
	/// The assembled tree waiting for the host loop to hand it to the surface.
	pending: Option<Element>,
	commits: u32,
	/// Wire id by committed node, resynced after every commit.
	wire_by_node: BTreeMap<usize, i64>,
}

/// Errors raised by the host.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
	/// The module could not be compiled.
	#[error("could not compile the wasm module: {0}")]
	Compile(String),
	/// A host function trapped.
	#[error("host call failed: {0}")]
	Trap(String),
	/// The module does not export what the ABI requires.
	#[error("missing export `{0}`; the module does not implement the leptoskit ABI")]
	MissingExport(String),
	/// A prop payload failed to parse.
	#[error("could not parse node props: {0}")]
	Props(String),
	/// An event payload failed to parse.
	#[error("could not parse event payload: {0}")]
	Event(String),
	/// The module ran out of fuel.
	#[error("the module exhausted its fuel budget")]
	OutOfFuel,
}

impl From<wasmi::Error> for WasmError {
	fn from(error: wasmi::Error) -> Self {
		Self::Trap(error.to_string())
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
	/// Compiles the module and links the host functions.
	pub fn new(backend: B, wasm: &[u8]) -> Result<Self, WasmError> {
		let mut config = Config::default();
		config.consume_fuel(true);
		let engine = Engine::new(&config);
		let module =
			Module::new(&engine, wasm).map_err(|error| WasmError::Compile(error.to_string()))?;
		let mut store = Store::new(&engine, HostState::default());
		// A generous per-entry budget: enough for a rebuild, small enough that
		// an infinite loop in a bad bundle gives up.
		store.set_fuel(2_000_000_000).map_err(WasmError::from)?;

		let mut linker: Linker<HostState> = Linker::new(&engine);
		linker.func_wrap("leptoskit", "node_create", node_create)?;
		linker.func_wrap("leptoskit", "node_set_props", node_set_props)?;
		linker.func_wrap("leptoskit", "node_insert", node_insert)?;
		linker.func_wrap("leptoskit", "node_remove", node_remove)?;
		linker.func_wrap("leptoskit", "node_delete", node_delete)?;
		linker.func_wrap("leptoskit", "commit", commit)?;

		let instance = linker
			.instantiate_and_start(&mut store, &module)
			.map_err(|error| WasmError::Compile(error.to_string()))?;

		let main_fn = export_func(instance, &store, "leptoskit_main")?;
		let event_fn = export_func(instance, &store, "leptoskit_event")?;
		let alloc_fn = instance
			.get_export(&store, "leptoskit_alloc")
			.and_then(Extern::into_func);

		Ok(Self {
			surface: NativeSurface::new(backend),
			store,
			instance,
			main_fn,
			event_fn,
			alloc_fn,
			fuel_budget: 2_000_000_000,
		})
	}

	/// The renderer surface, for layout queries and backend access.
	pub fn surface(&self) -> &NativeSurface<B> {
		&self.surface
	}

	/// The number of commits the module has made.
	pub fn commits(&self) -> u32 {
		self.store.data().commits
	}

	/// Runs `leptoskit_main` and commits the initial tree.
	pub fn start(&mut self, width: f32, height: f32) -> Result<(), WasmError> {
		self.surface.set_viewport(width, height);
		self.refuel()?;
		self.main_fn
			.call(&mut self.store, &[], &mut [])
			.map_err(WasmError::from)?;
		self.drain_commit();
		Ok(())
	}

	/// Delivers an event to the module at the node the surface committed.
	pub fn dispatch(
		&mut self,
		target: NodeId,
		event: &NativeEvent,
	) -> Result<EventOutcome, WasmError> {
		let Some(wire_id) = self.store.data().wire_by_node.get(&target.index()).copied() else {
			return Ok(EventOutcome::Unhandled);
		};
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
		};
		self.dispatch_wire(wire_id, kind, payload.as_deref())
	}

	/// Delivers an event by wire id, with an optional JSON payload.
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
					Val::I32(ptr),
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

	/// Writes bytes into the module's memory through its allocator.
	fn write_into_wasm(&mut self, bytes: &[u8]) -> Result<(i32, usize), WasmError> {
		let alloc = self
			.alloc_fn
			.ok_or_else(|| WasmError::MissingExport("leptoskit_alloc".to_owned()))?;
		let mut pointer = [Val::I32(0)];
		self.refuel()?;
		alloc
			.call(
				&mut self.store,
				&[Val::I32(bytes.len() as i32)],
				&mut pointer,
			)
			.map_err(WasmError::from)?;
		let Val::I32(ptr) = pointer[0] else {
			return Err(WasmError::Trap("allocator returned no pointer".to_owned()));
		};
		if ptr <= 0 {
			return Err(WasmError::Trap("allocator refused the request".to_owned()));
		}
		let memory = export_memory(self.instance, &self.store)?;
		memory
			.write(&mut self.store, ptr as usize, bytes)
			.map_err(|error| WasmError::Trap(error.to_string()))?;
		Ok((ptr, bytes.len()))
	}

	/// Hands a pending committed tree to the surface and resyncs the id map.
	fn drain_commit(&mut self) {
		if let Some(tree) = self.store.data_mut().pending.take() {
			self.surface.set_root(&tree);
			self.resync_map();
		}
	}

	fn resync_map(&mut self) {
		let mut map = BTreeMap::new();
		if let Some(root) = self.surface.root() {
			let wire_root = self.store.data().root.unwrap_or_default();
			walk_and_map(
				&self.surface,
				root,
				wire_root,
				&self.store.data().children,
				&mut map,
			);
		}
		self.store.data_mut().wire_by_node = map;
	}
}

fn walk_and_map<B: NativeBackend>(
	surface: &NativeSurface<B>,
	node: NodeId,
	wire_id: i64,
	children: &BTreeMap<i64, Vec<i64>>,
	map: &mut BTreeMap<usize, i64>,
) {
	map.insert(node.index(), wire_id);
	let Some(wire_children) = children.get(&wire_id) else {
		return;
	};
	for (index, child) in surface.children(node).to_vec().iter().enumerate() {
		if let Some(&wire_child) = wire_children.get(index) {
			walk_and_map(surface, *child, wire_child, children, map);
		}
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

fn node_insert(mut caller: Caller<'_, HostState>, parent: i64, child: i64, index: i32) {
	let state = caller.data_mut();
	state
		.children
		.entry(parent)
		.or_default()
		.insert(index.max(0) as usize, child);
	state.root.get_or_insert(parent);
	if state.root == Some(child) {
		state.root = Some(parent);
	}
}

fn node_remove(mut caller: Caller<'_, HostState>, parent: i64, child: i64) {
	let state = caller.data_mut();
	if let Some(children) = state.children.get_mut(&parent) {
		children.retain(|&existing| existing != child);
	}
}

fn node_delete(mut caller: Caller<'_, HostState>, id: i64) {
	let state = caller.data_mut();
	state.nodes.remove(&id);
	state.kinds.remove(&id);
	state.children.remove(&id);
	for children in state.children.values_mut() {
		children.retain(|&existing| existing != id);
	}
}

fn commit(mut caller: Caller<'_, HostState>) {
	let Some(root_id) = caller.data().root else {
		return;
	};
	let state = caller.data_mut();
	let nodes = std::mem::take(&mut state.nodes);
	let children = std::mem::take(&mut state.children);
	let kinds = std::mem::take(&mut state.kinds);
	let tree = assemble(root_id, &nodes, &kinds, &children);
	state.nodes = nodes;
	state.children = children;
	state.kinds = kinds;
	state.pending = Some(tree);
	state.commits += 1;
}

fn assemble(
	id: i64,
	nodes: &BTreeMap<i64, WireNode>,
	kinds: &BTreeMap<i64, NativeKind>,
	children: &BTreeMap<i64, Vec<i64>>,
) -> Element {
	let wire = nodes.get(&id).cloned().unwrap_or_default();
	let kind = kinds.get(&id).copied().unwrap_or(NativeKind::View);
	let mut element = Element::new(kind);
	element.props = wire.props;
	element.style = wire.style;
	if let Some(child_ids) = children.get(&id) {
		for &child in child_ids {
			element
				.children
				.push(assemble(child, nodes, kinds, children));
		}
	}
	element
}

fn read_memory(
	caller: &mut Caller<'_, HostState>,
	ptr: i32,
	len: i32,
) -> Result<Vec<u8>, wasmi::Error> {
	if len <= 0 {
		return Ok(Vec::new());
	}
	let memory = caller
		.get_export("memory")
		.and_then(Extern::into_memory)
		.ok_or_else(|| host_trap("the module does not export memory"))?;
	memory
		.data(&*caller)
		.get(ptr as usize..(ptr as usize) + len as usize)
		.map(<[u8]>::to_vec)
		.ok_or_else(|| host_trap("props pointer out of bounds"))
}

fn serialize_value(value: &str) -> String {
	format!(
		r#"{{"value":{}}}"#,
		serde_json::Value::String(value.to_owned())
	)
}
