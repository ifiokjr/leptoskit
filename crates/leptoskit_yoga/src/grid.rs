//! The CSS grid layout algorithm.
//!
//! Implements track sizing (`auto`, lengths, percentages, `fr`, `minmax`),
//! line-based placement with spans, the auto-placement algorithm in row-major,
//! column-major, sparse, and dense modes, grid gaps, and area alignment
//! (`justify-self`/`align-self` including stretch).
//!
//! Columns are sized completely (including leftover-space distribution)
//! before rows, so row heights come from items measured at the width of the
//! columns they span.

use crate::AvailableSpace;
use crate::BoxMetrics;
use crate::Edges;
use crate::NodeId;
use crate::RunMode;
use crate::Size;
use crate::Style;
use crate::Yoga;
use crate::bounded_by_max;
use crate::clamp_height;
use crate::clamp_width;
use crate::content_available;
use crate::definite_border_box;
use crate::inner_definite;
use crate::relative_offset;
use crate::resolve_margins;
use crate::resolve_signed;
use crate::style::Align;
use crate::style::GridLine;
use crate::style::GridPlacement;
use crate::style::TrackSizing;

/// The largest line number, and the longest span, a placement may use.
///
/// Larger values are clamped so a single item cannot allocate an unbounded
/// implicit grid; browsers bound the implicit grid the same way.
pub(crate) const MAX_GRID_LINE: i64 = 10_000;

/// Tolerance for float comparisons while growing tracks.
const EPSILON: f32 = 0.01;

/// A placed grid item.
struct GridItem {
	node: NodeId,
	/// 0-based starting column track.
	col_start: usize,
	/// The number of column tracks spanned.
	col_span: usize,
	/// 0-based starting row track.
	row_start: usize,
	/// The number of row tracks spanned.
	row_span: usize,
	/// Resolved margins.
	margins: Edges,
}

/// The state of one axis of the grid.
#[derive(Default)]
struct AxisTracks {
	/// The original explicit track count, anchoring negative line references.
	explicit: usize,
	/// The sizing function of each track, explicit first, then implicit.
	sizings: Vec<TrackSizing>,
	/// The resolved base size of each track.
	bases: Vec<f32>,
	/// Whether the track still grows from content.
	intrinsic: Vec<bool>,
	/// The definite maximum of a `minmax()` track, which free space grows the
	/// track toward.
	limits: Vec<Option<f32>>,
	/// Fractional maxima, including `minmax(0, 1fr)` tracks.
	fractions: Vec<f32>,
	/// Max-content contributions crossing fractional tracks, resolved as a
	/// single span so unequal flex factors preserve their proportions.
	fraction_contributions: Vec<TrackContribution>,
	/// The final offset of each track's start edge.
	offsets: Vec<f32>,
	/// The total axis length once sized.
	total: f32,
}

/// Physical edges of one axis of a containing box.
#[derive(Clone, Copy)]
struct AxisBounds {
	start: f32,
	end: f32,
}

/// One physical axis of an absolutely positioned grid descendant.
struct AbsoluteAxis {
	bounds: AxisBounds,
	start: Option<f32>,
	end: Option<f32>,
	margin_start: f32,
	margin_end: f32,
	auto_margin_start: bool,
	auto_margin_end: bool,
	alignment: Align,
	static_alignment: Align,
	reverse: bool,
}

impl AbsoluteAxis {
	fn size(&self) -> f32 {
		(self.bounds.end - self.bounds.start).max(0.0)
	}

	fn available(&self) -> f32 {
		(self.size()
			- self.start.unwrap_or(0.0)
			- self.end.unwrap_or(0.0)
			- self.margin_start
			- self.margin_end)
			.max(0.0)
	}

	fn forced(&self, preferred: Option<f32>) -> Option<f32> {
		preferred.or_else(|| {
			(self.start.is_some()
				&& self.end.is_some()
				&& matches!(self.alignment, Align::Auto | Align::Stretch))
			.then(|| self.available())
		})
	}

	fn position(&self, size: f32) -> f32 {
		let containing_start = self.bounds.start;

		let (start, end, alignment) = match (self.start, self.end) {
			(Some(start), None) => return containing_start + start + self.margin_start,
			(None, Some(end)) => return self.bounds.end - end - size - self.margin_end,
			(None, None) => (self.bounds.start, self.bounds.end, self.static_alignment),
			(Some(start), Some(end)) => {
				(
					containing_start + start,
					self.bounds.end - end,
					self.alignment,
				)
			}
		};

		let free = end - start - self.margin_start - self.margin_end - size;

		if self.start.is_some() && self.end.is_some() && free > 0.0 {
			let auto_margins = u8::from(self.auto_margin_start) + u8::from(self.auto_margin_end);

			if auto_margins > 0 {
				return start
					+ self.margin_start
					+ if self.auto_margin_start {
						free / f32::from(auto_margins)
					} else {
						0.0
					};
			}
		}

		let offset = match alignment {
			Align::Center => free / 2.0,

			Align::FlexEnd if !self.reverse => free,
			Align::FlexEnd => 0.0,

			_ if self.reverse => free,
			_ => 0.0,
		};

		start + self.margin_start + offset
	}
}

/// A max-content contribution used to find an intrinsic fractional unit.
struct TrackContribution {
	start: usize,
	span: usize,
	size: f32,
}

