//! The surface: owns the shadow tree, diffs new element trees against it,
//! commits mutations to the backend, and dispatches events.

use std::sync::Arc;
use std::sync::PoisonError;
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
use crate::props::TextStyle;
use crate::tailwind::StyleContext;

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
	/// Live text content and font shared with the yoga measure function, so a
	/// props update re-measures without rebuilding the node.
	text_cell: Option<Arc<RwLock<MeasuredText>>>,
}

/// Everything a text node's measurement depends on.
#[derive(Debug, Clone, PartialEq)]
struct MeasuredText {
	content: String,
	style: TextStyle,
}

/// Uses the string actually displayed by the widget for intrinsic sizing.
fn measured_content(element: &Element) -> Option<&str> {
	if matches!(
		element.kind,
		NativeKind::TextInput | NativeKind::PasswordInput
	) {
		return Some(
			element
				.props
				.value
				.as_deref()
				.or(element.props.text.as_deref())
				.filter(|value| !value.is_empty())
				.or(element.props.placeholder.as_deref())
				.unwrap_or_default(),
		);
	}

	match element.kind {
		NativeKind::Text | NativeKind::TextRun | NativeKind::Button => {
			element.props.text.as_deref()
		}
		_ => None,
	}
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
	source: Option<Element>,
	dark: bool,
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
			source: None,
			dark: false,
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
			self.commit_source();
		}
	}

	/// Applies the host's color scheme to native controls and retained `dark:`
	/// utilities. Call even for an initial light scheme to override system
	/// appearance in a host with a fixed light theme.
	pub fn set_dark_mode(&mut self, dark: bool) {
		self.backend.set_dark_mode(dark);
		if self.dark != dark {
			self.dark = dark;
			self.commit_source();
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
		self.try_set_root(element)
			.unwrap_or_else(|error| panic!("invalid leptoskit tree: {error}"));
	}

	/// Validates a complete tree before changing source, layout, or backend
	/// state. Invalid trees leave the previously committed interface intact.
	pub fn try_set_root(&mut self, element: &Element) -> Result<(), crate::ControlTreeError> {
		element.validate()?;
		self.source = Some(element.clone());
		self.commit_source();
		Ok(())
	}

	fn commit_source(&mut self) {
		let Some(source) = &self.source else {
			return;
		};
		let element = source.resolved(&StyleContext {
			viewport: self.viewport,
			dark: self.dark,
		});
		let mut mutations = MutationList::new();
		let new_root = match self.root {
			None => self.build_subtree(None, &element, &mut mutations),
			Some(root) => self.update_node(root, &element, &mut mutations),
		};
		self.root = Some(new_root);
		self.backend.apply(&mutations);
		self.commit_layout();
	}

	/// Dispatches an event at a target, bubbling to ancestors until handled.
	pub fn dispatch_event(&self, target: NodeId, event: &NativeEvent) -> EventOutcome {
		let target_node = &self.shadows[target.index()];
		let normalized;
		let event = match event {
			NativeEvent::Toggle { checked } => {
				if !matches!(
					target_node.kind,
					NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
				) || (target_node.kind == NativeKind::Radio && !checked)
				{
					return EventOutcome::Unhandled;
				}

				event
			}
			NativeEvent::ValueChange { value } => {
				if target_node.kind != NativeKind::Slider {
					return EventOutcome::Unhandled;
				}

				let state = target_node.props.slider.unwrap_or_default();
				let Some(value) = state.proposed_value(*value) else {
					return EventOutcome::Unhandled;
				};
				normalized = NativeEvent::ValueChange { value };
				&normalized
			}
			_ => event,
		};
		let mut ancestor = Some(target);

		while let Some(id) = ancestor {
			let node = &self.shadows[id.index()];

			if node.props.hidden == Some(true)
				|| node.props.enabled == Some(false)
				|| (node.kind == NativeKind::ActivityIndicator && node.props.running == Some(false))
			{
				return EventOutcome::Unhandled;
			}

			ancestor = node.parent;
		}

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
		let props = &self.shadows[id.index()].props;

		if props.hidden == Some(true)
			|| (self.shadows[id.index()].kind == NativeKind::ActivityIndicator
				&& props.running == Some(false))
		{
			return;
		}

		let layout = self.yoga.layout(id);
		let left = origin_x + layout.x;
		let top = origin_y + layout.y;
		let mut contains =
			x >= left && x <= left + layout.width && y >= top && y <= top + layout.height;
		if contains && props.clip == Some(true) {
			let radius = props
				.corner_radius
				.unwrap_or_default()
				.max(0.0)
				.min(layout.width.min(layout.height) * 0.5);
			let dx = x - x.clamp(left + radius, left + layout.width - radius);
			let dy = y - y.clamp(top + radius, top + layout.height - radius);
			contains = dx * dx + dy * dy <= radius * radius;
		}
		if contains {
			*best = Some(id);
		}

		if self.shadows[id.index()].kind == NativeKind::Pressable {
			return;
		}

		if contains || props.clip != Some(true) {
			for child in &self.shadows[id.index()].children {
				self.hit_test_walk(*child, left, top, x, y, best);
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

	fn attach_text_measure(&mut self, id: NodeId, text: &str, style: TextStyle) {
		let measurer = self.backend.text_measurer();
		let single_line = matches!(
			self.shadows[id.index()].kind,
			NativeKind::TextInput | NativeKind::PasswordInput
		);
		let cell = Arc::new(RwLock::new(MeasuredText {
			content: text.to_owned(),
			style,
		}));
		let closure_cell = Arc::clone(&cell);
		self.yoga.set_measure_func(
			id,
			Some(Arc::new(move |mut available| {
				// Writers only ever replace whole values, so a poisoned lock
				// still holds a valid text and style.
				let current = closure_cell.read().unwrap_or_else(PoisonError::into_inner);
				if single_line {
					available.width = AvailableSpace::MaxContent;
				}
				measurer.measure(&current.content, current.style, available)
			})),
		);
		self.shadows[id.index()].text_cell = Some(cell);
	}

	/// Builds a fresh subtree for `element`.
	///
	/// When `slot` is `Some((parent, index))` the new root is inserted into
	/// `parent` at `index` — once, in the shadow tree, the layout tree, and the
	/// mutation stream — before its own children are built, so the batch stays
	/// top-down. With `None` the subtree is left detached.
	fn build_subtree(
		&mut self,
		slot: Option<(NodeId, usize)>,
		element: &Element,
		out: &mut MutationList,
	) -> NodeId {
		let id = self.allocate(element.kind);
		self.shadows[id.index()].props = element.props.clone();
		self.shadows[id.index()].handlers = element.handlers().to_vec();
		self.yoga.set_style(id, element.style.clone());
		if let Some(text) = measured_content(element) {
			self.attach_text_measure(id, text, element.props.text_style());
		}
		out.push(Mutation::Create {
			id,
			kind: element.kind,
			props: element.props.clone(),
		});
		if let Some((parent, index)) = slot {
			self.attach_child(parent, id, index, out);
		}
		for (index, child) in element.children.iter().enumerate() {
			self.build_subtree(Some((id, index)), child, out);
		}
		id
	}

	/// Inserts a detached node into `parent` at `index` (clamped to the
	/// current child count) and records the `InsertChild` mutation.
	fn attach_child(
		&mut self,
		parent: NodeId,
		child: NodeId,
		index: usize,
		out: &mut MutationList,
	) {
		debug_assert!(self.shadows[child.index()].parent.is_none());
		let children = &mut self.shadows[parent.index()].children;
		let index = index.min(children.len());
		children.insert(index, child);
		self.shadows[child.index()].parent = Some(parent);
		self.yoga.insert_child(parent, child, index);
		out.push(Mutation::InsertChild {
			parent,
			child,
			index,
		});
	}

	fn update_node(&mut self, id: NodeId, element: &Element, out: &mut MutationList) -> NodeId {
		let kind_changed = self.shadows[id.index()].kind != element.kind;
		if kind_changed {
			// A different widget kind is a different widget: tear the old
			// subtree down and build the replacement in the same slot.
			let slot = self.shadows[id.index()].parent.map(|parent| {
				let index = self.shadows[parent.index()]
					.children
					.iter()
					.position(|&child| child == id)
					.expect("a child is listed under its parent");
				(parent, index)
			});
			self.teardown(id, out);
			return self.build_subtree(slot, element, out);
		}

		if self.shadows[id.index()].props != element.props {
			self.shadows[id.index()].props = element.props.clone();
			let style = element.props.text_style();
			match (
				measured_content(element),
				self.shadows[id.index()].text_cell.clone(),
			) {
				(Some(text), Some(cell)) => {
					let mut measured = cell.write().unwrap_or_else(PoisonError::into_inner);
					// Colors, borders, and the like do not change the text's
					// size, so only a new string or font invalidates layout.
					if measured.content != text || measured.style != style {
						text.clone_into(&mut measured.content);
						measured.style = style;
						drop(measured);
						// The measure function reads the cell lazily, so the
						// node and its cached measurement must be invalidated
						// by hand.
						self.yoga.mark_dirty(id);
					}
				}
				(Some(text), None) => self.attach_text_measure(id, text, style),
				(None, Some(_)) => {
					self.yoga.set_measure_func(id, None);
					self.shadows[id.index()].text_cell = None;
				}
				(None, None) => {}
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
					self.build_subtree(Some((parent, index)), element, out);
				}
			}
		}
		// Extra old children are removed back to front so the indices in the
		// removal mutations stay valid as the backend applies them.
		for &child in old.iter().skip(new.len()).rev() {
			self.teardown(child, out);
		}
	}

	/// Removes a subtree, post-order: every node is detached from its parent
	/// (`RemoveChild`) and then deleted (`Delete`), so a widget never outlives
	/// its children and is always parentless when deleted.
	///
	/// The shadow slot stays allocated (ids are never reused), but it is left
	/// inert: no handlers, no parent, no children, and no measure function, so
	/// a stale id can neither receive events nor keep closures alive.
	fn teardown(&mut self, id: NodeId, out: &mut MutationList) {
		let children = std::mem::take(&mut self.shadows[id.index()].children);
		for child in children {
			self.teardown(child, out);
		}
		if let Some(parent) = self.shadows[id.index()].parent.take() {
			self.shadows[parent.index()]
				.children
				.retain(|&child| child != id);
			self.yoga.detach(id);
			out.push(Mutation::RemoveChild { parent, child: id });
		}
		let shadow = &mut self.shadows[id.index()];
		shadow.handlers.clear();
		if shadow.text_cell.take().is_some() {
			self.yoga.set_measure_func(id, None);
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
