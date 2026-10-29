//! The flexbox layout algorithm.
//!
//! Follows CSS Flexible Box Layout Level 1: hypothetical main sizes, the
//! "resolve flexible lengths" freeze loop, line breaking under `flex_wrap`,
//! cross-axis stretching, and `justify`/`align` distribution including auto
//! margins. Absolute children are positioned against the parent's padding box.
//!
//! `Yoga::layout_flex` runs the steps of the spec's layout algorithm (§9) in
//! order, each in its own function whose documentation names the step.

use crate::AvailableSpace;
use crate::BoxMetrics;
use crate::Direction;
use crate::Display;
use crate::Edges;
use crate::Layout;
use crate::NodeId;
use crate::Rect;
use crate::RunMode;
use crate::Size;
use crate::Style;
use crate::Yoga;
use crate::bounded_by_max;
use crate::clamp_size;
use crate::content_available;
use crate::definite_border_box;
use crate::inner_definite;
use crate::relative_offset;
use crate::resolve_margins;
use crate::resolve_signed;
use crate::style::Align;
use crate::style::Dimension;
use crate::style::FlexWrap;
use crate::style::JustifyContent;

/// A child tracked through the flexible-lengths algorithm.
struct FlexItem {
	node: NodeId,
	/// The margin-inclusive hypothetical size used for line breaking.
	outer_base: f32,
	/// The original border-box flex base, before min/max constraints.
	flex_base: f32,
	/// The resolved inner main size after flexibility.
	inner_main: f32,
	/// The current cross size before stretching.
	inner_cross: f32,
	/// The item's margins.
	margins: Edges,
	/// Whether the item may still change size in the freeze loop.
	frozen: bool,
	/// The item's effective `flex_grow`.
	grow: f32,
	/// The item's effective `flex_shrink`.
	shrink: f32,
	/// The shrink factor weighted by the original content-box flex base.
	scaled_shrink: f32,
	/// The resolved minimum main size.
	min_main: f32,
	/// The resolved maximum main size.
	max_main: f32,
	/// The latest min/max correction, used to freeze only over-flexed items.
	violation: f32,
}

/// A wrapped line of items.
#[derive(Default)]
struct FlexLine {
	items: Vec<usize>,
	/// The line's cross size once measured.
	cross: f32,
	/// The line's cross offset inside the container's content box.
	cross_offset: f32,
}

/// How a flex container's main and cross axes map onto physical directions.
#[derive(Clone, Copy)]
struct FlexAxes {
	/// Whether the main axis is horizontal.
	is_row: bool,
	/// Whether items run from the main axis's physical end: the right edge of
	/// a row or the bottom of a column.
	main_reversed: bool,
	/// Whether lines run from the cross axis's physical end.
	cross_reversed: bool,
}

impl FlexAxes {
	fn new(style: &Style) -> Self {
		let is_row = style.flex_direction.is_row();
		let rtl = style.direction == Direction::Rtl;
		Self {
			is_row,
			// The main axis runs against the writing direction when reversed
			// or, for rows, under RTL.
			main_reversed: style.flex_direction.is_reverse() ^ (is_row && rtl),
			// Items are placed from logical cross-start, then the complete
			// coordinate is mirrored when writing direction or wrap-reverse
			// changes the physical start.
			cross_reversed: (!is_row && rtl) ^ (style.flex_wrap == FlexWrap::WrapReverse),
		}
	}

	/// The main-axis component of a physical size.
	fn main<T>(self, size: Size<T>) -> T {
		if self.is_row { size.width } else { size.height }
	}

	/// The cross-axis component of a physical size.
	fn cross<T>(self, size: Size<T>) -> T {
		if self.is_row { size.height } else { size.width }
	}

	/// A physical size from its main- and cross-axis components.
	fn size<T>(self, main: T, cross: T) -> Size<T> {
		if self.is_row {
			Size::new(main, cross)
		} else {
			Size::new(cross, main)
		}
	}
}

/// The container inputs every step of one flex layout reads.
struct FlexContainer<'a> {
	style: &'a Style,
	axes: FlexAxes,
	/// The container's padding and border.
	inner_padding: Edges,
	/// The content box children resolve percentages against; an indefinite
	/// axis leaves their percentages unresolved.
	child_basis: Size<Option<f32>>,
	/// The space the content box may fill.
	inner_available: Size<AvailableSpace>,
	/// The gap between adjacent items on a line.
	item_gap: f32,
	/// The gap between adjacent lines.
	line_gap: f32,
	mode: RunMode,
}