/// A grid area in flow-relative coordinates.
///
/// `major` is the axis the auto-placement cursor wraps along (rows under row
/// flow) and `minor` the axis it scans (columns under row flow), so a single
/// placement routine serves both flow directions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Area {
	major: usize,
	minor: usize,
	major_span: usize,
	minor_span: usize,
}

impl Area {
	fn overlaps(self, other: Self) -> bool {
		self.major < other.major + other.major_span
			&& other.major < self.major + self.major_span
			&& self.minor < other.minor + other.minor_span
			&& other.minor < self.minor + self.minor_span
	}
}

/// An item's placement request in flow-relative coordinates; `None` lines are
/// left to auto-placement.
struct PlacementRequest {
	major: Option<usize>,
	major_span: usize,
	minor: Option<usize>,
	minor_span: usize,
}

/// One axis's constraints on a grid container, which size its tracks and the
/// container along that axis.
struct AxisSpace {
	/// The content-box size percentages resolve against, when definite.
	basis: Option<f32>,
	/// The space the content box may fill.
	available: AvailableSpace,
	/// The container's definite border-box size.
	definite: Option<f32>,
	/// The container's padding and border along the axis.
	edges: f32,
	/// The gap between adjacent tracks.
	gap: f32,
	/// How leftover space is distributed between the tracks.
	alignment: Align,
}

/// One axis of a grid container once its tracks are sized.
struct SettledAxis {
	/// The container's border-box size along the axis.
	border: f32,
	/// The leading offset leftover-space distribution gives the tracks.
	offset: f32,
}

/// A grid container with both axes sized, ready for its children to be
/// placed.
struct SizedGrid<'a> {
	style: &'a Style,
	grid: &'a crate::GridStyle,
	columns: AxisTracks,
	rows: AxisTracks,
	column_gap: f32,
	row_gap: f32,
	/// The container's padding and border.
	inner_padding: Edges,
	/// The container's borders alone, which bound out-of-flow children.
	border: Edges,
	border_box: Size<f32>,
	/// The tracks' leading offsets from leftover-space distribution:
	/// `justify_content` along the columns and `align_content` along the rows.
	offset: Size<f32>,
	mode: RunMode,
}

impl Yoga {
	/// Lays out a grid container and its children and returns its border-box
	/// size.
	///
	/// Items are placed on the grid first. Columns are then sized completely,
	/// including leftover-space distribution, before rows, so row heights come
	/// from items measured at the width of the columns they span.
	pub(crate) fn layout_grid(
		&mut self,
		node: NodeId,
		style: &Style,
		ctx: crate::SizingCtx,
		inner_padding: Edges,
		border: Edges,
		mode: RunMode,
	) -> Size<f32> {
		let mut grid = style.grid.clone().unwrap_or_default();
		let explicit_columns = grid.template_columns.len();
		let explicit_rows = grid.template_rows.len();
		let definite_border = definite_border_box(style, &ctx);
		let child_basis = inner_definite(definite_border, inner_padding);
		let inner_available = content_available(
			child_basis,
			bounded_by_max(style, ctx.available, ctx.percent_basis),
			inner_padding,
		);

		let (flow, absolute) = self.layout_children(node);
		let mut items: Vec<GridItem> = flow
			.into_iter()
			.map(|child| {
				GridItem {
					node: child,
					col_start: 0,
					col_span: 1,
					row_start: 0,
					row_span: 1,
					margins: resolve_margins(&self.style(child).margin, child_basis.width),
				}
			})
			.collect();
		self.place_grid_items(&mut items, &grid);

		let mut columns = AxisTracks {
			explicit: explicit_columns,
			sizings: std::mem::take(&mut grid.template_columns),
			..AxisTracks::default()
		};
		let mut rows = AxisTracks {
			explicit: explicit_rows,
			sizings: std::mem::take(&mut grid.template_rows),
			..AxisTracks::default()
		};

		for item in &items {
			columns.reserve(item.col_start + item.col_span, &grid.auto_columns);
			rows.reserve(item.row_start + item.row_span, &grid.auto_rows);
		}

		// Percentage gaps resolve against the matching axis of the content box.
		let column_gap = style.gap.left.resolve(child_basis.width).unwrap_or(0.0);
		let row_gap = style.gap.top.resolve(child_basis.height).unwrap_or(0.0);

		// Columns: content contributions at max-content. The container width,
		// and with it every column, is settled before rows are sized.
		columns.init_bases(child_basis.width);
		self.contribute_column_widths(&items, &mut columns, column_gap);
		let width = columns.settle(
			&AxisSpace {
				basis: child_basis.width,
				available: inner_available.width,
				definite: definite_border.width,
				edges: inner_padding.horizontal(),
				gap: column_gap,
				alignment: justify_to_align(style.justify_content),
			},
			|width| clamp_width(style, width, ctx.percent_basis),
		);

		// Rows: content contributions at the width of the spanned columns.
		rows.init_bases(child_basis.height);
		self.contribute_row_heights(&items, &mut rows, &columns.bases, column_gap, row_gap);
		let height = rows.settle(
			&AxisSpace {
				basis: child_basis.height,
				available: inner_available.height,
				definite: definite_border.height,
				edges: inner_padding.vertical(),
				gap: row_gap,
				alignment: style.align_content,
			},
			|height| clamp_height(style, height, ctx.percent_basis),
		);

		let sized = SizedGrid {
			style,
			grid: &grid,
			columns,
			rows,
			column_gap,
			row_gap,
			inner_padding,
			border,
			border_box: Size::new(width.border, height.border),
			offset: Size::new(width.offset, height.offset),
			mode,
		};
		let mut content_max_width = 0.0f32;
		let mut content_max_height = 0.0f32;

		for (index, item) in items.iter().enumerate() {
			let corner = self.place_grid_item(index, item, &sized);
			content_max_width = content_max_width.max(corner.width);
			content_max_height = content_max_height.max(corner.height);
		}

		for child in absolute {
			let corner = self.place_absolute_grid_child(child, &sized);
			content_max_width = content_max_width.max(corner.width);
			content_max_height = content_max_height.max(corner.height);
		}

		let border_box = sized.border_box;
		{
			let layout = &mut self.nodes[node.0].layout;
			layout.width = border_box.width;
			layout.height = border_box.height;
			layout.content_width = content_max_width.max(border_box.width);
			layout.content_height = content_max_height.max(border_box.height);
		}
		border_box
	}

