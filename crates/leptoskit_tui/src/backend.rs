//! The backend's committed widget tree and the shared backend contract.

use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::SharedTextMeasurer;
use leptoskit_yoga::NodeId;

use crate::buffer::Buffer;
use crate::input::Dispatch;
use crate::input::Key;
use crate::input::Mouse;
use crate::text::CELL_HEIGHT;
use crate::text::CELL_WIDTH;
use crate::text::TuiTextMeasurer;

/// One committed widget: its kind, properties, tree links, and frame.
pub struct Node {
	/// The widget kind.
	pub kind: NativeKind,
	/// The replacement properties from the last commit.
	pub props: NativeProps,
	/// The parent, or `None` for the root.
	pub parent: Option<NodeId>,
	/// The children, in order.
	pub children: Vec<NodeId>,
	/// The frame relative to the parent's border-box origin.
	pub frame: Frame,
}

/// A committed frame, relative to the parent.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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

impl Frame {
	/// The point just past the frame's right edge.
	pub const fn right(self) -> f32 {
		self.x + self.width
	}

	/// The point just past the frame's bottom edge.
	pub const fn bottom(self) -> f32 {
		self.y + self.height
	}
}

/// A text field's editable draft: the value the backend last proposed, the
/// caret's character index, and the value the application last committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
	/// The field's current text, including any uncommitted keystrokes.
	pub value: String,
	/// The caret's position, in characters.
	pub caret: usize,
	/// The value the backend last emitted, so an application echo preserves
	/// the caret while an application rewrite replaces the draft.
	pub emitted: Option<String>,
}

/// A scroll view's offset into its content, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scroll {
	/// The horizontal offset.
	pub x: f32,
	/// The vertical offset.
	pub y: f32,
}

/// The terminal backend: the cell grid's widget tree.
///
/// It implements [`NativeBackend`] over a plain node store, owns focus, text
/// drafts, carets, and scroll offsets the way GTK owns its own input state,
/// and paints through [`TuiBackend::render`]. Nothing here touches terminal
/// input or output; embed it in a
/// [`NativeSurface`](leptoskit_native::NativeSurface) and drive it with
/// [`TuiHost`](crate::TuiHost) or your own loop.
#[derive(Default)]
pub struct TuiBackend {
	slots: Vec<Option<Node>>,
	root: Option<NodeId>,
	focus: Option<NodeId>,
	pub(crate) drafts: Vec<Option<Draft>>,
	scrolls: Vec<Option<Scroll>>,
	dark: bool,
	/// The animation tick, advanced between repaints.
	tick: u64,
	/// Whether any animation is live, so the host knows when ticking matters.
	animating: bool,
	/// Events raised outside input handling, waiting for the host.
	pending: Vec<Dispatch>,
}

impl TuiBackend {
	/// An empty backend with no committed tree.
	pub fn new() -> Self {
		Self::default()
	}

	/// The committed root, once a tree has been set.
	pub fn root(&self) -> Option<NodeId> {
		self.root
	}

	/// The keyboard-focused widget, if any.
	pub fn focused(&self) -> Option<NodeId> {
		self.focus.filter(|&id| self.alive(id))
	}

	/// The committed node, when it still exists.
	pub fn node(&self, id: NodeId) -> Option<&Node> {
		self.slots.get(id.index()).and_then(Option::as_ref)
	}

	/// A node's committed properties, when it still exists.
	pub fn props(&self, id: NodeId) -> Option<&NativeProps> {
		self.node(id).map(|node| &node.props)
	}

	/// A node's children, in order; empty when it does not exist.
	pub fn children(&self, id: NodeId) -> &[NodeId] {
		self.node(id).map_or(&[], |node| &node.children)
	}

	/// A node's parent, when it exists.
	pub fn parent(&self, id: NodeId) -> Option<NodeId> {
		self.node(id).and_then(|node| node.parent)
	}