/// The container's final geometry, which its items are placed within.
struct FlexPlacement {
	border_box: Size<f32>,
	/// The content box's main-axis size.
	inner_main: f32,
	/// The content box's cross-axis size.
	inner_cross: f32,
	/// The sized content box, which items resolve percentages against.
	content_basis: Size<Option<f32>>,
}

/// Walks one line along the main axis, spreading its free space.
struct MainCursor {
	/// Where the next item's margin box starts, measured from main-start.
	position: f32,
	/// The extra space after each item, from `justify_content`.
	spacing: f32,
	/// The free space each auto margin absorbs.
	auto_margin: f32,
}

/// Tolerance for float comparisons during the freeze loop.
const EPSILON: f32 = 0.01;

impl Yoga {
	/// Lays out one node and returns its final border-box size.
	///
	/// A clean node already run under the same constraints returns its cached
	/// size without touching its subtree; in [`RunMode::Commit`] the cache only
	/// hits when the subtree still holds that run's positions.
	pub(crate) fn layout_node(
		&mut self,
		node: NodeId,
		ctx: crate::SizingCtx,
		mode: RunMode,
	) -> Size<f32> {
		let ctx = ctx.normalized(&self.nodes[node.0].style);
		if let Some(size) = self.nodes[node.0].layout_cache.get(&ctx, mode) {
			return size;
		}
		let size = self.run_layout(node, ctx, mode);
		let pass = self.pass;
		let entry = &mut self.nodes[node.0];
		entry.layout_cache.insert(ctx, size, mode, pass);
		entry.dirty = false;
		size
	}

	/// Runs the node's layout algorithm, bypassing the cache.
	fn run_layout(&mut self, node: NodeId, ctx: crate::SizingCtx, mode: RunMode) -> Size<f32> {
		let style = self.style(node).clone();
		if style.display == Display::None {
			self.nodes[node.0].layout = Layout::default();
			return Size::new(0.0, 0.0);
		}

		let basis = ctx.percent_basis;
		let metrics = BoxMetrics::new(&style, basis);
		let border = metrics.border;
		let inner_padding = metrics.inner_padding;

		match style.display {
			Display::Grid => self.layout_grid(node, &style, ctx, inner_padding, border, mode),
			_ => self.layout_flex(node, &style, ctx, inner_padding, border, mode),
		}
	}

	/// Measures the node's content-driven size without committing positions.
	///
	/// `percent_basis` is the content box of the node's parent, the basis for
	/// the node's own percentage sizes and paddings.
	pub(crate) fn content_size(
		&mut self,
		node: NodeId,
		available: Size<AvailableSpace>,
		percent_basis: Size<Option<f32>>,
	) -> Size<f32> {
		let ctx = crate::SizingCtx {
			available,
			forced: Size::new(None, None),
			percent_basis,
		};
		self.layout_node(node, ctx, RunMode::Measure)
	}

	/// Lays out a flex container and its children and returns its border-box
	/// size.
	///
	/// Runs the CSS Flexbox layout algorithm (§9) one step at a time: item
	/// base sizes, line collection, flexible lengths, cross sizes, the
	/// container's own size, line alignment, then each item's placement.
	fn layout_flex(
		&mut self,
		node: NodeId,
		style: &Style,
		ctx: crate::SizingCtx,
		inner_padding: Edges,
		border: Edges,
		mode: RunMode,
	) -> Size<f32> {
		let axes = FlexAxes::new(style);
		let (flow, absolute) = self.layout_children(node);

		// The container's own definite size, when it has one, bounds its
		// content box; children resolve percentages against that content box
		// and indefinite axes leave their percentages unresolved.
		let definite_border = definite_border_box(style, &ctx);
		let child_basis = inner_definite(definite_border, inner_padding);
		let inner_available = content_available(
			child_basis,
			bounded_by_max(style, ctx.available, ctx.percent_basis),
			inner_padding,
		);

		// Leaf nodes defer to their measurement function.
		if self.nodes[node.0].measure.is_some() && flow.is_empty() && absolute.is_empty() {
			return self.layout_measured_leaf(
				node,
				style,
				ctx.percent_basis,
				definite_border,
				inner_padding,
				inner_available,
			);
		}

		// Percentage gaps resolve against the matching axis of the content box.
		// Column gaps separate items horizontally and row gaps vertically.
		let gaps = Size::new(
			style.gap.left.resolve(child_basis.width).unwrap_or(0.0),
			style.gap.top.resolve(child_basis.height).unwrap_or(0.0),
		);
		let container = FlexContainer {
			style,
			axes,
			inner_padding,
			child_basis,
			inner_available,
			item_gap: axes.main(gaps),
			line_gap: axes.cross(gaps),
			mode,
		};

		let mut items: Vec<FlexItem> = flow
			.iter()
			.map(|&child| self.flex_item(child, &container))
			.collect();
		let mut lines = collect_flex_lines(&items, &container);
		// §9.7 resolves flexible lengths line by line, against a definite main
		// size only.
		if let Some(limit) = axes.main(inner_available).definite() {
			for line in &lines {
				let gaps = container.item_gap * line.items.len().saturating_sub(1) as f32;
				resolve_flexible_lengths(&mut items, line, limit - gaps, axes.is_row);
			}
		}
		self.measure_cross_sizes(&mut items, &mut lines, &container);
		let placement = size_flex_container(
			&items,
			&lines,
			&container,
			definite_border,
			ctx.percent_basis,
		);
		align_flex_lines(&mut lines, &container, placement.inner_cross);

		let mut content_max_width = 0.0f32;
		let mut content_max_height = 0.0f32;
		for line in &lines {
			let mut cursor = self.main_cursor(line, &items, &container, placement.inner_main);
			for &index in &line.items {
				let corner = self.place_flex_item(
					index,
					&items[index],
					line,
					&mut cursor,
					&container,
					&placement,
				);
				content_max_width = content_max_width.max(corner.width);
				content_max_height = content_max_height.max(corner.height);
			}
		}

		// Out-of-flow children against this container's padding box.
		let border_box = placement.border_box;
		for child in absolute {
			self.layout_absolute(child, border_box, border, style.direction, mode);
			let layout = self.nodes[child.0].layout;
			content_max_width = content_max_width.max(layout.x + layout.width);
			content_max_height = content_max_height.max(layout.y + layout.height);
		}

		{
			let layout = &mut self.nodes[node.0].layout;
			layout.width = border_box.width;
			layout.height = border_box.height;
			layout.content_width = content_max_width.max(border_box.width);
			layout.content_height = content_max_height.max(border_box.height);
		}
		border_box
	}