	/// Grows the intrinsic columns each item spans to fit its max-content
	/// width.
	fn contribute_column_widths(&mut self, items: &[GridItem], columns: &mut AxisTracks, gap: f32) {
		for item in items {
			if !columns.spans_intrinsic(item.col_start, item.col_span) {
				continue;
			}

			let contribution = BoxMetrics::new(self.style(item.node), Size::new(None, None))
				.size
				.width
				.unwrap_or_else(|| {
					self.content_size(
						item.node,
						Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
						Size::new(None, None),
					)
					.width
				})
				+ item.margins.horizontal();
			columns.contribute(item.col_start, item.col_span, contribution, gap);
		}
	}

	/// Grows the intrinsic rows each item spans to fit its height when laid
	/// out at the width of the columns it spans.
	fn contribute_row_heights(
		&mut self,
		items: &[GridItem],
		rows: &mut AxisTracks,
		column_bases: &[f32],
		column_gap: f32,
		row_gap: f32,
	) {
		for item in items {
			if !rows.spans_intrinsic(item.row_start, item.row_span) {
				continue;
			}

			let area_width = spanned_size(column_bases, item.col_start, item.col_span, column_gap);
			let contribution =
				BoxMetrics::new(self.style(item.node), Size::new(Some(area_width), None))
					.size
					.height
					.unwrap_or_else(|| {
						let width = (area_width - item.margins.horizontal()).max(0.0);
						self.content_size(
							item.node,
							Size::new(AvailableSpace::Definite(width), AvailableSpace::MaxContent),
							Size::new(Some(area_width), None),
						)
						.height
					})
					+ item.margins.vertical();
			rows.contribute(item.row_start, item.row_span, contribution, row_gap);
		}
	}

	/// Lays out one in-flow item in its grid area and aligns it there by
	/// `justify_self` and `align_self`, stretching auto-sized axes to fill the
	/// area. Returns the far corner of the item's border box.
	fn place_grid_item(&mut self, index: usize, item: &GridItem, sized: &SizedGrid) -> Size<f32> {
		let SizedGrid {
			columns,
			rows,
			inner_padding,
			..
		} = sized;
		let child_style = self.shared_style(item.node);
		let child_grid = child_style.grid.clone().unwrap_or_default();
		let area_x = columns.offsets.get(item.col_start).copied().unwrap_or(0.0);
		let area_width = spanned_size(
			&columns.bases,
			item.col_start,
			item.col_span,
			sized.column_gap,
		);
		let area_y = rows.offsets.get(item.row_start).copied().unwrap_or(0.0);
		let area_height = spanned_size(&rows.bases, item.row_start, item.row_span, sized.row_gap);
		let metrics = BoxMetrics::new(&child_style, Size::new(Some(area_width), Some(area_height)));

		let margins = resolve_margins(&child_style.margin, Some(area_width));

		// `normal` (the `Auto` default) behaves as stretch for grid items.
		let justify = child_grid
			.justify_self
			.resolve_self(sized.grid.justify_items);
		let justify = if matches!(justify, Align::Auto) {
			Align::Stretch
		} else {
			justify
		};

		let align = child_style.align_self.resolve_self(sized.style.align_items);
		let stretch_width = matches!(justify, Align::Stretch) && child_style.width.is_auto();
		let stretch_height = matches!(align, Align::Stretch) && child_style.height.is_auto();

		let width = if stretch_width {
			Some((area_width - margins.horizontal()).max(0.0))
		} else {
			metrics.size.width
		};
		let height = if stretch_height {
			Some((area_height - margins.vertical()).max(0.0))
		} else {
			metrics.size.height
		};

		let child_ctx = crate::SizingCtx {
			available: Size::new(
				width.map_or(AvailableSpace::Definite(area_width), |w| {
					AvailableSpace::Definite(w.max(area_width))
				}),
				height.map_or(AvailableSpace::Definite(area_height), |h| {
					AvailableSpace::Definite(h.max(area_height))
				}),
			),
			forced: Size::new(width, height),
			percent_basis: Size::new(Some(area_width), Some(area_height)),
		};
		let size = self.layout_node(item.node, child_ctx, sized.mode);

		// Alignment inside the area for non-stretched axes.
		let free_x = (area_width - margins.horizontal() - size.width).max(0.0);
		let free_y = (area_height - margins.vertical() - size.height).max(0.0);

		let lead_x = match justify {
			Align::FlexEnd => free_x,
			Align::Center => free_x / 2.0,
			_ => 0.0,
		};

		let lead_y = match align {
			Align::FlexEnd => free_y,
			Align::Center => free_y / 2.0,
			_ => 0.0,
		};

		let relative = relative_offset(
			&child_style,
			Size::new(area_width, area_height),
			sized.style.direction,
		);
		let x = inner_padding.left
			+ area_x
			+ sized.offset.width
			+ lead_x
			+ margins.left
			+ relative.width;
		let y = inner_padding.top
			+ area_y
			+ sized.offset.height
			+ lead_y
			+ margins.top
			+ relative.height;
		let layout = &mut self.nodes[item.node.0].layout;
		layout.order = index as u32;
		layout.x = x;
		layout.y = y;
		Size::new(x + size.width, y + size.height)
	}

