//! The tree a native host reports, kept by replaying the mutations its
//! renderer applies, so the agent never walks the host's surface.

use std::collections::HashMap;

use leptoskit_devtools_protocol::Frame;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::SourceLocation;
use leptoskit_devtools_protocol::Tree;
use leptoskit_native::Mutation;
use leptoskit_native::NativeKind;
use leptoskit_native::devtools as keys;
use leptoskit_native::devtools::Origin;

/// One change to a native tree: the part of a renderer [`Mutation`] the
/// mirror keeps, or what the guest said about a node that no mutation
/// carries.
///
/// Ids are the surface's node ids, which the protocol's
/// [`NodeRef::Native`] and the platform's widgets use too.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
	/// A widget was created.
	Create {
		/// The widget.
		id: u64,
		/// Its kind.
		kind: NativeKind,
		/// The text it shows, if any.
		text: Option<String>,
	},
	/// A widget's properties changed; the mirror keeps only its text.
	Text {
		/// The widget.
		id: u64,
		/// The text it shows now, if any.
		text: Option<String>,
	},
	/// A widget was laid out.
	Layout {
		/// The widget.
		id: u64,
		/// Its frame, relative to its parent.
		frame: Frame,
	},
	/// A child was inserted into a parent.
	Insert {
		/// The parent.
		parent: u64,
		/// The child.
		child: u64,
		/// Where among the parent's children.
		index: usize,
	},
	/// A child was removed from its parent.
	Remove {
		/// The parent.
		parent: u64,
		/// The child.
		child: u64,
	},
	/// A widget was deleted.
	Delete {
		/// The widget.
		id: u64,
	},
	/// The classes and source location the guest sent for a widget.
	Details {
		/// The widget.
		id: u64,
		/// Its Tailwind classes, separated by spaces.
		classes: String,
		/// Where the guest created it, as `file:line:column`, when the guest
		/// was built with `devtools`.
		source: Option<String>,
	},
}

impl Change {
	/// What `mutation` changes in the mirror: everything but the widget
	/// properties other than text, which stay with the renderer.
	#[must_use]
	pub fn of(mutation: &Mutation) -> Self {
		match mutation {
			Mutation::Create { id, kind, props } => {
				Self::Create {
					id: id.index() as u64,
					kind: *kind,
					text: props.text.clone(),
				}
			}
			Mutation::UpdateProps { id, props } => {
				Self::Text {
					id: id.index() as u64,
					text: props.text.clone(),
				}
			}
			Mutation::UpdateLayout {
				id,
				x,
				y,
				width,
				height,
			} => {
				Self::Layout {
					id: id.index() as u64,
					frame: Frame {
						x: *x,
						y: *y,
						width: *width,
						height: *height,
					},
				}
			}
			Mutation::InsertChild {
				parent,
				child,
				index,
			} => {
				Self::Insert {
					parent: parent.index() as u64,
					child: child.index() as u64,
					index: *index,
				}
			}
			Mutation::RemoveChild { parent, child } => {
				Self::Remove {
					parent: parent.index() as u64,
					child: child.index() as u64,
				}
			}
			Mutation::Delete { id } => {
				Self::Delete {
					id: id.index() as u64,
				}
			}
		}
	}
}

/// One mirrored widget.
#[derive(Clone, Debug)]
struct Entry {
	kind: NativeKind,
	text: Option<String>,
	/// The frame relative to the parent, once laid out.
	frame: Option<Frame>,
	parent: Option<u64>,
	children: Vec<u64>,
	classes: String,
	source: Option<String>,
}

/// A native host's committed tree, kept by applying the same changes its
/// renderer applies, in the same order.
#[derive(Clone, Debug, Default)]
pub struct Mirror {
	nodes: HashMap<u64, Entry>,
	root: Option<u64>,
}