	/// Sizes a leaf through its measurement function: a definite size wins on
	/// each axis, else the measured content plus padding, clamped by the
	/// node's min and max sizes.
	fn layout_measured_leaf(
		&mut self,
		node: NodeId,
		style: &Style,
		percent_basis: Size<Option<f32>>,
		definite_border: Size<Option<f32>>,
		inner_padding: Edges,
		inner_available: Size<AvailableSpace>,
	) -> Size<f32> {
		let measured = self.measure(node, inner_available);
		let size = Size::new(
			definite_border
				.width
				.unwrap_or(measured.width + inner_padding.horizontal()),
			definite_border
				.height
				.unwrap_or(measured.height + inner_padding.vertical()),
		);
		let size = clamp_size(style, size, percent_basis);
		let layout = &mut self.nodes[node.0].layout;
		layout.width = size.width;
		layout.height = size.height;
		layout.content_width = size.width;
		layout.content_height = size.height;
		size
	}

	/// Builds an item with its flex base size and hypothetical main size.
	///
	/// CSS Flexbox §9.2 step 3 ("determine the flex base size and hypothetical
	/// main size of each item"): the base comes from `flex_basis`, else the
	/// main size, else the item's content, and the hypothetical size clamps it
	/// by the item's min and max main sizes.
	fn flex_item(&mut self, child: NodeId, container: &FlexContainer) -> FlexItem {
		let axes = container.axes;
		let is_row = axes.is_row;
		let child_basis = container.child_basis;
		let child_style = self.style(child).clone();
		let metrics = BoxMetrics::new(&child_style, child_basis);
		let margins = resolve_margins(&child_style.margin, child_basis.width);
		let flex_basis = if !child_style.flex_basis.is_auto() {
			child_style.flex_basis
		} else if is_row {
			child_style.width
		} else {
			child_style.height
		};
		let base = metrics
			.flex_basis(flex_basis, axes.main(child_basis), is_row)
			.unwrap_or_else(|| self.content_flex_base(child, &child_style, &margins, container));
		let (min_main, max_main) = (axes.main(metrics.min), axes.main(metrics.max));
		let padding = if is_row {
			metrics.inner_padding.horizontal()
		} else {
			metrics.inner_padding.vertical()
		};
		let inner = base.clamp(min_main, max_main.max(min_main));
		let outer = inner + main_margins(&margins, is_row);
		FlexItem {
			node: child,
			outer_base: outer,
			flex_base: base,
			inner_main: inner,
			inner_cross: 0.0,
			margins,
			frozen: false,
			grow: child_style.flex_grow,
			shrink: child_style.flex_shrink,
			scaled_shrink: child_style.flex_shrink * (base - padding).max(0.0),
			min_main,
			max_main,
			violation: 0.0,
		}
	}