	/// Positions an out-of-flow child against the grid area its lines name,
	/// with the padding box standing in for auto or missing lines. Returns the
	/// far corner of the child's border box.
	fn place_absolute_grid_child(&mut self, child: NodeId, sized: &SizedGrid) -> Size<f32> {
		let SizedGrid {
			border,
			border_box,
			inner_padding,
			offset,
			..
		} = sized;
		let child_grid = self.style(child).grid.as_ref();
		let horizontal = sized.columns.absolute_bounds(
			child_grid.map_or(GridPlacement::AUTO, |grid| grid.column),
			AxisBounds {
				start: border.left,
				end: border_box.width - border.right,
			},
			AxisBounds {
				start: inner_padding.left,
				end: border_box.width - inner_padding.right,
			},
			offset.width,
			sized.style.direction == crate::Direction::Rtl,
		);
		let vertical = sized.rows.absolute_bounds(
			child_grid.map_or(GridPlacement::AUTO, |grid| grid.row),
			AxisBounds {
				start: border.top,
				end: border_box.height - border.bottom,
			},
			AxisBounds {
				start: inner_padding.top,
				end: border_box.height - inner_padding.bottom,
			},
			offset.height,
			false,
		);
		// Grid placement chooses the containing block before physical insets,
		// percentages, and positioned-box alignment resolve.
		self.layout_absolute_grid(
			child,
			Size::new(horizontal, vertical),
			sized.style,
			sized.grid,
			sized.mode,
		);
		let layout = &self.nodes[child.0].layout;
		Size::new(layout.x + layout.width, layout.y + layout.height)
	}

	/// Applies positioned-box sizing and self-alignment within a grid area.
	fn layout_absolute_grid(
		&mut self,
		child: NodeId,
		bounds: Size<AxisBounds>,
		parent: &Style,
		grid: &crate::GridStyle,
		mode: RunMode,
	) {
		let style = self.shared_style(child);
		let width = (bounds.width.end - bounds.width.start).max(0.0);
		let height = (bounds.height.end - bounds.height.start).max(0.0);
		let basis = Size::new(Some(width), Some(height));
		let metrics = BoxMetrics::new(&style, basis);
		let margins = resolve_margins(&style.margin, Some(width));
		let justify = style
			.grid
			.as_ref()
			.map_or(Align::Auto, |grid| grid.justify_self);
		let horizontal = AbsoluteAxis {
			bounds: bounds.width,
			start: resolve_signed(style.inset.left, Some(width)),
			end: resolve_signed(style.inset.right, Some(width)),
			margin_start: margins.left,
			margin_end: margins.right,
			auto_margin_start: style.margin.left.is_auto(),
			auto_margin_end: style.margin.right.is_auto(),
			alignment: justify,
			static_alignment: justify.resolve_self(grid.justify_items),
			reverse: parent.direction == crate::Direction::Rtl,
		};

		let vertical = AbsoluteAxis {
			bounds: bounds.height,
			start: resolve_signed(style.inset.top, Some(height)),
			end: resolve_signed(style.inset.bottom, Some(height)),
			margin_start: margins.top,
			margin_end: margins.bottom,
			auto_margin_start: style.margin.top.is_auto(),
			auto_margin_end: style.margin.bottom.is_auto(),
			alignment: style.align_self,
			static_alignment: style.align_self.resolve_self(parent.align_items),
			reverse: false,
		};
		let size = self.layout_node(
			child,
			crate::SizingCtx {
				available: Size::new(
					AvailableSpace::Definite(horizontal.available()),
					AvailableSpace::Definite(vertical.available()),
				),
				forced: Size::new(
					horizontal.forced(metrics.size.width),
					vertical.forced(metrics.size.height),
				),
				percent_basis: basis,
			},
			mode,
		);
		let layout = &mut self.nodes[child.0].layout;
		layout.order = 0;
		layout.x = horizontal.position(size.width);
		layout.y = vertical.position(size.height);
	}