impl Mirror {
	/// An empty mirror.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Applies one change. Changes that name a widget the mirror does not
	/// have are ignored, as the renderer would never send them.
	pub fn apply(&mut self, change: Change) {
		match change {
			Change::Create { id, kind, text } => {
				self.nodes.insert(
					id,
					Entry {
						kind,
						text,
						frame: None,
						parent: None,
						children: Vec::new(),
						classes: String::new(),
						source: None,
					},
				);
			}
			Change::Text { id, text } => {
				if let Some(entry) = self.nodes.get_mut(&id) {
					entry.text = text;
				}
			}
			Change::Layout { id, frame } => {
				if let Some(entry) = self.nodes.get_mut(&id) {
					entry.frame = Some(frame);
				}
			}
			Change::Insert {
				parent,
				child,
				index,
			} => self.insert(parent, child, index),
			Change::Remove { parent, child } => self.detach(parent, child),
			Change::Delete { id } => self.delete(id),
			Change::Details {
				id,
				classes,
				source,
			} => {
				if let Some(entry) = self.nodes.get_mut(&id) {
					entry.classes = classes;
					entry.source = source;
				}
			}
		}
	}

	/// Names the root the renderer shows, or none.
	pub fn set_root(&mut self, root: Option<u64>) {
		self.root = root;
	}

	/// The root the renderer shows, if any.
	#[must_use]
	pub fn root(&self) -> Option<u64> {
		self.root.filter(|root| self.nodes.contains_key(root))
	}

	/// The number of widgets mirrored, attached or not.
	#[must_use]
	pub fn len(&self) -> usize {
		self.nodes.len()
	}

	/// Whether the mirror has no widgets.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.nodes.is_empty()
	}

	/// The tree under the root in the protocol's shape, depth first: each
	/// node's match key, its frame in viewport space, and its children in
	/// order. `None` before the renderer shows a root.
	#[must_use]
	pub fn tree(&self) -> Option<Tree> {
		let root = self.root()?;
		let mut nodes = Vec::with_capacity(self.nodes.len());
		let mut stack = vec![(root, keys::ROOT, None, 0.0, 0.0)];

		// The renderer keeps a tree, so this visits each node once; the bound
		// only stops a malformed stream from looping.
		while let Some((id, key, parent, left, top)) = stack.pop() {
			if nodes.len() == self.nodes.len() {
				break;
			}

			let Some(entry) = self.nodes.get(&id) else {
				continue;
			};
			let frame = entry.frame.map(|frame| {
				Frame {
					x: left + frame.x,
					y: top + frame.y,
					width: frame.width,
					height: frame.height,
				}
			});
			let (left, top) = frame.map_or((left, top), |frame| (frame.x, frame.y));
			let origins: Vec<Origin<'_>> = entry
				.children
				.iter()
				.map(|child| self.origin(*child))
				.collect();
			let ordinals = keys::ordinals(origins.iter().copied());

			for ((child, origin), ordinal) in entry.children.iter().zip(origins).zip(ordinals).rev()
			{
				stack.push((
					*child,
					keys::child(key, origin, ordinal),
					Some(id),
					left,
					top,
				));
			}

			nodes.push(Node {
				id: NodeRef::Native(id),
				key: MatchKey::new(keys::format(key)),
				parent: parent.map(NodeRef::Native),
				kind: format!("{:?}", entry.kind),
				source: entry
					.source
					.as_deref()
					.and_then(source)
					.map(|(file, line, column)| {
						SourceLocation {
							file: file.to_owned(),
							line,
							column,
						}
					}),
				frame,
				classes: entry
					.classes
					.split_whitespace()
					.map(str::to_owned)
					.collect(),
				text: entry.text.clone(),
				children: entry
					.children
					.iter()
					.map(|child| NodeRef::Native(*child))
					.collect(),
				component: None,
				styles: Vec::new(),
			});
		}

		Some(Tree {
			root: NodeRef::Native(root),
			nodes,
		})
	}

	/// What tells a node apart from its siblings: where the guest created
	/// it, or its kind when the guest sent no location.
	fn origin(&self, id: u64) -> Origin<'_> {
		let Some(entry) = self.nodes.get(&id) else {
			return Origin::Kind(NativeKind::View);
		};

		match entry.source.as_deref().and_then(source) {
			Some((file, line, column)) => Origin::Source(file, line, column),
			None => Origin::Kind(entry.kind),
		}
	}

	fn insert(&mut self, parent: u64, child: u64, index: usize) {
		if parent == child || !self.nodes.contains_key(&parent) || !self.nodes.contains_key(&child)
		{
			return;
		}

		if let Some(previous) = self.nodes[&child].parent {
			self.detach(previous, child);
		}

		if let Some(entry) = self.nodes.get_mut(&parent) {
			let index = index.min(entry.children.len());
			entry.children.insert(index, child);
		}

		if let Some(entry) = self.nodes.get_mut(&child) {
			entry.parent = Some(parent);
		}
	}

	fn detach(&mut self, parent: u64, child: u64) {
		if let Some(entry) = self.nodes.get_mut(&parent) {
			entry.children.retain(|existing| *existing != child);
		}

		if let Some(entry) = self.nodes.get_mut(&child)
			&& entry.parent == Some(parent)
		{
			entry.parent = None;
		}
	}

	fn delete(&mut self, id: u64) {
		let Some(entry) = self.nodes.remove(&id) else {
			return;
		};

		if let Some(parent) = entry.parent
			&& let Some(parent) = self.nodes.get_mut(&parent)
		{
			parent.children.retain(|existing| *existing != id);
		}

		for child in entry.children {
			if let Some(child) = self.nodes.get_mut(&child) {
				child.parent = None;
			}
		}
	}
}

