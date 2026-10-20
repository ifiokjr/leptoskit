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
use crate::Edges;
use crate::NodeId;
use crate::RunMode;
use crate::Size;
use crate::Style;
use crate::Yoga;
use crate::apply_aspect_helper as apply_aspect;
use crate::bounded_by_max;
use crate::clamp_height;
use crate::clamp_width;
use crate::content_available;
use crate::definite_border_box;
use crate::inner_definite;
use crate::resolve_edges;
use crate::style::Align;
use crate::style::Display;
use crate::style::GridLine;
use crate::style::GridPlacement;
use crate::style::PositionType;
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
	/// The sizing function of each track, explicit first, then implicit.
	sizings: Vec<TrackSizing>,
	/// The resolved base size of each track.
	bases: Vec<f32>,
	/// Whether the track still grows from content.
	intrinsic: Vec<bool>,
	/// The definite maximum of a `minmax()` track, which free space grows the
	/// track toward.
	limits: Vec<Option<f32>>,
	/// The final offset of each track's start edge.
	offsets: Vec<f32>,
	/// The total axis length once sized.
	total: f32,
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

impl Yoga {
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
		let definite_border = definite_border_box(style, &ctx);
		let child_basis = inner_definite(definite_border, inner_padding);
		let inner_available = content_available(
			child_basis,
			bounded_by_max(style, ctx.available, ctx.percent_basis),
			inner_padding,
		);

		// Flatten `display: contents` boxes so their children become direct
		// grid items, preserving document order.
		let mut items = Vec::new();
		let mut absolute = Vec::new();
		let mut stack: Vec<NodeId> = self.children(node).iter().rev().copied().collect();
		while let Some(child) = stack.pop() {
			let child_style = self.style(child);
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
				stack.extend(self.children(child).iter().rev().copied());
				continue;
			}
			items.push(GridItem {
				node: child,
				col_start: 0,
				col_span: 1,
				row_start: 0,
				row_span: 1,
				margins: resolve_edges(&child_style.margin, child_basis.width),
			});
		}

		self.place_grid_items(&mut items, &grid);

		let mut columns = AxisTracks {
			sizings: std::mem::take(&mut grid.template_columns),
			..AxisTracks::default()
		};
		let mut rows = AxisTracks {
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

		// Columns: content contributions at max-content.
		columns.init_bases(inner_available.width.definite());
		for item in &items {
			if !columns.spans_intrinsic(item.col_start, item.col_span) {
				continue;
			}
			let contribution = self
				.style(item.node)
				.width
				.resolve(None)
				.unwrap_or_else(|| {
					self.content_size(
						item.node,
						Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
						Size::new(None, None),
					)
					.width
				})
				+ item.margins.horizontal();
			columns.contribute(item.col_start, item.col_span, contribution, column_gap);
		}
		columns.finish(inner_available.width, column_gap);

		// The container width, and with it every column, is settled before
		// rows are sized.
		let ratio = style.aspect_ratio;
		let border_width = definite_border
			.width
			.or_else(|| {
				ratio
					.zip(definite_border.height)
					.map(|(ratio, height)| height * ratio)
			})
			.unwrap_or(columns.total + inner_padding.horizontal());
		let border_width = clamp_width(style, border_width, ctx.percent_basis.width);
		let inner_width = (border_width - inner_padding.horizontal()).max(0.0);
		let justify_offset = distribute_leftover(
			&mut columns,
			inner_width,
			column_gap,
			justify_to_align(style.justify_content),
		);

		// Rows: content contributions at the width of the spanned columns.
		rows.init_bases(inner_available.height.definite());
		for item in &items {
			if !rows.spans_intrinsic(item.row_start, item.row_span) {
				continue;
			}
			let area_width =
				spanned_size(&columns.bases, item.col_start, item.col_span, column_gap);
			let contribution = self
				.style(item.node)
				.height
				.resolve(None)
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
		rows.finish(inner_available.height, row_gap);

		let border_height = definite_border
			.height
			.or_else(|| {
				ratio
					.zip(definite_border.width)
					.map(|(ratio, width)| width / ratio)
			})
			.unwrap_or(rows.total + inner_padding.vertical());
		let border_height = clamp_height(style, border_height, ctx.percent_basis.height);
		let inner_height = (border_height - inner_padding.vertical()).max(0.0);
		let align_offset =
			distribute_leftover(&mut rows, inner_height, row_gap, style.align_content);
		let border_box = Size::new(border_width, border_height);

		let mut content_max_width = 0.0f32;
		let mut content_max_height = 0.0f32;
		for (index, item) in items.iter().enumerate() {
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
			let mut size = self.layout_node(item.node, child_ctx, mode);
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
			layout.order = index as u32;
			layout.x = x;
			layout.y = y;
			content_max_width = content_max_width.max(x + size.width);
			content_max_height = content_max_height.max(y + size.height);
		}

		for child in absolute {
			self.layout_absolute(child, border_box, border, mode);
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
		for (index, sizing) in self.sizings.iter().enumerate() {
			match sizing {
				TrackSizing::Points(value) => self.bases[index] = value.max(0.0),
				TrackSizing::Percent(fraction) => {
					self.bases[index] = container.map_or(0.0, |total| (fraction * total).max(0.0));
				}
				TrackSizing::MinMax(inner) => {
					let base = inner.min.resolve_base(container).unwrap_or(0.0).max(0.0);
					self.bases[index] = base;
					// A definite maximum is a growth limit; an `auto` or `fr`
					// maximum leaves the track sized by content.
					match inner.max.resolve_base(container) {
						Some(max) => self.limits[index] = Some(max.max(base)),
						None => self.intrinsic[index] = true,
					}
				}
				TrackSizing::Auto => self.intrinsic[index] = true,
				// `fr` tracks are resolved against leftover space in
				// `finish`, not against content.
				TrackSizing::Fraction(_) => {}
			}
		}
	}

	/// Whether any track in the span still grows from content.
	fn spans_intrinsic(&self, start: usize, span: usize) -> bool {
		self.intrinsic[start..start + span].iter().any(|&flag| flag)
	}

	/// Grows the spanned intrinsic tracks to fit an item's contribution.
	fn contribute(&mut self, start: usize, span: usize, contribution: f32, gap: f32) {
		if span == 1 {
			self.bases[start] = self.bases[start].max(contribution);
			return;
		}
		// Distribute the contribution across the spanned intrinsic tracks
		// after accounting for definite tracks and gaps.
		let tracks = start..start + span;
		let definite: f32 = tracks
			.clone()
			.filter(|&index| !self.intrinsic[index])
			.map(|index| self.bases[index])
			.sum();
		let intrinsic_count = tracks
			.clone()
			.filter(|&index| self.intrinsic[index])
			.count();
		let needed = (contribution - definite - gap * (span - 1) as f32).max(0.0);
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

		// Distribute leftover space to `fr` tracks.
		if let AvailableSpace::Definite(total) = available {
			let leftover = (total - self.used(gap)).max(0.0);
			let total_fr: f32 = self
				.sizings
				.iter()
				.map(|sizing| {
					match *sizing {
						TrackSizing::Fraction(fr) => fr,
						_ => 0.0,
					}
				})
				.sum();
			if total_fr > 0.0 {
				for index in 0..count {
					if let TrackSizing::Fraction(fr) = self.sizings[index] {
						let share = leftover * (fr / total_fr);
						self.bases[index] = self.bases[index].max(share);
					}
				}
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
