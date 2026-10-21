//! The JavaScript baseline: the same guest workload written in JavaScript and
//! run in `QuickJS` — a bytecode interpreter, the same execution class as the
//! Hermes and `JavaScriptCore` engines React Native uses on devices where JIT
//! compilation is unavailable. It drives the same JSON-props ABI into the
//! same surface, so the only variable is the layer running the app logic.
//!
//! Behind the `js` feature so targets without a C toolchain can still build
//! the native and wasm scenarios.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_wasm::WireNode;
use rquickjs::Context;
use rquickjs::Function;
use rquickjs::Runtime;
use rquickjs::function::Func;

use crate::report::Config;
use crate::report::Report;
use crate::report::sample;
use crate::view::ROWS;
use crate::view::VIEWPORT;
use crate::wasm_layer::ROUNDTRIP_EVENTS;

/// The guest, call for call the Rust one in `benchmarks/native/guest`.
const GUEST_JS: &str = r#"
const ROWS = 1000;
const ROOT = 1;
const ROW_BASE = 2;
const LABEL_BASE = 2 + ROWS;
const KIND_VIEW = 0;
const KIND_TEXT = 1;
const PRESS = 0;
const ROW_PROPS = '{"style":{"flex_direction":"Row","padding":{"top":{"Points":4.0},"right":{"Points":8.0},"bottom":{"Points":4.0},"left":{"Points":8.0}}}}';
let count = 0;
let generation = 0;

function labelJson(row, suffix) {
	return '{"text":"Row ' + row + suffix + '","font_size":14}';
}

function leptoskitMain() {
	lk_node_create(ROOT, KIND_VIEW);
	lk_node_set_props(ROOT, '{"style":{"flex_direction":"Column"}}');
	for (let row = 0; row < ROWS; row++) {
		const rowId = ROW_BASE + row;
		const labelId = LABEL_BASE + row;
		lk_node_create(rowId, KIND_VIEW);
		lk_node_create(labelId, KIND_TEXT);
		lk_node_set_props(rowId, ROW_PROPS);
		lk_node_set_props(labelId, labelJson(row, ""));
		lk_node_insert(rowId, labelId, 0);
		lk_node_insert(ROOT, rowId, row);
	}
	lk_commit();
}

function leptoskitEvent(id, kind) {
	if (kind !== PRESS) return 0;
	if (id === ROOT) {
		count += 1;
		lk_node_set_props(LABEL_BASE, labelJson(0, " · " + count));
		lk_commit();
		return 1;
	}
	if (id === ROW_BASE) {
		generation += 1;
		const suffix = " · gen " + generation;
		for (let row = 0; row < ROWS; row++) {
			lk_node_set_props(LABEL_BASE + row, labelJson(row, suffix));
		}
		lk_commit();
		return 1;
	}
	return 0;
}
"#;

/// The host side of the JSON-props ABI for a JavaScript guest: the same wire
/// tree, decode, and assembly the wasm host performs, committed to a
/// headless surface.
struct JsonTreeHost {
	nodes: BTreeMap<i64, (NativeKind, WireNode)>,
	children: BTreeMap<i64, Vec<i64>>,
	parents: BTreeMap<i64, i64>,
	surface: NativeSurface<HeadlessBackend>,
	commits: u32,
	error: Option<String>,
}

fn kind_from_code(code: i32) -> Option<NativeKind> {
	Some(match code {
		0 => NativeKind::View,
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
		14 => NativeKind::Pressable,
		_ => return None,
	})
}

impl JsonTreeHost {
	fn new() -> Self {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
		Self {
			nodes: BTreeMap::new(),
			children: BTreeMap::new(),
			parents: BTreeMap::new(),
			surface,
			commits: 0,
			error: None,
		}
	}

	fn fail(&mut self, message: String) {
		if self.error.is_none() {
			self.error = Some(message);
		}
	}

