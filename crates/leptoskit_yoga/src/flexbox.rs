//! The flexbox layout algorithm.
//!
//! Follows CSS Flexible Box Layout Level 1: hypothetical main sizes, the
//! "resolve flexible lengths" freeze loop, line breaking under `flex_wrap`,
//! cross-axis stretching, and `justify`/`align` distribution including auto
//! margins. Absolute children are positioned against the parent's padding box.

use crate::AvailableSpace;
use crate::Direction;
use crate::Display;
use crate::Edges;
use crate::Layout;
use crate::NodeId;
use crate::PositionType;
use crate::Rect;
use crate::RunMode;
use crate::Size;
use crate::Style;
use crate::Yoga;
use crate::apply_aspect_helper as apply_aspect;
use crate::bounded_by_max;
use crate::clamp_size;
use crate::content_available;
use crate::definite_border_box;
use crate::inner_definite;
use crate::resolve_edges;
use crate::style::Align;
use crate::style::Dimension;
use crate::style::FlexWrap;
use crate::style::JustifyContent;

/// A child tracked through the flexible-lengths algorithm.
struct FlexItem {
	node: NodeId,
	/// The margin-inclusive flex base size before flexibility.
	outer_base: f32,
	/// The margin-exclusive base size currently being resolved.
	inner_base: f32,
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
	/// The resolved minimum main size.
	min_main: f32,
	/// The resolved maximum main size.
	max_main: f32,
}

/// A wrapped line of items.
struct FlexLine {
	items: Vec<usize>,
	/// The line's cross size once measured.
	cross: f32,
	/// The line's cross offset inside the container's content box.
	cross_offset: f32,
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
		let padding = resolve_edges(&style.padding, basis.width);
		let border = Edges {
			top: style.border.top.max(0.0),
			right: style.border.right.max(0.0),
			bottom: style.border.bottom.max(0.0),
			left: style.border.left.max(0.0),
		};
		let inner_padding = Edges {
			top: padding.top + border.top,
			right: padding.right + border.right,
			bottom: padding.bottom + border.bottom,
			left: padding.left + border.left,
		};

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

