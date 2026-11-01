use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

use serde::Deserialize;
use serde::Serialize;

use super::region::RegionSlot;
use super::region::drain_dirty;
use crate::Element;
use crate::EventOutcome;
use crate::NativeEvent;
use crate::NativeKind;
use crate::NativeProps;
use crate::element::HandlerEntry;
use crate::hash::FastHash;
use crate::reactive::Binding;

/// The UI imports, separated from tree reconciliation so it can run in tests.
pub(super) trait Host {
	fn create(&mut self, id: i64, kind: i32);
	fn props(&mut self, id: i64, bytes: &[u8]);
	fn insert(&mut self, parent: i64, child: i64, index: i32);
	fn delete(&mut self, id: i64);
	fn commit(&mut self);
}

/// A committed element without duplicate copies of its descendants.
#[derive(Clone)]
struct Node {
	element: Element,
	parent: Option<i64>,
	children: Vec<i64>,
	wire: Option<Vec<u8>>,
	/// The key a [`For`](crate::reactive::For) or
	/// [`Show`](crate::reactive::Show) reuses this node by. `None` for a
	/// node that was not built as one of their children.
	key: Option<u64>,
}

/// The guest retains handlers and the host retains the corresponding wire tree.
#[derive(Default)]
pub(super) struct Tree {
	nodes: HashMap<i64, Node, FastHash>,
	/// Node ids in build order, parents before children: the order the host
	/// hears about them, independent of the map's.
	order: Vec<i64>,
	/// Node ids that read application state: what a refresh visits, so it
	/// costs the values that changed rather than the size of the tree.
	bound: Vec<i64>,
	root: Option<i64>,
	next_id: i64,
	/// Regions subscribed on this tree, keyed by the node that owns them.
	regions: HashMap<i64, std::sync::Arc<RegionSlot>, FastHash>,
}

/// A node's properties as the host decodes them. Unset properties, a default
/// style, and an empty class list are left out: the host fills each in with
/// its default, and most nodes set almost nothing, so the bytes to encode in
/// the interpreter shrink from about a kilobyte to a few dozen.
#[derive(Serialize)]
struct WireNode<'a> {
	#[serde(flatten)]
	props: &'a NativeProps,
	// The host reads a missing style or class as the default, which most
	// nodes have: their styling comes from classes the host compiles, and
	// encoding a full default style is most of a render's cost, so the
	// default style is sent as `None`.
	#[serde(skip_serializing_if = "Option::is_none")]
	style: Option<&'a leptoskit_yoga::Style>,
	#[serde(skip_serializing_if = "str::is_empty")]
	class: &'a str,
	#[serde(skip_serializing_if = "Option::is_none")]
	rive_inputs: Option<&'a BTreeMap<leptoskit_rive::Name, leptoskit_rive::Update>>,
	/// Where the element was created, as `file:line:column`, so the host's
	/// `DevTools` agent can name it and compute its match key.
	#[cfg(feature = "devtools")]
	#[serde(skip_serializing_if = "Option::is_none")]
	source: Option<String>,
	/// The `#[component]` whose body created the element.
	#[cfg(feature = "devtools")]
	#[serde(skip_serializing_if = "Option::is_none")]
	component: Option<&'static str>,
}

/// Whether a node kept in place is now built somewhere else in the source,
/// which the host's `DevTools` agent shows. Always false without the
/// feature.
fn moved(previous: &Element, element: &Element) -> bool {
	#[cfg(feature = "devtools")]
	{
		previous.source_location() != element.source_location()
			|| previous.component_name() != element.component_name()
	}
	#[cfg(not(feature = "devtools"))]
	{
		let _ = (previous, element);
		false
	}
}

/// The style to send: none at all when it is the default.
fn sent_style(style: &leptoskit_yoga::Style) -> Option<&leptoskit_yoga::Style> {
	thread_local! {
		static DEFAULT: leptoskit_yoga::Style = leptoskit_yoga::Style::default();
	}
	DEFAULT.with(|default| style != default).then_some(style)
}