	fn node_create(&mut self, id: i64, kind: i32) {
		match kind_from_code(kind) {
			Some(kind) => {
				self.nodes.insert(id, (kind, WireNode::default()));
			}
			None => {
				self.fail(format!(
					"js guest created node {id} with unknown kind {kind}"
				));
			}
		}
	}

	fn node_set_props(&mut self, id: i64, json: &str) {
		match serde_json::from_str::<WireNode>(json) {
			Ok(wire) => {
				if let Some(node) = self.nodes.get_mut(&id) {
					node.1 = wire;
				} else {
					self.fail(format!("js guest set props on unknown node {id}"));
				}
			}
			Err(error) => {
				self.fail(format!(
					"js guest sent invalid props for node {id}: {error}"
				));
			}
		}
	}

	fn node_insert(&mut self, parent: i64, child: i64, index: usize) {
		if let Some(previous) = self.parents.insert(child, parent) {
			if let Some(siblings) = self.children.get_mut(&previous) {
				siblings.retain(|existing| *existing != child);
			}
		}
		let siblings = self.children.entry(parent).or_default();
		siblings.insert(index.min(siblings.len()), child);
	}

	fn commit(&mut self) {
		let roots: Vec<i64> = self
			.nodes
			.keys()
			.filter(|id| !self.parents.contains_key(id))
			.copied()
			.collect();
		let [root] = roots.as_slice() else {
			self.fail(format!(
				"js guest committed {} parentless nodes",
				roots.len()
			));
			return;
		};
		match self.assemble(*root) {
			Ok(tree) => {
				self.surface.set_root(&tree);
				self.commits += 1;
			}
			Err(error) => self.fail(error),
		}
	}

	fn assemble(&self, id: i64) -> Result<Element, String> {
		let (kind, wire) = self
			.nodes
			.get(&id)
			.ok_or_else(|| format!("js guest referenced unknown node {id}"))?;
		let mut element = Element::new(*kind)
			.props(wire.props.clone())
			.style(wire.style.clone());
		if !wire.class.is_empty() {
			element = element
				.try_class(&wire.class)
				.map_err(|error| format!("js guest node {id} has invalid classes: {error}"))?;
		}
		if let Some(children) = self.children.get(&id) {
			for child in children {
				element = element.child(self.assemble(*child)?);
			}
		}
		Ok(element)
	}
}

/// A `QuickJS` runtime with the guest loaded and the host functions bound.
struct JsGuest {
	host: Rc<RefCell<JsonTreeHost>>,
	context: Context,
	_runtime: Runtime,
}

impl JsGuest {
	fn new() -> Result<Self, String> {
		let runtime = Runtime::new().map_err(|error| format!("quickjs runtime: {error}"))?;
		let context =
			Context::full(&runtime).map_err(|error| format!("quickjs context: {error}"))?;
		let host = Rc::new(RefCell::new(JsonTreeHost::new()));
		context
			.with(|ctx| -> rquickjs::Result<()> {
				let globals = ctx.globals();
				let create = Rc::clone(&host);
				globals.set(
					"lk_node_create",
					Func::from(move |id: f64, kind: i32| {
						create.borrow_mut().node_create(id as i64, kind);
					}),
				)?;
				let set_props = Rc::clone(&host);
				globals.set(
					"lk_node_set_props",
					Func::from(move |id: f64, json: String| {
						set_props.borrow_mut().node_set_props(id as i64, &json);
					}),
				)?;
				let insert = Rc::clone(&host);
				globals.set(
					"lk_node_insert",
					Func::from(move |parent: f64, child: f64, index: f64| {
						insert.borrow_mut().node_insert(
							parent as i64,
							child as i64,
							index.max(0.0) as usize,
						);
					}),
				)?;
				let commit = Rc::clone(&host);
				globals.set(
					"lk_commit",
					Func::from(move || {
						commit.borrow_mut().commit();
					}),
				)?;
				ctx.eval::<(), _>(GUEST_JS)?;
				Ok(())
			})
			.map_err(|error| format!("quickjs guest load: {error}"))?;
		Ok(Self {
			host,
			context,
			_runtime: runtime,
		})
	}