	/// Resolves every item's placement, writing its start tracks and spans.
	fn place_grid_items(&self, items: &mut [GridItem], grid: &crate::GridStyle) {
		let row_flow = grid.auto_flow.is_row();
		let explicit_columns = grid.template_columns.len();
		let explicit_rows = grid.template_rows.len();
		let requests: Vec<PlacementRequest> = items
			.iter()
			.map(|item| {
				let (column, row) = self
					.style(item.node)
					.grid
					.as_ref()
					.map_or((GridPlacement::AUTO, GridPlacement::AUTO), |child| {
						(child.column, child.row)
					});
				let (col, col_span) = resolve_axis(&column, explicit_columns);
				let (row, row_span) = resolve_axis(&row, explicit_rows);
				if row_flow {
					PlacementRequest {
						major: row,
						major_span: row_span,
						minor: col,
						minor_span: col_span,
					}
				} else {
					PlacementRequest {
						major: col,
						major_span: col_span,
						minor: row,
						minor_span: row_span,
					}
				}
			})
			.collect();

		// The number of tracks the cursor scans before wrapping: the explicit
		// tracks, extended by definite placements. Column flow with no rows at
		// all never wraps, so items stack down the first column.
		let explicit_minor = if row_flow {
			explicit_columns
		} else {
			explicit_rows
		};

		let minor_tracks = requests
			.iter()
			.filter_map(|request| request.minor.map(|minor| minor + request.minor_span))
			.fold(explicit_minor, usize::max);
		let minor_tracks = if minor_tracks == 0 && !row_flow {
			usize::MAX
		} else {
			minor_tracks.max(1)
		};

		let areas = place_areas(&requests, grid.auto_flow.is_dense(), minor_tracks);

		for (item, area) in items.iter_mut().zip(areas) {
			if row_flow {
				item.row_start = area.major;
				item.row_span = area.major_span;
				item.col_start = area.minor;
				item.col_span = area.minor_span;
			} else {
				item.col_start = area.major;
				item.col_span = area.major_span;
				item.row_start = area.minor;
				item.row_span = area.minor_span;
			}
		}
	}
}

impl AxisTracks {
	/// Resolves abspos grid lines without extending or sizing the implicit
	/// grid.
	fn absolute_bounds(
		&self,
		placement: GridPlacement,
		padding: AxisBounds,
		content: AxisBounds,
		alignment: f32,
		reverse: bool,
	) -> AxisBounds {
		let (start, end) = absolute_lines(placement, self.explicit, self.bases.len());
		let start_offset = |index: usize| {
			self.offsets.get(index).copied().unwrap_or_else(|| {
				self.offsets.last().copied().unwrap_or(0.0)
					+ self.bases.last().copied().unwrap_or(0.0)
			})
		};
		let end_offset = |index: usize| {
			index.checked_sub(1).map_or_else(
				|| self.offsets.first().copied().unwrap_or(0.0),
				|previous| self.offsets[previous] + self.bases[previous],
			)
		};

		let (start, end) = if reverse {
			(
				end.map(|index| content.end - alignment - end_offset(index)),
				start.map(|index| content.end - alignment - start_offset(index)),
			)
		} else {
			(
				start.map(|index| content.start + alignment + start_offset(index)),
				end.map(|index| content.start + alignment + end_offset(index)),
			)
		};

		AxisBounds {
			start: start.unwrap_or(padding.start),
			end: end.unwrap_or(padding.end),
		}
	}

	/// Sizes the tracks once every item has contributed, then the container
	/// along this axis, and distributes the leftover space.
	///
	/// The container follows its tracks unless its size is definite, and
	/// `clamp` applies its min and max sizes. The tracks are sized again
	/// against the final content box when percentage tracks only now have a
	/// basis, or when clamping changed the space `fr` tracks share.
	fn settle(&mut self, space: &AxisSpace, clamp: impl Fn(f32) -> f32) -> SettledAxis {
		let content_bases = self.bases.clone();
		self.finish(intrinsic_available(space.basis, space.available), space.gap);
		let preferred = space.definite.unwrap_or(self.total + space.edges);
		let border = clamp(preferred);
		let inner = (border - space.edges).max(0.0);

		let resolved_percentages =
			space.basis.is_none() && self.resolve_percentages(inner, &content_bases);

		if resolved_percentages
			|| ((border - preferred).abs() > f32::EPSILON
				&& self.fractions.iter().any(|&factor| factor > 0.0))
		{
			if !resolved_percentages {
				self.bases = content_bases;
			}

			self.finish(AvailableSpace::Definite(inner), space.gap);
		}

		SettledAxis {
			border,
			offset: distribute_leftover(self, inner, space.gap, space.alignment),
		}
	}

	/// Ensures at least `count` tracks exist, appending the implicit sizing.
	fn reserve(&mut self, count: usize, implicit: &TrackSizing) {
		while self.sizings.len() < count {
			self.sizings.push(implicit.clone());
		}
	}

	/// Base sizes, growth limits, and intrinsic flags from the sizing
	/// functions alone.
	fn init_bases(&mut self, container: Option<f32>) {
		let count = self.sizings.len();
		self.bases = vec![0.0; count];
		self.intrinsic = vec![false; count];
		self.limits = vec![None; count];
		self.fractions = vec![0.0; count];
		self.fraction_contributions.clear();

		for (index, sizing) in self.sizings.iter().enumerate() {
			match sizing {
				TrackSizing::Points(value) => self.bases[index] = value.max(0.0),
				TrackSizing::Percent(fraction) => {
					self.bases[index] = container.map_or(0.0, |total| (fraction * total).max(0.0));
					self.intrinsic[index] = container.is_none();
				}
				TrackSizing::MinMax(inner) => {
					let base = inner.min.resolve_base(container).unwrap_or(0.0).max(0.0);
					self.bases[index] = base;
					// Fractional maxima share space after fixed minimums; auto
					// maxima grow from intrinsic content contributions.
					match inner.max {
						TrackSizing::Fraction(fr) => {
							self.fractions[index] = crate::finite(fr).max(0.0);
							self.intrinsic[index] =
								matches!(inner.min, TrackSizing::Auto) || container.is_none();
						}
						_ => {
							match inner.max.resolve_base(container) {
								Some(max) => self.limits[index] = Some(max.max(base)),
								None => self.intrinsic[index] = true,
							}
						}
					}

					if matches!(inner.min, TrackSizing::Auto)
						|| (container.is_none() && contains_percentage(&inner.min))
					{
						self.intrinsic[index] = true;
					}
				}
				TrackSizing::Auto => self.intrinsic[index] = true,
				// `fr` tracks are resolved against leftover space in
				// `finish`, not against content.
				TrackSizing::Fraction(fr) => {
					self.fractions[index] = crate::finite(*fr).max(0.0);
					self.intrinsic[index] = container.is_none();
				}
			}
		}
	}