impl Tree {
	/// Whether a refresh or a region can update this tree without rebuilding
	/// it.
	///
	/// The wasm runtime is the caller. Host tests build the tree without it,
	/// so the method looks unused there.
	#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
	pub(super) fn has_dynamic(&self) -> bool {
		!self.bound.is_empty() || !self.regions.is_empty()
	}

	/// Validates and prepares the complete tree before mutating the host.
	pub(super) fn update(&mut self, element: Element, host: &mut impl Host) {
		element
			.validate()
			.unwrap_or_else(|error| panic!("invalid leptoskit element tree: {error}"));
		// A render usually keeps most of the previous tree, so reserving its
		// size spares the map from growing node by node.
		let mut next = Self {
			nodes: HashMap::with_capacity_and_hasher(self.nodes.len(), FastHash::default()),
			order: Vec::with_capacity(self.order.len()),
			bound: Vec::new(),
			root: None,
			next_id: self.next_id,
			regions: HashMap::with_capacity_and_hasher(self.regions.len(), FastHash::default()),
		};
		next.root = Some(next.build(element, None, self.root, self, 1));
		Self::publish(self, &mut next, host);
		*self = next;
	}

	/// Applies regions a signal write queued, then the bindings.
	///
	/// A list or a condition rebuilds only its own children. Bindings then
	/// rewrite the nodes whose value differs, in the same turn, so a handler
	/// that changes both pays for both and for nothing around them.
	pub(super) fn settle(&mut self, host: &mut impl Host) {
		// A region that runs while settling can queue another. A handful of
		// passes covers a list whose items are themselves lists; past that the
		// closure is writing a signal it reads, and stopping beats looping.
		for _ in 0..16 {
			let dirty = drain_dirty();

			if dirty.is_empty() {
				break;
			}

			for id in dirty {
				let Some(slot) = self.regions.get(&id).cloned() else {
					continue;
				};
				let children = slot.evaluate();
				self.splice(id, children, host);
			}
		}

		self.refresh_reactive(host);
	}

	/// Replaces the children of `parent` with `children`, reusing a child that
	/// keeps its key and its kind.
	fn splice(&mut self, parent: i64, children: Vec<(u64, Element)>, host: &mut impl Host) {
		if !self.nodes.contains_key(&parent) {
			return;
		}

		let replaced: HashSet<i64, FastHash> = descendants(self, parent).into_iter().collect();
		let old_keys: HashMap<u64, i64, FastHash> = self.nodes[&parent]
			.children
			.iter()
			.filter_map(|id| {
				self.nodes
					.get(id)
					.and_then(|node| node.key.map(|key| (key, *id)))
			})
			.collect();
		let child_depth = depth_of(self, parent) + 1;
		let kept = std::mem::take(&mut self.regions);
		let mut next = Self {
			nodes: HashMap::with_capacity_and_hasher(self.nodes.len(), FastHash::default()),
			order: Vec::with_capacity(self.order.len()),
			bound: Vec::new(),
			root: self.root,
			next_id: self.next_id,
			regions: HashMap::with_capacity_and_hasher(kept.len(), FastHash::default()),
		};

		for &id in &self.order {
			if replaced.contains(&id) {
				continue;
			}

			let node = self.nodes[&id].clone();

			if !node.element.bindings().is_empty() {
				next.bound.push(id);
			}

			next.order.push(id);
			next.nodes.insert(id, node);
		}

		let mut child_ids = Vec::with_capacity(children.len());

		for (key, child) in children {
			let previous = old_keys.get(&key).copied().filter(|id| {
				self.nodes
					.get(id)
					.is_some_and(|node| node.element.kind == child.kind)
			});
			let id = next.build(child, Some(parent), previous, self, child_depth);

			if let Some(node) = next.nodes.get_mut(&id) {
				node.key = Some(key);
			}

			child_ids.push(id);
		}

		if let Some(node) = next.nodes.get_mut(&parent) {
			node.children = child_ids;
		}

		let fresh = std::mem::take(&mut next.regions);
		let mut kept = kept;
		kept.retain(|id, _| next.nodes.contains_key(id) && !fresh.contains_key(id));
		next.regions = fresh;
		next.regions.extend(kept);
		Self::publish(self, &mut next, host);
		*self = next;
	}