	/// Measures an item's flex base size from its content, for an item whose
	/// flex basis does not resolve to a definite length.
	///
	/// CSS Flexbox §9.2 step 3E: the item is sized into the container's
	/// available space at its max-content main size.
	fn content_flex_base(
		&mut self,
		child: NodeId,
		child_style: &Style,
		margins: &Edges,
		container: &FlexContainer,
	) -> f32 {
		let axes = container.axes;
		let is_row = axes.is_row;
		let available = shrink_available(
			axes.main(container.inner_available),
			main_margins(margins, is_row),
		);
		let cross_space =
			shrink_available(axes.cross(container.inner_available), margins.horizontal());
		// An item a column will stretch already has its width, as in CSS: it
		// is the column's width, so percentages inside the item resolve
		// against it while its height is measured.
		let stretched_width = (!is_row
			&& child_style.width.is_auto()
			&& child_style
				.align_self
				.resolve_self(container.style.align_items)
				== Align::Stretch
			&& child_style.margin.left != Dimension::Auto
			&& child_style.margin.right != Dimension::Auto)
			.then(|| cross_space.definite())
			.flatten();
		let measured = self.layout_node(
			child,
			crate::SizingCtx {
				available: if is_row {
					Size::new(available, AvailableSpace::MaxContent)
				} else {
					// A column measures its items within its own width, so
					// text wraps at the column's width and the item's height
					// is the wrapped height, not a single line's.
					Size::new(cross_space, available)
				},
				forced: Size::new(stretched_width, None),
				percent_basis: container.child_basis,
			},
			RunMode::Measure,
		);
		axes.main(measured)
	}

	/// Sizes each item's cross axis at its resolved main size, and each line
	/// to fit its largest item.
	///
	/// CSS Flexbox §9.4 steps 7 and 8 ("determine the hypothetical cross size
	/// of each item", "calculate the cross size of each flex line"). Items are
	/// not stretched yet.
	fn measure_cross_sizes(
		&mut self,
		items: &mut [FlexItem],
		lines: &mut [FlexLine],
		container: &FlexContainer,
	) {
		for line in lines {
			let mut line_cross = 0.0f32;
			for &index in &line.items {
				let cross = self.hypothetical_cross_size(&items[index], container);
				items[index].inner_cross = cross;
				let outer_cross =
					cross + cross_margins(&items[index].margins, container.axes.is_row);
				line_cross = line_cross.max(outer_cross);
			}
			line.cross = line_cross;
		}
	}

	/// An item's cross size before stretching: its style size, else the size
	/// its aspect ratio transfers from the resolved main size, else its
	/// content size at that main size, clamped by its min and max cross sizes.
	fn hypothetical_cross_size(&mut self, item: &FlexItem, container: &FlexContainer) -> f32 {
		let axes = container.axes;
		let is_row = axes.is_row;
		let child_basis = container.child_basis;
		let inner_main = item.inner_main;
		let child_style = self.style(item.node).clone();
		let metrics = BoxMetrics::new(&child_style, child_basis);
		let (style_cross, cross_basis) = if is_row {
			(child_style.height, child_basis.height)
		} else {
			(child_style.width, child_basis.width)
		};
		// An auto cross size with an aspect ratio is transferred from the
		// resolved main size, as CSS sizes a ratio-bound box.
		let transferred = child_style.aspect_ratio.map(|ratio| {
			if is_row {
				metrics.ratio_height(inner_main, ratio)
			} else {
				metrics.ratio_width(inner_main, ratio)
			}
		});
		let cross = (if is_row {
			metrics.height(style_cross, cross_basis)
		} else {
			metrics.width(style_cross, cross_basis)
		})
		.or(transferred)
		.unwrap_or_else(|| {
			let available = axes.size(
				AvailableSpace::Definite(inner_main),
				axes.cross(container.inner_available),
			);
			axes.cross(self.content_size(item.node, available, child_basis))
		});
		cross.clamp(axes.cross(metrics.min), axes.cross(metrics.max))
	}