	/// Whether a node exists and is attached to the committed tree.
	pub fn alive(&self, id: NodeId) -> bool {
		self.node(id).is_some()
	}

	/// A node's frame in viewport coordinates, following scroll offsets the
	/// way hit testing and painting do.
	pub fn absolute_frame(&self, id: NodeId) -> Option<Frame> {
		let node = self.node(id)?;
		let mut frame = node.frame;
		let mut parent = node.parent;

		while let Some(id) = parent {
			let node = self.node(id)?;
			frame.x += node.frame.x;
			frame.y += node.frame.y;

			if let Some(scroll) = self.scrolls[id.index()] {
				frame.x -= scroll.x;
				frame.y -= scroll.y;
			}

			parent = node.parent;
		}

		Some(frame)
	}

	/// A text field's draft, when it has one.
	pub fn draft(&self, id: NodeId) -> Option<&Draft> {
		self.drafts.get(id.index()).and_then(Option::as_ref)
	}

	/// A scroll view's offset, when it has one.
	pub fn scroll(&self, id: NodeId) -> Option<Scroll> {
		self.scrolls.get(id.index()).copied().flatten()
	}

	/// The animation tick, which drives spinner frames and the caret.
	pub const fn tick(&self) -> u64 {
		self.tick
	}

	/// Whether the host's color scheme is dark.
	pub const fn dark_mode(&self) -> bool {
		self.dark
	}

	/// Paints the committed tree into a fresh cell buffer.
	///
	/// # Panics
	///
	/// Panics when either dimension is zero; a terminal always has at least
	/// one cell of each.
	pub fn render(&self, width: u16, height: u16) -> Buffer {
		crate::paint::paint(self, width, height)
	}

	/// Routes one key through the focused widget, updating drafts, scroll
	/// offsets, and focus, and returning the shared events to dispatch.
	pub fn handle_key(&mut self, key: &Key) -> Vec<Dispatch> {
		crate::input::handle_key(self, *key)
	}

	/// Routes one mouse event, returning the shared events to dispatch.
	pub fn handle_mouse(&mut self, mouse: &Mouse) -> Vec<Dispatch> {
		crate::input::handle_mouse(self, *mouse)
	}

	/// Advances the animation clock, reporting whether anything animates.
	pub fn advance_tick(&mut self) -> bool {
		self.tick = self.tick.wrapping_add(1);
		self.animating = self.animating();
		self.animating
	}

	/// Whether any visible node animates: a running spinner or an
	/// indeterminate progress bar.
	fn animating(&self) -> bool {
		let Some(root) = self.root else {
			return false;
		};

		let mut pending = vec![root];

		while let Some(id) = pending.pop() {
			let Some(node) = self.node(id) else {
				continue;
			};

			let visible =
				node.props.hidden != Some(true) && node.props.opacity.unwrap_or(1.0) > 0.0;

			if visible {
				match node.kind {
					NativeKind::ActivityIndicator => {
						if node.props.running.unwrap_or(true) {
							return true;
						}
					}
					NativeKind::LinearProgressIndicator
						if node.props.progress.unwrap_or_default()
							== leptoskit_native::Progress::Indeterminate =>
					{
						return true;
					}
					_ => {}
				}

				pending.extend(&node.children);
			}
		}

		false
	}

