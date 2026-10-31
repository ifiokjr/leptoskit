//! The `DevTools` snapshot of a committed surface.

use leptoskit_devtools_protocol::Frame as ProtocolFrame;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::SourceLocation;
use leptoskit_devtools_protocol::Tree;
use leptoskit_yoga::NodeId;

use super::NativeSurface;
use crate::backend::NativeBackend;

impl<B: NativeBackend> NativeSurface<B> {
	/// The committed tree in the `DevTools` protocol's shape.
	#[must_use]
	pub fn devtools_tree(&self) -> Tree {
		let Some(root) = self.root else {
			return Tree {
				root: NodeRef::Native(0),
				nodes: Vec::new(),
			};
		};
		let mut nodes = Vec::new();
		self.collect_devtools(root, &mut nodes);
		Tree {
			root: NodeRef::Native(root.index() as u64),
			nodes,
		}
	}

	fn collect_devtools(&self, id: NodeId, nodes: &mut Vec<Node>) {
		let shadow = &self.shadows[id.index()];
		let key = shadow
			.props
			.test_id()
			.map(str::to_owned)
			.or_else(|| shadow.props.id.clone())
			.unwrap_or_else(|| format!("/{}", id.index()));
		let frame = self.frames[id.index()].map(|frame| {
			ProtocolFrame {
				x: frame.x,
				y: frame.y,
				width: frame.width,
				height: frame.height,
			}
		});
		let source = shadow.source.map(|location| {
			SourceLocation {
				file: location.file().to_owned(),
				line: location.line(),
				column: location.column(),
			}
		});
		let children: Vec<NodeRef> = shadow
			.children
			.iter()
			.map(|child| NodeRef::Native(child.index() as u64))
			.collect();

		nodes.push(Node {
			id: NodeRef::Native(id.index() as u64),
			key: MatchKey::new(key),
			parent: shadow
				.parent
				.map(|parent| NodeRef::Native(parent.index() as u64)),
			kind: format!("{:?}", shadow.kind),
			source,
			frame,
			classes: shadow.classes.clone(),
			text: shadow.props.text.clone(),
			children,
		});

		for child in &shadow.children {
			self.collect_devtools(*child, nodes);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Element;
	use crate::HeadlessBackend;

	#[test]
	fn the_tree_carries_sources_frames_and_children() {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(100.0, 100.0);
		let element = Element::view().child(Element::text("hello"));
		surface.set_root(&element);
		let tree = surface.devtools_tree();
		assert_eq!(tree.nodes.len(), 2);
		assert!(tree.nodes.iter().all(|node| node.source.is_some()));
		assert_eq!(
			tree.nodes
				.iter()
				.find(|node| node.parent.is_none())
				.unwrap()
				.children
				.len(),
			1
		);
	}
}
