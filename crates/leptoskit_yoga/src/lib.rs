//! A from-scratch layout engine for the leptoskit native renderer.
//!
//! The engine implements the CSS box model that native apps actually use:
//! flexbox (the React Native layout system) and CSS grid, sharing one style
//! struct and one measurement pipeline. It is written against an arena rather
//! than a tree of pointers, so a whole interface lives in a single `Vec`.
//! Every node caches its results by input constraints: a relayout re-runs only
//! the dirty path (a changed node and its ancestors) and clean nodes whose
//! constraints changed, while clean subtrees laid out under the same
//! constraints are reused as they are.
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
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

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
	/// The node's index among its parent's in-flow flex or grid items, in
	/// document order after `display: contents` boxes are flattened. Zero for
	/// out-of-flow (absolute or fixed) nodes and for the layout root.
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

/// The default style every new node shares until it is given its own.
fn default_style() -> Arc<Style> {
	static DEFAULT: std::sync::LazyLock<Arc<Style>> =
		std::sync::LazyLock::new(|| Arc::new(Style::default()));
	Arc::clone(&DEFAULT)
}

/// A node's recent measurements, by the space each was offered.
///
/// One layout pass asks a label for its size under several constraints, its
/// max-content width for a flex basis and then its final width, so a cache
/// of the last answer alone missed on every call: creating 1,000 cards
/// measured each of their 2,000 labels four times.
#[derive(Clone, Copy, Default)]
pub(crate) struct MeasureCache {
	entries: [Option<(Size<AvailableSpace>, Size<f32>)>; Self::ENTRIES],
	next: usize,
}

impl MeasureCache {
	const ENTRIES: usize = 4;

	fn get(&self, available: Size<AvailableSpace>) -> Option<Size<f32>> {
		self.entries
			.iter()
			.flatten()
			.find(|(input, _)| *input == available)
			.map(|(_, output)| *output)
	}

	fn insert(&mut self, available: Size<AvailableSpace>, output: Size<f32>) {
		self.entries[self.next] = Some((available, output));
		self.next = (self.next + 1) % Self::ENTRIES;
	}

	fn clear(&mut self) {
		*self = Self::default();
	}
}

pub(crate) struct Node {
	/// Shared so a layout pass can hold a node's style while it mutates the
	/// tree, without copying it: layout reads every style on every pass.
	pub(crate) style: Arc<Style>,
	pub(crate) layout: Layout,
	pub(crate) parent: Option<NodeId>,
	pub(crate) children: Vec<NodeId>,
	pub(crate) measure: Option<MeasureFunc>,
	pub(crate) measure_cache: MeasureCache,
	pub(crate) layout_cache: LayoutCache,
	pub(crate) dirty: bool,
}

impl Node {
	fn new() -> Self {
		Self {
			style: default_style(),
			layout: Layout::default(),
			parent: None,
			children: Vec::new(),
			measure: None,
			measure_cache: MeasureCache::default(),
			layout_cache: LayoutCache::default(),
			dirty: true,
		}
	}
}

/// Why a node is being laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunMode {
	/// Only the border-box size is needed; positions written into the subtree
	/// along the way are scratch values a later commit overwrites.
	Measure,
	/// The size and every position in the subtree must be final.
	Commit,
}

/// How many sizes a node keeps from earlier [`Yoga::calculate_layout`]
/// passes.
///
/// Entries recorded during the current pass are never evicted: deeply nested
/// auto-sized containers ask a node for a number of distinct sizes that grows
/// with depth, and dropping any of them during the pass would make layout
/// exponential again. Each entry stands for a run it saves, so a pass never
/// stores more entries than it executed runs. Older passes' entries are only
/// kept while the node holds fewer than this many.
const RETAINED_CACHE_ENTRIES: usize = 8;

/// Results of earlier layout runs of one node.
///
/// A node's result depends only on its [`SizingCtx`] and on its own subtree,
/// so entries stay valid until [`Yoga::mark_dirty`] clears them on this node
/// or a descendant.
#[derive(Debug, Default)]
pub(crate) struct LayoutCache {
	/// Border-box sizes keyed by input constraints, with the pass that
	/// recorded them.
	sizes: Vec<CacheEntry>,
	/// The constraints of the last run that wrote the positions currently
	/// stored in this subtree, if no later run has overwritten them.
	committed: Option<(SizingCtx, Size<f32>)>,
}