	/// Starts the main-axis walk along one line, spreading its positive free
	/// space.
	///
	/// CSS Flexbox §9.5 step 12 ("distribute any remaining free space"): auto
	/// margins absorb the free space first, and `justify_content` only applies
	/// to a line without them.
	fn main_cursor(
		&self,
		line: &FlexLine,
		items: &[FlexItem],
		container: &FlexContainer,
		inner_main: f32,
	) -> MainCursor {
		let is_row = container.axes.is_row;
		let free_main = inner_main - line_outer_main(line, items, container.item_gap, is_row);

		let mut auto_margins = 0usize;
		for &index in &line.items {
			auto_margins += auto_margin_count(&self.style(items[index].node).margin, is_row);
		}
		let auto_margin = if auto_margins > 0 && free_main > 0.0 {
			free_main / auto_margins as f32
		} else {
			0.0
		};

		let count = line.items.len();
		let justify = if auto_margins > 0 {
			JustifyContent::FlexStart
		} else {
			container.style.justify_content
		};
		// Like CSS, end and center alignment overflow the start edge when the
		// items do not fit: a chat pinned to its newest message stays pinned.
		// The distributed alignments fall back to the start instead.
		let position = match justify {
			JustifyContent::FlexEnd => free_main,
			JustifyContent::Center => free_main / 2.0,
			JustifyContent::SpaceAround => free_main.max(0.0) / count.max(1) as f32 / 2.0,
			JustifyContent::SpaceEvenly => free_main.max(0.0) / (count + 1) as f32,
			_ => 0.0,
		};
		let spacing = match justify {
			JustifyContent::SpaceBetween if count > 1 => free_main.max(0.0) / (count - 1) as f32,
			JustifyContent::SpaceAround => free_main.max(0.0) / count.max(1) as f32,
			JustifyContent::SpaceEvenly => free_main.max(0.0) / (count + 1) as f32,
			_ => 0.0,
		};
		MainCursor {
			position,
			spacing,
			auto_margin,
		}
	}

	/// Lays out one item at its resolved main size, places it on its line,
	/// and advances the cursor past it. Returns the far corner of the item's
	/// border box, which bounds the container's scrollable content.
	///
	/// CSS Flexbox §9.4 step 11 ("determine the used cross size of each item")
	/// stretches an auto-sized item to its line, and §9.6 step 14 ("align all
	/// flex items along the cross-axis") positions the rest by `align_self`.
	fn place_flex_item(
		&mut self,
		index: usize,
		item: &FlexItem,
		line: &FlexLine,
		cursor: &mut MainCursor,
		container: &FlexContainer,
		placement: &FlexPlacement,
	) -> Size<f32> {
		let axes = container.axes;
		let FlexAxes {
			is_row,
			main_reversed,
			cross_reversed,
		} = axes;
		let inner_padding = container.inner_padding;
		let border_box = placement.border_box;
		let child_style = self.style(item.node).clone();
		// Margins are physical; on a reversed axis the main-start margin is the
		// right (or bottom) one.
		let (leading_margin, trailing_margin) =
			main_start_end(item.margins.as_rect(), is_row, main_reversed);
		let (margin_cross_lead, _) =
			main_start_end(item.margins.as_rect(), !is_row, cross_reversed);

		let align = child_style
			.align_self
			.resolve_self(container.style.align_items);
		let outer_cross = item.inner_cross + cross_margins(&item.margins, is_row);
		let cross_free = (line.cross - outer_cross).max(0.0);
		let cross_lead = match align {
			Align::FlexEnd => cross_free,
			Align::Center => cross_free / 2.0,
			_ => 0.0,
		};

		let cross_auto = if is_row {
			child_style.height.is_auto()
		} else {
			child_style.width.is_auto()
		};
		let stretch = matches!(align, Align::Stretch) && cross_auto;
		let final_cross = if stretch {
			(line.cross - cross_margins(&item.margins, is_row)).max(0.0)
		} else {
			item.inner_cross
		};
		// A stretched item fills the line; a ratio-bound item keeps the cross
		// size transferred from its main size.
		let forced_cross =
			(stretch || (cross_auto && child_style.aspect_ratio.is_some())).then_some(final_cross);

		let child_ctx = crate::SizingCtx {
			available: axes.size(
				AvailableSpace::Definite(item.inner_main),
				AvailableSpace::Definite(placement.inner_cross),
			),
			forced: axes.size(Some(item.inner_main), forced_cross),
			percent_basis: placement.content_basis,
		};
		let final_size = self.layout_node(item.node, child_ctx, container.mode);
		let item_main = axes.main(final_size);

		// Relative offsets (`top`/`left` on in-flow items) shift the item away
		// from its laid-out position.
		let relative = relative_offset(
			&child_style,
			placement.content_basis.map(|axis| axis.unwrap_or(0.0)),
			container.style.direction,
		);

		let lead = cursor.position
			+ leading_margin
			+ cursor.auto_margin
				* auto_margin_before_count(&child_style.margin, is_row, main_reversed) as f32;
		let cross_position = line.cross_offset + cross_lead + margin_cross_lead;
		let cross_position = if cross_reversed {
			placement.inner_cross - cross_position - axes.cross(final_size)
		} else {
			cross_position
		};

		let (x, y) = if is_row {
			let x = if main_reversed {
				border_box.width - inner_padding.right - lead - item_main
			} else {
				inner_padding.left + lead
			};
			(
				x + relative.width,
				inner_padding.top + cross_position + relative.height,
			)
		} else {
			let y = if main_reversed {
				border_box.height - inner_padding.bottom - lead - item_main
			} else {
				inner_padding.top + lead
			};
			(
				inner_padding.left + cross_position + relative.width,
				y + relative.height,
			)
		};

		let layout = &mut self.nodes[item.node.0].layout;
		layout.order = index as u32;
		layout.x = x;
		layout.y = y;

		cursor.position += cursor.spacing
			+ container.item_gap
			+ leading_margin
			+ item_main
			+ trailing_margin
			+ cursor.auto_margin * auto_margin_count(&child_style.margin, is_row) as f32;
		Size::new(x + final_size.width, y + final_size.height)
	}

