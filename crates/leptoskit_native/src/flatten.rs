//! View flattening: layout-only containers get no native view.
//!
//! A `View` that only arranges its children (no background, border, corner
//! radius, opacity, clipping, hiding, accessibility, or id) draws nothing,
//! yet each one costs a native view: creation, layout, and a layer the
//! system compositor draws every frame. React Native's Fabric flattens such
//! views into their parents, and so does [`Flattener`]: it translates the
//! surface's mutation stream, which describes every element, into one for a
//! native tree without the layout-only containers. Their children join the
//! nearest native ancestor at the right index, with frames offset by the
//! flattened containers' positions.
//!
//! A flattened container that later needs a view, because it gained a
//! background, say, becomes a native view in place and takes its descendants
//! back. It stays one: promotion is never undone, which keeps the stream
//! simple and the tree stable.
//!
//! Hit testing and event dispatch run on the surface's own tree, so they are
//! unaffected; only what the platform draws changes.

use std::collections::HashMap;

use leptoskit_yoga::NodeId;

use crate::Mutation;
use crate::MutationList;
use crate::NativeKind;
use crate::NativeProps;
use crate::aria::Aria;
use crate::hash::FastHash;

/// Whether an element of `kind` with `props` draws nothing of its own.
///
/// Text styling, disabled state, and accessibility hiding are inherited:
/// the surface has already copied them to every descendant, so on a
/// container they draw nothing. Everything else must be unset, so a property
/// added later keeps its container a native view until it is known to be
/// safe here.
#[must_use]
pub fn is_layout_only(kind: NativeKind, props: &NativeProps) -> bool {
	if kind != NativeKind::View {
		return false;
	}

	let mut rest = props.clone();
	rest.color = None;
	rest.font_size = None;
	rest.font_weight = None;
	rest.text_align = None;
	rest.direction = None;
	rest.enabled = None;

	if rest.aria.is_some() {
		rest.aria_mut().hidden = None;

		if *rest.aria() == Aria::EMPTY {
			rest.aria = None;
		}
	}

	rest == NativeProps::default()
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Frame {
	x: f32,
	y: f32,
	width: f32,
	height: f32,
}

#[derive(Debug)]
struct Node {
	kind: NativeKind,
	props: NativeProps,
	parent: Option<NodeId>,
	children: Vec<NodeId>,
	/// Its layout frame inside its parent, once one has been reported.
	frame: Option<Frame>,
	/// Whether the platform has a view for this element.
	native: bool,
	/// Created but not yet inserted anywhere: the decision waits for the
	/// insertion, because only the root is never inserted and it must stay
	/// native.
	undecided: bool,
	/// For a flattened node, the native views it stands for among its native
	/// ancestor's children; one for a native node.
	roots: usize,
	/// The native views its children stand for: their `roots`, summed. A
	/// flattened node's `roots` equals it; a native node holds that many
	/// native children.
	inner: usize,
}

/// Translates a surface's mutation stream into a flattened native tree.
/// Keep one per surface and pass every batch through [`Self::translate`] in
/// order.
#[derive(Debug, Default)]
pub struct Flattener {
	nodes: HashMap<NodeId, Node, FastHash>,
}

impl Flattener {
	/// Translates one commit's mutations for the platform.
	pub fn translate(&mut self, mutations: Vec<Mutation>) -> Vec<Mutation> {
		let mut out = MutationList::new();

		for mutation in mutations {
			self.apply(mutation, &mut out);
		}

		// A node created but never inserted and never given children is a
		// new root with no content yet; it must exist natively.
		let undecided: Vec<NodeId> = self
			.nodes
			.iter()
			.filter(|(_, node)| node.undecided)
			.map(|(id, _)| *id)
			.collect();

		for id in undecided {
			self.decide(id, true, &mut out);
		}

		out.into_mutations()
	}

	/// Whether the platform has a view for `id`.
	#[must_use]
	pub fn is_native(&self, id: NodeId) -> bool {
		self.nodes.get(&id).is_some_and(|node| node.native)
	}

	fn node(&self, id: NodeId) -> &Node {
		self.nodes
			.get(&id)
			.expect("the surface names only elements it created")
	}

	fn node_mut(&mut self, id: NodeId) -> &mut Node {
		self.nodes
			.get_mut(&id)
			.expect("the surface names only elements it created")
	}

	fn apply(&mut self, mutation: Mutation, out: &mut MutationList) {
		match mutation {
			Mutation::Create { id, kind, props } => {
				self.nodes.insert(
					id,
					Node {
						kind,
						props,
						parent: None,
						children: Vec::new(),
						frame: None,
						native: false,
						undecided: true,
						roots: 0,
						inner: 0,
					},
				);
			}
			Mutation::InsertChild {
				parent,
				child,
				index,
			} => self.insert(parent, child, index, out),
			Mutation::RemoveChild { parent, child } => self.remove(parent, child, out),
			Mutation::Delete { id } => {
				if self.nodes.remove(&id).is_some_and(|node| node.native) {
					out.push(Mutation::Delete { id });
				}
			}
			Mutation::UpdateProps { id, props } => {
				let node = self.node_mut(id);
				node.props = props.clone();

				if node.native {
					out.push(Mutation::UpdateProps { id, props });
				} else if !node.undecided && !is_layout_only(node.kind, &node.props) {
					self.promote(id, out);
				}
			}
			Mutation::UpdateLayout {
				id,
				x,
				y,
				width,
				height,
			} => {
				self.node_mut(id).frame = Some(Frame {
					x,
					y,
					width,
					height,
				});

				if self.node(id).native {
					self.emit_frame(id, out);
				} else {
					// Moving a flattened container moves every native view it
					// stands for.
					self.emit_frames_within(id, out);
				}
			}
		}
	}

	/// Settles whether `id` gets a native view, creating it if so.
	fn decide(&mut self, id: NodeId, native: bool, out: &mut MutationList) {
		let node = self.node_mut(id);
		node.undecided = false;
		node.native = native;
		node.roots = usize::from(native);

		if native {
			out.push(Mutation::Create {
				id,
				kind: node.kind,
				props: node.props.clone(),
			});
		}
	}

	fn insert(&mut self, parent: NodeId, child: NodeId, index: usize, out: &mut MutationList) {
		// A parent still undecided is a root receiving children before it
		// is inserted anywhere: it stays native.
		if self.node(parent).undecided {
			self.decide(parent, true, out);
		}

		if self.node(child).undecided {
			let flatten = is_layout_only(self.node(child).kind, &self.node(child).props);
			self.decide(child, !flatten, out);
		}

		// The child's place among the native views, found before it joins
		// its siblings so the counts it is measured against exclude it.
		let index = index.min(self.node(parent).children.len());
		let start = self.native_index(parent, index);
		self.node_mut(parent).children.insert(index, child);
		self.node_mut(child).parent = Some(parent);

		let roots = self.native_roots(child);
		let host = self.native_host(parent);

		for (offset, root) in roots.iter().enumerate() {
			out.push(Mutation::InsertChild {
				parent: host,
				child: *root,
				index: start + offset,
			});
		}

		self.adjust_roots(parent, roots.len(), 0);

		for root in roots {
			self.emit_frame(root, out);
		}
	}

	fn remove(&mut self, parent: NodeId, child: NodeId, out: &mut MutationList) {
		let roots = self.native_roots(child);
		let host = self.native_host(parent);

		for root in &roots {
			out.push(Mutation::RemoveChild {
				parent: host,
				child: *root,
			});
		}

		self.adjust_roots(parent, 0, roots.len());
		self.node_mut(parent).children.retain(|&id| id != child);
		self.node_mut(child).parent = None;
	}

	/// Gives a flattened container a native view, in place, and moves the
	/// views it stood for into it.
	fn promote(&mut self, id: NodeId, out: &mut MutationList) {
		let parent = self
			.node(id)
			.parent
			.expect("a flattened container is always inserted");
		let roots = self.native_roots(id);
		let host = self.native_host(parent);
		let index = self
			.node(parent)
			.children
			.iter()
			.rposition(|&child| child == id)
			.expect("a child is listed under its parent");
		let start = self.native_index(parent, index);

		for root in &roots {
			out.push(Mutation::RemoveChild {
				parent: host,
				child: *root,
			});
		}

		let node = self.node_mut(id);
		node.native = true;
		node.roots = 1;
		out.push(Mutation::Create {
			id,
			kind: node.kind,
			props: node.props.clone(),
		});
		out.push(Mutation::InsertChild {
			parent: host,
			child: id,
			index: start,
		});

		for (offset, root) in roots.iter().enumerate() {
			out.push(Mutation::InsertChild {
				parent: id,
				child: *root,
				index: offset,
			});
		}

		self.adjust_roots(parent, 1, roots.len());
		self.emit_frame(id, out);

		for root in roots {
			self.emit_frame(root, out);
		}
	}

	/// The native views `id` stands for: itself, or for a flattened
	/// container, its descendants' views in order.
	fn native_roots(&self, id: NodeId) -> Vec<NodeId> {
		let node = self.node(id);

		if node.native {
			return vec![id];
		}

		node.children
			.iter()
			.flat_map(|&child| self.native_roots(child))
			.collect()
	}

	/// The nearest native element at or above `id`, which holds the native
	/// views of `id`'s children.
	fn native_host(&self, mut id: NodeId) -> NodeId {
		while !self.node(id).native {
			id = self.node(id).parent.expect("the root is always native");
		}

		id
	}

	/// The native index, inside [`Self::native_host`], of what `parent`'s
	/// child at `index` would occupy.
	///
	/// It counts the native views before `index` from whichever end of the
	/// children is nearer, and finds a flattened parent's own place by
	/// searching from the end: building a screen appends, so both are
	/// constant time there rather than growing with the number of siblings.
	fn native_index(&self, parent: NodeId, index: usize) -> usize {
		let node = self.node(parent);
		let index = index.min(node.children.len());
		let roots = |children: &[NodeId]| -> usize {
			children.iter().map(|child| self.node(*child).roots).sum()
		};
		let before = if index * 2 <= node.children.len() {
			roots(&node.children[..index])
		} else {
			node.inner - roots(&node.children[index..])
		};

		if node.native {
			return before;
		}

		let grandparent = node
			.parent
			.expect("a flattened container is always inserted");
		let position = self
			.node(grandparent)
			.children
			.iter()
			.rposition(|&child| child == parent)
			.expect("a child is listed under its parent");
		self.native_index(grandparent, position) + before
	}

	/// Updates the view counts of `id` and its flattened ancestors, through
	/// the first native one, after `added` views joined and `removed` left.
	fn adjust_roots(&mut self, mut id: NodeId, added: usize, removed: usize) {
		loop {
			let node = self.node_mut(id);
			node.inner = node.inner + added - removed;

			if node.native {
				return;
			}

			node.roots = node.inner;
			id = node
				.parent
				.expect("a flattened container is always inserted");
		}
	}

	/// The offset of `id`'s frame from its native host: the positions of the
	/// flattened containers between them.
	fn host_offset(&self, id: NodeId) -> (f32, f32) {
		let mut offset = (0.0, 0.0);
		let mut current = self.node(id).parent;

		while let Some(ancestor) = current {
			let node = self.node(ancestor);

			if node.native {
				break;
			}

			let frame = node.frame.unwrap_or_default();
			offset.0 += frame.x;
			offset.1 += frame.y;
			current = node.parent;
		}

		offset
	}

	/// Reports `id`'s frame relative to its native host, once its layout is
	/// known; a view inserted before layout gets its frame when layout runs.
	fn emit_frame(&self, id: NodeId, out: &mut MutationList) {
		let Some(frame) = self.node(id).frame else {
			return;
		};
		let (dx, dy) = self.host_offset(id);
		out.push(Mutation::UpdateLayout {
			id,
			x: frame.x + dx,
			y: frame.y + dy,
			width: frame.width,
			height: frame.height,
		});
	}

	fn emit_frames_within(&self, id: NodeId, out: &mut MutationList) {
		for root in self.native_roots(id) {
			self.emit_frame(root, out);
		}
	}
}
