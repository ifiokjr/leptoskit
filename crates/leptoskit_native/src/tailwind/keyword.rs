//! CSS keywords for typed style values, shared by the CSS that utility classes
//! compile to and the web renderer's base styles, so both always spell a
//! value the same way.

use leptoskit_yoga::Align;
use leptoskit_yoga::BoxSizing;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::GridAutoFlow;
use leptoskit_yoga::GridLine;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::PositionType;

use crate::FontWeight;
use crate::TextAlign;

pub(crate) fn display(value: Display) -> &'static str {
	match value {
		Display::Flex => "flex",
		Display::Grid => "grid",
		Display::Contents => "contents",
		Display::None => "none",
	}
}

pub(crate) fn position(value: PositionType) -> &'static str {
	match value {
		PositionType::Relative => "relative",
		PositionType::Absolute => "absolute",
		PositionType::Fixed => "fixed",
	}
}

pub(crate) fn box_sizing(value: BoxSizing) -> &'static str {
	match value {
		BoxSizing::BorderBox => "border-box",
		BoxSizing::ContentBox => "content-box",
	}
}

pub(crate) fn flex_direction(value: FlexDirection) -> &'static str {
	match value {
		FlexDirection::Row => "row",
		FlexDirection::RowReverse => "row-reverse",
		FlexDirection::Column => "column",
		FlexDirection::ColumnReverse => "column-reverse",
	}
}

pub(crate) fn flex_wrap(value: FlexWrap) -> &'static str {
	match value {
		FlexWrap::NoWrap => "nowrap",
		FlexWrap::Wrap => "wrap",
		FlexWrap::WrapReverse => "wrap-reverse",
	}
}

pub(crate) fn justify_content(value: JustifyContent) -> &'static str {
	match value {
		JustifyContent::FlexStart => "flex-start",
		JustifyContent::FlexEnd => "flex-end",
		JustifyContent::Center => "center",
		JustifyContent::SpaceBetween => "space-between",
		JustifyContent::SpaceAround => "space-around",
		JustifyContent::SpaceEvenly => "space-evenly",
	}
}

/// The flexbox spelling of an alignment, as `align-*` properties take it.
pub(crate) fn align(value: Align) -> &'static str {
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

pub(crate) fn grid_auto_flow(value: GridAutoFlow) -> &'static str {
	match value {
		GridAutoFlow::Row => "row",
		GridAutoFlow::Column => "column",
		GridAutoFlow::RowDense => "row dense",
		GridAutoFlow::ColumnDense => "column dense",
	}
}

/// A `grid-row-*` or `grid-column-*` line value.
pub(crate) fn grid_line(value: GridLine) -> String {
	match value {
		GridLine::Auto => "auto".to_owned(),
		GridLine::Line(line) => line.to_string(),
		GridLine::Span(span) => format!("span {span}"),
	}
}

pub(crate) fn font_weight(value: FontWeight) -> &'static str {
	match value {
		FontWeight::Regular => "400",
		FontWeight::Medium => "500",
		FontWeight::Semibold => "600",
		FontWeight::Bold => "700",
	}
}

pub(crate) fn text_align(value: TextAlign) -> &'static str {
	match value {
		TextAlign::Start => "start",
		TextAlign::Center => "center",
		TextAlign::End => "end",
	}
}