	/// Positions an out-of-flow child against the container's padding box.
	pub(crate) fn layout_absolute(
		&mut self,
		child: NodeId,
		parent_box: Size<f32>,
		border: Edges,
		direction: Direction,
		mode: RunMode,
	) {
		let style = self.style(child).clone();
		// The containing block is the padding box: the border box minus the
		// borders themselves.
		let padding_box = Size::new(
			(parent_box.width - border.horizontal()).max(0.0),
			(parent_box.height - border.vertical()).max(0.0),
		);
		let basis = padding_box.map(Some);
		let metrics = BoxMetrics::new(&style, basis);
		let margins = resolve_margins(&style.margin, Some(padding_box.width));

		let left = resolve_signed(style.inset.left, Some(padding_box.width));
		let right = resolve_signed(style.inset.right, Some(padding_box.width));
		let top = resolve_signed(style.inset.top, Some(padding_box.height));
		let bottom = resolve_signed(style.inset.bottom, Some(padding_box.height));

		let width = metrics.size.width.or_else(|| {
			left.zip(right).map(|(left, right)| {
				(padding_box.width - left - right - margins.horizontal()).max(0.0)
			})
		});
		let height = metrics.size.height.or_else(|| {
			top.zip(bottom).map(|(top, bottom)| {
				(padding_box.height - top - bottom - margins.vertical()).max(0.0)
			})
		});

		let child_ctx = crate::SizingCtx {
			available: Size::new(
				width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
				height.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
			),
			forced: Size::new(width, height),
			percent_basis: Size::new(Some(padding_box.width), Some(padding_box.height)),
		};
		let size = self.layout_node(child, child_ctx, mode);

		let x = if let Some(right) = right.filter(|_| direction == Direction::Rtl) {
			parent_box.width - border.right - right - size.width - margins.right
		} else if let Some(left) = left {
			border.left + left + margins.left
		} else if let Some(right) = right {
			parent_box.width - border.right - right - size.width - margins.right
		} else {
			border.left + margins.left
		};
		let y = if let Some(top) = top {
			border.top + top + margins.top
		} else if let Some(bottom) = bottom {
			parent_box.height - border.bottom - bottom - size.height - margins.bottom
		} else {
			border.top + margins.top
		};

		let layout = &mut self.nodes[child.0].layout;
		layout.order = 0;
		layout.x = x;
		layout.y = y;
		layout.width = size.width;
		layout.height = size.height;
		layout.content_width = size.width;
		layout.content_height = size.height;
	}
}

/// Breaks the items into lines, always returning at least one.
///
/// CSS Flexbox §9.3 step 5 ("collect flex items into flex lines"). Wrapping
/// requires a definite main size, and an item wider than a fresh line stays on
/// that line alone.
fn collect_flex_lines(items: &[FlexItem], container: &FlexContainer) -> Vec<FlexLine> {
	let wraps = container.style.flex_wrap != FlexWrap::NoWrap;
	let definite_inner_main = container.axes.main(container.inner_available).definite();
	let mut lines = vec![FlexLine::default()];
	let mut used_on_line = 0.0f32;
	let mut count_on_line = 0usize;
	for (index, item) in items.iter().enumerate() {
		let fits = !wraps
			|| count_on_line == 0
			|| !matches!(definite_inner_main, Some(limit)
				if used_on_line + container.item_gap + item.outer_base > limit + EPSILON);
		if !fits {
			lines.push(FlexLine::default());
			used_on_line = 0.0;
			count_on_line = 0;
		}
		// A gap only separates items on the same line: the first item of a
		// line, wrapped or not, has none before it.
		let gap_before = if count_on_line > 0 {
			container.item_gap
		} else {
			0.0
		};
		used_on_line += gap_before + item.outer_base;
		count_on_line += 1;
		lines
			.last_mut()
			.expect("a line always exists")
			.items
			.push(index);
	}
	lines
}

