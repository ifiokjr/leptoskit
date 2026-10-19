//! A from-scratch layout engine for the leptoskit native renderer.
//!
//! The engine implements the CSS box model that native apps actually use:
//! flexbox (the React Native layout system) and CSS grid, sharing one style
//! struct and one measurement pipeline. It is written against an arena rather
//! than a tree of pointers, so a whole interface is a single `Vec` allocation,
//! relayouts touch only dirty subtrees, and text measurement results are cached
//! per node.
//!
//! The entry point is [`Yoga::calculate_layout`]; results are read back with
//! [`Yoga::layout`].
//!
//! ```no_run
//! use leptoskit_yoga::AvailableSpace;
//! use leptoskit_yoga::Size;
//! use leptoskit_yoga::Style;
//! use leptoskit_yoga::Yoga;
//!
//! let mut tree = Yoga::new();
//! let root = tree.create_node();
//! tree.set_style(
//! 	root,
//! 	Style {
//! 		width: leptoskit_yoga::Dimension::Points(320.0),
//! 		height: leptoskit_yoga::Dimension::Points(240.0),
//! 		..Style::default()
//! 	},
//! );
//! let child = tree.create_node();
//! tree.set_style(
//! 	child,
//! 	Style {
//! 		flex_grow: 1.0,
//! 		..Style::default()
//! 	},
//! );
//! tree.insert_child(root, child, 0);
//! tree.calculate_layout(
//! 	root,
//! 	Size {
//! 		width: AvailableSpace::Definite(320.0),
//! 		height: AvailableSpace::Definite(240.0),
//! 	},
//! );
//! assert_eq!(tree.layout(child).height, 240.0);
//! ```

mod flexbox;
mod grid;
mod style;

use std::sync::Arc;

pub use style::Align;
pub use style::AvailableSpace;
pub use style::BoxSizing;
pub use style::Dimension;
pub use style::Direction;
pub use style::Display;
pub use style::FlexDirection;
pub use style::FlexWrap;
pub use style::GridAutoFlow;
pub use style::GridLine;
pub use style::GridPlacement;
pub use style::GridStyle;
pub use style::JustifyContent;
pub use style::MinMax;
pub use style::Overflow;
pub use style::PositionType;
pub use style::Rect;
pub use style::Size;
pub use style::Style;
pub use style::TrackSizing;
pub use style::minmax;

/// A handle to a node inside a [`Yoga`] arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeId(usize);

impl NodeId {
	/// The node's index in the arena; also usable as a dense node key.
	pub fn index(self) -> usize {
		self.0
	}
}

/// The result of layout for one node.
///
/// Coordinates are relative to the parent's border-box origin, matching the
/// React Native layout metrics that platform backends consume.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Layout {
	/// The creation order of the node within its parent.
	pub order: u32,
	/// Offset from the parent's left border edge.
	pub x: f32,
	/// Offset from the parent's top border edge.
	pub y: f32,
	/// The final border-box width.
	pub width: f32,
	/// The final border-box height.
	pub height: f32,
	/// The union of the children's border boxes, for scroll views.
	pub content_width: f32,
	/// The union of the children's border boxes along the block axis.
	pub content_height: f32,
}

/// A text (or other intrinsic-content) measurement callback.
///
/// Measurement functions must be pure: the engine caches results and assumes
/// the same inputs always produce the same output.
pub type MeasureFunc = Arc<dyn Fn(Size<AvailableSpace>) -> Size<f32> + Send + Sync>;

pub(crate) struct Node {
	pub(crate) style: Style,
	pub(crate) layout: Layout,
	pub(crate) parent: Option<NodeId>,
	pub(crate) children: Vec<NodeId>,
	pub(crate) measure: Option<MeasureFunc>,
	pub(crate) measure_cache: Option<(Size<AvailableSpace>, Size<f32>)>,
	pub(crate) dirty: bool,
}

impl Node {
	fn new() -> Self {
		Self {
			style: Style::default(),
			layout: Layout::default(),
			parent: None,
			children: Vec::new(),
			measure: None,
			measure_cache: None,
			dirty: true,
		}
	}
}

/// What the parent algorithm has already decided about this node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SizingCtx {
	/// Space available for the node's border box.
	pub available: Size<AvailableSpace>,
	/// Sizes the parent's flex or grid algorithm resolved for this node,
	/// overriding the style's own size properties.
	pub forced: Size<Option<f32>>,
	/// The parent's content box, the basis for resolving percentage margins,
	/// paddings, and insets.
	pub percent_basis: Size<Option<f32>>,
}