/// A `file:line:column` location, as the guest and the web renderer write
/// it.
fn source(text: &str) -> Option<(&str, u32, u32)> {
	let mut parts = text.rsplitn(3, ':');
	let column = parts.next()?.parse().ok()?;
	let line = parts.next()?.parse().ok()?;
	let file = parts.next().filter(|file| !file.is_empty())?;
	Some((file, line, column))
}

#[cfg(test)]
mod tests {
	use leptoskit_native::Element;
	use leptoskit_native::HeadlessBackend;
	use leptoskit_native::NativeSurface;

	use super::*;

	/// A surface that has committed `element`, and a mirror fed the same
	/// mutations, with each node's classes and source as a guest sends them.
	fn mirrored(element: &Element) -> (NativeSurface<HeadlessBackend>, Mirror) {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(320.0, 480.0);
		surface.set_root(element);
		let mut mirror = Mirror::new();
		feed(&mut surface, &mut mirror);
		(surface, mirror)
	}

	fn feed(surface: &mut NativeSurface<HeadlessBackend>, mirror: &mut Mirror) {
		for mutation in surface.backend_mut().take_log() {
			mirror.apply(Change::of(&mutation));
		}

		for node in surface.devtools_tree().nodes {
			let NodeRef::Native(id) = node.id else {
				unreachable!("a native tree names native nodes");
			};
			mirror.apply(Change::Details {
				id,
				classes: node.classes.join(" "),
				source: node
					.source
					.map(|source| format!("{}:{}:{}", source.file, source.line, source.column)),
			});
		}

		mirror.set_root(surface.root().map(|root| root.index() as u64));
	}

	fn screen(rows: usize) -> Element {
		Element::view()
			.class("flex-col gap-2 p-4")
			.child(Element::text("Title").class("text-lg"))
			.children((0..rows).map(|row| {
				Element::view()
					.class("flex-row gap-1")
					.child(Element::text(format!("Row {row}")))
					.child(Element::button("Open"))
			}))
	}