/// Sizes the container on each axis from its definite size or, on an
/// indefinite axis, from its content, clamped by its min and max sizes.
///
/// CSS Flexbox §9.2 step 4 ("determine the main size of the flex container")
/// and §9.4 step 15 ("determine the flex container's used cross size"). Forced
/// sizes from the parent algorithm and style sizes win; otherwise the content
/// drives the axis.
fn size_flex_container(
	items: &[FlexItem],
	lines: &[FlexLine],
	container: &FlexContainer,
	definite_border: Size<Option<f32>>,
	percent_basis: Size<Option<f32>>,
) -> FlexPlacement {
	let axes = container.axes;
	let is_row = axes.is_row;
	let inner_padding = container.inner_padding;
	let borders = Size::new(inner_padding.horizontal(), inner_padding.vertical());
	let main_borders = axes.main(borders);
	let cross_borders = axes.cross(borders);
	let lines_cross_total: f32 = lines.iter().map(|line| line.cross).sum::<f32>()
		+ container.line_gap * (lines.len() - 1) as f32;
	// The main axis content size: the widest line.
	let mut lines_main_total = 0.0f32;
	for line in lines {
		lines_main_total =
			lines_main_total.max(line_outer_main(line, items, container.item_gap, is_row));
	}
	// The cross axis content size when items do not wrap: the widest item.
	let mut items_cross_total = 0.0f32;
	for item in items {
		let outer = item.inner_cross + cross_margins(&item.margins, is_row);
		items_cross_total = items_cross_total.max(outer);
	}
	let (content_width, content_height) = if is_row {
		(
			lines_main_total + main_borders,
			lines_cross_total + cross_borders,
		)
	} else {
		(
			items_cross_total + cross_borders,
			lines_main_total + main_borders,
		)
	};
	let border_box = Size::new(
		definite_border.width.unwrap_or(content_width),
		definite_border.height.unwrap_or(content_height),
	);

	let border_box = clamp_size(container.style, border_box, percent_basis);
	let inner_main = (axes.main(border_box) - main_borders).max(0.0);
	let inner_cross = (axes.cross(border_box) - cross_borders).max(0.0);
	FlexPlacement {
		border_box,
		inner_main,
		inner_cross,
		content_basis: axes.size(Some(inner_main), Some(inner_cross)),
	}
}

/// Sizes and offsets the lines along the container's cross axis.
///
/// CSS Flexbox §9.4 step 8 gives a single-line container's line exactly the
/// container's inner cross size, where `align_content` does not apply, and
/// §9.6 step 13 ("align all flex lines") aligns a multi-line container's lines
/// by `align_content`.
fn align_flex_lines(lines: &mut [FlexLine], container: &FlexContainer, inner_cross: f32) {
	if container.style.flex_wrap == FlexWrap::NoWrap {
		let line = lines.first_mut().expect("a line always exists");
		line.cross = inner_cross;
		line.cross_offset = 0.0;
		return;
	}
	let leftover_cross = inner_cross
		- (lines.iter().map(|line| line.cross).sum::<f32>()
			+ container.line_gap * lines.len().saturating_sub(1) as f32);
	place_lines(
		lines,
		leftover_cross,
		container.line_gap,
		container.style.align_content,
	);
}

/// The main-axis size of a line's items, margins and gaps included.
fn line_outer_main(line: &FlexLine, items: &[FlexItem], item_gap: f32, is_row: bool) -> f32 {
	line.items
		.iter()
		.map(|&index| items[index].inner_main + main_margins(&items[index].margins, is_row))
		.sum::<f32>()
		+ item_gap * line.items.len().saturating_sub(1) as f32
}

