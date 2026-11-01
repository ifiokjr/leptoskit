//! Event bubbling and viewport-space hit testing over committed nodes.

use std::borrow::Cow;

use leptoskit_yoga::NodeId;

use super::HitArea;
use super::NativeSurface;
use crate::backend::NativeBackend;
use crate::props::EventOutcome;
use crate::props::NativeEvent;
use crate::props::NativeKind;

impl<B: NativeBackend> NativeSurface<B> {
	/// Dispatches an event at a target, bubbling to ancestors until handled.
	pub fn dispatch_event(&self, target: NodeId, event: &NativeEvent) -> EventOutcome {
		let Some(event) = self.normalized_event(target, event) else {
			return EventOutcome::Unhandled;
		};

		if !self.event_path_is_active(target) {
			return EventOutcome::Unhandled;
		}

		let mut current = Some(target);

		while let Some(id) = current {
			if self.dispatch_handlers(id, &event) == EventOutcome::Handled {
				return EventOutcome::Handled;
			}

			current = self.shadows[id.index()].parent;
		}

		EventOutcome::Unhandled
	}

	fn normalized_event<'a>(
		&self,
		target: NodeId,
		event: &'a NativeEvent,
	) -> Option<Cow<'a, NativeEvent>> {
		let node = &self.shadows[target.index()];

		match event {
			NativeEvent::Toggle { checked } => {
				Self::accepts_toggle(node.kind, *checked).then_some(Cow::Borrowed(event))
			}

			NativeEvent::ValueChange { value } if node.kind == NativeKind::Slider => {
				let state = node.props.slider.unwrap_or_default();
				let value = state.proposed_value(*value)?;

				Some(Cow::Owned(NativeEvent::ValueChange { value }))
			}

			NativeEvent::ValueChange { .. } => None,
			_ => Some(Cow::Borrowed(event)),
		}
	}

	fn accepts_toggle(kind: NativeKind, checked: bool) -> bool {
		matches!(
			kind,
			NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
		) && (kind != NativeKind::Radio || checked)
	}

	pub(super) fn event_path_is_active(&self, target: NodeId) -> bool {
		let mut ancestor = Some(target);

		while let Some(id) = ancestor {
			let node = &self.shadows[id.index()];

			if node.is_inert() || node.props.enabled == Some(false) {
				return false;
			}

			ancestor = node.parent;
		}

		true
	}

	/// Every matching handler on one node runs before bubbling is stopped.
	fn dispatch_handlers(&self, id: NodeId, event: &NativeEvent) -> EventOutcome {
		let mut outcome = EventOutcome::Unhandled;

		for entry in &self.shadows[id.index()].handlers {
			if entry.event.matches(event) && (entry.handler)(event) == EventOutcome::Handled {
				outcome = EventOutcome::Handled;
			}
		}

		outcome
	}

	/// Finds the deepest node whose frame contains the viewport-space point.
	pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
		let root = self.root?;
		let mut best = None;

		if self.hit_areas[root.index()].subtree.contains(x, y) {
			self.hit_test_walk(root, x, y, &mut best);
		}

		best
	}

	/// Visits a node whose subtree bounds contain the point.
	fn hit_test_walk(&self, id: NodeId, x: f32, y: f32, best: &mut Option<NodeId>) {
		let shadow = &self.shadows[id.index()];
		let contains = self.hit_frame_contains(id, x, y);

		if contains {
			*best = Some(id);
		}

		if shadow.kind == NativeKind::Pressable || (!contains && shadow.props.clip == Some(true)) {
			return;
		}

		for &child in &shadow.children {
			if self.hit_areas[child.index()].subtree.contains(x, y) {
				self.hit_test_walk(child, x, y, best);
			}
		}
	}

	fn hit_frame_contains(&self, id: NodeId, x: f32, y: f32) -> bool {
		let props = &self.shadows[id.index()].props;
		let HitArea {
			left,
			top,
			width,
			height,
			..
		} = self.hit_areas[id.index()];
		let contains = x >= left && x <= left + width && y >= top && y <= top + height;

		if !contains || props.clip != Some(true) {
			return contains;
		}

		let radius = props
			.corner_radius
			.unwrap_or_default()
			.max(0.0)
			.min(width.min(height) * 0.5);
		let dx = x - x.clamp(left + radius, left + width - radius);
		let dy = y - y.clamp(top + radius, top + height - radius);

		dx * dx + dy * dy <= radius * radius
	}
}