	/// Sends the difference between `old` and `next`, then commits.
	fn publish(old: &Self, next: &mut Self, host: &mut impl Host) {
		for id in old.order.iter().filter(|id| !next.nodes.contains_key(id)) {
			host.delete(*id);
		}

		for &id in &next.order {
			let node = next.nodes.get_mut(&id).expect("every built id has a node");

			if !old.nodes.contains_key(&id) {
				host.create(id, kind_code(node.element.kind));
			}

			if let Some(wire) = node.wire.take() {
				host.props(id, &wire);
			}
		}

		for &id in &next.order {
			let node = &next.nodes[&id];

			if old
				.nodes
				.get(&id)
				.is_some_and(|previous| previous.children == node.children)
			{
				continue;
			}

			for (index, &child) in node.children.iter().enumerate() {
				host.insert(id, child, index as i32);
			}
		}

		host.commit();
	}

	/// Sends the nodes whose bound value changed, without rebuilding the tree.
	///
	/// This is the update path for
	/// [`Element::text_from`](crate::Element::text_from)
	/// and [`Element::background_from`](crate::Element::background_from):
	/// reading each binding is a signal read, and a node is encoded only when
	/// its value actually differs, so a frame costs the nodes that changed
	/// rather than every element in the tree.
	pub(super) fn refresh_reactive(&mut self, host: &mut impl Host) -> bool {
		let mut changed = false;

		for index in 0..self.bound.len() {
			let id = self.bound[index];
			let node = self.nodes.get_mut(&id).expect("every bound id has a node");
			let text = node.element.bindings().text.as_ref().map(Binding::read);
			let background = node
				.element
				.bindings()
				.background
				.as_ref()
				.map(Binding::read);
			let mut node_changed = false;

			if let Some(text) = text
				&& node.element.props.text.as_deref() != Some(text.as_str())
			{
				node.element.props.text = Some(text);
				node_changed = true;
			}

			if let Some(background) = background
				&& node.element.props.background != Some(background)
			{
				node.element.props.background = Some(background);
				node_changed = true;
			}

			if !node_changed {
				continue;
			}

			let wire = serde_json::to_vec(&WireNode {
				props: &node.element.props,
				style: sent_style(&node.element.style),
				class: node.element.classes().classes(),
				rive_inputs: None,
				// The host kept the source from the node's first render.
				#[cfg(feature = "devtools")]
				source: None,
				#[cfg(feature = "devtools")]
				component: None,
			})
			.expect("native element properties serialize");
			host.props(id, &wire);
			changed = true;
		}

		if changed {
			host.commit();
		}

		changed
	}