/// Resolves CSS flexible lengths while preserving the original flex bases.
///
/// Each pass freezes the items responsible for the total min/max violation.
/// Reusing partially grown sizes as bases would distort the remaining items'
/// proportions, particularly when the sum of their flex factors is below one.
fn resolve_flexible_lengths(items: &mut [FlexItem], line: &FlexLine, space: f32, is_row: bool) {
	let hypothetical: f32 = line
		.items
		.iter()
		.map(|&index| items[index].outer_base)
		.sum();
	let growing = hypothetical < space;
	let factor = |item: &FlexItem| if growing { item.grow } else { item.shrink };
	let weight = |item: &FlexItem| {
		if growing {
			item.grow
		} else {
			item.scaled_shrink
		}
	};

	for &index in &line.items {
		let item = &mut items[index];
		item.frozen = factor(item) == 0.0
			|| if growing {
				item.flex_base > item.inner_main
			} else {
				item.flex_base < item.inner_main
			};
	}

	let remaining = |items: &[FlexItem]| {
		space
			- line
				.items
				.iter()
				.map(|&index| {
					let item = &items[index];
					(if item.frozen {
						item.inner_main
					} else {
						item.flex_base
					}) + main_margins(&item.margins, is_row)
				})
				.sum::<f32>()
	};
	let initial_free = remaining(items);

	for _ in 0..line.items.len() {
		let factors: f32 = line
			.items
			.iter()
			.filter(|&&index| !items[index].frozen)
			.map(|&index| factor(&items[index]))
			.sum();
		let weights: f32 = line
			.items
			.iter()
			.filter(|&&index| !items[index].frozen)
			.map(|&index| weight(&items[index]))
			.sum();
		let mut free = remaining(items);

		if factors < 1.0 && (initial_free * factors).abs() < free.abs() {
			free = initial_free * factors;
		}

		let mut total_violation = 0.0;

		for &index in &line.items {
			let item = &mut items[index];

			if item.frozen {
				continue;
			}

			let target = item.flex_base
				+ if weights > 0.0 {
					free * weight(item) / weights
				} else {
					0.0
				};
			item.inner_main = target.clamp(item.min_main, item.max_main);
			item.violation = item.inner_main - target;
			total_violation += item.violation;
		}

		for &index in &line.items {
			let item = &mut items[index];
			item.frozen |= total_violation.abs() <= EPSILON
				|| if total_violation > 0.0 {
					item.violation > 0.0
				} else {
					item.violation < 0.0
				};
		}

		if line.items.iter().all(|&index| items[index].frozen) {
			break;
		}
	}
}

/// The margins along the main axis.
fn main_margins(margins: &Edges, is_row: bool) -> f32 {
	if is_row {
		margins.horizontal()
	} else {
		margins.vertical()
	}
}

/// The margins along the cross axis.
fn cross_margins(margins: &Edges, is_row: bool) -> f32 {
	if is_row {
		margins.vertical()
	} else {
		margins.horizontal()
	}
}

fn shrink_available(space: AvailableSpace, subtract: f32) -> AvailableSpace {
	match space {
		AvailableSpace::Definite(value) => AvailableSpace::Definite((value - subtract).max(0.0)),
		other => other,
	}
}

/// The physical margin edges at the main-start and main-end sides.
///
/// Margins are physical while the main axis has a direction, so a reversed
/// row starts at its right edge and a reversed column at its bottom edge.
fn main_start_end<T>(margins: Rect<T>, is_row: bool, reversed: bool) -> (T, T) {
	match (is_row, reversed) {
		(true, false) => (margins.left, margins.right),
		(true, true) => (margins.right, margins.left),
		(false, false) => (margins.top, margins.bottom),
		(false, true) => (margins.bottom, margins.top),
	}
}

/// Auto margins on the main-start edge, which shift the item before it is
/// placed.
fn auto_margin_before_count(margin: &Rect<Dimension>, is_row: bool, reversed: bool) -> usize {
	let (start, _) = main_start_end(*margin, is_row, reversed);
	usize::from(start == Dimension::Auto)
}

fn auto_margin_count(margin: &Rect<Dimension>, is_row: bool) -> usize {
	let (start, end) = main_start_end(*margin, is_row, false);
	usize::from(start == Dimension::Auto) + usize::from(end == Dimension::Auto)
}

/// Distributes leftover cross space between wrapped lines, writing each line's
/// `cross_offset`.
fn place_lines(lines: &mut [FlexLine], leftover: f32, line_gap: f32, align: Align) {
	let count = lines.len();
	let leftover = leftover.max(0.0);
	// `normal` (the `Auto` default) behaves as stretch for multi-line flex
	// containers.
	let stretch_share = if matches!(align, Align::Stretch | Align::Auto) && count > 0 {
		leftover / count as f32
	} else {
		0.0
	};
	let mut cursor = match align {
		Align::FlexEnd => leftover,
		Align::Center => leftover / 2.0,
		Align::SpaceAround => leftover / count.max(1) as f32 / 2.0,
		Align::SpaceEvenly => leftover / (count + 1) as f32,
		_ => 0.0,
	};
	let spacing = match align {
		Align::SpaceBetween if count > 1 => leftover / (count - 1) as f32,
		Align::SpaceAround => leftover / count.max(1) as f32,
		Align::SpaceEvenly => leftover / (count + 1) as f32,
		_ => 0.0,
	};
	for line in lines.iter_mut() {
		if stretch_share > 0.0 {
			line.cross += stretch_share;
		}
		line.cross_offset = cursor;
		cursor += line.cross + line_gap + spacing;
	}
}