	fn layout_flex(
		&mut self,
		node: NodeId,
		style: &Style,
		ctx: crate::SizingCtx,
		inner_padding: Edges,
		border: Edges,
		mode: RunMode,
	) -> Size<f32> {
		let is_row = style.flex_direction.is_row();
		let rtl = style.direction == Direction::Rtl;
		// The main axis runs against the writing direction when reversed or,
		// for rows, under RTL.
		let main_reversed = style.flex_direction.is_reverse() || (is_row && rtl);

		let children = self.children(node).to_vec();
		let mut flow: Vec<NodeId> = Vec::with_capacity(children.len());
		let mut absolute: Vec<NodeId> = Vec::new();
		// Flatten `display: contents` boxes so their children become direct
		// flex items, preserving document order.
		let mut stack: Vec<NodeId> = children.into_iter().rev().collect();
		while let Some(child) = stack.pop() {
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
				stack.extend(self.children(child).iter().rev().copied());
				continue;
			}
			flow.push(child);
		}

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
			let measured = self.measure(node, inner_available);
			let mut size = Size::new(
				definite_border
					.width
					.unwrap_or(measured.width + inner_padding.horizontal()),
				definite_border
					.height
					.unwrap_or(measured.height + inner_padding.vertical()),
			);
			size = clamp_size(style, size, ctx.percent_basis);
			size = apply_aspect(style, size, ctx.percent_basis);
			let layout = &mut self.nodes[node.0].layout;
			layout.width = size.width;
			layout.height = size.height;
			layout.content_width = size.width;
			layout.content_height = size.height;
			return size;
		}

		// The space items flex into: the content box, already net of padding
		// and border.
		let (inner_main_available, inner_cross_available) = if is_row {
			(inner_available.width, inner_available.height)
		} else {
			(inner_available.height, inner_available.width)
		};
		let definite_inner_main = inner_main_available.definite();

		// Build the flex item list with hypothetical main sizes.
		let mut items: Vec<FlexItem> = Vec::with_capacity(flow.len());
		for child in &flow {
			let child_style = self.style(*child).clone();
			let margins = resolve_edges(&child_style.margin, child_basis.width);
			let child_main_basis = if is_row {
				child_basis.width
			} else {
				child_basis.height
			};
			let flex_basis = if !child_style.flex_basis.is_auto() {
				child_style.flex_basis
			} else if is_row {
				child_style.width
			} else {
				child_style.height
			};
			let style_main = if is_row {
				child_style.width
			} else {
				child_style.height
			};
			let mut base = flex_basis
				.resolve(child_main_basis)
				.or_else(|| style_main.resolve(child_main_basis));
			if base.is_none() {
				let available =
					shrink_available(inner_main_available, main_margins(&margins, is_row));
				let cross_space = shrink_available(inner_cross_available, margins.horizontal());
				// An item a column will stretch already has its width, as in
				// CSS: it is the column's width, so percentages inside the
				// item resolve against it while its height is measured.
				let stretched_width = (!is_row
					&& child_style.width.is_auto()
					&& child_style.align_self.resolve_self(style.align_items) == Align::Stretch
					&& child_style.margin.left != Dimension::Auto
					&& child_style.margin.right != Dimension::Auto)
					.then(|| cross_space.definite())
					.flatten();
				let measured = self.layout_node(
					*child,
					crate::SizingCtx {
						available: if is_row {
							Size::new(available, AvailableSpace::MaxContent)
						} else {
							// A column measures its items within its own
							// width, so text wraps at the column's width and
							// the item's height is the wrapped height, not a
							// single line's.
							Size::new(cross_space, available)
						},
						forced: Size::new(stretched_width, None),
						percent_basis: child_basis,
					},
					RunMode::Measure,
				);
				base = Some(if is_row {
					measured.width
				} else {
					measured.height
				});
			}
			let (min_main, max_main) = if is_row {
				(
					child_style
						.min_width
						.resolve(child_main_basis)
						.unwrap_or(0.0),
					child_style
						.max_width
						.resolve(child_main_basis)
						.unwrap_or(f32::INFINITY),
				)
			} else {
				(
					child_style
						.min_height
						.resolve(child_main_basis)
						.unwrap_or(0.0),
					child_style
						.max_height
						.resolve(child_main_basis)
						.unwrap_or(f32::INFINITY),
				)
			};
			let inner = base.unwrap_or(0.0).clamp(min_main, max_main.max(min_main));
			let outer = inner + main_margins(&margins, is_row);
			items.push(FlexItem {
				node: *child,
				outer_base: outer,
				inner_base: inner,
				inner_main: inner,
				inner_cross: 0.0,
				margins,
				frozen: child_style.flex_grow <= 0.0 && child_style.flex_shrink <= 0.0,
				grow: child_style.flex_grow,
				shrink: child_style.flex_shrink,
				min_main,
				max_main,
			});
		}

		// Percentage gaps resolve against the matching axis of the content box.
		let row_gap = style.gap.top.resolve(child_basis.height).unwrap_or(0.0);
		let column_gap = style.gap.left.resolve(child_basis.width).unwrap_or(0.0);
		let line_gap = if is_row { row_gap } else { column_gap };
		let item_gap = if is_row { column_gap } else { row_gap };

		// Break items into lines. Wrapping requires a definite main axis; an
		// item wider than a fresh line stays on that line alone.
		let mut lines: Vec<FlexLine> = vec![FlexLine {
			items: Vec::new(),
			cross: 0.0,
			cross_offset: 0.0,
		}];
		let wraps = style.flex_wrap != FlexWrap::NoWrap;
		let mut used_on_line = 0.0f32;
		let mut count_on_line = 0usize;
		for (index, item) in items.iter().enumerate() {
			let gap_before = if count_on_line > 0 { item_gap } else { 0.0 };
			let fits = !wraps
				|| count_on_line == 0
				|| !matches!(definite_inner_main, Some(limit)
					if used_on_line + gap_before + item.outer_base > limit + EPSILON);
			if !fits {
				lines.push(FlexLine {
					items: Vec::new(),
					cross: 0.0,
					cross_offset: 0.0,
				});
				used_on_line = 0.0;
				count_on_line = 0;
			}
			used_on_line += gap_before + items[index].outer_base;
			count_on_line += 1;
			lines
				.last_mut()
				.expect("a line always exists")
				.items
				.push(index);
		}

		// Resolve flexible lengths per line.
		for line in &mut lines {
			let gaps = item_gap * line.items.len().saturating_sub(1) as f32;
			for _ in 0..=line.items.len() {
				let Some(limit) = definite_inner_main else {
					break;
				};
				let total: f32 = line
					.items
					.iter()
					.map(|&index| {
						items[index].inner_base + main_margins(&items[index].margins, is_row)
					})
					.sum();
				let free_space = limit - gaps - total;
				if free_space.abs() <= EPSILON {
					break;
				}
				let mut any_violation = false;
				if free_space > 0.0 {
					let total_grow: f32 = line
						.items
						.iter()
						.filter(|&&index| !items[index].frozen && items[index].grow > 0.0)
						.map(|&index| items[index].grow)
						.sum();
					if total_grow <= 0.0 {
						break;
					}
					for &index in &line.items {
						if !items[index].frozen && items[index].grow > 0.0 {
							let grown = items[index].inner_base
								+ free_space * (items[index].grow / total_grow);
							let clamped = grown.clamp(items[index].min_main, items[index].max_main);
							if (clamped - grown).abs() > EPSILON {
								any_violation = true;
								items[index].frozen = true;
							}
							items[index].inner_base = clamped;
						}
					}
				} else {
					let total_scaled: f32 = line
						.items
						.iter()
						.filter(|&&index| !items[index].frozen && items[index].shrink > 0.0)
						.map(|&index| items[index].shrink * items[index].outer_base)
						.sum();
					if total_scaled <= 0.0 {
						break;
					}
					for &index in &line.items {
						if !items[index].frozen && items[index].shrink > 0.0 {
							let scaled = items[index].shrink * items[index].outer_base;
							let shrunk =
								items[index].inner_base + free_space * (scaled / total_scaled);
							let clamped =
								shrunk.clamp(items[index].min_main, items[index].max_main);
							if (clamped - shrunk).abs() > EPSILON {
								any_violation = true;
								items[index].frozen = true;
							}
							items[index].inner_base = clamped;
						}
					}
				}
				if !any_violation {
					break;
				}
			}
			for &index in &line.items {
				items[index].inner_main = items[index].inner_base;
				items[index].frozen = true;
			}
		}

		// Cross sizes before stretching: style size, else content size at the
		// resolved main size.
		for line in &mut lines {
			let mut line_cross = 0.0f32;
			for &index in &line.items {
				let (node, inner_main, margins) = {
					let item = &items[index];
					(item.node, item.inner_main, item.margins)
				};
				let child_style = self.style(node).clone();
				let (style_cross, cross_basis) = if is_row {
					(child_style.height, child_basis.height)
				} else {
					(child_style.width, child_basis.width)
				};
				// An auto cross size with an aspect ratio is transferred from
				// the resolved main size, as CSS sizes a ratio-bound box.
				let transferred = child_style.aspect_ratio.map(|ratio| {
					if is_row {
						inner_main / ratio
					} else {
						inner_main * ratio
					}
				});
				let cross = style_cross
					.resolve(cross_basis)
					.or(transferred)
					.unwrap_or_else(|| {
						let available = if is_row {
							Size::new(AvailableSpace::Definite(inner_main), inner_cross_available)
						} else {
							Size::new(inner_cross_available, AvailableSpace::Definite(inner_main))
						};
						let measured = self.content_size(node, available, child_basis);
						if is_row {
							measured.height
						} else {
							measured.width
						}
					});
				items[index].inner_cross = cross;
				let outer_cross = cross + cross_margins(&margins, is_row);
				line_cross = line_cross.max(outer_cross);
			}
			line.cross = line_cross;
		}

		// Container sizes: forced (from the parent algorithm) and style sizes
		// win; otherwise the content drives the axis.
		let main_borders = if is_row {
			inner_padding.horizontal()
		} else {
			inner_padding.vertical()
		};
		let cross_borders = if is_row {
			inner_padding.vertical()
		} else {
			inner_padding.horizontal()
		};
		let lines_cross_total: f32 =
			lines.iter().map(|line| line.cross).sum::<f32>() + line_gap * (lines.len() - 1) as f32;
		// The main axis content size: the widest line.
		let mut lines_main_total = 0.0f32;
		for line in &lines {
			let total: f32 = line
				.items
				.iter()
				.map(|&index| items[index].inner_main + main_margins(&items[index].margins, is_row))
				.sum::<f32>()
				+ item_gap * line.items.len().saturating_sub(1) as f32;
			lines_main_total = lines_main_total.max(total);
		}
		// The cross axis content size when items do not wrap: the widest item.
		let mut items_cross_total = 0.0f32;
		for item in &items {
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

		let mut border_box = clamp_size(style, border_box, ctx.percent_basis);
		border_box = apply_aspect(style, border_box, ctx.percent_basis);

		let container_inner_main = if is_row {
			(border_box.width - main_borders).max(0.0)
		} else {
			(border_box.height - main_borders).max(0.0)
		};
		let container_inner_cross = if is_row {
			(border_box.height - cross_borders).max(0.0)
		} else {
			(border_box.width - cross_borders).max(0.0)
		};

		if wraps {
			// Wrap-reverse stacks the lines in the opposite cross order.
			if style.flex_wrap == FlexWrap::WrapReverse {
				lines.reverse();
			}
			// Align the lines within the cross axis.
			let leftover_cross = container_inner_cross
				- (lines.iter().map(|line| line.cross).sum::<f32>()
					+ line_gap * lines.len().saturating_sub(1) as f32);
			place_lines(&mut lines, leftover_cross, line_gap, style.align_content);
		} else {
			// A single-line container's line is exactly its inner cross size,
			// and `align_content` does not apply.
			let line = lines.first_mut().expect("a line always exists");
			line.cross = container_inner_cross;
			line.cross_offset = 0.0;
		}

		let mut content_max_width = 0.0f32;
		let mut content_max_height = 0.0f32;
		let content_basis = if is_row {
			Size::new(Some(container_inner_main), Some(container_inner_cross))
		} else {
			Size::new(Some(container_inner_cross), Some(container_inner_main))
		};

		for line in &lines {
			let total_outer: f32 = line
				.items
				.iter()
				.map(|&index| items[index].inner_main + main_margins(&items[index].margins, is_row))
				.sum::<f32>()
				+ item_gap * line.items.len().saturating_sub(1) as f32;
			let free_main = container_inner_main - total_outer;

			// Auto margins absorb positive free space before `justify` runs.
			let mut auto_margins = 0usize;
			for &index in &line.items {
				auto_margins += auto_margin_count(&self.style(items[index].node).margin, is_row);
			}
			let auto_margin_space = if auto_margins > 0 && free_main > 0.0 {
				free_main / auto_margins as f32
			} else {
				0.0
			};

			let count = line.items.len();
			let justify = if auto_margins > 0 {
				JustifyContent::FlexStart
			} else {
				style.justify_content
			};
			let mut cursor = match justify {
				JustifyContent::FlexEnd => free_main.max(0.0),
				JustifyContent::Center => free_main.max(0.0) / 2.0,
				JustifyContent::SpaceAround => free_main.max(0.0) / count.max(1) as f32 / 2.0,
				JustifyContent::SpaceEvenly => free_main.max(0.0) / (count + 1) as f32,
				_ => 0.0,
			};
			let spacing = match justify {
				JustifyContent::SpaceBetween if count > 1 => {
					free_main.max(0.0) / (count - 1) as f32
				}
				JustifyContent::SpaceAround => free_main.max(0.0) / count.max(1) as f32,
				JustifyContent::SpaceEvenly => free_main.max(0.0) / (count + 1) as f32,
				_ => 0.0,
			};

			for &index in &line.items {
				let item = &items[index];
				let child_style = self.style(item.node).clone();
				// Margins are physical; on a reversed axis the main-start
				// margin is the right (or bottom) one.
				let (leading_margin, trailing_margin) =
					main_start_end(item.margins.as_rect(), is_row, main_reversed);
				let margin_cross_lead = if is_row {
					item.margins.top
				} else {
					item.margins.left
				};

				let align = child_style.align_self.resolve_self(style.align_items);
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
				// A stretched item fills the line; a ratio-bound item keeps the
				// cross size transferred from its main size.
				let forced_cross = (stretch || (cross_auto && child_style.aspect_ratio.is_some()))
					.then_some(final_cross);

				let child_ctx = crate::SizingCtx {
					available: if is_row {
						Size::new(
							AvailableSpace::Definite(item.inner_main),
							AvailableSpace::Definite(container_inner_cross),
						)
					} else {
						Size::new(
							AvailableSpace::Definite(container_inner_cross),
							AvailableSpace::Definite(item.inner_main),
						)
					},
					forced: if is_row {
						Size::new(Some(item.inner_main), forced_cross)
					} else {
						Size::new(forced_cross, Some(item.inner_main))
					},
					percent_basis: content_basis,
				};
				let final_size = self.layout_node(item.node, child_ctx, mode);
				let final_size = apply_aspect(&child_style, final_size, content_basis);

				let item_main = if is_row {
					final_size.width
				} else {
					final_size.height
				};

				// Relative offsets (`top`/`left` on in-flow items) shift the
				// item away from its laid-out position.
				let relative_main = if is_row {
					child_style
						.inset
						.left
						.resolve(Some(container_inner_main))
						.unwrap_or(0.0)
						- child_style
							.inset
							.right
							.resolve(Some(container_inner_main))
							.unwrap_or(0.0)
				} else {
					child_style
						.inset
						.top
						.resolve(Some(container_inner_main))
						.unwrap_or(0.0)
						- child_style
							.inset
							.bottom
							.resolve(Some(container_inner_main))
							.unwrap_or(0.0)
				};
				let relative_cross = if is_row {
					child_style
						.inset
						.top
						.resolve(Some(container_inner_cross))
						.unwrap_or(0.0)
						- child_style
							.inset
							.bottom
							.resolve(Some(container_inner_cross))
							.unwrap_or(0.0)
				} else {
					child_style
						.inset
						.left
						.resolve(Some(container_inner_cross))
						.unwrap_or(0.0)
						- child_style
							.inset
							.right
							.resolve(Some(container_inner_cross))
							.unwrap_or(0.0)
				};

				let lead = cursor
					+ leading_margin
					+ auto_margin_space
						* auto_margin_before_count(&child_style.margin, is_row, main_reversed)
							as f32;
				let cross_position = line.cross_offset + cross_lead + margin_cross_lead;

				let (x, y) = if is_row {
					let x = if main_reversed {
						border_box.width - inner_padding.right - lead - item_main
					} else {
						inner_padding.left + lead
					};
					(
						x + relative_main,
						inner_padding.top + cross_position + relative_cross,
					)
				} else {
					let y = if main_reversed {
						border_box.height - inner_padding.bottom - lead - item_main
					} else {
						inner_padding.top + lead
					};
					(
						inner_padding.left + cross_position + relative_cross,
						y + relative_main,
					)
				};

				let layout = &mut self.nodes[item.node.0].layout;
				layout.order = index as u32;
				layout.x = x;
				layout.y = y;
				content_max_width = content_max_width.max(x + final_size.width);
				content_max_height = content_max_height.max(y + final_size.height);

				cursor += spacing
					+ item_gap
					+ leading_margin
					+ item_main
					+ trailing_margin
					+ auto_margin_space * auto_margin_count(&child_style.margin, is_row) as f32;
			}
		}

		// Out-of-flow children against this container's padding box.
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

	/// Positions an out-of-flow child against the container's padding box.
	pub(crate) fn layout_absolute(
		&mut self,
		child: NodeId,
		parent_box: Size<f32>,
		border: Edges,
		mode: RunMode,
	) {
		let style = self.style(child).clone();
		// The containing block is the padding box: the border box minus the
		// borders themselves.
		let padding_box = Size::new(
			(parent_box.width - border.horizontal()).max(0.0),
			(parent_box.height - border.vertical()).max(0.0),
		);
		let margins = resolve_edges(&style.margin, Some(padding_box.width));

		let left = style.inset.left.resolve(Some(padding_box.width));
		let right = style.inset.right.resolve(Some(padding_box.width));
		let top = style.inset.top.resolve(Some(padding_box.height));
		let bottom = style.inset.bottom.resolve(Some(padding_box.height));

		let width = left
			.zip(right)
			.map(|(left, right)| (padding_box.width - left - right - margins.horizontal()).max(0.0))
			.or_else(|| style.width.resolve(Some(padding_box.width)));
		let height = top
			.zip(bottom)
			.map(|(top, bottom)| (padding_box.height - top - bottom - margins.vertical()).max(0.0))
			.or_else(|| style.height.resolve(Some(padding_box.height)));

		let child_ctx = crate::SizingCtx {
			available: Size::new(
				width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
				height.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
			),
			forced: Size::new(width, height),
			percent_basis: Size::new(Some(padding_box.width), Some(padding_box.height)),
		};
		let mut size = self.layout_node(child, child_ctx, mode);
		size = apply_aspect(
			&style,
			size,
			Size::new(Some(padding_box.width), Some(padding_box.height)),
		);

		let x = if let Some(left) = left {
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
