use std::fmt::Write;

use leptoskit_yoga::Align;
use leptoskit_yoga::BoxSizing;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::GridAutoFlow;
use leptoskit_yoga::GridLine;
use leptoskit_yoga::GridPlacement;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::MinMax;
use leptoskit_yoga::Overflow;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;
use leptoskit_yoga::Style;
use leptoskit_yoga::TrackSizing;

use super::StyleContext;
use super::keyword;
use super::value::Length;
use super::value::safe_inset;
use crate::Color;
use crate::FontWeight;
use crate::NativeProps;
use crate::TextAlign;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SizeField {
	Width,
	Height,
	MinWidth,
	MinHeight,
	MaxWidth,
	MaxHeight,
	Basis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EdgeField {
	Padding,
	Margin,
	Inset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Edge {
	Top,
	Right,
	Bottom,
	Left,
}

impl Edge {
	fn of<T: Copy>(self, rect: Rect<T>) -> T {
		match self {
			Self::Top => rect.top,
			Self::Right => rect.right,
			Self::Bottom => rect.bottom,
			Self::Left => rect.left,
		}
	}

	fn of_mut<T>(self, rect: &mut Rect<T>) -> &mut T {
		match self {
			Self::Top => &mut rect.top,
			Self::Right => &mut rect.right,
			Self::Bottom => &mut rect.bottom,
			Self::Left => &mut rect.left,
		}
	}

	fn css(self) -> &'static str {
		match self {
			Self::Top => "top",
			Self::Right => "right",
			Self::Bottom => "bottom",
			Self::Left => "left",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Axis {
	X,
	Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AlignField {
	Items,
	SelfItem,
	Content,
	JustifyItems,
	JustifySelf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ColorField {
	Text,
	Background,
	Border,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Track {
	Auto,
	Length(Length),
	Fraction(f32),
	MinMax(Box<Self>, Box<Self>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Declaration {
	Size(SizeField, Length),
	Edge(EdgeField, Edge, Length),
	/// Padding of the host's safe-area inset on `Edge` plus an offset.
	SafePadding(Edge, Length),
	Gap(Axis, Length),
	Display(Display),
	Position(PositionType),
	BoxSizing(BoxSizing),
	Overflow(Overflow),
	FlexDirection(FlexDirection),
	FlexWrap(FlexWrap),
	Flex(f32, f32, Length),
	Grow(f32),
	Shrink(f32),
	Justify(JustifyContent),
	Align(AlignField, Align),
	GridTracks(Axis, Vec<Track>),
	GridAutoTrack(Axis, Track),
	GridAutoFlow(GridAutoFlow),
	GridLine(Axis, bool, GridLine),
	GridPlacement(Axis, GridPlacement),
	Aspect(Option<f32>),
	Color(ColorField, Color),
	FontSize(Length),
	FontWeight(FontWeight),
	Italic(bool),
	TextAlign(TextAlign),
	BorderWidth(Length),
	/// A border width on some edges only, from `border-t`, `border-x-2`, and
	/// the like.
	EdgeBorderWidth(&'static [Edge], Length),
	Radius(Length),
	Opacity(f32),
}

/// Writes one declaration, marked important when the utility is.
fn write(output: &mut String, important: bool, name: &str, value: &str) {
	let _ = write!(
		output,
		"{name}:{value}{};",
		if important { "!important" } else { "" }
	);
}

impl Declaration {
	pub(super) fn apply(&self, style: &mut Style, props: &mut NativeProps, context: &StyleContext) {
		if self.apply_box(style, props, context) {
			return;
		}

		self.apply_paint(style, props, context);
	}

	/// The size, spacing, and layout declarations. Says whether this was
	/// one.
	fn apply_box(
		&self,
		style: &mut Style,
		props: &mut NativeProps,
		context: &StyleContext,
	) -> bool {
		match *self {
			Self::Size(field, length) => {
				let value = match field {
					SizeField::Width => &mut style.width,
					SizeField::Height => &mut style.height,
					SizeField::MinWidth => &mut style.min_width,
					SizeField::MinHeight => &mut style.min_height,
					SizeField::MaxWidth => &mut style.max_width,
					SizeField::MaxHeight => &mut style.max_height,
					SizeField::Basis => &mut style.flex_basis,
				};
				*value = length.dimension(context);
			}

			Self::Edge(field, edge, length) => {
				let rect = match field {
					EdgeField::Padding => &mut style.padding,
					EdgeField::Margin => &mut style.margin,
					EdgeField::Inset => &mut style.inset,
				};
				*edge.of_mut(rect) = length.dimension(context);
			}

			Self::SafePadding(edge, offset) => {
				let points = safe_inset(edge.of(context.safe_area), offset, context);
				*edge.of_mut(&mut style.padding) = Dimension::Points(points);
			}
			Self::Gap(axis, length) => {
				let value = length.dimension(context);

				match axis {
					Axis::X => {
						style.gap.left = value;
						style.gap.right = value;
					}
					Axis::Y => {
						style.gap.top = value;
						style.gap.bottom = value;
					}
				}
			}
			Self::Display(value) => {
				style.display = value;
				props.hidden = Some(value == Display::None);
			}
			Self::Position(value) => style.position = value,
			Self::BoxSizing(value) => style.box_sizing = value,
			Self::Overflow(value) => {
				style.overflow = value;
				props.clip = Some(value != Overflow::Visible);
			}
			Self::FlexDirection(value) => style.flex_direction = value,
			Self::FlexWrap(value) => style.flex_wrap = value,
			Self::Flex(grow, shrink, basis) => {
				style.flex_grow = grow;
				style.flex_shrink = shrink;
				style.flex_basis = basis.dimension(context);
			}
			Self::Grow(value) => style.flex_grow = value,
			Self::Shrink(value) => style.flex_shrink = value,
			Self::Justify(value) => style.justify_content = value,
			Self::Align(field, align) => {
				match field {
					AlignField::Items => style.align_items = align,
					AlignField::SelfItem => style.align_self = align,
					AlignField::Content => style.align_content = align,
					AlignField::JustifyItems => {
						style.grid.get_or_insert_default().justify_items = align;
					}
					AlignField::JustifySelf => {
						style.grid.get_or_insert_default().justify_self = align;
					}
				}
			}
			Self::GridTracks(axis, ref tracks) => {
				let tracks = tracks.iter().map(|track| track.resolve(context)).collect();
				let grid = style.grid.get_or_insert_default();

				match axis {
					Axis::X => grid.template_columns = tracks,
					Axis::Y => grid.template_rows = tracks,
				}
			}
			Self::GridAutoTrack(axis, ref track) => {
				let track = track.resolve(context);
				let grid = style.grid.get_or_insert_default();

				match axis {
					Axis::X => grid.auto_columns = track,
					Axis::Y => grid.auto_rows = track,
				}
			}
			Self::GridAutoFlow(flow) => style.grid.get_or_insert_default().auto_flow = flow,
			Self::GridLine(axis, start, line) => {
				let grid = style.grid.get_or_insert_default();
				let placement = match axis {
					Axis::X => &mut grid.column,
					Axis::Y => &mut grid.row,
				};

				if start {
					placement.start = line;
				} else {
					placement.end = line;
				}
			}
			Self::GridPlacement(axis, value) => {
				let grid = style.grid.get_or_insert_default();

				match axis {
					Axis::X => grid.column = value,
					Axis::Y => grid.row = value,
				}
			}
			Self::Aspect(value) => style.aspect_ratio = value,
			_ => return false,
		}

		true
	}

	/// The color, text, border, and radius declarations.
	fn apply_paint(&self, style: &mut Style, props: &mut NativeProps, context: &StyleContext) {
		match *self {
			Self::Color(field, value) => {
				match field {
					ColorField::Text => props.color = Some(value),
					ColorField::Background => props.background = Some(value),
					ColorField::Border => props.border_color = Some(value),
				}
			}
			Self::FontSize(value) => props.font_size = Some(value.points(context)),
			Self::FontWeight(value) => props.font_weight = Some(value),
			Self::Italic(value) => props.italic = Some(value),
			Self::TextAlign(value) => props.text_align = Some(value),
			Self::BorderWidth(value) => {
				let width = value.points(context);
				style.border = Rect::new(|| width);
				props.border_width = Some(width);
				if let Some(widths) = props.border_widths.as_mut() {
					*widths = crate::props::BorderWidths::uniform(width);
				}
			}
			Self::EdgeBorderWidth(edges, value) => {
				let width = value.points(context);
				let mut widths = props.border_edges();

				for &edge in edges {
					*edge.of_mut(&mut style.border) = width;
					*match edge {
						Edge::Top => &mut widths.top,
						Edge::Right => &mut widths.right,
						Edge::Bottom => &mut widths.bottom,
						Edge::Left => &mut widths.left,
					} = width;
				}

				props.border_widths = Some(widths);
			}
			Self::Radius(value) => props.corner_radius = Some(value.points(context)),
			Self::Opacity(value) => props.opacity = Some(value),
			_ => {}
		}
	}

	pub(super) fn css(&self, output: &mut String, important: bool) {
		if self.css_box(output, important) {
			return;
		}

		self.css_paint(output, important);
	}

	/// The size, spacing, and layout declarations. Says whether this was
	/// one.
	fn css_box(&self, output: &mut String, important: bool) -> bool {
		if self.css_spacing(output, important) {
			return true;
		}

		match *self {
			Self::Display(value) => write(output, important, "display", keyword::display(value)),
			Self::Position(value) => write(output, important, "position", keyword::position(value)),
			Self::BoxSizing(value) => {
				write(output, important, "box-sizing", keyword::box_sizing(value));
			}
			Self::Overflow(value) => {
				write(
					output,
					important,
					"overflow",
					match value {
						Overflow::Visible => "visible",
						Overflow::Hidden => "hidden",
						Overflow::Scroll => "scroll",
					},
				);
			}

			Self::FlexDirection(value) => {
				write(
					output,
					important,
					"flex-direction",
					keyword::flex_direction(value),
				);
			}
			Self::FlexWrap(value) => {
				write(output, important, "flex-wrap", keyword::flex_wrap(value));
			}
			Self::Flex(grow, shrink, basis) => {
				write(output, important, "flex-grow", &grow.to_string());
				write(output, important, "flex-shrink", &shrink.to_string());
				write(output, important, "flex-basis", &basis.css());
			}
			Self::Grow(value) => write(output, important, "flex-grow", &value.to_string()),
			Self::Shrink(value) => write(output, important, "flex-shrink", &value.to_string()),
			Self::Justify(value) => {
				write(
					output,
					important,
					"justify-content",
					keyword::justify_content(value),
				);
			}
			Self::Align(field, value) => {
				write(output, important, field.css(), align_css(value, field));
			}
			Self::GridTracks(axis, ref tracks) => {
				write(
					output,
					important,
					axis.property("grid-template-columns", "grid-template-rows"),
					&if tracks.is_empty() {
						"none".to_owned()
					} else {
						tracks.iter().map(Track::css).collect::<Vec<_>>().join(" ")
					},
				);
			}

			Self::GridAutoTrack(axis, ref track) => {
				write(
					output,
					important,
					axis.property("grid-auto-columns", "grid-auto-rows"),
					&track.css(),
				);
			}
			Self::GridAutoFlow(value) => {
				write(
					output,
					important,
					"grid-auto-flow",
					keyword::grid_auto_flow(value),
				);
			}
			Self::GridLine(axis, start, line) => {
				write(
					output,
					important,
					match (axis, start) {
						(Axis::X, true) => "grid-column-start",
						(Axis::X, false) => "grid-column-end",
						(Axis::Y, true) => "grid-row-start",
						(Axis::Y, false) => "grid-row-end",
					},
					&keyword::grid_line(line),
				);
			}

			Self::GridPlacement(axis, placement) => {
				write(
					output,
					important,
					axis.property("grid-column", "grid-row"),
					&format!(
						"{}/{}",
						keyword::grid_line(placement.start),
						keyword::grid_line(placement.end)
					),
				);
			}
			Self::Aspect(value) => {
				write(
					output,
					important,
					"aspect-ratio",
					&value.map_or_else(|| "auto".to_owned(), |value| value.to_string()),
				);
			}
			_ => return false,
		}

		true
	}

	/// The size and spacing declarations. Says whether this was one.
	fn css_spacing(&self, output: &mut String, important: bool) -> bool {
		match *self {
			Self::Size(field, Length::Auto)
				if matches!(field, SizeField::MaxWidth | SizeField::MaxHeight) =>
			{
				write(output, important, field.css(), "none");
			}
			Self::Size(field, length) => write(output, important, field.css(), &length.css()),
			Self::Edge(field, edge, length) => {
				let prefix = match field {
					EdgeField::Padding => "padding-",
					EdgeField::Margin => "margin-",
					EdgeField::Inset => "",
				};

				write(
					output,
					important,
					&format!("{prefix}{}", edge.css()),
					&length.css(),
				);
			}
			Self::SafePadding(edge, offset) => {
				let inset = format!("env(safe-area-inset-{})", edge.css());
				let value = if offset.is_zero() {
					inset
				} else {
					format!("calc({inset} + {})", offset.css())
				};

				write(
					output,
					important,
					&format!("padding-{}", edge.css()),
					&value,
				);
			}
			Self::Gap(axis, length) => {
				write(
					output,
					important,
					axis.property("column-gap", "row-gap"),
					&length.css(),
				);
			}
			_ => return false,
		}

		true
	}

	/// The color, text, border, and radius declarations.
	fn css_paint(&self, output: &mut String, important: bool) {
		match *self {
			Self::Color(field, value) => {
				write(
					output,
					important,
					match field {
						ColorField::Text => "color",
						ColorField::Background => "background-color",
						ColorField::Border => "border-color",
					},
					&format!("#{:08x}", value.0),
				);

				if field == ColorField::Border {
					write(
						output,
						important,
						"--lk-border-color",
						&format!("#{:08x}", value.0),
					);
				}
			}
			Self::FontSize(value) => write(output, important, "font-size", &value.css()),
			Self::FontWeight(value) => {
				write(
					output,
					important,
					"font-weight",
					keyword::font_weight(value),
				);
			}
			Self::Italic(value) => {
				write(output, important, "font-style", keyword::font_style(value));
			}
			Self::TextAlign(value) => {
				let value = keyword::text_align(value);
				write(output, important, "text-align", value);
				write(output, important, "--lk-text-align", value);
			}
			Self::BorderWidth(value) => {
				write(output, important, "border-width", &value.css());
				write(output, important, "border-style", "solid");
				write(
					output,
					important,
					"border-color",
					"var(--lk-border-color,currentColor)",
				);
				write(output, important, "box-shadow", "none");
			}
			Self::EdgeBorderWidth(edges, value) => {
				for edge in edges {
					write(
						output,
						important,
						&format!("border-{}-width", edge.css()),
						&value.css(),
					);
				}

				write(output, important, "border-style", "solid");
				write(
					output,
					important,
					"border-color",
					"var(--lk-border-color,currentColor)",
				);
				write(output, important, "box-shadow", "none");
			}
			Self::Radius(value) => {
				let radius = value.css();
				write(output, important, "border-radius", &radius);
				let clip = if value.is_zero() {
					"none".to_owned()
				} else {
					format!("inset(0 round {radius})")
				};

				write(output, important, "clip-path", &clip);
			}
			Self::Opacity(value) => write(output, important, "opacity", &value.to_string()),
			_ => {}
		}
	}
}

impl Axis {
	/// The one of a horizontal and a vertical CSS property that applies to
	/// this axis.
	fn property(self, x: &'static str, y: &'static str) -> &'static str {
		match self {
			Self::X => x,
			Self::Y => y,
		}
	}
}

impl AlignField {
	fn css(self) -> &'static str {
		match self {
			Self::Items => "align-items",
			Self::SelfItem => "align-self",
			Self::Content => "align-content",
			Self::JustifyItems => "justify-items",
			Self::JustifySelf => "justify-self",
		}
	}
}

impl SizeField {
	fn css(self) -> &'static str {
		match self {
			Self::Width => "width",
			Self::Height => "height",
			Self::MinWidth => "min-width",
			Self::MinHeight => "min-height",
			Self::MaxWidth => "max-width",
			Self::MaxHeight => "max-height",
			Self::Basis => "flex-basis",
		}
	}
}

impl Track {
	fn resolve(&self, context: &StyleContext) -> TrackSizing {
		match *self {
			Self::Auto => TrackSizing::Auto,
			Self::Length(Length::Percent(value)) => TrackSizing::Percent(value),
			Self::Length(length) => TrackSizing::Points(length.points(context)),
			Self::Fraction(value) => TrackSizing::Fraction(value),
			Self::MinMax(ref min, ref max) => {
				TrackSizing::MinMax(Box::new(MinMax {
					min: min.resolve(context),
					max: max.resolve(context),
				}))
			}
		}
	}

	fn css(&self) -> String {
		match *self {
			Self::Auto => "auto".to_owned(),
			Self::Length(length) => length.css(),
			Self::Fraction(value) => format!("{value}fr"),
			Self::MinMax(ref min, ref max) => format!("minmax({},{})", min.css(), max.css()),
		}
	}
}

fn align_css(align: Align, field: AlignField) -> &'static str {
	// The `justify-*` properties of grid items take the box alignment spelling.
	let justify = matches!(field, AlignField::JustifyItems | AlignField::JustifySelf);

	match align {
		Align::FlexStart if justify => "start",

		Align::FlexEnd if justify => "end",
		_ => keyword::align(align),
	}
}