#[derive(Debug, Clone, Copy)]
struct CacheEntry {
	ctx: SizingCtx,
	size: Size<f32>,
	pass: u64,
}

impl LayoutCache {
	/// The cached size for `ctx`, if the run can be skipped in `mode`.
	pub(crate) fn get(&self, ctx: &SizingCtx, mode: RunMode) -> Option<Size<f32>> {
		let committed = self
			.committed
			.filter(|(input, _)| input == ctx)
			.map(|(_, size)| size);

		match mode {
			RunMode::Commit => committed,
			RunMode::Measure => {
				committed.or_else(|| {
					self.sizes
						.iter()
						.rev()
						.find(|entry| entry.ctx == *ctx)
						.map(|entry| entry.size)
				})
			}
		}
	}

	/// Records an executed run, which rewrote every position in the subtree.
	pub(crate) fn insert(&mut self, ctx: SizingCtx, size: Size<f32>, mode: RunMode, pass: u64) {
		self.committed = (mode == RunMode::Commit).then_some((ctx, size));

		if let Some(entry) = self.sizes.iter_mut().find(|entry| entry.ctx == ctx) {
			entry.pass = pass;

			return;
		}

		if self.sizes.len() >= RETAINED_CACHE_ENTRIES {
			self.sizes.retain(|entry| entry.pass == pass);
		}

		self.sizes.push(CacheEntry { ctx, size, pass });
	}

	fn clear(&mut self) {
		self.sizes.clear();
		self.committed = None;
	}

	/// The constraints of the last committed run, if they make this node a
	/// relayout boundary: its parent fixed both its width and its height, and
	/// no run under any other constraints is cached, so the parent never asked
	/// for the size of its content. Its size, and so everything outside it,
	/// is then independent of what it contains.
	pub(crate) fn boundary(&self) -> Option<SizingCtx> {
		let (ctx, _) = self.committed?;
		let fixed = ctx.forced.width.is_some() && ctx.forced.height.is_some();
		let only_committed = self.sizes.iter().all(|entry| entry.ctx == ctx);
		(fixed && only_committed).then_some(ctx)
	}
}

/// What the parent algorithm has already decided about this node.
#[derive(Debug, Clone, Copy, PartialEq)]
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

impl SizingCtx {
	/// Drops inputs that cannot change this node's result, so equivalent
	/// constraints share one cache entry.
	///
	/// Available space is ignored on an axis whose border-box size is already
	/// definite, and the percentage basis only feeds the node's own
	/// percentage lengths and aspect ratio.
	pub(crate) fn normalized(mut self, style: &Style) -> Self {
		let definite = definite_border_box(style, &self);

		if let Some(width) = definite.width {
			self.available.width = AvailableSpace::Definite(width);
		}

		if let Some(height) = definite.height {
			self.available.height = AvailableSpace::Definite(height);
		}

		let is_percent = |dimension: Dimension| matches!(dimension, Dimension::Percent(_));
		let uses_basis = style.aspect_ratio.is_some()
			|| [
				style.width,
				style.height,
				style.min_width,
				style.max_width,
				style.min_height,
				style.max_height,
				style.padding.top,
				style.padding.right,
				style.padding.bottom,
				style.padding.left,
			]
			.into_iter()
			.any(is_percent);

		if !uses_basis {
			self.percent_basis = Size::new(None, None);
		}

		self
	}
}

/// The layout arena.
///
/// Nodes are stored in a flat `Vec` and reference each other by index, so
/// there are no pointers and no reference counting. Nodes are
/// never freed until the arena is dropped, which keeps [`NodeId`] values stable
/// for the lifetime of the tree.
#[derive(Default)]
pub struct Yoga {
	nodes: Vec<Node>,
	/// Counts [`Yoga::calculate_layout`] calls, tagging cache entries with the
	/// pass that recorded them.
	pub(crate) pass: u64,
	/// Relayout boundaries marked dirty from inside, with the constraints
	/// their parent last gave them: laid out again on their own, without
	/// their ancestors (see [`Yoga::mark_dirty`]).
	pending: Vec<(NodeId, SizingCtx)>,
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

