use std::collections::BTreeMap;
use std::collections::HashMap;

use serde::Deserialize;
use serde::Serialize;

use crate::Element;
use crate::EventOutcome;
use crate::NativeEvent;
use crate::NativeKind;
use crate::NativeProps;
use crate::element::HandlerEntry;
use crate::hash::FastHash;

/// The UI imports, separated from tree reconciliation so it can run in tests.
pub(super) trait Host {
	fn create(&mut self, id: i64, kind: i32);
	fn props(&mut self, id: i64, bytes: &[u8]);
	fn insert(&mut self, parent: i64, child: i64, index: i32);
	fn delete(&mut self, id: i64);
	fn commit(&mut self);
}

/// A committed element without duplicate copies of its descendants.
struct Node {
	element: Element,
	parent: Option<i64>,
	children: Vec<i64>,
	wire: Option<Vec<u8>>,
}

/// The guest retains handlers and the host retains the corresponding wire tree.
#[derive(Default)]
pub(super) struct Tree {
	nodes: HashMap<i64, Node, FastHash>,
	/// Node ids in build order, parents before children: the order the host
	/// hears about them, independent of the map's.
	order: Vec<i64>,
	root: Option<i64>,
	next_id: i64,
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
}

/// Whether a node kept in place is now built somewhere else in the source,
/// which the host's `DevTools` agent shows. Always false without the
/// feature.
fn moved(previous: &Element, element: &Element) -> bool {
	#[cfg(feature = "devtools")]
	{
		previous.source_location() != element.source_location()
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
			root: None,
			next_id: self.next_id,
		};
		next.root = Some(next.build(element, None, self.root, self, 1));

		for id in self.order.iter().filter(|id| !next.nodes.contains_key(id)) {
			host.delete(*id);
		}

		for &id in &next.order {
			let node = next.nodes.get_mut(&id).expect("every built id has a node");

			if !self.nodes.contains_key(&id) {
				host.create(id, kind_code(node.element.kind));
			}

			if let Some(wire) = node.wire.take() {
				host.props(id, &wire);
			}
		}

		for &id in &next.order {
			let node = &next.nodes[&id];

			if self
				.nodes
				.get(&id)
				.is_some_and(|old| old.children == node.children)
			{
				continue;
			}

			for (index, &child) in node.children.iter().enumerate() {
				host.insert(id, child, index as i32);
			}
		}

		host.commit();
		*self = next;
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
		let children = std::mem::take(&mut element.children)
			.into_iter()
			.enumerate()
			.map(|(index, child)| {
				let previous_child = previous.and_then(|(_, node)| node.children.get(index));
				self.build(child, Some(id), previous_child.copied(), old, depth + 1)
			})
			.collect();
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
