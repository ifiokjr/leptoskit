//! Render a drawing as inline SVG with the same geometry native backends draw.

use leptos::prelude::*;
use leptos::svg;

use super::styles::color_css;
use crate::vector::Drawing;
use crate::vector::LineCap;
use crate::vector::LineJoin;
use crate::vector::Stroke;
use crate::vector::VectorPath;
use crate::vector::VectorScaling;

/// An `<svg>` covering its element, with one `<path>` per drawing path.
///
/// Strokes use `vector-effect: non-scaling-stroke`, so their width stays in
/// layout units under any scaling, as on native.
pub(super) fn svg(drawing: Option<&Drawing>) -> AnyView {
	let default = Drawing::default();
	let drawing = drawing.unwrap_or(&default);
	let aspect = match drawing.vector_scaling() {
		VectorScaling::Fit => "xMidYMid meet",
		VectorScaling::Stretch => "none",
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

	svg::path()
		.attr("d", path.data().to_string())
		.attr(
			"fill",
			path.fill_color()
				.map_or_else(|| "none".to_owned(), color_css),
		)
		.attr("stroke", stroke.map(|stroke| color_css(stroke.color())))
		.attr(
			"stroke-width",
			stroke.map(|stroke| stroke.width().to_string()),
		)
		.attr("stroke-linecap", stroke.map(cap))
		.attr("stroke-linejoin", stroke.map(join))
		.attr(
			"stroke-miterlimit",
			stroke.map(|_| Stroke::MITER_LIMIT.to_string()),
		)
		.attr("vector-effect", stroke.map(|_| "non-scaling-stroke"))
		.into_any()
}

fn cap(stroke: Stroke) -> &'static str {
	match stroke.line_cap() {
		LineCap::Butt => "butt",
		LineCap::Round => "round",
		LineCap::Square => "square",
	}
}

fn join(stroke: Stroke) -> &'static str {
	match stroke.line_join() {
		LineJoin::Miter => "miter",
		LineJoin::Round => "round",
		LineJoin::Bevel => "bevel",
	}
}