	fn main(&self) -> Result<(), String> {
		self.context
			.with(|ctx| -> rquickjs::Result<()> {
				let main: Function = ctx.globals().get("leptoskitMain")?;
				main.call::<_, ()>(())
			})
			.map_err(|error| format!("js leptoskitMain: {error}"))?;
		self.check()
	}

	fn event(&self, id: i64, kind: i32) -> Result<i32, String> {
		let handled = self
			.context
			.with(|ctx| -> rquickjs::Result<i32> {
				let event: Function = ctx.globals().get("leptoskitEvent")?;
				event.call((id as f64, kind))
			})
			.map_err(|error| format!("js leptoskitEvent: {error}"))?;
		self.check()?;
		Ok(handled)
	}

	fn check(&self) -> Result<(), String> {
		match self.host.borrow().error.clone() {
			Some(error) => Err(error),
			None => Ok(()),
		}
	}

	fn label(&self, row: usize) -> Result<String, String> {
		let host = self.host.borrow();
		let surface = &host.surface;
		let root = surface.root().ok_or("js guest committed no root")?;
		let row_node = *surface
			.children(root)
			.get(row)
			.ok_or_else(|| format!("js guest has no row {row}"))?;
		let label = *surface
			.children(row_node)
			.first()
			.ok_or_else(|| format!("js row {row} has no label"))?;
		Ok(surface.props(label).text.clone().unwrap_or_default())
	}

	fn rows(&self) -> Result<usize, String> {
		let host = self.host.borrow();
		let root = host.surface.root().ok_or("js guest committed no root")?;
		Ok(host.surface.children(root).len())
	}
}

/// Runs every JavaScript-layer scenario into `report`.
pub fn run(report: &mut Report, config: Config) -> Result<(), String> {
	let instantiate = sample(
		config,
		|| (),
		|()| {
			std::hint::black_box(JsGuest::new().expect("the JavaScript guest loads"));
		},
	);
	report.duration(
		"js.instantiate",
		"js: create a QuickJS runtime and load the guest script",
		instantiate,
	);

	let start = sample(
		config,
		|| JsGuest::new().expect("the JavaScript guest loads"),
		|guest| guest.main().expect("the JavaScript guest starts"),
	);
	report.duration(
		"js.start-1000",
		"js: guest builds 1,000 rows through the ABI and the host commits them",
		start,
	);

	let mut started = || {
		let guest = JsGuest::new().expect("the JavaScript guest loads");
		guest.main().expect("the JavaScript guest starts");
		guest
	};
	let rows = started().rows()?;
	if rows != ROWS {
		return Err(format!("js guest committed {rows} rows, expected {ROWS}"));
	}

	let roundtrip = sample(config, &mut started, |guest| {
		for _ in 0..ROUNDTRIP_EVENTS {
			guest
				.event(1, 0)
				.expect("the JavaScript guest handles a press");
		}
	});
	report.throughput(
		"js.event-throughput",
		"js: press → guest update → commit round trips per second",
		ROUNDTRIP_EVENTS as f64,
		&roundtrip,
	);
	report.duration(
		"js.event-roundtrip-100",
		"js: 100 press round trips, one label updated each",
		roundtrip,
	);
	{
		let guest = started();
		guest.event(1, 0)?;
		let label = guest.label(0)?;
		if label != "Row 0 · 1" {
			return Err(format!("js row 0 reads {label:?} after a press"));
		}
	}

	let rerender = sample(config, &mut started, |guest| {
		guest.event(2, 0).expect("the JavaScript guest re-renders");
	});
	report.duration(
		"js.rerender-1000",
		"js: guest rewrites all 1,000 labels and the host diffs them",
		rerender,
	);
	{
		let guest = started();
		guest.event(2, 0)?;
		let label = guest.label(ROWS - 1)?;
		let expected = format!("Row {} · gen 1", ROWS - 1);
		if label != expected {
			return Err(format!(
				"js row {} reads {label:?}, expected {expected:?}",
				ROWS - 1
			));
		}
	}
	Ok(())
}
