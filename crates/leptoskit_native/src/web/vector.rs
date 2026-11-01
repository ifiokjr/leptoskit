//! Render a drawing as inline SVG with the same geometry native backends draw.

use leptos::prelude::*;
use leptos::svg;

use super::styles::color_css;
use crate::vector::Drawing;
use crate::vector::FillRule;
use crate::vector::LineCap;
use crate::vector::LineJoin;
use crate::vector::Paint;
use crate::vector::Stroke;
use crate::vector::StrokeScaling;
use crate::vector::VectorPath;
use crate::vector::VectorScaling;

/// An `<svg>` covering its element, with one `<path>` per drawing path.
///
/// Fixed strokes use `vector-effect: non-scaling-stroke`, so their width and
/// dashes stay in layout units under any scaling, as on native. Path data is
/// the canonical string the path formats once and keeps.
pub(super) fn svg(drawing: Option<&Drawing>) -> AnyView {
	let default = Drawing::default();
	let drawing = drawing.unwrap_or(&default);
	let alignment = drawing.alignment().svg_keyword();

	let aspect = match drawing.vector_scaling() {
		VectorScaling::Fit => format!("{alignment} meet"),
		VectorScaling::Fill => format!("{alignment} slice"),
		VectorScaling::Stretch => "none".to_owned(),
	};

	svg::svg()
		.attr("viewBox", drawing.view_box().to_string())
		.attr("preserveAspectRatio", aspect)
		.attr("aria-hidden", "true")
		.attr("focusable", "false")
		.child(drawing.paths().iter().map(path).collect::<Vec<_>>())
		.into_any()
}

fn path(path: &VectorPath) -> AnyView {
	let stroke = path.stroke_style();
	let dashes = stroke
		.filter(|stroke| !stroke.dash_lengths().is_empty())
		.map(|stroke| {
			stroke
				.dash_lengths()
				.iter()
				.map(f32::to_string)
				.collect::<Vec<_>>()
				.join(" ")
		});

	svg::path()
		.attr("d", path.data().canonical().to_owned())
		.attr(
			"fill",
			path.fill_paint().map_or_else(|| "none".to_owned(), paint),
		)
		.attr(
			"fill-rule",
			(path.rule() == FillRule::Evenodd).then_some("evenodd"),
		)
		.attr("stroke", stroke.map(|stroke| paint(stroke.paint())))
		.attr(
			"stroke-width",
			stroke.map(|stroke| stroke.width().to_string()),
		)
		.attr("stroke-linecap", stroke.map(cap))
		.attr("stroke-linejoin", stroke.map(join))
		.attr(
			"stroke-miterlimit",
			stroke.map(|stroke| stroke.miter().to_string()),
		)
		.attr("stroke-dasharray", dashes)
		.attr(
			"stroke-dashoffset",
			stroke
				.filter(|stroke| stroke.dash_offset() != 0.0)
				.map(|stroke| stroke.dash_offset().to_string()),
		)
		.attr(
			"vector-effect",
			stroke
				.filter(|stroke| stroke.stroke_scaling() == StrokeScaling::Fixed)
				.map(|_| "non-scaling-stroke"),
		)
		.into_any()
}

/// A fixed color, or `currentColor`, which the element's CSS `color`
/// resolves.
fn paint(paint: Paint) -> String {
	match paint {
		Paint::Color(color) => color_css(color),
		Paint::CurrentColor => "currentColor".to_owned(),
	}
}

fn cap(stroke: &Stroke) -> &'static str {
	match stroke.line_cap() {
		LineCap::Butt => "butt",
		LineCap::Round => "round",
		LineCap::Square => "square",
	}
}

fn join(stroke: &Stroke) -> &'static str {
	match stroke.line_join() {
		LineJoin::Miter => "miter",
		LineJoin::Round => "round",
		LineJoin::Bevel => "bevel",
	}
}
