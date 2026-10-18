//! The surface: owns the shadow tree, diffs new element trees against it,
//! commits mutations to the backend, and dispatches events.

use std::sync::Arc;
use std::sync::RwLock;

use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Size;
use leptoskit_yoga::Yoga;

use crate::backend::Mutation;
use crate::backend::MutationList;
use crate::backend::NativeBackend;
use crate::element::Element;
use crate::element::HandlerEntry;
use crate::props::EventOutcome;
use crate::props::NativeEvent;
use crate::props::NativeKind;
use crate::props::NativeProps;

/// A committed widget frame, relative to the parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
	/// The x offset within the parent.
	pub x: f32,
	/// The y offset within the parent.
	pub y: f32,
	/// The border-box width.
	pub width: f32,
	/// The border-box height.
	pub height: f32,
}

/// A committed shadow node.
struct ShadowNode {
	kind: NativeKind,
	props: NativeProps,
	handlers: Vec<HandlerEntry>,
	children: Vec<NodeId>,
	parent: Option<NodeId>,
	/// Live text content shared with the yoga measure function, so a props
	/// update re-measures without rebuilding the node.
	text_cell: Option<Arc<RwLock<String>>>,
}

/// Owns one interface: the diffing renderer, the layout tree, and the backend.
///
/// The flow mirrors React Native's renderer: describe the whole interface as
/// an [`Element`] tree each time state changes, call
/// [`set_root`](NativeSurface::set_root), and the surface computes the minimal
/// mutation batch, applies it to the backend, lays out, and pushes only the
/// changed frames.
pub struct NativeSurface<B: NativeBackend> {
	yoga: Yoga,
	shadows: Vec<ShadowNode>,
	backend: B,
	root: Option<NodeId>,
	viewport: Size<f32>,
	frames: Vec<Option<Frame>>,
}

impl<B: NativeBackend> NativeSurface<B> {
	/// Creates a surface over a backend.
	pub fn new(backend: B) -> Self {
		Self {
			yoga: Yoga::new(),
			shadows: Vec::new(),
			backend,
			root: None,
			viewport: Size::new(0.0, 0.0),
			frames: Vec::new(),
		}
	}

	/// The backend, for platform-specific calls outside the mutation stream.
	pub fn backend(&self) -> &B {
		&self.backend
	}

	/// Mutable access to the backend.
	pub fn backend_mut(&mut self) -> &mut B {
		&mut self.backend
	}

	/// The committed root, if one has been set.
	pub fn root(&self) -> Option<NodeId> {
		self.root
	}

	/// Sets the viewport size and re-lays-out when it changed.
	pub fn set_viewport(&mut self, width: f32, height: f32) {
		let changed = (self.viewport.width - width).abs() > f32::EPSILON
			|| (self.viewport.height - height).abs() > f32::EPSILON;
		if changed {
			self.viewport = Size::new(width, height);
			self.commit_layout();
		}
	}

	/// The committed frame of a node.
	pub fn frame(&self, id: NodeId) -> Option<Frame> {
		self.frames.get(id.index()).copied().flatten()
	}

	/// The committed children of a node, in order.
	pub fn children(&self, id: NodeId) -> &[NodeId] {
		&self.shadows[id.index()].children
	}

	/// The parent of a node, if it has one.
	pub fn parent(&self, id: NodeId) -> Option<NodeId> {
		self.shadows[id.index()].parent
	}

	/// The kind of a committed node.
	pub fn kind(&self, id: NodeId) -> NativeKind {
		self.shadows[id.index()].kind
	}

	/// The committed props of a node.
	pub fn props(&self, id: NodeId) -> &NativeProps {
		&self.shadows[id.index()].props
	}

	/// Commits a new element tree: diff, apply, lay out, push frames.
	pub fn set_root(&mut self, element: &Element) {
		let mut mutations = MutationList::new();
		let new_root = match self.root {
			None => self.build_subtree(None, element, &mut mutations),
			Some(root) => self.update_node(root, element, &mut mutations),
		};
		self.root = Some(new_root);
		self.backend.apply(&mutations);
		self.commit_layout();
	}

