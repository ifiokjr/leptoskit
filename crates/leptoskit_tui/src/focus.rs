//! The keyboard focus ring, derived from the committed tree.

use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_yoga::NodeId;

use crate::backend::Node;
use crate::backend::TuiBackend;

/// Whether a widget kind takes keyboard focus.
#[must_use]
pub const fn focusable_kind(kind: NativeKind) -> bool {
	matches!(
		kind,
		NativeKind::Button
			| NativeKind::Pressable
			| NativeKind::TextInput
			| NativeKind::PasswordInput
			| NativeKind::Switch
			| NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Slider
			| NativeKind::ScrollView
	)
}

/// Whether the walk skips this node, and so its subtree.
fn skips(node: &Node, inert: bool) -> bool {
	inert
		|| node.props.hidden == Some(true)
		|| node.props.enabled == Some(false)
		|| node.props.opacity.unwrap_or(1.0) <= 0.0
}

/// Files a focusable widget by its `tab_index`: a positive one comes first,
/// and a negative one takes the widget out of the order.
fn file(node: &Node, id: NodeId, positive: &mut Vec<(i32, NodeId)>, rest: &mut Vec<NodeId>) {
	match node.props.aria().tab_index {
		Some(index) if index > 0 => positive.push((index, id)),

		Some(index) if index < 0 => {}
		_ => rest.push(id),
	}
}

/// The focus order of the committed tree, in tab-navigation order.
///
/// The walk is document order with inherited inertness: a hidden or disabled
/// subtree is skipped. Widgets with a positive `tab_index` come first, in
/// ascending order; everything else follows in document order; a negative
/// `tab_index` removes a widget from the order — the HTML rules, which the
/// web renderer follows too.
pub(crate) fn focusables(backend: &TuiBackend) -> Vec<NodeId> {
	let mut positive: Vec<(i32, NodeId)> = Vec::new();
	let mut rest: Vec<NodeId> = Vec::new();
	let Some(root) = backend.root() else {
		return Vec::new();
	};

	let mut pending = vec![(root, false)];

	while let Some((id, inert)) = pending.pop() {
		let Some(node) = backend.node(id) else {
			continue;
		};

		if !skips(node, inert) {
			if focusable_kind(node.kind) {
				file(node, id, &mut positive, &mut rest);
			}

			pending.extend(node.children.iter().rev().map(|&child| (child, false)));
		}
	}

	positive.sort_by_key(|(index, _)| *index);
	positive.into_iter().map(|(_, id)| id).chain(rest).collect()
}

/// The nearest focusable node at or above `id`, for mouse focus follows.
pub(crate) fn nearest_focusable(backend: &TuiBackend, id: NodeId) -> Option<NodeId> {
	let mut current = Some(id);

	while let Some(id) = current {
		let node = backend.node(id)?;

		if focusable_kind(node.kind) && path_active(backend, id) {
			return Some(id);
		}

		current = node.parent;
	}

	None
}

/// Whether a node and its ancestors are all enabled and visible, matching the
/// shared dispatch rules for event paths.
pub(crate) fn path_active(backend: &TuiBackend, id: NodeId) -> bool {
	let mut current = Some(id);

	while let Some(id) = current {
		let Some(node) = backend.node(id) else {
			return false;
		};

		if node.props.hidden == Some(true)
			|| node.props.enabled == Some(false)
			|| node.kind == NativeKind::ActivityIndicator && node.props.running == Some(false)
		{
			return false;
		}

		current = node.parent;
	}

	true
}

/// Whether a widget takes focus by its kind and state.
#[must_use]
pub fn is_focusable(kind: NativeKind, props: &NativeProps) -> bool {
	focusable_kind(kind) && props.hidden != Some(true) && props.enabled != Some(false)
}