	/// Cyclic percentages size intrinsically as auto, then resolve for layout.
	fn resolve_percentages(&mut self, container: f32, content_bases: &[f32]) -> bool {
		if !self.sizings.iter().any(contains_percentage) {
			return false;
		}

		self.init_bases(Some(container));

		for (index, content_base) in content_bases.iter().enumerate() {
			if self.intrinsic[index] {
				self.bases[index] = self.bases[index].max(*content_base);
			}
		}

		true
	}

	/// Whether any track in the span still grows from content.
	fn spans_intrinsic(&self, start: usize, span: usize) -> bool {
		self.intrinsic[start..start + span].iter().any(|&flag| flag)
	}

	/// Grows the spanned intrinsic tracks to fit an item's contribution.
	fn contribute(&mut self, start: usize, span: usize, contribution: f32, gap: f32) {
		if self.fractions[start..start + span]
			.iter()
			.any(|&factor| factor > 0.0)
		{
			self.fraction_contributions.push(TrackContribution {
				start,
				span,
				size: contribution,
			});

			return;
		}

		if span == 1 {
			self.bases[start] = self.bases[start].max(contribution);

			return;
		}

		// Distribute the contribution across the spanned intrinsic tracks
		// after accounting for definite tracks and gaps.
		let tracks = start..start + span;
		let current: f32 = tracks.clone().map(|index| self.bases[index]).sum();
		let intrinsic_count = tracks
			.clone()
			.filter(|&index| self.intrinsic[index])
			.count();
		let needed = (contribution - current - gap * (span - 1) as f32).max(0.0);
		let share = needed / intrinsic_count as f32;

		for index in tracks {
			if self.intrinsic[index] {
				self.bases[index] += share;
			}
		}
	}

	/// The tracks' summed size including gaps.
	fn used(&self, gap: f32) -> f32 {
		self.bases.iter().sum::<f32>() + gap * self.bases.len().saturating_sub(1) as f32
	}

	/// Grows `minmax()` tracks toward their limits, expands `fr` tracks into
	/// what is left, and computes the track offsets.
	fn finish(&mut self, available: AvailableSpace, gap: f32) {
		let count = self.bases.len();

		match available {
			// Free space is shared equally, freezing tracks at their limits.
			AvailableSpace::Definite(total) => {
				for _ in 0..=count {
					let free = total - self.used(gap);
					let growable: Vec<usize> = (0..count)
						.filter(|&index| {
							self.limits[index]
								.is_some_and(|limit| limit > self.bases[index] + EPSILON)
						})
						.collect();

					if growable.is_empty() || free <= EPSILON {
						break;
					}

					let share = free / growable.len() as f32;

					for index in growable {
						let limit = self.limits[index].unwrap_or(self.bases[index]);
						self.bases[index] = (self.bases[index] + share).min(limit);
					}
				}
			}
			// Under a max-content constraint every track reaches its limit.
			AvailableSpace::MaxContent => {
				for (base, limit) in self.bases.iter_mut().zip(&self.limits) {
					if let Some(limit) = limit {
						*base = limit.max(*base);
					}
				}
			}
			AvailableSpace::MinContent => {}
		}

		let unit = match available {
			AvailableSpace::Definite(total) => self.fraction_unit(0, count, total, gap),
			AvailableSpace::MaxContent => {
				let minimum_unit = self
					.bases
					.iter()
					.zip(&self.fractions)
					.filter(|(_, factor)| **factor > 0.0)
					.map(|(base, factor)| base / factor.max(1.0))
					.fold(0.0, f32::max);

				self.fraction_contributions
					.iter()
					.map(|item| self.fraction_unit(item.start, item.span, item.size, gap))
					.fold(minimum_unit, f32::max)
			}
			AvailableSpace::MinContent => 0.0,
		};

		for (base, factor) in self.bases.iter_mut().zip(&self.fractions) {
			if *factor > 0.0 {
				*base = base.max(unit * factor);
			}
		}

		// Offsets accumulate along the axis.
		self.total = self.used(gap);
		self.offsets = Vec::with_capacity(count);
		let mut offset = 0.0;

		for base in &self.bases {
			self.offsets.push(offset);
			offset += base + gap;
		}
	}

	/// Finds an `fr` unit across one span, freezing tracks at their base size.
	fn fraction_unit(&self, start: usize, span: usize, space: f32, gap: f32) -> f32 {
		let bases = &self.bases[start..start + span];
		let factors = &self.fractions[start..start + span];
		let mut flexible: Vec<bool> = factors.iter().map(|&factor| factor > 0.0).collect();

		loop {
			let fixed: f32 = bases
				.iter()
				.zip(&flexible)
				.filter(|(_, flexible)| !**flexible)
				.map(|(base, _)| base)
				.sum();
			let factor_sum: f32 = factors
				.iter()
				.zip(&flexible)
				.filter(|(_, flexible)| **flexible)
				.map(|(factor, _)| factor)
				.sum();

			if factor_sum == 0.0 {
				return 0.0;
			}

			let unit = ((space - gap * span.saturating_sub(1) as f32 - fixed)
				/ factor_sum.max(1.0))
			.max(0.0);
			let mut froze = false;

			for (index, flexible) in flexible.iter_mut().enumerate() {
				if *flexible && bases[index] > unit * factors[index] {
					*flexible = false;
					froze = true;
				}
			}

			if !froze {
				return unit;
			}
		}
	}
}