/// The layout arena.
///
/// Nodes are stored in a flat `Vec`; children reference parents by index, so
/// there is no per-node heap allocation and no reference counting. Nodes are
/// never freed until the arena is dropped, which keeps [`NodeId`] values stable
/// for the lifetime of the tree.
#[derive(Default)]
pub struct Yoga {
	nodes: Vec<Node>,
	last_root_constraints: Option<(NodeId, Size<AvailableSpace>)>,
}

impl std::fmt::Debug for Yoga {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Yoga")
			.field("nodes", &self.nodes.len())
			.finish_non_exhaustive()
	}
}

impl Yoga {
	/// Creates an empty arena.
	pub fn new() -> Self {
		Self::default()
	}

	/// Allocates a node with default style.
	pub fn create_node(&mut self) -> NodeId {
		self.nodes.push(Node::new());
		NodeId(self.nodes.len() - 1)
	}

	/// Replaces the style of a node, marking it and its ancestors dirty.
	pub fn set_style(&mut self, node: NodeId, style: Style) {
		let sanitized = sanitize_style(style);
		let entry = &mut self.nodes[node.0];
		if entry.style != sanitized {
			entry.style = sanitized;
			self.mark_dirty(node);
		}
	}

	/// Returns the style of a node.
	pub fn style(&self, node: NodeId) -> &Style {
		&self.nodes[node.0].style
	}

	/// Attaches an intrinsic-content measurement function.
	pub fn set_measure_func(&mut self, node: NodeId, measure: Option<MeasureFunc>) {
		self.nodes[node.0].measure = measure;
		self.nodes[node.0].measure_cache = None;
		self.mark_dirty(node);
	}

	/// Inserts a child at an index, detaching it from any previous parent.
	pub fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		self.detach(child);
		let entry = &mut self.nodes[parent.0];
		let index = index.min(entry.children.len());
		entry.children.insert(index, child);
		self.nodes[child.0].parent = Some(parent);
		self.mark_dirty(parent);
	}

	/// Removes a child from its parent.
	pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		let entry = &mut self.nodes[parent.0];
		entry.children.retain(|&c| c != child);
		if self.nodes[child.0].parent == Some(parent) {
			self.nodes[child.0].parent = None;
		}
		self.mark_dirty(parent);
	}

	/// Detaches a node from its parent, returning the former parent.
	pub fn detach(&mut self, node: NodeId) -> Option<NodeId> {
		let parent = self.nodes[node.0].parent;
		if let Some(parent) = parent {
			self.remove_child(parent, node);
		}
		parent
	}

	/// Replaces the whole child list of a node.
	pub fn set_children(&mut self, parent: NodeId, children: impl IntoIterator<Item = NodeId>) {
		for existing in std::mem::take(&mut self.nodes[parent.0].children) {
			self.nodes[existing.0].parent = None;
		}
		for child in children {
			self.detach(child);
			self.nodes[parent.0].children.push(child);
			self.nodes[child.0].parent = Some(parent);
		}
		self.mark_dirty(parent);
	}

	/// The parent of a node, if attached.
	pub fn parent(&self, node: NodeId) -> Option<NodeId> {
		self.nodes[node.0].parent
	}

	/// The children of a node, in order.
	pub fn children(&self, node: NodeId) -> &[NodeId] {
		&self.nodes[node.0].children
	}

	/// Marks a node and its ancestors for relayout.
	pub fn mark_dirty(&mut self, node: NodeId) {
		let mut current = Some(node);
		while let Some(id) = current {
			let entry = &mut self.nodes[id.0];
			entry.dirty = true;
			entry.measure_cache = None;
			current = entry.parent;
		}
	}

	/// Returns `true` when the node still needs layout.
	pub fn is_dirty(&self, node: NodeId) -> bool {
		self.nodes[node.0].dirty
	}

	/// Runs layout from `root` with the given available space.
	///
	/// When nothing below `root` changed and the constraints match the previous
	/// run, the call is a no-op; that check is what makes re-running layout
	/// after a small state change cheap.
	pub fn calculate_layout(&mut self, root: NodeId, available: Size<AvailableSpace>) {
		let unchanged =
			!self.nodes[root.0].dirty && self.last_root_constraints == Some((root, available));
		if unchanged {
			return;
		}
		let ctx = SizingCtx {
			available,
			forced: Size::new(None, None),
			percent_basis: available.map(AvailableSpace::definite),
		};
		self.layout_node(root, ctx);
		self.nodes[root.0].dirty = false;
		self.last_root_constraints = Some((root, available));
	}

	/// The computed layout of a node.
	pub fn layout(&self, node: NodeId) -> &Layout {
		&self.nodes[node.0].layout
	}

	/// Runs the node's measurement function through the cache.
	pub(crate) fn measure(&mut self, node: NodeId, available: Size<AvailableSpace>) -> Size<f32> {
		let cached = self.nodes[node.0]
			.measure_cache
			.filter(|(input, _)| *input == available)
			.map(|(_, output)| output);
		if let Some(size) = cached {
			return size;
		}
		let measure = self.nodes[node.0]
			.measure
			.clone()
			.expect("measure called on a node without a measure function");
		let output = measure(available);
		let output = Size::new(output.width.max(0.0), output.height.max(0.0));
		self.nodes[node.0].measure_cache = Some((available, output));
		output
	}
}

