use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

use crate::Element;
use crate::EventOutcome;
use crate::NativeEvent;
use crate::NativeKind;
use crate::NativeProps;
use crate::element::HandlerEntry;

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
	wire: Vec<u8>,
}

/// The guest retains handlers and the host retains the corresponding wire tree.
#[derive(Default)]
pub(super) struct Tree {
	nodes: BTreeMap<i64, Node>,
	root: Option<i64>,
	next_id: i64,
}

#[derive(Serialize)]
struct WireNode<'a> {
	#[serde(flatten)]
	props: &'a NativeProps,
	style: &'a leptoskit_yoga::Style,
	class: &'a str,
}

impl Tree {
	/// Validates and prepares the complete tree before mutating the host.
	pub(super) fn update(&mut self, element: Element, host: &mut impl Host) {
		element
			.validate()
			.unwrap_or_else(|error| panic!("invalid leptoskit element tree: {error}"));
		let mut next = Self {
			next_id: self.next_id,
			..Self::default()
		};
		next.root = Some(next.build(element, None, self.root, self, 1));

		for id in self.nodes.keys().filter(|id| !next.nodes.contains_key(id)) {
			host.delete(*id);
		}

		for (&id, node) in &next.nodes {
			if !self.nodes.contains_key(&id) {
				host.create(id, kind_code(node.element.kind));
			}

			if self.nodes.get(&id).is_none_or(|old| old.wire != node.wire) {
				host.props(id, &node.wire);
			}
		}

		for (&id, node) in &next.nodes {
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
		let children = std::mem::take(&mut element.children)
			.into_iter()
			.enumerate()
			.map(|(index, child)| {
				let previous_child = previous.and_then(|(_, node)| node.children.get(index));
				self.build(child, Some(id), previous_child.copied(), old, depth + 1)
			})
			.collect();
		let wire = serde_json::to_vec(&WireNode {
			props: &element.props,
			style: &element.style,
			class: element.classes().classes(),
		})
		.expect("native element properties serialize");
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
		NativeKind::Custom(name) => panic!("custom widget `{name}` has no native guest ABI code"),
	}
}

#[cfg(test)]
mod tests;