	/// The deepest node whose frame contains a viewport-space point, matching
	/// the shared hit-test rules: hidden and zero-size nodes never hit, a
	/// `Pressable`'s children are decorative, and clipped subtrees stop at
	/// their clip.
	pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
		let root = self.root?;
		let mut best = None;
		self.hit_walk(root, x, y, &mut best);
		best
	}

	fn hit_walk(&self, id: NodeId, x: f32, y: f32, best: &mut Option<NodeId>) {
		let Some(node) = self.node(id) else {
			return;
		};

		if node.props.hidden == Some(true)
			|| node.props.opacity.unwrap_or(1.0) <= 0.0
			|| node.frame.width <= 0.0
			|| node.frame.height <= 0.0
		{
			return;
		}

		let Some(frame) = self.absolute_frame(id) else {
			return;
		};

		let contains = x >= frame.x && x <= frame.right() && y >= frame.y && y <= frame.bottom();

		if contains {
			*best = Some(id);
		}

		if node.kind == NativeKind::Pressable || (!contains && node.props.clip == Some(true)) {
			return;
		}

		for &child in &node.children {
			self.hit_walk(child, x, y, best);
		}
	}

	/// Moves focus to the widget whose props ask for it with
	/// [`focused`](leptoskit_native::Element::focused), the last such widget
	/// in focus order, unless it already has it. The `Blur` and `Focus`
	/// events wait in [`Self::take_events`].
	fn claim_focus(&mut self) {
		let wanted = self
			.focusables()
			.into_iter()
			.rfind(|&node| self.props(node).and_then(|props| props.focused) == Some(true));

		if let Some(node) = wanted.filter(|&node| self.focus != Some(node)) {
			let events = self.set_focus(Some(node));
			self.pending.extend(events);
		}
	}

	/// The events the backend raised on its own since the last call, such as
	/// the `Blur` and `Focus` of a widget that asked for focus. Hosts
	/// dispatch them after each commit.
	pub fn take_events(&mut self) -> Vec<Dispatch> {
		std::mem::take(&mut self.pending)
	}

	/// Focuses a node, returning the `Blur` and `Focus` events the change
	/// dispatches.
	pub(crate) fn set_focus(&mut self, id: Option<NodeId>) -> Vec<Dispatch> {
		let mut events = Vec::new();

		if self.focus != id {
			if let Some(previous) = self.focus.take().filter(|&previous| self.alive(previous)) {
				events.push(Dispatch {
					node: previous,
					event: NativeEvent::Blur,
				});
			}

			if let Some(id) = id.filter(|&id| self.alive(id)) {
				self.focus = Some(id);
				events.push(Dispatch {
					node: id,
					event: NativeEvent::Focus,
				});
			}
		}

		events
	}

	/// The keyboard focus order, re-derived from the committed tree.
	pub(crate) fn focusables(&self) -> Vec<NodeId> {
		crate::focus::focusables(self)
	}

	/// Scrolls a scroll view by a delta in logical pixels, clamped to its
	/// content, and returns the event carrying the new offsets.
	pub(crate) fn scroll_by(&mut self, id: NodeId, dx: f32, dy: f32) -> Option<NativeEvent> {
		let node = self.node(id)?;
		let frame = node.frame;
		let extent = self.content_extent(id)?;

		let current = self.scrolls[id.index()].unwrap_or_default();
		let max_x = (extent.0 - frame.width).max(0.0);
		let max_y = (extent.1 - frame.height).max(0.0);
		let next = Scroll {
			x: (current.x + dx).clamp(0.0, max_x),
			y: (current.y + dy).clamp(0.0, max_y),
		};

		if next == current {
			return None;
		}

		self.scrolls[id.index()] = Some(next);
		Some(NativeEvent::Scroll {
			offset_x: next.x,
			offset_y: next.y,
		})
	}

	/// The size of a scroll view's content, in logical pixels relative to its
	/// own origin, from the frames of everything inside it.
	pub(crate) fn content_extent(&self, id: NodeId) -> Option<(f32, f32)> {
		let node = self.node(id)?;
		let mut width = 0.0f32;
		let mut height = 0.0f32;
		let mut pending = node.children.clone();

		while let Some(child) = pending.pop() {
			let Some(node) = self.node(child) else {
				continue;
			};

			width = width.max(node.frame.right());
			height = height.max(node.frame.bottom());
			pending.extend(&node.children);
		}

		Some((width, height))
	}

	/// Replaces a field's draft, keeping the caret when the application echoed
	/// the backend's proposal and clamping it otherwise.
	pub(crate) fn commit_draft(&mut self, id: NodeId, value: &str) {
		let chars = value.chars().count();
		let draft = self.drafts[id.index()].get_or_insert_with(|| {
			Draft {
				value: String::new(),
				caret: 0,
				emitted: None,
			}
		});

		if draft.value != value && draft.emitted.as_deref() != Some(value) {
			draft.caret = draft.caret.min(chars);
		}

		value.clone_into(&mut draft.value);
		draft.emitted = None;
	}

	/// Records the draft the backend itself last proposed.
	pub(crate) fn note_emitted(&mut self, id: NodeId, value: &str) {
		if let Some(draft) = self.drafts.get_mut(id.index()).and_then(Option::as_mut) {
			draft.emitted = Some(value.to_owned());
		}
	}

	/// Drops every node below `id`, as the shared tree's teardown order does.
	fn delete_slots(&mut self, id: NodeId) {
		let children = self
			.node(id)
			.map(|node| node.children.clone())
			.unwrap_or_default();

		for child in children {
			self.delete_slots(child);
		}

		if let Some(slot) = self.slots.get_mut(id.index()) {
			*slot = None;
		}

		if let Some(draft) = self.drafts.get_mut(id.index()) {
			*draft = None;
		}

		if let Some(scroll) = self.scrolls.get_mut(id.index()) {
			*scroll = None;
		}
	}
}