/// Replaces non-finite values, which have no meaning in layout, with zero.
pub(crate) fn finite(value: f32) -> f32 {
	if value.is_finite() { value } else { 0.0 }
}

fn sanitize_style(style: Style) -> Style {
	let mut style = style;
	style.flex_grow = finite(style.flex_grow).max(0.0);
	style.flex_shrink = finite(style.flex_shrink).max(0.0);
	style.aspect_ratio = style
		.aspect_ratio
		.filter(|ratio| ratio.is_finite() && *ratio > 0.0);
	style
}

/// Resolves a dimension against a basis, treating an indefinite basis as
/// "not specified" for sizing purposes.
pub(crate) fn resolve(dimension: Dimension, basis: Option<f32>) -> Option<f32> {
	match dimension {
		Dimension::Points(points) => Some(finite(points).max(0.0)),
		Dimension::Percent(fraction) => basis.map(|total| (finite(fraction) * total).max(0.0)),
		Dimension::Auto => None,
	}
}

/// Clamps a size through the style's min and max constraints.
pub(crate) fn clamp_size(style: &Style, size: Size<f32>, basis: Size<Option<f32>>) -> Size<f32> {
	let min_width = resolve(style.min_width, basis.width).unwrap_or(0.0);
	let max_width = resolve(style.max_width, basis.width).unwrap_or(f32::INFINITY);
	let min_height = resolve(style.min_height, basis.height).unwrap_or(0.0);
	let max_height = resolve(style.max_height, basis.height).unwrap_or(f32::INFINITY);
	Size::new(
		size.width.clamp(min_width, max_width.max(min_width)),
		size.height.clamp(min_height, max_height.max(min_height)),
	)
}

/// Resolved edges of a `Rect<Dimension>` against a basis.
pub(crate) fn resolve_edges(rect: &Rect<Dimension>, basis: Option<f32>) -> Edges {
	Edges {
		top: resolve(rect.top, basis).unwrap_or(0.0),
		right: resolve(rect.right, basis).unwrap_or(0.0),
		bottom: resolve(rect.bottom, basis).unwrap_or(0.0),
		left: resolve(rect.left, basis).unwrap_or(0.0),
	}
}

/// Fully resolved edges.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Edges {
	pub(crate) top: f32,
	pub(crate) right: f32,
	pub(crate) bottom: f32,
	pub(crate) left: f32,
}

impl Edges {
	pub(crate) fn horizontal(self) -> f32 {
		self.left + self.right
	}

	pub(crate) fn vertical(self) -> f32 {
		self.top + self.bottom
	}
}

/// Applies the `aspect_ratio` property to the axis the style leaves unset.
pub(crate) fn apply_aspect_helper(
	style: &Style,
	mut size: Size<f32>,
	basis: Size<Option<f32>>,
) -> Size<f32> {
	let Some(ratio) = style.aspect_ratio else {
		return size;
	};
	let width_set = !style.width.is_auto() || basis.width.is_none();
	let height_set = !style.height.is_auto() || basis.height.is_none();
	if width_set && !height_set {
		size.height = size.width / ratio;
	} else if height_set && !width_set {
		size.width = size.height * ratio;
	}
	size
}