	#[test]
	fn the_mirror_matches_the_surface_it_follows() {
		let (surface, mirror) = mirrored(&screen(3));
		let expected = surface.devtools_tree();
		let tree = mirror.tree().unwrap();
		assert_eq!(tree.root, expected.root);
		assert_eq!(tree.nodes.len(), expected.nodes.len());

		for (node, expected) in tree.nodes.iter().zip(&expected.nodes) {
			assert_eq!(node.id, expected.id);
			assert_eq!(node.key, expected.key, "{} keys", node.kind);
			assert_eq!(node.parent, expected.parent);
			assert_eq!(node.kind, expected.kind);
			assert_eq!(node.source, expected.source);
			assert_eq!(node.classes, expected.classes);
			assert_eq!(node.text, expected.text);
			assert_eq!(node.children, expected.children);
		}

		let highlights: HashMap<usize, leptoskit_native::surface::Frame> =
			surface.highlight_rects().collect();

		for node in &tree.nodes {
			let NodeRef::Native(id) = node.id else {
				unreachable!("a native tree names native nodes");
			};
			let frame = node.frame.unwrap();
			let expected = highlights[&(id as usize)];
			assert_eq!(
				(frame.x, frame.y, frame.width, frame.height),
				(expected.x, expected.y, expected.width, expected.height),
				"{} frames are in viewport space",
				node.kind
			);
		}
	}

	#[test]
	fn the_mirror_follows_later_commits() {
		let (mut surface, mut mirror) = mirrored(&screen(3));
		surface.set_root(&screen(1));
		feed(&mut surface, &mut mirror);
		surface.set_root(&screen(4).class("p-8"));
		feed(&mut surface, &mut mirror);
		let tree = mirror.tree().unwrap();
		let expected = surface.devtools_tree();
		let ids = |tree: &Tree| {
			tree.nodes
				.iter()
				.map(|node| (node.id.clone(), node.key.clone(), node.children.clone()))
				.collect::<Vec<_>>()
		};
		assert_eq!(ids(&tree), ids(&expected));
		assert_eq!(tree.nodes[0].classes, ["flex-col", "gap-2", "p-4", "p-8"]);
		assert_eq!(mirror.len(), tree.nodes.len(), "deleted widgets are gone");
	}

	#[test]
	fn text_and_layout_changes_reach_the_mirror() {
		let mut mirror = Mirror::new();
		mirror.apply(Change::Create {
			id: 1,
			kind: NativeKind::View,
			text: None,
		});
		mirror.apply(Change::Create {
			id: 2,
			kind: NativeKind::Text,
			text: Some("before".into()),
		});
		mirror.apply(Change::Insert {
			parent: 1,
			child: 2,
			index: 0,
		});
		let frame = |x, y| {
			Frame {
				x,
				y,
				width: 10.0,
				height: 5.0,
			}
		};
		mirror.apply(Change::Layout {
			id: 1,
			frame: frame(4.0, 8.0),
		});
		mirror.apply(Change::Layout {
			id: 2,
			frame: frame(1.0, 2.0),
		});
		mirror.apply(Change::Text {
			id: 2,
			text: Some("after".into()),
		});
		assert!(mirror.tree().is_none(), "nothing is shown before a root");
		mirror.set_root(Some(1));
		let tree = mirror.tree().unwrap();
		assert_eq!(tree.nodes[1].text.as_deref(), Some("after"));
		assert_eq!(tree.nodes[1].frame, Some(frame(5.0, 10.0)));
		assert_eq!(tree.nodes[1].kind, "Text");

		mirror.apply(Change::Remove {
			parent: 1,
			child: 2,
		});
		mirror.apply(Change::Delete { id: 2 });
		let tree = mirror.tree().unwrap();
		assert_eq!(tree.nodes.len(), 1);
		assert_eq!(tree.nodes[0].children, [] as [NodeRef; 0]);
	}

	#[test]
	fn a_source_without_a_location_falls_back_to_the_kind() {
		assert_eq!(source("src/app.rs:12:5"), Some(("src/app.rs", 12, 5)));
		assert_eq!(
			source("C:\\app\\src\\a.rs:3:1"),
			Some(("C:\\app\\src\\a.rs", 3, 1))
		);
		assert_eq!(source("src/app.rs"), None);
		assert_eq!(source(":1:2"), None);
	}
}