impl NativeBackend for TuiBackend {
	fn apply(&mut self, mutations: &leptoskit_native::MutationList) {
		for mutation in mutations.mutations() {
			self.apply_one(mutation);
		}

		self.claim_focus();
	}

	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps) {
		if id.index() >= self.slots.len() {
			self.slots.resize_with(id.index() + 1, || None);
			self.drafts.resize_with(id.index() + 1, || None);
			self.scrolls.resize_with(id.index() + 1, || None);
		}

		self.slots[id.index()] = Some(Node {
			kind,
			props: props.clone(),
			parent: None,
			children: Vec::new(),
			frame: Frame::default(),
		});

		if matches!(kind, NativeKind::TextInput | NativeKind::PasswordInput) {
			let value = props
				.value
				.as_deref()
				.or(props.text.as_deref())
				.unwrap_or_default();

			self.drafts[id.index()] = Some(Draft {
				value: value.to_owned(),
				caret: value.chars().count(),
				emitted: None,
			});
		}

		// The mutation stream creates the root before any other node, and a
		// root replacement is the first create after the old root's delete.
		if self.root.is_none() {
			self.root = Some(id);
		}
	}

	fn update_props(&mut self, id: NodeId, props: &NativeProps) {
		let Some(node) = self.slots.get_mut(id.index()).and_then(Option::as_mut) else {
			return;
		};

		node.props = props.clone();

		if matches!(node.kind, NativeKind::TextInput | NativeKind::PasswordInput) {
			let value = props
				.value
				.as_deref()
				.or(props.text.as_deref())
				.unwrap_or_default();
			self.commit_draft(id, value);
		}
	}

	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32) {
		if let Some(node) = self.slots.get_mut(id.index()).and_then(Option::as_mut) {
			node.frame = Frame {
				x,
				y,
				width,
				height,
			};
		}
	}

	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		let Some(node) = self.slots.get_mut(parent.index()).and_then(Option::as_mut) else {
			return;
		};

		let index = index.min(node.children.len());
		node.children.insert(index, child);

		if let Some(child_node) = self.slots.get_mut(child.index()).and_then(Option::as_mut) {
			child_node.parent = Some(parent);
		}
	}

	fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		if let Some(node) = self.slots.get_mut(parent.index()).and_then(Option::as_mut) {
			node.children.retain(|&id| id != child);
		}

		if let Some(child_node) = self.slots.get_mut(child.index()).and_then(Option::as_mut) {
			child_node.parent = None;
		}
	}

	fn delete(&mut self, id: NodeId) {
		self.delete_slots(id);

		if self.root == Some(id) {
			self.root = None;
		}

		if self.focus == Some(id) {
			self.focus = None;
		}
	}

	fn text_measurer(&self) -> SharedTextMeasurer {
		TuiTextMeasurer::shared()
	}

	fn set_dark_mode(&mut self, dark: bool) {
		self.dark = dark;
	}
}