	/// Reuses the host renderer's positional, same-kind identity convention.
	fn build(
		&mut self,
		mut element: Element,
		parent: Option<i64>,
		previous: Option<i64>,
		old: &Self,
		depth: usize,
	) -> i64 {
		assert!(depth <= 256, "the native guest tree exceeds 256 levels");
		// Custom kinds have no code in this ABI; failing here avoids silently
		// rendering an unrelated container on the host.
		let _ = kind_code(element.kind);
		let previous = previous
			.and_then(|id| old.nodes.get(&id).map(|node| (id, node)))
			.filter(|(_, node)| node.element.kind == element.kind);
		let id = previous.map_or_else(
			|| {
				self.next_id = self
					.next_id
					.checked_add(1)
					.expect("guest node ids exhausted");
				self.next_id
			},
			|(id, _)| id,
		);
		self.order.push(id);

		if !element.bindings().is_empty() {
			self.bound.push(id);
		}

		// A region subscribes here, so a later write queues this node. The
		// read is its own observer: it must not subscribe the screen that is
		// being built around it.
		let mut child_keys = Vec::new();

		if let Some(region) = element.region.clone() {
			let slot = RegionSlot::new(id, region);
			let produced = slot.evaluate();
			child_keys = produced.iter().map(|(key, _)| *key).collect();
			element.children = produced.into_iter().map(|(_, child)| child).collect();
			self.regions.insert(id, slot);
		}

		let children: Vec<i64> = std::mem::take(&mut element.children)
			.into_iter()
			.enumerate()
			.map(|(index, child)| {
				let previous_child = previous.and_then(|(_, node)| node.children.get(index));
				self.build(child, Some(id), previous_child.copied(), old, depth + 1)
			})
			.collect();

		for (child, key) in children.iter().zip(child_keys) {
			if let Some(node) = self.nodes.get_mut(child) {
				node.key = Some(key);
			}
		}

		// Comparing shared drawings avoids serializing unchanged paths.
		let changed = previous.is_none_or(|(_, node)| {
			node.element.props != element.props
				|| node.element.style != element.style
				|| node.element.classes().classes() != element.classes().classes()
				|| moved(&node.element, &element)
		});
		let wire = changed.then(|| {
			let mut props = element.props.clone();
			// Reuse only this node's previously transmitted scene. The host
			// never looks up global asset identifiers or retains a separate
			// file cache.
			let retained = element.props.rive.as_ref().filter(|content| {
				previous
					.and_then(|(_, node)| node.element.props.rive.as_ref())
					.is_some_and(|old| old.same_scene(content))
			});
			if retained.is_some() {
				props.rive = None;
			}

			serde_json::to_vec(&WireNode {
				props: &props,
				style: sent_style(&element.style),
				class: element.classes().classes(),
				rive_inputs: retained.map(|content| content.inputs()),
				#[cfg(feature = "devtools")]
				source: element.source_location().map(|location| {
					format!(
						"{}:{}:{}",
						location.file(),
						location.line(),
						location.column()
					)
				}),
				#[cfg(feature = "devtools")]
				component: element.component_name(),
			})
			.expect("native element properties serialize")
		});
		self.nodes.insert(
			id,
			Node {
				element,
				parent,
				children,
				wire,
				key: None,
			},
		);
		id
	}

	/// Snapshots callbacks so application handlers may request another render.
	pub(super) fn handlers(
		&self,
		id: i64,
		event: NativeEvent,
	) -> Option<(NativeEvent, Vec<Vec<HandlerEntry>>)> {
		let target = self.nodes.get(&id)?;
		let event = normalize_event(&target.element, event)?;
		let mut current = Some(id);
		let mut handlers = Vec::new();

		while let Some(id) = current {
			let node = &self.nodes[&id];

			if node.element.props.hidden == Some(true)
				|| node.element.props.enabled == Some(false)
				|| (node.element.kind == NativeKind::ActivityIndicator
					&& node.element.props.running == Some(false))
			{
				return None;
			}

			handlers.push(node.element.handlers().to_vec());
			current = node.parent;
		}

		Some((event, handlers))
	}
}

/// Runs every matching handler at a node before deciding whether to bubble.
pub(super) fn invoke(event: &NativeEvent, handlers: &[Vec<HandlerEntry>]) -> EventOutcome {
	for entries in handlers {
		let mut handled = false;

		for entry in entries {
			if entry.event.matches(event) && (entry.handler)(event) == EventOutcome::Handled {
				handled = true;
			}
		}

		if handled {
			return EventOutcome::Handled;
		}
	}

	EventOutcome::Unhandled
}

