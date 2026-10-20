//! Serialize typed base styles below the specificity of utility classes.

use std::fmt::Write;

use leptoskit_yoga::Align;
use leptoskit_yoga::BoxSizing;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Direction;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::GridAutoFlow;
use leptoskit_yoga::GridLine;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::Overflow;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;
use leptoskit_yoga::TrackSizing;

use crate::Color;
use crate::Element;
use crate::FontWeight;
use crate::NativeKind;
use crate::TextAlign;
use crate::TextStyle;

/// Native defaults are explicit so browser defaults cannot change layout.
pub(super) fn base(element: &Element, root: bool) -> String {
	let style = &element.style;
	let props = &element.props;
	let mut css = String::from(
		"appearance:none;margin:0;border:0 solid \
		 transparent;--lk-border-color:currentColor;background:transparent;font:inherit;color:\
		 inherit;text-align:inherit;white-space:pre-wrap;text-decoration:none;",
	);

	if root {
		write!(
			css,
			"font-family:system-ui,sans-serif;font-size:{}px;font-weight:400;text-align:start;",
			TextStyle::DEFAULT_FONT_SIZE
		)
		.expect("write to String");
	}

	property(
		&mut css,
		"display",
		match style.display {
			Display::Flex => "flex",
			Display::Grid => "grid",
			Display::Contents => "contents",
			Display::None => "none",
		},
	);
	property(
		&mut css,
		"box-sizing",
		match style.box_sizing {
			BoxSizing::BorderBox => "border-box",
			BoxSizing::ContentBox => "content-box",
		},
	);
	property(
		&mut css,
		"position",
		match style.position {
			PositionType::Relative => "relative",
			PositionType::Absolute => "absolute",
			PositionType::Fixed => "fixed",
		},
	);
	property(
		&mut css,
		"direction",
		match style.direction {
			Direction::Ltr => "ltr",
			Direction::Rtl => "rtl",
		},
	);
	property(
		&mut css,
		"flex-direction",
		match style.flex_direction {
			FlexDirection::Row => "row",
			FlexDirection::RowReverse => "row-reverse",
			FlexDirection::Column => "column",
			FlexDirection::ColumnReverse => "column-reverse",
		},
	);
	property(
		&mut css,
		"flex-wrap",
		match style.flex_wrap {
			FlexWrap::NoWrap => "nowrap",
			FlexWrap::Wrap => "wrap",
			FlexWrap::WrapReverse => "wrap-reverse",
		},
	);
	property(
		&mut css,
		"justify-content",
		match style.justify_content {
			JustifyContent::FlexStart => "flex-start",
			JustifyContent::FlexEnd => "flex-end",
			JustifyContent::Center => "center",
			JustifyContent::SpaceBetween => "space-between",
			JustifyContent::SpaceAround => "space-around",
			JustifyContent::SpaceEvenly => "space-evenly",
		},
	);
	property(&mut css, "align-items", align(style.align_items));
	property(&mut css, "align-self", align(style.align_self));
	property(&mut css, "align-content", align(style.align_content));
	property(
		&mut css,
		"overflow",
		match (element.kind, style.overflow) {
			(_, Overflow::Hidden) => "hidden",
			(NativeKind::ScrollView, _) | (_, Overflow::Scroll) => "auto",
			_ => "visible",
		},
	);
	write!(
		css,
		"flex-grow:{};flex-shrink:{};",
		style.flex_grow, style.flex_shrink
	)
	.expect("write to String");

	for (name, value, auto) in [
		("flex-basis", style.flex_basis, "auto"),
		("width", style.width, "auto"),
		("height", style.height, "auto"),
		("min-width", style.min_width, "0px"),
		("min-height", style.min_height, "0px"),
		("max-width", style.max_width, "none"),
		("max-height", style.max_height, "none"),
		("row-gap", style.gap.top, "0px"),
		("column-gap", style.gap.left, "0px"),
	] {
		property(&mut css, name, &dimension(value, auto));
	}

	edges(&mut css, "margin", style.margin, "auto");
	edges(&mut css, "padding", style.padding, "0px");

	for (edge, value, border) in [
		("top", style.inset.top, style.border.top),
		("right", style.inset.right, style.border.right),
		("bottom", style.inset.bottom, style.border.bottom),
		("left", style.inset.left, style.border.left),
	] {
		property(&mut css, edge, &dimension(value, "auto"));
		write!(css, "border-{edge}-width:{border}px;").expect("write to String");
	}

	if let Some(ratio) = style.aspect_ratio {
		write!(css, "aspect-ratio:{ratio};").expect("write to String");
	}

	if let Some(grid) = &style.grid {
		property(
			&mut css,
			"grid-template-columns",
			&tracks(&grid.template_columns),
		);
		property(&mut css, "grid-template-rows", &tracks(&grid.template_rows));
		property(&mut css, "grid-auto-columns", &track(&grid.auto_columns));
		property(&mut css, "grid-auto-rows", &track(&grid.auto_rows));
		property(
			&mut css,
			"grid-auto-flow",
			match grid.auto_flow {
				GridAutoFlow::Row => "row",
				GridAutoFlow::Column => "column",
				GridAutoFlow::RowDense => "row dense",
				GridAutoFlow::ColumnDense => "column dense",
			},
		);
		property(&mut css, "justify-items", align(grid.justify_items));
		property(&mut css, "justify-self", align(grid.justify_self));

		for (name, line) in [
			("grid-column-start", grid.column.start),
			("grid-column-end", grid.column.end),
			("grid-row-start", grid.row.start),
			("grid-row-end", grid.row.end),
		] {
			property(&mut css, name, &grid_line(line));
		}
	}

	for (name, color) in [
		("color", props.color),
		("background-color", props.background),
		("--lk-border-color", props.border_color),
	] {
		if let Some(color) = color {
			property(&mut css, name, &color_css(color));
		}
	}

	if let Some(width) = props.border_width {
		// Legacy border props paint inside the box without affecting layout.
		write!(
			css,
			"box-shadow:inset 0 0 0 {width}px var(--lk-border-color);"
		)
		.expect("write to String");
	}

	if let Some(radius) = props.corner_radius {
		write!(css, "border-radius:{radius}px;").expect("write to String");

		property(
			&mut css,
			"clip-path",
			&if radius > 0.0 {
				format!("inset(0 round {radius}px)")
			} else {
				"none".to_owned()
			},
		);
	}

	if let Some(opacity) = props.opacity {
		write!(css, "opacity:{};", opacity.clamp(0.0, 1.0)).expect("write to String");
	}

	if let Some(size) = props.font_size {
		write!(css, "font-size:{size}px;").expect("write to String");
	}

	if let Some(weight) = props.font_weight {
		property(
			&mut css,
			"font-weight",
			match weight {
				FontWeight::Regular => "400",
				FontWeight::Medium => "500",
				FontWeight::Semibold => "600",
				FontWeight::Bold => "700",
			},
		);
	}

	if let Some(alignment) = props.text_align {
		let value = match alignment {
			TextAlign::Start => "start",
			TextAlign::Center => "center",
			TextAlign::End => "end",
		};
		property(&mut css, "text-align", value);
		property(&mut css, "--lk-text-align", value);
	} else if element.kind == NativeKind::Button {
		property(&mut css, "text-align", "var(--lk-text-align,center)");
	}

	if element.kind == NativeKind::Image {
		property(&mut css, "object-fit", "contain");
	}

	if props.clip == Some(true) && element.kind != NativeKind::ScrollView {
		property(&mut css, "overflow", "hidden");
	}

	if props.hidden == Some(true) {
		property(&mut css, "display", "none");
	}

	css
}