		if *entry.style != sanitized {
			entry.style = Arc::new(sanitized);
			self.mark_dirty(node);
		}
	}

	/// The style of a node, shared rather than copied, for layout code that
	/// keeps it while mutating the tree.
	pub(crate) fn shared_style(&self, node: NodeId) -> Arc<Style> {
		Arc::clone(&self.nodes[node.0].style)
	}

	/// Returns the style of a node.
	pub fn style(&self, node: NodeId) -> &Style {
		&self.nodes[node.0].style
	}

	/// Attaches an intrinsic-content measurement function.
	pub fn set_measure_func(&mut self, node: NodeId, measure: Option<MeasureFunc>) {
		self.nodes[node.0].measure = measure;
		self.nodes[node.0].measure_cache.clear();
		self.mark_dirty(node);
	}

	/// Inserts a child at an index, detaching it from any previous parent.
	///
	/// # Panics
	///
	/// Panics when `child` is `parent` itself or one of its ancestors: the
	/// insert would make the tree cyclic. Callers that build trees from
	/// untrusted input check [`Yoga::parent`] chains first.
	pub fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		self.assert_acyclic(parent, child);
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
	///
	/// # Panics
	///
	/// Panics, before changing anything, when any child is `parent` itself or
	/// one of its ancestors.
	pub fn set_children(&mut self, parent: NodeId, children: impl IntoIterator<Item = NodeId>) {
		let children: Vec<NodeId> = children.into_iter().collect();

		for &child in &children {
			self.assert_acyclic(parent, child);
		}

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

	/// Marks a node and its ancestors for relayout, discarding their cached
	/// results. Descendants keep theirs: their inputs did not change.
	///
	/// The walk stops at a relayout boundary above `node`: an ancestor whose
	/// size its parent fixed in both axes without ever measuring its content
	/// (see [`LayoutCache::boundary`]). Nothing outside such a node can depend
	/// on what changed inside it, so only its own subtree is laid out again,
	/// under the constraints it last had. A grid of fixed cells whose labels
	/// change then costs the changed cells, not the grid.
	pub fn mark_dirty(&mut self, node: NodeId) {
		let mut current = Some(node);

		while let Some(id) = current {
			// `node` itself always reaches its parent: a change to its own
			// style can change how the parent sizes it.
			let boundary = (id != node).then(|| self.relayout_boundary(id)).flatten();
			let entry = &mut self.nodes[id.0];
			entry.dirty = true;
			entry.measure_cache.clear();
			entry.layout_cache.clear();

			if let Some(ctx) = boundary {
				self.pending.push((id, ctx));

				return;
			}

			current = entry.parent;
		}
	}

	/// The constraints to lay `node` out again with on its own, when it is a
	/// relayout boundary.
	fn relayout_boundary(&self, node: NodeId) -> Option<SizingCtx> {
		let entry = &self.nodes[node.0];
		let parent = entry.parent?;
		// A baseline depends on content, and a baseline-aligned parent reads
		// it.
		let baseline = entry.style.align_self == Align::Baseline
			|| (entry.style.align_self == Align::Auto
				&& self.nodes[parent.0].style.align_items == Align::Baseline);

		if baseline || entry.dirty {
			return None;
		}

		entry.layout_cache.boundary()
	}

	/// Panics when attaching `child` under `parent` would create a cycle.
	fn assert_acyclic(&self, parent: NodeId, child: NodeId) {
		let mut current = Some(parent);

		while let Some(id) = current {
			assert!(
				id != child,
				"inserting {child:?} under {parent:?} would create a cycle in the layout tree"
			);
			current = self.nodes[id.0].parent;
		}
	}

	/// Returns `true` when the node still needs layout.
	pub fn is_dirty(&self, node: NodeId) -> bool {
		self.nodes[node.0].dirty
	}

	/// Runs layout from `root` with the given available space.
	///
	/// When nothing below `root` changed and the constraints match the previous
	/// run, the call is a no-op. Otherwise only dirty nodes, and clean nodes
	/// whose constraints changed, are laid out again; that is what makes
	/// re-running layout after a small state change cheap.
	pub fn calculate_layout(&mut self, root: NodeId, available: Size<AvailableSpace>) {
		self.layout_root(root, available, Size::new(None, None));
	}

	/// Runs layout from `root` the way a block element fills its containing
	/// block: when the root's width is `auto` and the available width is
	/// definite, the root takes that width less its horizontal margins, within
	/// its min and max widths, instead of shrinking to its content. Its height
	/// resolves as in [`Self::calculate_layout`].
	pub fn calculate_layout_filling_width(
		&mut self,
		root: NodeId,
		available: Size<AvailableSpace>,
	) {
		let style = &self.nodes[root.0].style;

		let width = match available.width {
			AvailableSpace::Definite(width) if style.width.is_auto() => {
				let margin = resolve_margins(&style.margin, Some(width));
				Some((width - margin.left - margin.right).max(0.0))
			}

			_ => None,
		};

		self.layout_root(root, available, Size::new(width, None));
	}

	fn layout_root(
		&mut self,
		root: NodeId,
		available: Size<AvailableSpace>,
		forced: Size<Option<f32>>,
	) {
		let ctx = SizingCtx {
			available,
			forced,
			percent_basis: available.map(AvailableSpace::definite),
		};
		self.pass += 1;
		self.layout_node(root, ctx, RunMode::Commit);
		// Boundaries dirtied from inside: their ancestors stayed clean, so the
		// pass above did not reach them. One the pass did reach is clean now.
		for (node, ctx) in std::mem::take(&mut self.pending) {
			if self.nodes[node.0].dirty {
				self.layout_node(node, ctx, RunMode::Commit);
			}
		}

		// Laying out a subtree on its own rewrites sizes its ancestors' last
		// runs positioned, so those runs no longer describe the tree.
		let mut ancestor = self.nodes[root.0].parent;

		while let Some(id) = ancestor {
			self.nodes[id.0].layout_cache.committed = None;
			ancestor = self.nodes[id.0].parent;
		}
	}

	/// The computed layout of a node.
	pub fn layout(&self, node: NodeId) -> &Layout {
		&self.nodes[node.0].layout
	}

	/// Runs the node's measurement function through the cache.
	pub(crate) fn measure(&mut self, node: NodeId, available: Size<AvailableSpace>) -> Size<f32> {
		if let Some(size) = self.nodes[node.0].measure_cache.get(available) {
			return size;
		}

		let measure = self.nodes[node.0]
			.measure
			.clone()
			.expect("measure called on a node without a measure function");
		let output = measure(available);
		let output = Size::new(output.width.max(0.0), output.height.max(0.0));
		self.nodes[node.0].measure_cache.insert(available, output);
		output
	}

	/// Splits a container's children into in-flow items and out-of-flow
	/// (absolute or fixed) children, each in document order.
	///
	/// `display: none` children take no part, and `display: contents` boxes
	/// are flattened so their children take part directly, as flex and grid
	/// items both do in CSS.
	pub(crate) fn layout_children(&self, node: NodeId) -> (Vec<NodeId>, Vec<NodeId>) {
		let children = self.children(node);
		let mut flow = Vec::with_capacity(children.len());
		let mut absolute = Vec::new();
		let mut stack: Vec<NodeId> = children.iter().rev().copied().collect();

		while let Some(child) = stack.pop() {
			let style = self.style(child);

			if style.display == Display::None {
				continue;
			}

			if matches!(style.position, PositionType::Absolute | PositionType::Fixed) {
				absolute.push(child);
				continue;
			}

			if style.display == Display::Contents {
				stack.extend(self.children(child).iter().rev().copied());
				continue;
			}

			flow.push(child);
		}

		(flow, absolute)
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
	resolve_signed(dimension, basis).map(|value| value.max(0.0))
}

/// Resolves a margin or inset, whose sign determines its layout direction.
pub(crate) fn resolve_signed(dimension: Dimension, basis: Option<f32>) -> Option<f32> {
	match dimension {
		Dimension::Points(points) => Some(finite(points)),
		Dimension::Percent(fraction) => basis.map(|total| finite(fraction) * total),
		Dimension::Auto => None,
	}
}

/// The box-model values shared by flex, grid, and absolute sizing.
///
/// Style lengths are converted exactly once to border-box units. Sizes forced
/// by a parent algorithm are already border-box sizes and only need clamping.
pub(crate) struct BoxMetrics {
	pub(crate) inner_padding: Edges,
	pub(crate) border: Edges,
	pub(crate) size: Size<Option<f32>>,
	pub(crate) min: Size<f32>,
	pub(crate) max: Size<f32>,
	content_box: bool,
}

impl BoxMetrics {
	/// Resolves the node's own style against its containing block.
	pub(crate) fn new(style: &Style, basis: Size<Option<f32>>) -> Self {
		let padding = resolve_edges(&style.padding, basis.width);
		let border = Edges {
			top: finite(style.border.top).max(0.0),
			right: finite(style.border.right).max(0.0),
			bottom: finite(style.border.bottom).max(0.0),
			left: finite(style.border.left).max(0.0),
		};
		let inner_padding = Edges {
			top: padding.top + border.top,
			right: padding.right + border.right,
			bottom: padding.bottom + border.bottom,
			left: padding.left + border.left,
		};
		let content_box = style.box_sizing == BoxSizing::ContentBox;
		let axis = |value, basis, padding| Self::dimension(value, basis, padding, content_box);
		let horizontal = inner_padding.horizontal();
		let vertical = inner_padding.vertical();
		let min = Size::new(
			axis(style.min_width, basis.width, horizontal).unwrap_or(horizontal),
			axis(style.min_height, basis.height, vertical).unwrap_or(vertical),
		);

		let max = Size::new(
			axis(style.max_width, basis.width, horizontal)
				.unwrap_or(f32::INFINITY)
				.max(min.width),
			axis(style.max_height, basis.height, vertical)
				.unwrap_or(f32::INFINITY)
				.max(min.height),
		);

		Self {
			inner_padding,
			border,
			size: Size::new(
				axis(style.width, basis.width, horizontal)
					.map(|width| width.clamp(min.width, max.width)),
				axis(style.height, basis.height, vertical)
					.map(|height| height.clamp(min.height, max.height)),
			),
			min,
			max,
			content_box,
		}
	}

	/// Applies min/max constraints and the zero-content decoration floor.
	pub(crate) fn clamp(&self, size: Size<f32>) -> Size<f32> {
		Size::new(
			size.width.clamp(self.min.width, self.max.width),
			size.height.clamp(self.min.height, self.max.height),
		)
	}

	/// Converts a width to border-box units.
	pub(crate) fn width(&self, dimension: Dimension, basis: Option<f32>) -> Option<f32> {
		Self::dimension(
			dimension,
			basis,
			self.inner_padding.horizontal(),
			self.content_box,
		)
	}

	/// Converts a height to border-box units.
	pub(crate) fn height(&self, dimension: Dimension, basis: Option<f32>) -> Option<f32> {
		Self::dimension(
			dimension,
			basis,
			self.inner_padding.vertical(),
			self.content_box,
		)
	}

	/// Flex-base determination ignores the zero-content decoration floor.
	/// The hypothetical and final sizes apply that floor after flexibility.
	pub(crate) fn flex_basis(
		&self,
		dimension: Dimension,
		basis: Option<f32>,
		is_row: bool,
	) -> Option<f32> {
		let padding = if is_row {
			self.inner_padding.horizontal()
		} else {
			self.inner_padding.vertical()
		};

		resolve(dimension, basis).map(|value| {
			if self.content_box {
				value + padding
			} else {
				value
			}
		})
	}

	/// CSS sizes the selected box while its content size remains nonnegative.
	fn dimension(
		dimension: Dimension,
		basis: Option<f32>,
		padding: f32,
		content_box: bool,
	) -> Option<f32> {
		resolve(dimension, basis).map(|value| {
			if content_box {
				value + padding
			} else {
				value.max(padding)
			}
		})
	}

	/// Transfers an aspect ratio using the box selected by `box-sizing`.
	pub(crate) fn ratio_height(&self, width: f32, ratio: f32) -> f32 {
		if self.content_box {
			(width - self.inner_padding.horizontal()).max(0.0) / ratio
				+ self.inner_padding.vertical()
		} else {
			width / ratio
		}
	}

	/// Transfers an aspect ratio using the box selected by `box-sizing`.
	pub(crate) fn ratio_width(&self, height: f32, ratio: f32) -> f32 {
		if self.content_box {
			(height - self.inner_padding.vertical()).max(0.0) * ratio
				+ self.inner_padding.horizontal()
		} else {
			height * ratio
		}
	}
}

/// Clamps a size through the style's min and max constraints.
pub(crate) fn clamp_size(style: &Style, size: Size<f32>, basis: Size<Option<f32>>) -> Size<f32> {
	BoxMetrics::new(style, basis).clamp(size)
}

/// Clamps a width through `min_width` and `max_width`; the minimum wins.
pub(crate) fn clamp_width(style: &Style, width: f32, basis: Size<Option<f32>>) -> f32 {
	let metrics = BoxMetrics::new(style, basis);
	width.clamp(metrics.min.width, metrics.max.width)
}

/// Clamps a height through `min_height` and `max_height`; the minimum wins.
pub(crate) fn clamp_height(style: &Style, height: f32, basis: Size<Option<f32>>) -> f32 {
	let metrics = BoxMetrics::new(style, basis);
	height.clamp(metrics.min.height, metrics.max.height)
}

/// The node's border-box size on each axis where it is already definite:
/// imposed by the parent algorithm, or a style length that resolves against
/// the containing block. Content-sized axes are `None`.
pub(crate) fn definite_border_box(style: &Style, ctx: &SizingCtx) -> Size<Option<f32>> {
	let metrics = BoxMetrics::new(style, ctx.percent_basis);
	let mut size = Size::new(
		ctx.forced
			.width
			.or(metrics.size.width)
			.map(|width| width.clamp(metrics.min.width, metrics.max.width)),
		ctx.forced
			.height
			.or(metrics.size.height)
			.map(|height| height.clamp(metrics.min.height, metrics.max.height)),
	);

	if let Some(ratio) = style.aspect_ratio {
		match (size.width, size.height) {
			(Some(width), None) => size.height = Some(metrics.ratio_height(width, ratio)),
			(None, Some(height)) => size.width = Some(metrics.ratio_width(height, ratio)),
			_ => {}
		}
	}

	Size::new(
		size.width
			.map(|width| width.clamp(metrics.min.width, metrics.max.width)),
		size.height
			.map(|height| height.clamp(metrics.min.height, metrics.max.height)),
	)
}

/// The space a node may grow into: the parent's available space, capped by
/// the node's own `max_width` and `max_height`.
///
/// A box never exceeds its maximum, so content laid out inside it (text
/// wrapping in a capped-width bubble, say) must see the cap, not the parent's
/// space. An unbounded axis with a maximum becomes bounded by it.
pub(crate) fn bounded_by_max(
	style: &Style,
	available: Size<AvailableSpace>,
	basis: Size<Option<f32>>,
) -> Size<AvailableSpace> {
	let axis = |space: AvailableSpace, max: Option<f32>| {
		match (space, max) {
			(AvailableSpace::Definite(space), Some(max)) => {
				AvailableSpace::Definite(space.min(max))
			}
			(AvailableSpace::MaxContent, Some(max)) => AvailableSpace::Definite(max),
			(space, _) => space,
		}
	};
	let metrics = BoxMetrics::new(style, basis);

	Size::new(
		axis(
			available.width,
			metrics.max.width.is_finite().then_some(metrics.max.width),
		),
		axis(
			available.height,
			metrics.max.height.is_finite().then_some(metrics.max.height),
		),
	)
}

/// The space available to the node's content box: its definite size when it
/// has one, else the parent's available space, minus padding and border.
pub(crate) fn content_available(
	definite_inner: Size<Option<f32>>,
	available: Size<AvailableSpace>,
	inner_padding: Edges,
) -> Size<AvailableSpace> {
	let axis = |inner: Option<f32>, space: AvailableSpace, padding: f32| {
		match (inner, space) {
			(Some(inner), _) => AvailableSpace::Definite(inner),
			(None, AvailableSpace::Definite(space)) => {
				AvailableSpace::Definite((space - padding).max(0.0))
			}
			(None, other) => other,
		}
	};
	Size::new(
		axis(
			definite_inner.width,
			available.width,
			inner_padding.horizontal(),
		),
		axis(
			definite_inner.height,
			available.height,
			inner_padding.vertical(),
		),
	)
}

/// Removes padding and border from a definite border-box size.
pub(crate) fn inner_definite(
	border_box: Size<Option<f32>>,
	inner_padding: Edges,
) -> Size<Option<f32>> {
	Size::new(
		border_box
			.width
			.map(|width| (width - inner_padding.horizontal()).max(0.0)),
		border_box
			.height
			.map(|height| (height - inner_padding.vertical()).max(0.0)),
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

/// Margins can pull boxes outside their parent's content box.
pub(crate) fn resolve_margins(rect: &Rect<Dimension>, basis: Option<f32>) -> Edges {
	Edges {
		top: resolve_signed(rect.top, basis).unwrap_or(0.0),
		right: resolve_signed(rect.right, basis).unwrap_or(0.0),
		bottom: resolve_signed(rect.bottom, basis).unwrap_or(0.0),
		left: resolve_signed(rect.left, basis).unwrap_or(0.0),
	}
}

/// Relative offsets move the box without changing the space it occupies.
pub(crate) fn relative_offset(style: &Style, basis: Size<f32>, direction: Direction) -> Size<f32> {
	let left = resolve_signed(style.inset.left, Some(basis.width));
	let right = resolve_signed(style.inset.right, Some(basis.width));
	let top = resolve_signed(style.inset.top, Some(basis.height));
	let bottom = resolve_signed(style.inset.bottom, Some(basis.height));
	let x = if direction == Direction::Rtl {
		right.map(|right| -right).or(left)
	} else {
		left.or_else(|| right.map(|right| -right))
	};

	Size::new(
		x.unwrap_or(0.0),
		top.or_else(|| bottom.map(|bottom| -bottom)).unwrap_or(0.0),
	)
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

	pub(crate) fn as_rect(self) -> Rect<f32> {
		Rect::all_edges(self.top, self.right, self.bottom, self.left)
	}
}

#[cfg(test)]
// Layout is exact arithmetic on these inputs, as in the integration tests.
#[allow(clippy::float_cmp)]
mod boundary_tests {
	use std::sync::Arc;
	use std::sync::atomic::AtomicU32;
	use std::sync::atomic::Ordering;

	use super::*;

	/// A label whose measured width can change, like a text whose content
	/// changes.
	struct Label {
		node: NodeId,
		width: Arc<AtomicU32>,
	}

	fn label(yoga: &mut Yoga, width: u32) -> Label {
		let node = yoga.create_node();
		let shared = Arc::new(AtomicU32::new(width));
		let read = Arc::clone(&shared);
		yoga.set_measure_func(
			node,
			Some(Arc::new(move |_| {
				Size::new(read.load(Ordering::Relaxed) as f32, 10.0)
			})),
		);
		Label {
			node,
			width: shared,
		}
	}

	fn relabel(yoga: &mut Yoga, label: &Label, width: u32) {
		label.width.store(width, Ordering::Relaxed);
		// What a text change does: the measured content is stale.
		yoga.set_measure_func(label.node, yoga.nodes[label.node.0].measure.clone());
	}

	const SPACE: Size<AvailableSpace> = Size::new(
		AvailableSpace::Definite(300.0),
		AvailableSpace::Definite(800.0),
	);

	#[test]
	fn a_label_inside_a_fixed_tile_lays_out_only_that_tile() {
		let mut yoga = Yoga::new();
		let row = yoga.create_node();
		yoga.set_style(
			row,
			Style {
				flex_direction: FlexDirection::Row,
				height: Dimension::Points(32.0),
				..Style::default()
			},
		);
		let mut labels = Vec::new();
		let mut tiles = Vec::new();

		for _ in 0..3 {
			let tile = yoga.create_node();
			yoga.set_style(
				tile,
				Style {
					flex_grow: 1.0,
					flex_basis: Dimension::Points(0.0),
					align_items: Align::Center,
					justify_content: JustifyContent::Center,
					..Style::default()
				},
			);

			let text = label(&mut yoga, 20);
			yoga.set_children(tile, [text.node]);
			tiles.push(tile);
			labels.push(text);
		}

		yoga.set_children(row, tiles.clone());
		yoga.calculate_layout(row, SPACE);

		relabel(&mut yoga, &labels[1], 60);
		assert!(
			!yoga.is_dirty(row),
			"the row's layout does not depend on the label"
		);
		assert!(yoga.is_dirty(tiles[1]));
		yoga.calculate_layout(row, SPACE);
		assert!(!yoga.is_dirty(tiles[1]));
		// The label is re-centred in its unchanged tile.
		assert_eq!(yoga.layout(tiles[1]).width, 100.0);
		assert_eq!(yoga.layout(labels[1].node).x, 20.0);
		assert_eq!(yoga.layout(labels[1].node).width, 60.0);
	}

	/// A small, fixed-seed generator, so a failure reproduces.
	struct Random(u64);

	impl Random {
		fn below(&mut self, bound: u64) -> u64 {
			self.0 = self
				.0
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			(self.0 >> 33) % bound
		}
	}

	/// Builds the same random tree in `yoga` from `seed`: nested rows and
	/// columns, sometimes fixed, wrapping, centring, or with margins, around
	/// labels of the given widths.
	fn random_tree(yoga: &mut Yoga, seed: u64, widths: &[u32]) -> (NodeId, Vec<Label>) {
		/// A style with the fixed sizes and stretched children real screens
		/// use, which are what make relayout boundaries.
		fn random_style(random: &mut Random) -> Style {
			let dimension = |random: &mut Random| {
				match random.below(2) {
					0 => Dimension::Points(20.0 + random.below(60) as f32),
					_ => Dimension::Auto,
				}
			};

			Style {
				flex_direction: if random.below(2) == 0 {
					FlexDirection::Row
				} else {
					FlexDirection::Column
				},

				flex_wrap: if random.below(4) == 0 {
					FlexWrap::Wrap
				} else {
					FlexWrap::NoWrap
				},

				align_items: [
					Align::Stretch,
					Align::Stretch,
					Align::Center,
					Align::FlexStart,
				][random.below(4) as usize],
				justify_content: [JustifyContent::FlexStart, JustifyContent::Center]
					[random.below(2) as usize],
				flex_grow: random.below(2) as f32,
				// `flex-1` sets a zero basis, so the parent never measures the
				// item's content along the main axis.
				flex_basis: if random.below(2) == 0 {
					Dimension::Points(0.0)
				} else {
					Dimension::Auto
				},

				width: dimension(random),
				height: dimension(random),
				margin: Rect {
					top: Dimension::Points(random.below(3) as f32),
					right: Dimension::Points(0.0),
					bottom: Dimension::Points(0.0),
					left: Dimension::Points(random.below(3) as f32),
				},
				..Style::default()
			}
		}

		fn container(
			yoga: &mut Yoga,
			random: &mut Random,
			depth: u32,
			widths: &[u32],
			labels: &mut Vec<Label>,
		) -> NodeId {
			let node = yoga.create_node();
			yoga.set_style(node, random_style(random));
			let mut children = Vec::new();

			for _ in 0..=random.below(3) {
				if depth < 3 && random.below(2) == 0 {
					children.push(container(yoga, random, depth + 1, widths, labels));
				} else {
					let width = widths[labels.len() % widths.len()];
					let text = label(yoga, width);
					children.push(text.node);
					labels.push(text);
				}
			}

			yoga.set_children(node, children);
			node
		}
		let mut random = Random(seed);
		let mut labels = Vec::new();
		let root = container(yoga, &mut random, 0, widths, &mut labels);
		(root, labels)
	}

	fn layouts(yoga: &Yoga, root: NodeId) -> Vec<Layout> {
		let mut out = vec![*yoga.layout(root)];

		for &child in yoga.children(root) {
			out.extend(layouts(yoga, child));
		}

		out
	}

	#[test]
	fn incremental_layout_matches_a_fresh_layout() {
		let mut with_boundaries = 0;

		for seed in 0..2000 {
			let before: Vec<u32> = (0..64)
				.map(|index| 10 + (index * 7 + seed as u32) % 50)
				.collect();
			let mut incremental = Yoga::new();
			let (root, labels) = random_tree(&mut incremental, seed, &before);
			incremental.calculate_layout(root, SPACE);

			// Change some labels, lay out incrementally, and compare with a
			// tree built afresh with the new widths.
			let mut after = before.clone();
			let mut random = Random(seed ^ 0x5eed);

			for (index, text) in labels.iter().enumerate() {
				if random.below(3) == 0 {
					let width = 5 + random.below(80) as u32;
					after[index] = width;
					relabel(&mut incremental, text, width);
				}
			}

			with_boundaries += usize::from(!incremental.pending.is_empty());

			incremental.calculate_layout(root, SPACE);

			let mut fresh = Yoga::new();
			let (fresh_root, _) = random_tree(&mut fresh, seed, &after[..labels.len()]);
			fresh.calculate_layout(fresh_root, SPACE);
			assert_eq!(
				layouts(&incremental, root),
				layouts(&fresh, fresh_root),
				"seed {seed}"
			);
		}

		// The comparison means something only if boundaries actually formed.
		assert!(
			with_boundaries > 150,
			"only {with_boundaries} of 2000 trees stopped at a boundary"
		);
	}
}