	/// Dispatches an event at a target, bubbling to ancestors until handled.
	pub fn dispatch_event(&self, target: NodeId, event: &NativeEvent) -> EventOutcome {
		let mut current = Some(target);
		let mut outcome = EventOutcome::Unhandled;
		while let Some(id) = current {
			for entry in &self.shadows[id.index()].handlers {
				if entry.event.matches(event) && (entry.handler)(event) == EventOutcome::Handled {
					outcome = EventOutcome::Handled;
				}
			}
			if outcome == EventOutcome::Handled {
				break;
			}
			current = self.shadows[id.index()].parent;
		}
		outcome
	}

	/// Finds the deepest node whose frame contains the point, in viewport
	/// coordinates.
	pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
		let root = self.root?;
		let mut best: Option<NodeId> = None;
		self.hit_test_walk(root, 0.0, 0.0, x, y, &mut best);
		best
	}

	/// The yoga layout of a committed node, in parent-relative coordinates.
	pub fn layout(&self, id: NodeId) -> leptoskit_yoga::Layout {
		*self.yoga.layout(id)
	}

	fn hit_test_walk(
		&self,
		id: NodeId,
		origin_x: f32,
		origin_y: f32,
		x: f32,
		y: f32,
		best: &mut Option<NodeId>,
	) {
		let layout = self.yoga.layout(id);
		let left = origin_x + layout.x;
		let top = origin_y + layout.y;
		let contains =
			x >= left && x <= left + layout.width && y >= top && y <= top + layout.height;
		if contains {
			*best = Some(id);
			for child in self.shadows[id.index()].children.clone() {
				self.hit_test_walk(child, left, top, x, y, best);
			}
		}
	}

	fn allocate(&mut self, kind: NativeKind) -> NodeId {
		let id = self.yoga.create_node();
		debug_assert_eq!(id.index(), self.shadows.len());
		self.shadows.push(ShadowNode {
			kind,
			props: NativeProps::default(),
			handlers: Vec::new(),
			children: Vec::new(),
			parent: None,
			text_cell: None,
		});
		self.frames.push(None);
		id
	}

	fn attach_text_measure(&mut self, id: NodeId, text: &str) {
		let measurer = self.backend.text_measurer();
		let cell = Arc::new(RwLock::new(text.to_owned()));
		let closure_cell = Arc::clone(&cell);
		self.yoga.set_measure_func(
			id,
			Some(Arc::new(move |available| {
				let content = closure_cell
					.read()
					.map(|guard| guard.clone())
					.unwrap_or_default();
				measurer.measure(&content, available)
			})),
		);
		self.shadows[id.index()].text_cell = Some(cell);
	}

	fn build_subtree(
		&mut self,
		parent: Option<NodeId>,
		element: &Element,
		out: &mut MutationList,
	) -> NodeId {
		let id = self.allocate(element.kind);
		self.shadows[id.index()].parent = parent;
		self.shadows[id.index()].props = element.props.clone();
		self.shadows[id.index()].handlers = element.handlers().to_vec();
		self.yoga.set_style(id, element.style.clone());
		if let Some(text) = element.props.text.as_ref() {
			self.attach_text_measure(id, text);
		}
		out.push(Mutation::Create {
			id,
			kind: element.kind,
			props: element.props.clone(),
		});
		if let Some(parent) = parent {
			let index = self.shadows[parent.index()].children.len();
			self.shadows[parent.index()].children.push(id);
			self.yoga.insert_child(parent, id, index);
			out.push(Mutation::InsertChild {
				parent,
				child: id,
				index,
			});
		}
		for child in &element.children {
			self.build_subtree(Some(id), child, out);
		}
		id
	}

	fn update_node(&mut self, id: NodeId, element: &Element, out: &mut MutationList) -> NodeId {
		let kind_changed = self.shadows[id.index()].kind != element.kind;
		if kind_changed {
			// A different widget kind is a different widget: replace the
			// subtree in place.
			let parent = self.shadows[id.index()].parent;
			let index = parent
				.and_then(|parent| {
					self.shadows[parent.index()]
						.children
						.iter()
						.position(|&c| c == id)
				})
				.unwrap_or(0);
			self.teardown(id, out);
			let replacement = self.build_subtree(parent, element, out);
			if let Some(parent) = parent {
				let children = &mut self.shadows[parent.index()].children;
				children.insert(index.min(children.len()), replacement);
				out.push(Mutation::InsertChild {
					parent,
					child: replacement,
					index: index.min(children.len().saturating_sub(1)),
				});
			}
			return replacement;
		}

		if self.shadows[id.index()].props != element.props {
			self.shadows[id.index()].props = element.props.clone();
			if let Some(text) = element.props.text.as_ref() {
				match &self.shadows[id.index()].text_cell {
					Some(cell) => {
						if let Ok(mut guard) = cell.write() {
							(*guard).clone_from(text);
						}
					}
					None => self.attach_text_measure(id, text),
				}
			}
			out.push(Mutation::UpdateProps {
				id,
				props: element.props.clone(),
			});
		}
		self.shadows[id.index()].handlers = element.handlers().to_vec();
		self.yoga.set_style(id, element.style.clone());

		let old_children = self.shadows[id.index()].children.clone();
		self.reconcile_children(id, &old_children, &element.children, out);
		id
	}

	fn reconcile_children(
		&mut self,
		parent: NodeId,
		old: &[NodeId],
		new: &[Element],
		out: &mut MutationList,
	) {
		for (index, element) in new.iter().enumerate() {
			match old.get(index) {
				Some(&child) => {
					self.update_node(child, element, out);
				}
				None => {
					self.build_subtree(Some(parent), element, out);
				}
			}
		}
		// Extra old children are removed back to front so the indices in the
		// removal mutations stay valid as the backend applies them.
		for &child in old.iter().skip(new.len()).rev() {
			self.teardown(child, out);
		}
	}

	/// Removes a subtree: `RemoveChild` at the root of it, then post-order
	/// `Delete` for every node.
	fn teardown(&mut self, id: NodeId, out: &mut MutationList) {
		let children = self.shadows[id.index()].children.clone();
		for child in children {
			self.teardown(child, out);
		}
		if let Some(parent) = self.shadows[id.index()].parent {
			self.shadows[parent.index()]
				.children
				.retain(|&child| child != id);
			self.yoga.detach(id);
			out.push(Mutation::RemoveChild { parent, child: id });
		}
		self.frames[id.index()] = None;
		out.push(Mutation::Delete { id });
	}

	fn commit_layout(&mut self) {
		let Some(root) = self.root else {
			return;
		};
		self.yoga.calculate_layout(
			root,
			Size::new(
				AvailableSpace::Definite(self.viewport.width.max(0.0)),
				AvailableSpace::Definite(self.viewport.height.max(0.0)),
			),
		);
		let mut frames = MutationList::new();
		self.collect_frames(root, &mut frames);
		self.backend.apply(&frames);
	}

	fn collect_frames(&mut self, id: NodeId, out: &mut MutationList) {
		let layout = *self.yoga.layout(id);
		let frame = Frame {
			x: layout.x,
			y: layout.y,
			width: layout.width,
			height: layout.height,
		};
		if self.frames.get(id.index()).copied().flatten() != Some(frame) {
			if self.frames.len() <= id.index() {
				self.frames.resize(id.index() + 1, None);
			}
			self.frames[id.index()] = Some(frame);
			out.push(Mutation::UpdateLayout {
				id,
				x: frame.x,
				y: frame.y,
				width: frame.width,
				height: frame.height,
			});
		}
		for child in self.shadows[id.index()].children.clone() {
			self.collect_frames(child, out);
		}
	}
}