/// Whether a track needs a second pass once an intrinsic axis is settled.
fn contains_percentage(track: &TrackSizing) -> bool {
	match track {
		TrackSizing::Percent(_) => true,
		TrackSizing::MinMax(inner) => {
			contains_percentage(&inner.min) || contains_percentage(&inner.max)
		}
		_ => false,
	}
}

/// A bounded available size does not make an intrinsic grid axis definite.
fn intrinsic_available(definite: Option<f32>, available: AvailableSpace) -> AvailableSpace {
	definite.map_or_else(
		|| {
			match available {
				AvailableSpace::MinContent => AvailableSpace::MinContent,
				AvailableSpace::Definite(_) | AvailableSpace::MaxContent => {
					AvailableSpace::MaxContent
				}
			}
		},
		AvailableSpace::Definite,
	)
}

/// Resolves absolute start/end lines against existing tracks only.
///
/// CSS Grid §9.1 substitutes padding edges for auto or nonexistent lines.
/// Resolve conflicts and spans before that substitution, without reserving
/// tracks: out-of-flow placement must not change the grid's size or shape.
fn absolute_lines(
	placement: GridPlacement,
	explicit: usize,
	tracks: usize,
) -> (Option<usize>, Option<usize>) {
	let line = |value: GridLine| {
		match value {
			GridLine::Line(0) | GridLine::Auto | GridLine::Span(_) => None,
			GridLine::Line(number) => {
				let number = i64::from(number).clamp(-MAX_GRID_LINE, MAX_GRID_LINE);
				Some(if number < 0 {
					explicit as i64 + 1 + number
				} else {
					number - 1
				})
			}
		}
	};
	let span = |value: GridLine| {
		match value {
			GridLine::Span(count) => Some(i64::from(count).clamp(1, MAX_GRID_LINE)),
			GridLine::Line(_) | GridLine::Auto => None,
		}
	};

	let (start, end) = match (line(placement.start), line(placement.end)) {
		(Some(start), Some(end)) if start == end => (Some(start), Some(start + 1)),
		(Some(start), Some(end)) => (Some(start.min(end)), Some(start.max(end))),
		(Some(start), None) => (Some(start), span(placement.end).map(|span| start + span)),
		(None, Some(end)) => (span(placement.start).map(|span| end - span), Some(end)),
		(None, None) => (None, None),
	};

	let existing = |line: Option<i64>| {
		line.and_then(|line| usize::try_from(line).ok())
			.filter(|&line| line <= tracks)
	};
	(existing(start), existing(end))
}

/// Resolves one axis of a placement into a 0-based start track and a span.
///
/// Positive lines count from the start (line 1 is the first edge), negative
/// lines from the last explicit line (line -1). Lines and spans are clamped to
/// [`MAX_GRID_LINE`].
fn resolve_axis(placement: &GridPlacement, explicit: usize) -> (Option<usize>, usize) {
	let explicit = (explicit as i64).min(MAX_GRID_LINE);
	let line = |value: GridLine| -> Option<i64> {
		match value {
			GridLine::Line(number) => {
				let number = i64::from(number).clamp(-MAX_GRID_LINE, MAX_GRID_LINE);
				Some(if number < 0 {
					(explicit + 1 + number).max(0)
				} else {
					(number - 1).max(0)
				})
			}
			GridLine::Auto | GridLine::Span(_) => None,
		}
	};
	let span = |value: GridLine| -> usize {
		match value {
			GridLine::Span(count) => i64::from(count).clamp(1, MAX_GRID_LINE) as usize,
			GridLine::Auto | GridLine::Line(_) => 1,
		}
	};

	match (line(placement.start), line(placement.end)) {
		(Some(start), Some(end)) => {
			// Lines given in reverse order still bound the same area.
			let (low, high) = (start.min(end), start.max(end));
			(Some(low as usize), (high - low).max(1) as usize)
		}
		(Some(start), None) => (Some(start as usize), span(placement.end)),
		(None, Some(end)) => {
			let span = span(placement.start);
			(Some((end - span as i64).max(0) as usize), span)
		}
		(None, None) => (None, span(placement.end).max(span(placement.start))),
	}
}

