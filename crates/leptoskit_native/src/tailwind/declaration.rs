use std::fmt::Write;

use leptoskit_yoga::Align;
use leptoskit_yoga::BoxSizing;
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
use super::value::Length;
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
	TextAlign(TextAlign),
	BorderWidth(Length),
	Radius(Length),
	Opacity(f32),
}

impl Declaration {
	pub(super) fn apply(&self, style: &mut Style, props: &mut NativeProps, context: &StyleContext) {
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
				let value = match edge {
					Edge::Top => &mut rect.top,
					Edge::Right => &mut rect.right,
					Edge::Bottom => &mut rect.bottom,
					Edge::Left => &mut rect.left,
				};
				*value = length.dimension(context);
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
			Self::Color(field, value) => {
				match field {
					ColorField::Text => props.color = Some(value),
					ColorField::Background => props.background = Some(value),
					ColorField::Border => props.border_color = Some(value),
				}
			}
			Self::FontSize(value) => props.font_size = Some(value.points(context)),
			Self::FontWeight(value) => props.font_weight = Some(value),
			Self::TextAlign(value) => props.text_align = Some(value),
			Self::BorderWidth(value) => {
				let width = value.points(context);
				style.border = Rect::new(|| width);
				props.border_width = Some(width);
			}
			Self::Radius(value) => props.corner_radius = Some(value.points(context)),
			Self::Opacity(value) => props.opacity = Some(value),
		}
	}

	pub(super) fn css(&self, output: &mut String, important: bool) {
		let mut write = |name: &str, value: &str| {
			let _ = write!(
				output,
				"{name}:{value}{};",
				if important { "!important" } else { "" }
			);
		};

		match *self {
			Self::Size(field, Length::Auto)
				if matches!(field, SizeField::MaxWidth | SizeField::MaxHeight) =>
			{
				write(field.css(), "none");
			}
			Self::Size(field, length) => write(field.css(), &length.css()),
			Self::Edge(field, edge, length) => {
				let prefix = match field {
					EdgeField::Padding => "padding-",
					EdgeField::Margin => "margin-",
					EdgeField::Inset => "",
				};
				let edge = match edge {
					Edge::Top => "top",
					Edge::Right => "right",
					Edge::Bottom => "bottom",
					Edge::Left => "left",
				};
				write(&format!("{prefix}{edge}"), &length.css());
			}
			Self::Gap(axis, length) => {
				write(
					match axis {
						Axis::X => "column-gap",
						Axis::Y => "row-gap",
					},
					&length.css(),
				);
			}
			Self::Display(value) => {
				write(
					"display",
					match value {
						Display::Flex => "flex",
						Display::Grid => "grid",
						Display::Contents => "contents",
						Display::None => "none",
					},
				);
			}
			Self::Position(value) => {
				write(
					"position",
					match value {
						PositionType::Relative => "relative",
						PositionType::Absolute => "absolute",
						PositionType::Fixed => "fixed",
					},
				);
			}
			Self::BoxSizing(value) => {
				write(
					"box-sizing",
					match value {
						BoxSizing::BorderBox => "border-box",
						BoxSizing::ContentBox => "content-box",
					},
				);
			}
			Self::Overflow(value) => {
				write(
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
					"flex-direction",
					match value {
						FlexDirection::Row => "row",
						FlexDirection::RowReverse => "row-reverse",
						FlexDirection::Column => "column",
						FlexDirection::ColumnReverse => "column-reverse",
					},
				);
			}
			Self::FlexWrap(value) => {
				write(
					"flex-wrap",
					match value {
						FlexWrap::NoWrap => "nowrap",
						FlexWrap::Wrap => "wrap",
						FlexWrap::WrapReverse => "wrap-reverse",
					},
				);
			}
			Self::Flex(grow, shrink, basis) => {
				write("flex-grow", &grow.to_string());
				write("flex-shrink", &shrink.to_string());
				write("flex-basis", &basis.css());
			}
			Self::Grow(value) => write("flex-grow", &value.to_string()),
			Self::Shrink(value) => write("flex-shrink", &value.to_string()),
			Self::Justify(value) => {
				write(
					"justify-content",
					match value {
						JustifyContent::FlexStart => "flex-start",
						JustifyContent::FlexEnd => "flex-end",
						JustifyContent::Center => "center",
						JustifyContent::SpaceBetween => "space-between",
						JustifyContent::SpaceAround => "space-around",
						JustifyContent::SpaceEvenly => "space-evenly",
					},
				);
			}
			Self::Align(field, value) => {
				write(
					match field {
						AlignField::Items => "align-items",
						AlignField::SelfItem => "align-self",
						AlignField::Content => "align-content",
						AlignField::JustifyItems => "justify-items",
						AlignField::JustifySelf => "justify-self",
					},
					align_css(value, field),
				);
			}
			Self::GridTracks(axis, ref tracks) => {
				write(
					match axis {
						Axis::X => "grid-template-columns",
						Axis::Y => "grid-template-rows",
					},
					&if tracks.is_empty() {
						"none".to_owned()
					} else {
						tracks.iter().map(Track::css).collect::<Vec<_>>().join(" ")
					},
				);
			}
			Self::GridAutoTrack(axis, ref track) => {
				write(
					match axis {
						Axis::X => "grid-auto-columns",
						Axis::Y => "grid-auto-rows",
					},
					&track.css(),
				);
			}
			Self::GridAutoFlow(value) => {
				write(
					"grid-auto-flow",
					match value {
						GridAutoFlow::Row => "row",
						GridAutoFlow::Column => "column",
						GridAutoFlow::RowDense => "row dense",
						GridAutoFlow::ColumnDense => "column dense",
					},
				);
			}
			Self::GridLine(axis, start, line) => {
				write(
					match (axis, start) {
						(Axis::X, true) => "grid-column-start",
						(Axis::X, false) => "grid-column-end",
						(Axis::Y, true) => "grid-row-start",
						(Axis::Y, false) => "grid-row-end",
					},
					&line_css(line),
				);
			}
			Self::GridPlacement(axis, placement) => {
				write(
					match axis {
						Axis::X => "grid-column",
						Axis::Y => "grid-row",
					},
					&format!("{}/{}", line_css(placement.start), line_css(placement.end)),
				);
			}
			Self::Aspect(value) => {
				write(
					"aspect-ratio",
					&value.map_or_else(|| "auto".to_owned(), |value| value.to_string()),
				);
			}
			Self::Color(field, value) => {
				write(
					match field {
						ColorField::Text => "color",
						ColorField::Background => "background-color",
						ColorField::Border => "border-color",
					},
					&format!("#{:08x}", value.0),
				);

				if field == ColorField::Border {
					write("--lk-border-color", &format!("#{:08x}", value.0));
				}
			}
			Self::FontSize(value) => write("font-size", &value.css()),
			Self::FontWeight(value) => {
				write(
					"font-weight",
					match value {
						FontWeight::Regular => "400",
						FontWeight::Medium => "500",
						FontWeight::Semibold => "600",
						FontWeight::Bold => "700",
					},
				);
			}
			Self::TextAlign(value) => {
				let value = match value {
					TextAlign::Start => "start",
					TextAlign::Center => "center",
					TextAlign::End => "end",
				};
				write("text-align", value);
				write("--lk-text-align", value);
			}
			Self::BorderWidth(value) => {
				write("border-width", &value.css());
				write("border-style", "solid");
				write("border-color", "var(--lk-border-color,currentColor)");
				write("box-shadow", "none");
			}
			Self::Radius(value) => {
				let radius = value.css();
				write("border-radius", &radius);
				let clip = if value.is_zero() {
					"none".to_owned()
				} else {
					format!("inset(0 round {radius})")
				};
				write("clip-path", &clip);
			}
			Self::Opacity(value) => write("opacity", &value.to_string()),
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
	match align {
		Align::Auto => "auto",
		Align::FlexStart if matches!(field, AlignField::JustifyItems | AlignField::JustifySelf) => {
			"start"
		}
		Align::FlexEnd if matches!(field, AlignField::JustifyItems | AlignField::JustifySelf) => {
			"end"
		}
		Align::FlexStart => "flex-start",
		Align::FlexEnd => "flex-end",
		Align::Center => "center",
		Align::Stretch => "stretch",
		Align::Baseline => "baseline",
		Align::SpaceBetween => "space-between",
		Align::SpaceAround => "space-around",
		Align::SpaceEvenly => "space-evenly",
	}
}

fn line_css(line: GridLine) -> String {
	match line {
		GridLine::Auto => "auto".to_owned(),
		GridLine::Line(line) => line.to_string(),
		GridLine::Span(span) => format!("span {span}"),
	}
}
