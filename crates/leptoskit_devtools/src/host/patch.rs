//! The tree patch that brings the hub's copy of a tree up to date.

use std::collections::HashMap;

use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Tree;
use leptoskit_devtools_protocol::TreeOp;

/// The operations that turn `sent`, the tree the hub has, into `current`,
/// as the hub applies them ([`leptoskit_devtools_protocol::SessionView`]).
/// `None` when only a snapshot can: the root became a node the hub already
/// has, and a patch cannot move the root.
///
/// The hub appends an inserted node to its parent's children, so new nodes
/// go in without children, parents first, and every node whose children or
/// details changed is then replaced whole, which puts each child list in
/// order. Removals come last, so a node that moved out of a removed subtree
/// already names its new parent.
pub(super) fn diff(sent: &Tree, current: &Tree) -> Option<Vec<TreeOp>> {
	let before: HashMap<&NodeRef, &Node> = sent.nodes.iter().map(|node| (&node.id, node)).collect();
	let after: HashMap<&NodeRef, &Node> =
		current.nodes.iter().map(|node| (&node.id, node)).collect();

	if current.root != sent.root && before.contains_key(&current.root) {
		return None;
	}

	let mut ops = Vec::new();

	for node in &current.nodes {
		if !before.contains_key(&node.id) {
			ops.push(TreeOp::Insert {
				node: Node {
					children: Vec::new(),
					..node.clone()
				},
			});
		}
	}

	for node in &current.nodes {
		let changed = before
			.get(&node.id)
			.map_or(!node.children.is_empty(), |old| *old != node);

		if changed {
			ops.push(TreeOp::Update { node: node.clone() });
		}
	}

	for node in &sent.nodes {
		let topmost = node
			.parent
			.as_ref()
			.is_none_or(|parent| after.contains_key(parent));

		if !after.contains_key(&node.id) && topmost {
			ops.push(TreeOp::Remove {
				id: node.id.clone(),
			});
		}
	}

	Some(ops)
}

#[cfg(test)]
mod tests {
	use leptoskit_devtools_protocol::Event;
	use leptoskit_devtools_protocol::MatchKey;
	use leptoskit_devtools_protocol::SessionView;
	use leptoskit_devtools_protocol::Snapshot;
	use leptoskit_devtools_protocol::TreePatch;

	use super::*;

	/// A tree from `(id, parent, text)` rows in depth-first order.
	fn tree(rows: &[(u64, Option<u64>, &str)]) -> Tree {
		let mut nodes: Vec<Node> = rows
			.iter()
			.map(|(id, parent, text)| {
				Node {
					id: NodeRef::Native(*id),
					key: MatchKey::new(format!("k{id}")),
					parent: parent.map(NodeRef::Native),
					kind: "View".into(),
					source: None,
					frame: None,
					classes: Vec::new(),
					text: Some((*text).to_owned()),
					children: Vec::new(),
					component: None,
					styles: Vec::new(),
				}
			})
			.collect();

		for (id, parent, _) in rows {
			if let Some(parent) = parent {
				let slot = nodes
					.iter_mut()
					.find(|node| node.id == NodeRef::Native(*parent))
					.unwrap();
				slot.children.push(NodeRef::Native(*id));
			}
		}

		Tree {
			root: NodeRef::Native(rows[0].0),
			nodes,
		}
	}

	/// The hub's copy after it has `sent` and applies the patch to `current`.
	fn patched(sent: &Tree, current: &Tree) -> Tree {
		let mut view = SessionView::default();
		view.apply(&Event::Snapshot {
			snapshot: Snapshot {
				tree: sent.clone(),
				route: None,
				queries: Vec::new(),
				mutations: Vec::new(),
				forms: Vec::new(),
				state: Vec::new(),
				calls: Vec::new(),
				rpc: Vec::new(),
			},
		});
		view.apply(&Event::TreePatch {
			patch: TreePatch {
				ops: diff(sent, current).unwrap(),
			},
		});
		view.tree.unwrap()
	}

	/// Two trees hold the same nodes, whatever order they list them in.
	fn assert_same(actual: &Tree, expected: &Tree) {
		let index = |tree: &Tree| {
			let mut nodes = tree.nodes.clone();
			nodes.sort_by(|a, b| a.id.cmp(&b.id));
			nodes
		};
		assert_eq!(actual.root, expected.root);
		assert_eq!(index(actual), index(expected));
	}

	#[test]
	fn an_unchanged_tree_needs_no_patch() {
		let sent = tree(&[(1, None, "a"), (2, Some(1), "b")]);
		assert_eq!(diff(&sent, &sent), Some(Vec::new()));
	}

	#[test]
	fn inserts_in_the_middle_keep_the_children_in_order() {
		let sent = tree(&[(1, None, "a"), (2, Some(1), "b"), (3, Some(1), "c")]);
		let current = tree(&[
			(1, None, "a"),
			(2, Some(1), "b"),
			(4, Some(1), "new"),
			(5, Some(4), "nested"),
			(3, Some(1), "c"),
		]);
		assert_same(&patched(&sent, &current), &current);
	}

	#[test]
	fn removals_take_their_subtrees_but_not_nodes_that_moved_out() {
		let sent = tree(&[
			(1, None, "a"),
			(2, Some(1), "gone"),
			(3, Some(2), "moves"),
			(4, Some(2), "goes too"),
			(5, Some(1), "stays"),
		]);
		let current = tree(&[(1, None, "a"), (5, Some(1), "stays"), (3, Some(5), "moves")]);
		assert_same(&patched(&sent, &current), &current);
	}

	#[test]
	fn text_changes_and_a_new_root_are_patched() {
		let sent = tree(&[(1, None, "a"), (2, Some(1), "b")]);
		let current = tree(&[(7, None, "new root"), (8, Some(7), "b, again")]);
		assert_same(&patched(&sent, &current), &current);
		let edited = tree(&[(1, None, "a"), (2, Some(1), "edited")]);
		assert_eq!(diff(&sent, &edited).unwrap().len(), 1);
	}

	#[test]
	fn a_root_the_hub_already_has_needs_a_snapshot() {
		let sent = tree(&[(1, None, "a"), (2, Some(1), "b")]);
		let current = tree(&[(2, None, "b")]);
		assert_eq!(diff(&sent, &current), None);
	}
}