/// Interprets only the payload schema belonging to the selected ABI event.
pub(super) fn decode_event(kind: i32, payload: &[u8]) -> Option<NativeEvent> {
	#[derive(Deserialize)]
	struct Text {
		value: String,
	}

	#[derive(Deserialize)]
	struct Toggle {
		checked: bool,
	}

	#[derive(Deserialize)]
	struct Value {
		value: f64,
	}

	#[derive(Deserialize)]
	struct Scroll {
		x: f32,
		y: f32,
	}

	match kind {
		0 => Some(NativeEvent::Press),
		1 | 2 => {
			let Text { value } = serde_json::from_slice(payload).ok()?;

			Some(if kind == 1 {
				NativeEvent::Change { value }
			} else {
				NativeEvent::Submit { value }
			})
		}
		3 => Some(NativeEvent::Focus),
		4 => Some(NativeEvent::Blur),
		5 => {
			let Scroll { x, y } = serde_json::from_slice(payload).ok()?;

			(x.is_finite() && y.is_finite()).then_some(NativeEvent::Scroll {
				offset_x: x,
				offset_y: y,
			})
		}
		6 => {
			let Toggle { checked } = serde_json::from_slice(payload).ok()?;
			Some(NativeEvent::Toggle { checked })
		}
		7 => {
			let Value { value } = serde_json::from_slice(payload).ok()?;
			value
				.is_finite()
				.then_some(NativeEvent::ValueChange { value })
		}
		8 => serde_json::from_slice(payload).ok().map(NativeEvent::Key),
		_ => None,
	}
}

/// Applies controlled-widget proposal rules before application handlers run.
fn normalize_event(element: &Element, event: NativeEvent) -> Option<NativeEvent> {
	match event {
		NativeEvent::Toggle { checked } => {
			(matches!(
				element.kind,
				NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
			) && (element.kind != NativeKind::Radio || checked))
				.then_some(event)
		}
		NativeEvent::ValueChange { value } => {
			if element.kind != NativeKind::Slider {
				return None;
			}

			element
				.props
				.slider
				.unwrap_or_default()
				.proposed_value(value)
				.map(|value| NativeEvent::ValueChange { value })
		}
		_ => Some(event),
	}
}

/// Every node under `parent`, not including it. The list is the subtree a
/// region replaces.
fn descendants(tree: &Tree, parent: i64) -> Vec<i64> {
	let mut out = Vec::new();
	let mut stack = tree
		.nodes
		.get(&parent)
		.map(|node| node.children.clone())
		.unwrap_or_default();

	while let Some(id) = stack.pop() {
		if let Some(node) = tree.nodes.get(&id) {
			stack.extend(node.children.iter().copied());
		}

		out.push(id);
	}

	out
}

/// How deep `id` sits, the root being one. The guest refuses a tree past 256.
fn depth_of(tree: &Tree, id: i64) -> usize {
	let mut depth = 1;
	let mut current = tree.nodes.get(&id).and_then(|node| node.parent);

	while let Some(id) = current {
		depth += 1;
		current = tree.nodes.get(&id).and_then(|node| node.parent);
	}

	depth
}

/// The codes of the native UI ABI, independent of enum declaration order.
fn kind_code(kind: NativeKind) -> i32 {
	match kind {
		NativeKind::View => 0,
		NativeKind::Text => 1,
		NativeKind::TextRun => 2,
		NativeKind::Image => 3,
		NativeKind::ScrollView => 4,
		NativeKind::TextInput => 5,
		NativeKind::Button => 6,
		NativeKind::Switch => 7,
		NativeKind::Checkbox => 8,
		NativeKind::Radio => 9,
		NativeKind::Slider => 10,
		NativeKind::LinearProgressIndicator => 11,
		NativeKind::ActivityIndicator => 12,
		NativeKind::PasswordInput => 13,
		NativeKind::Pressable => 14,
		NativeKind::Vector => 15,
		NativeKind::Rive => 16,
		NativeKind::Custom(name) => panic!("custom widget `{name}` has no native guest ABI code"),
	}
}

#[cfg(test)]
mod tests;
