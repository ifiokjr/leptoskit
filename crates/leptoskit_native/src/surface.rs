//! The surface: owns the shadow tree, diffs new element trees against it,
//! commits mutations to the backend, and dispatches events.

use std::sync::Arc;
use std::sync::RwLock;
#[cfg(feature = "devtools")]
use std::time::Instant;

#[cfg(feature = "devtools")]
use leptoskit_devtools_protocol::Commit;
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

#[cfg(feature = "devtools")]
mod devtools;
mod events;
mod layout;
mod measurement;
mod reconcile;
#[cfg(all(feature = "rive", not(target_arch = "wasm32")))]
mod rive;

#[cfg(all(feature = "rive", not(target_arch = "wasm32")))]
pub use rive::RiveEnvironment;
#[cfg(all(feature = "rive", not(target_arch = "wasm32")))]
pub use rive::RivePresenter;

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
	text_cell: Option<Arc<RwLock<TextCell>>>,
	/// Where the element that built this node was created.
	#[cfg(feature = "devtools")]
	source: Option<&'static std::panic::Location<'static>>,
	/// The `#[component]` whose body created that element.
	#[cfg(feature = "devtools")]
	component: Option<&'static str>,
	/// The element's active Tailwind classes.
	#[cfg(feature = "devtools")]
	classes: Vec<String>,
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

/// A text node's measurement inputs and the sizes already measured from them.
#[derive(Debug)]
struct TextCell {
	text: MeasuredText,
	/// Sizes by the width they were measured at. Text measurement depends on
	/// the offered width alone, while one layout pass offers each label
	/// several heights; keyed by width, creating 1,000 cards measures each
	/// label once instead of three times.
	sizes: [Option<(leptoskit_yoga::AvailableSpace, Size<f32>)>; 2],
}

impl TextCell {
	fn new(text: MeasuredText) -> Self {
		Self {
			text,
			sizes: [None; 2],
		}
	}

	fn size(&self, width: leptoskit_yoga::AvailableSpace) -> Option<Size<f32>> {
		self.sizes
			.iter()
			.flatten()
			.find(|(measured, _)| *measured == width)
			.map(|(_, size)| *size)
	}

	/// Keeps the newest two sizes: a label is offered its max-content width
	/// and its final width.
	fn remember(&mut self, width: leptoskit_yoga::AvailableSpace, size: Size<f32>) {
		self.sizes = [Some((width, size)), self.sizes[0]];
	}
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
/// A commit observer for the `DevTools` host agent.
#[cfg(feature = "devtools")]
type CommitObserver = Box<dyn FnMut(&Commit)>;

/// Commit observation for the `DevTools` host agent.
#[cfg(feature = "devtools")]
struct Devtools {
	observer: Option<CommitObserver>,
	profiling: bool,
	seq: u64,
	mutations: usize,
	apply_us: u64,
	started: Instant,
}

#[cfg(feature = "devtools")]
impl Default for Devtools {
	fn default() -> Self {
		Self {
			observer: None,
			profiling: false,
			seq: 0,
			mutations: 0,
			apply_us: 0,
			started: Instant::now(),
		}
	}
}

#[cfg(feature = "devtools")]
impl Devtools {
	fn source(&mut self, mutations: usize, start: Instant) {
		self.mutations += mutations;

		if self.profiling {
			self.apply_us += micros(start);
		}
	}

