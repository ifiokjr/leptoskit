//! The surface: owns the shadow tree, diffs new element trees against it,
//! commits mutations to the backend, and dispatches events.

use std::sync::Arc;
use std::sync::RwLock;

use leptoskit_yoga::NodeId;
use leptoskit_yoga::Rect;
use leptoskit_yoga::Size;
use leptoskit_yoga::Yoga;

use crate::backend::NativeBackend;
use crate::element::Element;
use crate::element::HandlerEntry;
use crate::props::NativeKind;
use crate::props::NativeProps;
use crate::props::TextStyle;

mod events;
mod layout;
mod measurement;
mod reconcile;

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

/// A viewport-space box enclosing every point that can hit a node in a
/// subtree: the node's own frame plus, unless it clips or is a `Pressable`,
/// its descendants' boxes. Inert subtrees are [`HitBounds::EMPTY`].
#[derive(Debug, Clone, Copy)]
struct HitBounds {
	left: f32,
	top: f32,
	right: f32,
	bottom: f32,
}

impl HitBounds {
	const EMPTY: Self = Self {
		left: f32::INFINITY,
		top: f32::INFINITY,
		right: f32::NEG_INFINITY,
		bottom: f32::NEG_INFINITY,
	};

	fn union(self, other: Self) -> Self {
		Self {
			left: self.left.min(other.left),
			top: self.top.min(other.top),
			right: self.right.max(other.right),
			bottom: self.bottom.max(other.bottom),
		}
	}

	fn contains(&self, x: f32, y: f32) -> bool {
		x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
	}
}

/// What hit testing reads for a committed node, cached by each layout commit
/// so a query touches neither yoga nor the node's props until its subtree
/// bounds contain the point.
#[derive(Debug, Clone, Copy)]
struct HitArea {
	/// The node's own frame in viewport coordinates.
	left: f32,
	top: f32,
	width: f32,
	height: f32,
	/// The subtree's bounds. Its edges are the same `left + width` sums the
	/// walk tests against, so a point on an edge is never pruned by rounding.
	subtree: HitBounds,
}

impl HitArea {
	const DETACHED: Self = Self {
		left: 0.0,
		top: 0.0,
		width: 0.0,
		height: 0.0,
		subtree: HitBounds::EMPTY,
	};
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

impl ShadowNode {
	/// Hidden nodes and stopped activity indicators neither receive events
	/// nor are hit, and neither are their descendants.
	///
	/// Inlined because event dispatch, which other crates monomorphize,
	/// calls it for every ancestor.
	#[inline]
	fn is_inert(&self) -> bool {
		self.props.hidden == Some(true)
			|| (self.kind == NativeKind::ActivityIndicator && self.props.running == Some(false))
	}
}

/// The height a text field's native chrome adds around its text.
///
/// It holds GTK's entry border, AppKit's rounded bezel, and Android's
/// underline, so an unstyled field at the default font size is 28 tall, the
/// same as `PasswordInput`'s default, on every backend.
const TEXT_FIELD_CHROME_HEIGHT: f32 = 12.0;

/// Everything a text node's measurement depends on.
#[derive(Debug, Clone, PartialEq)]
struct MeasuredText {
	content: String,
	style: TextStyle,
	/// Whether the widget draws native chrome around the text.
	native_chrome: bool,
}

/// Whether `element` is a text control that keeps its native chrome.
///
/// Custom-drawn fields replace that chrome with their own border and
/// padding, which Yoga already adds around the text.
fn has_native_chrome(element: &Element) -> bool {
	matches!(
		element.kind,
		NativeKind::TextInput | NativeKind::PasswordInput | NativeKind::Button
	) && !element.props.is_custom_drawn()
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
		NativeKind::Button => Some(element.props.text.as_deref().unwrap_or_default()),
		NativeKind::Text | NativeKind::TextRun => element.props.text.as_deref(),
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
	/// Per-node hit areas, rebuilt by every layout commit. Kept apart from
	/// the shadow nodes so pruning reads a dense array.
	hit_areas: Vec<HitArea>,
	source: Option<Element>,
	dark: bool,
	safe_area: Rect<f32>,
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
			hit_areas: Vec::new(),
			source: None,
			dark: false,
			safe_area: Rect::default(),
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

	/// Sets the viewport edges covered by system UI, in logical pixels, and
	/// re-lays-out when they changed. `*-safe` padding utilities resolve
	/// against them; see [`crate::tailwind::StyleContext::safe_area`].
	pub fn set_safe_area(&mut self, insets: Rect<f32>) {
		if self.safe_area != insets {
			self.safe_area = insets;
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

	/// The yoga layout of a committed node, in parent-relative coordinates.
	pub fn layout(&self, id: NodeId) -> leptoskit_yoga::Layout {
		*self.yoga.layout(id)
	}
}
