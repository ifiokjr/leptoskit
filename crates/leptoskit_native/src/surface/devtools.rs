//! The `DevTools` snapshot of a committed surface.

use leptoskit_devtools_protocol::Frame as ProtocolFrame;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::SourceLocation;
use leptoskit_devtools_protocol::StyleRule;
use leptoskit_devtools_protocol::Tree;
use leptoskit_yoga::NodeId;

use super::NativeSurface;
use crate::backend::NativeBackend;
use crate::devtools as keys;
use crate::devtools::Origin;
use crate::tailwind::StyleContext;
use crate::tailwind::Tailwind;

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
		self.collect_devtools(root, keys::ROOT, &mut nodes);
		Tree {
			root: NodeRef::Native(root.index() as u64),
			nodes,
		}
	}

	fn collect_devtools(&self, id: NodeId, key: u64, nodes: &mut Vec<Node>) {
		let shadow = &self.shadows[id.index()];
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
			key: MatchKey::new(keys::format(key)),
			parent: shadow
				.parent
				.map(|parent| NodeRef::Native(parent.index() as u64)),
			kind: format!("{:?}", shadow.kind),
			source,
			frame,
			classes: shadow.classes.clone(),
			text: shadow.props.text.clone(),
			children,
			styles: self.explain(id, &shadow.classes),
		});

		let origins: Vec<Origin> = shadow
			.children
			.iter()
			.map(|child| {
				let child = &self.shadows[child.index()];
				Origin::of(child.source, child.kind)
			})
			.collect();
		let ordinals = keys::ordinals(origins.iter().copied());

		for ((child, origin), ordinal) in shadow.children.iter().zip(origins).zip(ordinals) {
			self.collect_devtools(*child, keys::child(key, origin, ordinal), nodes);
		}
	}
}

impl<B: NativeBackend> NativeSurface<B> {
	/// The node's active classes, resolved against the surface's viewport,
	/// appearance, and pressed control.
	fn explain(&self, id: NodeId, classes: &[String]) -> Vec<StyleRule> {
		if classes.is_empty() {
			return Vec::new();
		}

		// The shadow keeps the classes that already parsed, so this parse
		// fails only if the compiler changed underneath a running surface.
		let Ok(tailwind) = Tailwind::parse(&classes.join(" ")) else {
			return Vec::new();
		};
		let context = StyleContext {
			viewport: self.viewport,
			dark: self.dark,
			safe_area: self.safe_area,
			active: self.pressed == Some(id),
		};
		tailwind
			.explain(&context)
			.into_iter()
			.map(|explained| {
				StyleRule {
					class: explained.class,
					declarations: explained.declarations,
					variant: explained.variant,
				}
			})
			.collect()
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

	#[test]
	fn the_tree_explains_the_active_classes() {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(400.0, 100.0);
		let element = Element::view().class("p-4 md:p-8 dark:bg-black");
		surface.set_root(&element);
		let tree = surface.devtools_tree();
		let root = &tree.nodes[0];
		assert_eq!(root.classes, ["dark:bg-black", "md:p-8", "p-4"]);
		let classes: Vec<&str> = root.styles.iter().map(|rule| rule.class.as_str()).collect();
		assert_eq!(classes, ["p-4"]);
		assert!(root.styles[0].declarations.contains("padding"));

		surface.set_viewport(800.0, 100.0);
		surface.set_dark_mode(true);
		let tree = surface.devtools_tree();
		let root = &tree.nodes[0];
		assert!(root.styles.iter().any(|rule| {
			rule.class == "md:p-8" && rule.variant.as_deref() == Some("min-width: 768px")
		}));
		assert!(root.styles.iter().any(|rule| {
			rule.class == "dark:bg-black" && rule.variant.as_deref() == Some("dark")
		}));
	}
}