	fn finish(&mut self, layout_mutations: usize, layout_start: Instant) -> Commit {
		let commit = Commit {
			seq: self.seq,
			at_ms: millis(self.started),
			mutations: self.mutations + layout_mutations,
			layout_us: self.profiling.then(|| micros(layout_start)),
			apply_us: self.profiling.then_some(self.apply_us),
		};
		self.seq += 1;
		self.mutations = 0;
		self.apply_us = 0;
		commit
	}
}

#[cfg(feature = "devtools")]
fn micros(start: Instant) -> u64 {
	u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

#[cfg(feature = "devtools")]
fn millis(start: Instant) -> u64 {
	u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub struct NativeSurface<B: NativeBackend> {
	#[cfg(all(feature = "rive", not(target_arch = "wasm32")))]
	rive: rive::Players,
	yoga: Yoga,
	shadows: Vec<ShadowNode>,
	backend: B,
	root: Option<NodeId>,
	viewport: Size<f32>,
	frames: Vec<Option<Frame>>,
	/// Per-node hit areas, rebuilt by every layout commit. Kept apart from
	/// the shadow nodes so pruning reads a dense array.
	hit_areas: Vec<HitArea>,
	/// Scratch space for reconciliation: the children a commit walks, so the
	/// steady state allocates nothing to snapshot them.
	scratch_children: Vec<NodeId>,
	source: Option<Element>,
	dark: bool,
	safe_area: Rect<f32>,
	/// The control the user is holding down, resolved with its `active:`
	/// utilities until it is released or leaves the tree.
	pressed: Option<NodeId>,
	#[cfg(feature = "devtools")]
	devtools: Devtools,
}

impl<B: NativeBackend> NativeSurface<B> {
	/// Creates a surface over a backend.
	pub fn new(backend: B) -> Self {
		Self {
			#[cfg(all(feature = "rive", not(target_arch = "wasm32")))]
			rive: rive::Players::default(),
			yoga: Yoga::new(),
			shadows: Vec::new(),
			backend,
			root: None,
			viewport: Size::new(0.0, 0.0),
			frames: Vec::new(),
			hit_areas: Vec::new(),
			scratch_children: Vec::new(),
			source: None,
			dark: false,
			safe_area: Rect::default(),
			pressed: None,
			#[cfg(feature = "devtools")]
			devtools: Devtools::default(),
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

	/// The viewport the surface lays out against, in logical pixels.
	pub fn viewport(&self) -> Size<f32> {
		self.viewport
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

	/// Starts holding a `Button` or `Pressable` down, as a backend's
	/// [`NativeEvent::PressIn`](crate::NativeEvent::PressIn) reports, and
	/// re-renders it with its `active:` utilities (see
	/// [`Element::resolved`]).
	///
	/// One control is held at a time: pressing another releases the first.
	/// Other kinds, detached nodes, and nodes that are disabled or hidden,
	/// or inside a disabled or hidden ancestor, are ignored.
	pub fn press_in(&mut self, target: NodeId) {
		let Some(shadow) = self.shadows.get(target.index()) else {
			return;
		};

		if self.pressed == Some(target)
			|| !crate::element::is_pressable(shadow.kind)
			|| !self.is_committed(target)
			|| !self.event_path_is_active(target)
		{
			return;
		}

		self.pressed = Some(target);
		self.commit_source();
	}

	/// Releases a held control, as a backend's
	/// [`NativeEvent::PressOut`](crate::NativeEvent::PressOut) reports, and
	/// re-renders it without its `active:` utilities. Releasing a control
	/// that is not held does nothing.
	pub fn press_out(&mut self, target: NodeId) {
		if self.pressed == Some(target) {
			self.pressed = None;
			self.commit_source();
		}
	}

	/// The control currently held down, if any.
	pub fn pressed(&self) -> Option<NodeId> {
		self.pressed
	}

	/// Whether a node is part of the committed tree, rather than a slot left
	/// behind by a teardown.
	fn is_committed(&self, id: NodeId) -> bool {
		self.root == Some(id) || self.shadows[id.index()].parent.is_some()
	}

	/// The child indices from the root down to a committed node.
	fn path_to(&self, id: NodeId) -> Vec<usize> {
		let mut path = Vec::new();
		let mut current = id;

		while let Some(parent) = self.shadows[current.index()].parent {
			let index = self.shadows[parent.index()]
				.children
				.iter()
				.position(|&child| child == current)
				.expect("a child is listed under its parent");
			path.push(index);
			current = parent;
		}

		path.reverse();
		path
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

	/// Number of allocated shadow slots, including inert deleted nodes.
	/// IDs remain stable for the surface's lifetime, so sandbox hosts use
	/// this count to bound cumulative widget replacement allocations.
	pub fn allocated_nodes(&self) -> usize {
		self.shadows.len()
	}

	/// Commits a new element tree: diff, apply, lay out, push frames.
	pub fn set_root(&mut self, element: &Element) {
		self.try_set_root(element)
			.unwrap_or_else(|error| panic!("invalid leptoskit tree: {error}"));
	}

	/// Validates a complete tree before changing source, layout, or backend
	/// state. Invalid trees leave the previously committed interface intact.
	pub fn try_set_root(&mut self, element: &Element) -> Result<(), crate::ControlTreeError> {
		self.try_set_owned_root(element.clone())
	}

	/// Commits a tree the caller hands over, so a host that builds one fresh
	/// per render does not clone it to keep the source.
	pub fn set_owned_root(&mut self, element: Element) {
		self.try_set_owned_root(element)
			.unwrap_or_else(|error| panic!("invalid leptoskit tree: {error}"));
	}

	/// [`try_set_root`](Self::try_set_root) without the clone.
	pub fn try_set_owned_root(&mut self, element: Element) -> Result<(), crate::ControlTreeError> {
		element.validate()?;
		self.source = Some(element);
		self.commit_source();
		Ok(())
	}

	/// The yoga layout of a committed node, in parent-relative coordinates.
	pub fn layout(&self, id: NodeId) -> leptoskit_yoga::Layout {
		*self.yoga.layout(id)
	}

	/// Records every commit, for the `DevTools` host agent.
	#[cfg(feature = "devtools")]
	pub fn set_commit_observer(&mut self, observer: CommitObserver) {
		self.devtools.observer = Some(observer);
	}

	/// Turns commit timing on or off.
	#[cfg(feature = "devtools")]
	pub fn set_profiling(&mut self, on: bool) {
		self.devtools.profiling = on;
	}

	/// What the surface resolves classes against: its viewport, appearance,
	/// and safe area. `active` is unset; only [`Self::pressed`] has it.
	#[cfg(feature = "devtools")]
	pub fn style_context(&self) -> crate::tailwind::StyleContext {
		crate::tailwind::StyleContext {
			viewport: self.viewport,
			dark: self.dark,
			safe_area: self.safe_area,
			active: false,
		}
	}

	/// The viewport-space highlight rectangle for every committed node, keyed
	/// by its dense node index.
	#[cfg(feature = "devtools")]
	pub fn highlight_rects(&self) -> impl Iterator<Item = (usize, Frame)> + '_ {
		self.hit_areas
			.iter()
			.enumerate()
			.filter(|(_, area)| area.width > 0.0 || area.height > 0.0)
			.map(|(index, area)| {
				(
					index,
					Frame {
						x: area.left,
						y: area.top,
						width: area.width,
						height: area.height,
					},
				)
			})
	}
}

#[cfg(all(test, feature = "devtools"))]
mod devtools_tests {
	use std::sync::Arc;
	use std::sync::Mutex;

	use super::NativeSurface;
	use crate::Element;
	use crate::HeadlessBackend;

	#[test]
	fn a_commit_reaches_the_observer() {
		let seen = Arc::new(Mutex::new(Vec::new()));
		let commits = Arc::clone(&seen);
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_commit_observer(Box::new(move |commit| {
			commits.lock().unwrap().push(commit.clone());
		}));
		let element = Element::view();
		surface.set_root(&element);
		let commits = seen.lock().unwrap();
		assert!(!commits.is_empty());

		assert!(commits.last().unwrap().mutations > 0);
	}

	#[test]
	fn highlight_rects_cover_committed_nodes() {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(100.0, 100.0);
		let element = Element::view();
		surface.set_root(&element);
		let rects: Vec<_> = surface.highlight_rects().collect();
		assert_ne!(rects, Vec::new());
		assert!(
			rects
				.iter()
				.any(|(_, frame)| frame.width > 0.0 || frame.height > 0.0)
		);
	}

	#[test]
	fn profiling_records_timings() {
		let seen = Arc::new(Mutex::new(Vec::new()));
		let commits = Arc::clone(&seen);
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_profiling(true);
		surface.set_commit_observer(Box::new(move |commit| {
			commits.lock().unwrap().push(commit.clone());
		}));
		let element = Element::view();
		surface.set_root(&element);
		let commits = seen.lock().unwrap();

		assert!(commits.last().unwrap().layout_us.is_some());
	}
}
