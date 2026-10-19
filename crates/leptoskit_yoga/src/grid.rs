//! The CSS grid layout algorithm.
//!
//! Implements track sizing (`auto`, lengths, percentages, `fr`, `minmax`),
//! line-based placement with spans, the auto-placement algorithm in row-major,
//! column-major, sparse, and dense modes, grid gaps, and area alignment
//! (`justify-self`/`align-self` including stretch).

use crate::AvailableSpace;
use crate::Edges;
use crate::NodeId;
use crate::Size;
use crate::Style;
use crate::Yoga;
use crate::apply_aspect_helper as apply_aspect;
use crate::clamp_size;
use crate::resolve_edges;
use crate::style::Align;
use crate::style::Display;
use crate::style::GridAutoFlow;
use crate::style::GridLine;
use crate::style::GridPlacement;
use crate::style::PositionType;
use crate::style::TrackSizing;

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
	/// The sizing function of each track, explicit first, then implicit.
	sizings: Vec<TrackSizing>,
	/// The resolved base size of each track.
	bases: Vec<f32>,
	/// Whether the track still grows from content.
	intrinsic: Vec<bool>,
	/// The final offset of each track's start edge.
	offsets: Vec<f32>,
	/// The total axis length once sized.
	total: f32,
}

impl Yoga {
	pub(crate) fn layout_grid(
		&mut self,
		node: NodeId,
		style: &Style,
		ctx: crate::SizingCtx,
		inner_padding: Edges,
		border: Edges,
	) -> Size<f32> {
		let mut grid = style.grid.clone().unwrap_or_default();
		let children = self.children(node).to_vec();
		let mut items = Vec::new();
		let mut absolute = Vec::new();
		for child in children {
			let child_style = self.style(child).clone();
			if child_style.display == Display::None {
				continue;
			}
			if matches!(
				child_style.position,
				PositionType::Absolute | PositionType::Fixed
			) {
				absolute.push(child);
				continue;
			}
			if child_style.display == Display::Contents {
				continue;
			}
			items.push(GridItem {
				node: child,
				col_start: 0,
				col_span: 1,
				row_start: 0,
				row_span: 1,
				margins: resolve_edges(&child_style.margin, ctx.percent_basis.width),
			});
		}

		let mut columns = AxisTracks {
			sizings: std::mem::take(&mut grid.template_columns),
			..AxisTracks::default()
		};
		let mut rows = AxisTracks {
			sizings: std::mem::take(&mut grid.template_rows),
			..AxisTracks::default()
		};

		// Resolve placements. Items with both lines definite occupy their
		// cells first; the rest flow through the auto-placement cursor.
		let mut occupied: Vec<(usize, usize, usize, usize)> = Vec::new();
		let mut auto_items: Vec<usize> = Vec::new();
		// The loop writes back into `items[index]`, so it iterates by index.
		#[allow(clippy::needless_range_loop)]
		for index in 0..items.len() {
			let child_style = self.style(items[index].node).clone();
			let child_grid = child_style.grid.clone().unwrap_or_default();
			let (col_start, col_span) = resolve_axis(&child_grid.column, columns.sizings.len());
			let (row_start, row_span) = resolve_axis(&child_grid.row, rows.sizings.len());
			if let (Some(col), Some(row)) = (col_start, row_start) {
				items[index].col_start = col;
				items[index].col_span = col_span;
				items[index].row_start = row;
				items[index].row_span = row_span;
				columns.reserve(col + col_span, &grid.auto_columns);
				rows.reserve(row + row_span, &grid.auto_rows);
				occupied.push((row, col, row_span, col_span));
			} else {
				items[index].col_span = col_span;
				items[index].row_span = row_span;
				auto_items.push(index);
			}
		}

		// The auto-placement cursor never moves backward in sparse mode; dense
		// mode restarts the scan from the origin for every item. The cursor
		// advances along the flow axis: right for row flow, down for column
		// flow.
		let explicit_columns = columns.sizings.len();
		let explicit_rows = rows.sizings.len();
		let wrap_bound = if grid.auto_flow.is_row() {
			explicit_columns.max(1)
		} else if explicit_rows == 0 {
			usize::MAX
		} else {
			explicit_rows
		};
		let mut cursor_row = 0usize;
		let mut cursor_col = 0usize;
		for index in auto_items {
			let definite_row = items[index].row_start;
			let definite_col = items[index].col_start;
			let row_span = items[index].row_span;
			let col_span = items[index].col_span;
			let (row, col) = if grid.auto_flow.is_dense() {
				find_free_cell(
					&occupied,
					0,
					0,
					row_span,
					col_span,
					grid.auto_flow,
					wrap_bound,
				)
			} else if definite_row > 0 {
				find_free_cell(
					&occupied,
					definite_row,
					0,
					row_span,
					col_span,
					GridAutoFlow::Row,
					explicit_columns.max(1),
				)
			} else if definite_col > 0 {
				find_free_cell(
					&occupied,
					0,
					definite_col,
					row_span,
					col_span,
					GridAutoFlow::Column,
					usize::MAX,
				)
			} else {
				find_free_cell(
					&occupied,
					cursor_row,
					cursor_col,
					row_span,
					col_span,
					grid.auto_flow,
					wrap_bound,
				)
			};
			items[index].row_start = row;
			items[index].col_start = col;
			occupied.push((row, col, row_span, col_span));
			if !grid.auto_flow.is_dense() {
				if grid.auto_flow.is_row() {
					cursor_row = row;
					cursor_col = col + col_span;
				} else {
					cursor_row = row + row_span;
					cursor_col = col;
				}
			}
		}
		// Reserve tracks for every placement, including band-limited ones.
		for item in &items {
			columns.reserve(item.col_start + item.col_span, &grid.auto_columns);
			rows.reserve(item.row_start + item.row_span, &grid.auto_rows);
		}

		let content_available = Size::new(
			shrink(ctx.available.width, inner_padding.horizontal()),
			shrink(ctx.available.height, inner_padding.vertical()),
		);

		// Size both axes: columns first at max-content, then rows with the
		// column results as the inline constraint.
		let column_gap = style
			.gap
			.left
			.resolve(ctx.percent_basis.width)
			.unwrap_or(0.0);
		let row_gap = style
			.gap
			.top
			.resolve(ctx.percent_basis.width)
			.unwrap_or(0.0);

		size_axis(
			self,
			&mut columns,
			&items,
			true,
			content_available.width,
			column_gap,
		);
		let column_total = columns.total;
		size_axis(
			self,
			&mut rows,
			&items,
			false,
			content_available.height,
			row_gap,
		);
		let _ = column_total;

		// Container size: forced or style size wins, else the tracks decide.
		let mut border_box = Size::new(
			ctx.forced
				.width
				.or_else(|| style.width.resolve(ctx.percent_basis.width))
				.unwrap_or(0.0),
			ctx.forced
				.height
				.or_else(|| style.height.resolve(ctx.percent_basis.height))
				.unwrap_or(0.0),
		);
		if style.width.is_auto() && ctx.forced.width.is_none() {
			border_box.width = columns.total + inner_padding.horizontal();
		}
		if style.height.is_auto() && ctx.forced.height.is_none() {
			border_box.height = rows.total + inner_padding.vertical();
		}
		let mut border_box = clamp_size(style, border_box, ctx.percent_basis);
		border_box = apply_aspect(style, border_box, ctx.percent_basis);
		let inner_width = (border_box.width - inner_padding.horizontal()).max(0.0);
		let inner_height = (border_box.height - inner_padding.vertical()).max(0.0);

		// Leftover space distribution and track offsets.
		let justify_offset = distribute_leftover(
			&mut columns,
			inner_width,
			column_gap,
			justify_to_align(style.justify_content),
		);
		let align_offset =
			distribute_leftover(&mut rows, inner_height, row_gap, style.align_content);

		let mut content_max_width = 0.0f32;
		let mut content_max_height = 0.0f32;
		for item in &items {
			let child_style = self.style(item.node).clone();
			let child_grid = child_style.grid.clone().unwrap_or_default();
			let area_x = columns.offsets.get(item.col_start).copied().unwrap_or(0.0);
			let area_width =
				spanned_size(&columns.bases, item.col_start, item.col_span, column_gap);
			let area_y = rows.offsets.get(item.row_start).copied().unwrap_or(0.0);
			let area_height = spanned_size(&rows.bases, item.row_start, item.row_span, row_gap);

			// `normal` (the `Auto` default) behaves as stretch for grid items.
			let justify = child_grid.justify_self.resolve_self(grid.justify_items);
			let justify = if matches!(justify, Align::Auto) {
				Align::Stretch
			} else {
				justify
			};
			let align = child_style.align_self.resolve_self(style.align_items);
			let stretch_width = matches!(justify, Align::Stretch) && child_style.width.is_auto();
			let stretch_height = matches!(align, Align::Stretch) && child_style.height.is_auto();
			let width = if stretch_width {
				Some((area_width - item.margins.horizontal()).max(0.0))
			} else {
				child_style.width.resolve(Some(area_width))
			};
			let height = if stretch_height {
				Some((area_height - item.margins.vertical()).max(0.0))
			} else {
				child_style.height.resolve(Some(area_height))
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
			let mut size = self.layout_node(item.node, child_ctx);
			size = apply_aspect(
				&child_style,
				size,
				Size::new(Some(area_width), Some(area_height)),
			);

			// Alignment inside the area for non-stretched axes.
			let free_x = (area_width - item.margins.horizontal() - size.width).max(0.0);
			let free_y = (area_height - item.margins.vertical() - size.height).max(0.0);
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

			let x = inner_padding.left + area_x + justify_offset + lead_x + item.margins.left;
			let y = inner_padding.top + area_y + align_offset + lead_y + item.margins.top;
			let layout = &mut self.nodes[item.node.0].layout;
			layout.x = x;
			layout.y = y;
			content_max_width = content_max_width.max(x + size.width);
			content_max_height = content_max_height.max(y + size.height);
		}

		for child in absolute {
			self.layout_absolute(child, border_box, border);
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
}

impl AxisTracks {
	/// Ensures at least `count` tracks exist, appending the implicit sizing.
	fn reserve(&mut self, count: usize, implicit: &TrackSizing) {
		while self.sizings.len() < count {
			self.sizings.push(implicit.clone());
		}
	}
}

/// Resolves one axis of a placement into a start track and a span.
fn resolve_axis(placement: &GridPlacement, explicit: usize) -> (Option<usize>, usize) {
	let line = |value: GridLine| -> Option<i64> {
		match value {
			GridLine::Line(n) => {
				if n < 0 {
					Some((explicit as i64 + i64::from(n)).max(0))
				} else {
					Some(i64::from(n.saturating_sub(1).max(0)))
				}
			}
			GridLine::Auto | GridLine::Span(_) => None,
		}
	};
	let span = |value: GridLine| -> usize {
		match value {
			GridLine::Span(count) => count.max(1) as usize,
			_ => 1,
		}
	};
	match (line(placement.start), line(placement.end)) {
		(Some(start), Some(end)) => {
			let span = (end - start).max(1) as usize;
			(Some(start.max(0) as usize), span)
		}
		(Some(start), None) => {
			let span = span(placement.end);
			(Some(start as usize), span)
		}
		(None, Some(end)) => {
			let span = span(placement.start).max(1);
			(Some((end - span as i64).max(0) as usize), span)
		}
		(None, None) => {
			let span = span(placement.end).max(span(placement.start));
			(None, span)
		}
	}
}

/// Finds the first free cell for an item of the given span.
///
/// `wrap_bound` is the number of tracks along the flow's cross axis at which
/// the cursor wraps to the next line; `usize::MAX` never wraps, which is how
/// column flow behaves with no explicit rows (items keep stacking down the
/// first column, growing implicit rows).
fn find_free_cell(
	occupied: &[(usize, usize, usize, usize)],
	from_row: usize,
	from_col: usize,
	row_span: usize,
	col_span: usize,
	flow: GridAutoFlow,
	wrap_bound: usize,
) -> (usize, usize) {
	let occupied_at = |row: usize, col: usize| {
		occupied
			.iter()
			.any(|&(r, c, rs, cs)| row >= r && row < r + rs && col >= c && col < c + cs)
	};
	let fits = |row: usize, col: usize| {
		(0..row_span).all(|dr| (0..col_span).all(|dc| !occupied_at(row + dr, col + dc)))
	};
	if flow.is_row() {
		let bound = wrap_bound.max(col_span);
		let mut row = from_row;
		loop {
			let mut col = if row == from_row { from_col } else { 0 };
			while col + col_span <= bound {
				if fits(row, col) {
					return (row, col);
				}
				col += 1;
			}
			row += 1;
		}
	} else {
		let bound = wrap_bound.max(row_span);
		let mut col = from_col;
		loop {
			let mut row = if col == from_col { from_row } else { 0 };
			while row + row_span <= bound {
				if fits(row, col) {
					return (row, col);
				}
				row += 1;
			}
			col += 1;
		}
	}
}

/// Sizes one axis of the grid.
fn size_axis(
	tree: &mut Yoga,
	axis: &mut AxisTracks,
	items: &[GridItem],
	columns: bool,
	available: AvailableSpace,
	gap: f32,
) {
	let count = axis.sizings.len();
	axis.bases = vec![0.0; count];
	axis.intrinsic = vec![false; count];
	let container = available.definite();

	// Base sizes from the track sizing functions.
	for index in 0..count {
		let sizing = axis.sizings[index].clone();
		match sizing {
			TrackSizing::Points(value) => axis.bases[index] = value.max(0.0),
			TrackSizing::Percent(fraction) => {
				axis.bases[index] = container.map_or(0.0, |total| (fraction * total).max(0.0));
			}
			TrackSizing::MinMax(inner) => {
				axis.bases[index] = inner.min.resolve_base(container).unwrap_or(0.0).max(0.0);
				axis.intrinsic[index] = matches!(inner.max, TrackSizing::Auto)
					|| matches!(inner.max, TrackSizing::Fraction(_))
					|| inner.max.resolve_base(container).is_none();
			}
			TrackSizing::Auto => {
				axis.bases[index] = 0.0;
				axis.intrinsic[index] = true;
			}
			TrackSizing::Fraction(_) => {
				// `fr` tracks are resolved against leftover space below, not
				// against content.
				axis.bases[index] = 0.0;
				axis.intrinsic[index] = false;
			}
		}
	}

	// Content contributions grow intrinsic tracks.
	for item in items {
		let (start, span) = if columns {
			(item.col_start, item.col_span)
		} else {
			(item.row_start, item.row_span)
		};
		let style = tree.style(item.node).clone();
		let contribution = if columns {
			style.width.resolve(None).unwrap_or_else(|| {
				tree.content_size(
					item.node,
					Size {
						width: AvailableSpace::MaxContent,
						height: AvailableSpace::MaxContent,
					},
				)
				.width
			}) + item.margins.horizontal()
		} else {
			style.height.resolve(None).unwrap_or_else(|| {
				tree.content_size(
					item.node,
					Size {
						width: AvailableSpace::MaxContent,
						height: AvailableSpace::MaxContent,
					},
				)
				.height
			}) + item.margins.vertical()
		};
		if span == 1 {
			if axis.intrinsic[start] {
				axis.bases[start] = axis.bases[start].max(contribution);
			}
		} else {
			// Distribute the contribution across the spanned intrinsic tracks
			// after accounting for definite tracks and gaps.
			let definite: f32 = (0..span)
				.filter(|&offset| !axis.intrinsic[start + offset])
				.map(|offset| axis.bases[start + offset])
				.sum();
			let intrinsic_count = (0..span)
				.filter(|&offset| axis.intrinsic[start + offset])
				.count();
			if intrinsic_count > 0 {
				let needed = (contribution - definite - gap * (span - 1) as f32).max(0.0);
				let share = needed / intrinsic_count as f32;
				for offset in 0..span {
					if axis.intrinsic[start + offset] {
						axis.bases[start + offset] += share;
					}
				}
			}
		}
	}

	// Distribute leftover space to `fr` tracks.
	if let Some(total) = container {
		let used: f32 = axis.bases.iter().sum::<f32>() + gap * count.saturating_sub(1) as f32;
		let leftover = (total - used).max(0.0);
		let total_fr: f32 = axis
			.sizings
			.iter()
			.filter(|sizing| matches!(sizing, TrackSizing::Fraction(_)))
			.map(|sizing| {
				match *sizing {
					TrackSizing::Fraction(fr) => fr,
					_ => 0.0,
				}
			})
			.sum();
		if total_fr > 0.0 {
			for index in 0..count {
				if let TrackSizing::Fraction(fr) = axis.sizings[index] {
					let share = leftover * (fr / total_fr);
					axis.bases[index] = axis.bases[index].max(share);
				}
			}
		}
	}

	// Offsets accumulate along the axis.
	let total: f32 = axis.bases.iter().sum::<f32>() + gap * count.saturating_sub(1) as f32;
	axis.total = total;
	axis.offsets = Vec::with_capacity(count);
	let mut offset = 0.0;
	for index in 0..count {
		axis.offsets.push(offset);
		offset += axis.bases[index] + gap;
	}
}

/// Distributes leftover space along an axis and returns the leading offset.
fn distribute_leftover(axis: &mut AxisTracks, inner: f32, gap: f32, align: Align) -> f32 {
	let count = axis.bases.len();
	let used: f32 = axis.bases.iter().sum::<f32>() + gap * count.saturating_sub(1) as f32;
	let leftover = (inner - used).max(0.0);
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

fn shrink(space: AvailableSpace, subtract: f32) -> AvailableSpace {
	match space {
		AvailableSpace::Definite(value) => AvailableSpace::Definite((value - subtract).max(0.0)),
		other => other,
	}
}