/// Runs the CSS grid placement algorithm in flow-relative coordinates and
/// returns one area per request, in request order.
///
/// Items fixed on both axes are placed first, then items locked to a major
/// line, then the rest through the auto-placement cursor. Sparse placement
/// never moves the cursor backward; dense placement restarts every search
/// from the start of the grid.
fn place_areas(requests: &[PlacementRequest], dense: bool, minor_tracks: usize) -> Vec<Area> {
	let mut areas = vec![Area::default(); requests.len()];
	let mut occupied: Vec<Area> = Vec::with_capacity(requests.len());
	let is_free = |occupied: &[Area], area: Area| !occupied.iter().any(|&o| o.overlaps(area));
	let spans = |request: &PlacementRequest| {
		Area {
			major_span: request.major_span,
			minor_span: request.minor_span,
			..Area::default()
		}
	};

	for (index, request) in requests.iter().enumerate() {
		if let (Some(major), Some(minor)) = (request.major, request.minor) {
			let area = Area {
				major,
				minor,
				..spans(request)
			};
			occupied.push(area);
			areas[index] = area;
		}
	}

	// Items locked to a major line scan along it; in sparse mode each search
	// starts past the items this step already placed on that line.
	let mut line_ends: Vec<(usize, usize)> = Vec::new();

	for (index, request) in requests.iter().enumerate() {
		let (Some(major), None) = (request.major, request.minor) else {
			continue;
		};
		let line_end = line_ends.iter().position(|&(line, _)| line == major);
		let mut area = Area {
			major,

			minor: match line_end {
				Some(position) if !dense => line_ends[position].1,
				_ => 0,
			},
			..spans(request)
		};

		while !is_free(&occupied, area) {
			area.minor += 1;
		}

		occupied.push(area);
		areas[index] = area;
		let end = area.minor + area.minor_span;

		match line_end {
			Some(position) => line_ends[position].1 = end,
			None => line_ends.push((major, end)),
		}
	}

	let (mut cursor_major, mut cursor_minor) = (0usize, 0usize);

	for (index, request) in requests.iter().enumerate() {
		if request.major.is_some() {
			continue;
		}
		let area = if let Some(minor) = request.minor {
			// A definite minor line: walk down the major axis from the cursor,
			// moving to the next line when the cursor already passed it.
			let mut area = Area {
				major: if dense {
					0
				} else if minor < cursor_minor {
					cursor_major + 1
				} else {
					cursor_major
				},

				minor,
				..spans(request)
			};

			while !is_free(&occupied, area) {
				area.major += 1;
			}

			area
		} else {
			let from = if dense {
				(0, 0)
			} else {
				(cursor_major, cursor_minor)
			};

			find_free_area(&occupied, from, spans(request), minor_tracks)
		};

		occupied.push(area);
		areas[index] = area;

		if !dense {
			cursor_major = area.major;
			cursor_minor = area.minor + area.minor_span;
		}
	}

	areas
}

/// Scans from `from` for the first position where an area of the given spans
/// fits, wrapping to the next major line after `minor_tracks` tracks.
///
/// `usize::MAX` never wraps.
fn find_free_area(
	occupied: &[Area],
	from: (usize, usize),
	spans: Area,
	minor_tracks: usize,
) -> Area {
	let bound = minor_tracks.max(spans.minor_span);
	let mut area = Area {
		major: from.0,
		minor: from.1,
		..spans
	};

	loop {
		while area.minor.saturating_add(area.minor_span) <= bound {
			if !occupied.iter().any(|&o| o.overlaps(area)) {
				return area;
			}

			area.minor += 1;
		}

		area.major += 1;
		area.minor = 0;
	}
}

/// Distributes leftover space along an axis and returns the leading offset.
fn distribute_leftover(axis: &mut AxisTracks, inner: f32, gap: f32, align: Align) -> f32 {
	let count = axis.bases.len();
	let leftover = (inner - axis.used(gap)).max(0.0);

	match align {
		// `normal` (the default) behaves as stretch for grid: leftover space
		// goes to intrinsic tracks, which is how `auto` fills fill the row.
		Align::Auto | Align::Stretch => {
			let intrinsic_count = axis.intrinsic.iter().filter(|&&flag| flag).count();
			if intrinsic_count > 0 {
				let share = leftover / intrinsic_count as f32;
				let mut offset = 0.0;

				for index in 0..count {
					axis.offsets[index] = offset;

					if axis.intrinsic[index] {
						axis.bases[index] += share;
					}

					offset += axis.bases[index] + gap;
				}
			}

			0.0
		}
		Align::FlexEnd => leftover,
		Align::Center => leftover / 2.0,

		Align::SpaceBetween if count > 1 => {
			let spacing = leftover / (count - 1) as f32;
			let mut offset = 0.0;
			for index in 0..count {
				axis.offsets[index] = offset;
				offset += axis.bases[index] + gap + spacing;
			}

			0.0
		}

		Align::SpaceAround => {
			let spacing = leftover / count.max(1) as f32;
			let mut offset = spacing / 2.0;
			for index in 0..count {
				axis.offsets[index] = offset;
				offset += axis.bases[index] + gap + spacing;
			}

			0.0
		}
		Align::SpaceEvenly => {
			let spacing = leftover / (count + 1) as f32;
			let mut offset = spacing;
			for index in 0..count {
				axis.offsets[index] = offset;
				offset += axis.bases[index] + gap + spacing;
			}

			0.0
		}
		_ => 0.0,
	}
}

/// The size spanned by `span` tracks starting at `start`.
fn spanned_size(bases: &[f32], start: usize, span: usize, gap: f32) -> f32 {
	let end = (start + span).min(bases.len());
	bases.get(start..end).map_or(0.0, |tracks| {
		tracks.iter().sum::<f32>() + gap * tracks.len().saturating_sub(1) as f32
	})
}

/// Maps a `justify-content` value onto the shared alignment enum for grid.
fn justify_to_align(justify: crate::JustifyContent) -> Align {
	match justify {
		crate::JustifyContent::FlexStart => Align::FlexStart,
		crate::JustifyContent::FlexEnd => Align::FlexEnd,
		crate::JustifyContent::Center => Align::Center,
		crate::JustifyContent::SpaceBetween => Align::SpaceBetween,
		crate::JustifyContent::SpaceAround => Align::SpaceAround,
		crate::JustifyContent::SpaceEvenly => Align::SpaceEvenly,
	}
}