fn property(css: &mut String, name: &str, value: &str) {
	write!(css, "{name}:{value};").expect("write to String");
}

fn dimension(value: Dimension, auto: &str) -> String {
	match value {
		Dimension::Auto => auto.to_owned(),
		Dimension::Points(value) => format!("{value}px"),
		Dimension::Percent(value) => format!("{}%", value * 100.0),
	}
}

fn edges(css: &mut String, name: &str, edges: Rect<Dimension>, auto: &str) {
	for (edge, value) in [
		("top", edges.top),
		("right", edges.right),
		("bottom", edges.bottom),
		("left", edges.left),
	] {
		write!(css, "{name}-{edge}:{};", dimension(value, auto)).expect("write to String");
	}
}

fn align(value: Align) -> &'static str {
	match value {
		Align::Auto => "auto",
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

fn tracks(values: &[TrackSizing]) -> String {
	if values.is_empty() {
		return "none".to_owned();
	}

	values.iter().map(track).collect::<Vec<_>>().join(" ")
}

fn track(value: &TrackSizing) -> String {
	match value {
		TrackSizing::Auto => "auto".to_owned(),
		TrackSizing::Points(value) => format!("{value}px"),
		TrackSizing::Percent(value) => format!("{}%", value * 100.0),
		TrackSizing::Fraction(value) => format!("{value}fr"),
		TrackSizing::MinMax(bounds) => {
			format!("minmax({},{})", track(&bounds.min), track(&bounds.max))
		}
	}
}

fn grid_line(value: GridLine) -> String {
	match value {
		GridLine::Auto => "auto".to_owned(),
		GridLine::Line(line) => line.to_string(),
		GridLine::Span(span) => format!("span {span}"),
	}
}

fn color_css(color: Color) -> String {
	format!("#{:08x}", color.0)
}