/// The viewport dimensions for a terminal size, in logical pixels.
#[must_use]
pub const fn viewport(cols: u16, rows: u16) -> (f32, f32) {
	(cols as f32 * CELL_WIDTH, rows as f32 * CELL_HEIGHT)
}

#[cfg(test)]
mod tests {
	use leptoskit_yoga::Yoga;

	use super::*;

	fn ids(count: usize) -> Vec<NodeId> {
		let mut yoga = Yoga::new();
		(0..count).map(|_| yoga.create_node()).collect()
	}

	#[test]
	fn tracks_the_root_through_insert_and_remove() {
		let mut backend = TuiBackend::new();
		let ids = ids(2);
		let [root, child] = ids[..] else {
			unreachable!();
		};

		NativeBackend::create(
			&mut backend,
			root,
			NativeKind::View,
			&NativeProps::default(),
		);
		NativeBackend::create(
			&mut backend,
			child,
			NativeKind::Text,
			&NativeProps::text("hi"),
		);
		NativeBackend::insert_child(&mut backend, root, child, 0);

		assert_eq!(backend.root(), Some(root));
		assert_eq!(backend.parent(child), Some(root));
		assert_eq!(backend.children(root), &[child]);

		NativeBackend::remove_child(&mut backend, root, child);
		assert_eq!(backend.parent(child), None);

		NativeBackend::delete(&mut backend, root);
		assert_eq!(backend.root(), None);
		assert!(!backend.alive(root));
	}

	#[test]
	fn text_fields_start_with_a_draft_and_clamp_the_caret_on_rewrites() {
		let mut backend = TuiBackend::new();
		let [field] = ids(1)[..] else {
			unreachable!();
		};
		let props = NativeProps {
			value: Some("hello".into()),
			..NativeProps::default()
		};

		NativeBackend::create(&mut backend, field, NativeKind::TextInput, &props);
		assert_eq!(backend.draft(field).map(|draft| draft.caret), Some(5));

		backend.note_emitted(field, "hello!");
		let mut echo = props.clone();
		echo.value = Some("hello!".into());
		NativeBackend::update_props(&mut backend, field, &echo);
		assert_eq!(backend.draft(field).map(|draft| draft.caret), Some(5));

		let mut rewrite = props.clone();
		rewrite.value = Some("no".into());
		NativeBackend::update_props(&mut backend, field, &rewrite);
		let draft = backend.draft(field).expect("draft survives");
		assert_eq!(draft.value, "no");
		assert_eq!(draft.caret, 2);
	}

	#[test]
	fn absolute_frame_follows_parent_offsets() {
		let mut backend = TuiBackend::new();
		let ids = ids(2);
		let [root, child] = ids[..] else {
			unreachable!();
		};

		NativeBackend::create(
			&mut backend,
			root,
			NativeKind::View,
			&NativeProps::default(),
		);
		NativeBackend::create(
			&mut backend,
			child,
			NativeKind::Text,
			&NativeProps::default(),
		);
		NativeBackend::insert_child(&mut backend, root, child, 0);
		NativeBackend::update_layout(&mut backend, root, 0.0, 0.0, 800.0, 600.0);
		NativeBackend::update_layout(&mut backend, child, 16.0, 32.0, 64.0, 16.0);

		let frame = backend.absolute_frame(child).expect("child frame");
		assert_eq!((frame.x, frame.y), (16.0, 32.0));
	}
}
