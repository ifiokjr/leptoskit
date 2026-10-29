//! Serialize typed base styles below the specificity of utility classes.

use std::fmt::Write;

use leptoskit_yoga::Dimension;
use leptoskit_yoga::Direction;
use leptoskit_yoga::Display;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::Overflow;
use leptoskit_yoga::Rect;
use leptoskit_yoga::TrackSizing;

use crate::Color;
use crate::Element;
use crate::NativeKind;
use crate::TextStyle;
use crate::tailwind::keyword;

/// Native defaults are explicit so browser defaults cannot change layout.
pub(super) fn base(element: &Element, root: bool) -> String {
	let mut css = String::from(
		"appearance:none;margin:0;border:0 solid \
		 transparent;--lk-border-color:currentColor;background:transparent;font:inherit;color:\
		 inherit;text-align:inherit;white-space:pre-wrap;text-decoration:none;",
	);
	widget_defaults(&mut css, element);

	if root {
		write!(
			css,
			"font-family:system-ui,sans-serif;font-size:{}px;font-weight:400;text-align:start;",
			TextStyle::DEFAULT_FONT_SIZE
		)
		.expect("write to String");
	}

	box_layout(&mut css, element);

	if let Some(grid) = &element.style.grid {
		grid_layout(&mut css, grid);
	}

	presentation(&mut css, element);
	css
}

/// The browser defaults a widget kind's element needs replaced, such as list
/// bullets and native control appearance.
fn widget_defaults(css: &mut String, element: &Element) {
	let props = &element.props;

	if matches!(
		super::semantics::tag(
			element.kind,
			props.aria(),
			props.names_itself(),
			element.children.is_empty(),
		),
		super::semantics::Tag::List | super::semantics::Tag::ListItem
	) {
		css.push_str("list-style:none;");
	}

	if matches!(
		element.kind,
		NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Slider
			| NativeKind::LinearProgressIndicator
	) {
		property(css, "appearance", "auto");
		property(css, "accent-color", "currentColor");
	}

	if element.kind == NativeKind::Switch {
		css.push_str(
			"border-radius:999px;background-color:#777;background-image:radial-gradient(circle,#\
			 fff 68%,transparent 72%);background-size:20px \
			 20px;background-repeat:no-repeat;background-position:2px center;",
		);

		if let Some(color) = props.background {
			property(css, "--lk-switch-active", &color_css(color));
		}
	}
}

/// The flexbox and box-model properties of the typed style. A `Text` of
/// styled runs flows them inline, as one wrapping paragraph, so its flex
/// display becomes a block.
fn box_layout(css: &mut String, element: &Element) {
	let style = &element.style;
	let display = match style.display {
		Display::Flex if element.props.spans.is_some() => "block",
		display => keyword::display(display),
	};

	for (name, value) in [
		("display", display),
		("box-sizing", keyword::box_sizing(style.box_sizing)),
		("position", keyword::position(style.position)),
		("direction", direction(style.direction)),
		(
			"flex-direction",
			keyword::flex_direction(style.flex_direction),
		),
		("flex-wrap", keyword::flex_wrap(style.flex_wrap)),
		(
			"justify-content",
			keyword::justify_content(style.justify_content),
		),
		("align-items", keyword::align(style.align_items)),
		("align-self", keyword::align(style.align_self)),
		("align-content", keyword::align(style.align_content)),
		("overflow", overflow(element.kind, style.overflow)),
	] {
		property(css, name, value);
	}

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
		property(css, name, &dimension(value, auto));
	}

	edges(css, "margin", style.margin, "auto");
	edges(css, "padding", style.padding, "0px");

	for (edge, value, border) in [
		("top", style.inset.top, style.border.top),
		("right", style.inset.right, style.border.right),
		("bottom", style.inset.bottom, style.border.bottom),
		("left", style.inset.left, style.border.left),
	] {
		property(css, edge, &dimension(value, "auto"));
		write!(css, "border-{edge}-width:{border}px;").expect("write to String");
	}

	if let Some(ratio) = style.aspect_ratio {
		write!(css, "aspect-ratio:{ratio};").expect("write to String");
	}
}

/// The grid container and grid item properties of the typed style.
fn grid_layout(css: &mut String, grid: &GridStyle) {
	property(
		css,
		"grid-template-columns",
		&tracks(&grid.template_columns),
	);
	property(css, "grid-template-rows", &tracks(&grid.template_rows));
	property(css, "grid-auto-columns", &track(&grid.auto_columns));
	property(css, "grid-auto-rows", &track(&grid.auto_rows));
	property(
		css,
		"grid-auto-flow",
		keyword::grid_auto_flow(grid.auto_flow),
	);
	property(css, "justify-items", keyword::align(grid.justify_items));
	property(css, "justify-self", keyword::align(grid.justify_self));

	for (name, line) in [
		("grid-column-start", grid.column.start),
		("grid-column-end", grid.column.end),
		("grid-row-start", grid.row.start),
		("grid-row-end", grid.row.end),
	] {
		property(css, name, &keyword::grid_line(line));
	}
}

/// The props that change how a box looks rather than where it sits: colors,
/// borders, corners, opacity, text, image fit, and the clip and hidden
/// overrides, which come last so they win over the layout properties.
fn presentation(css: &mut String, element: &Element) {
	let props = &element.props;

	for (name, color) in [
		("color", props.color),
		("background-color", props.background),
		("--lk-border-color", props.border_color),
	] {
		if let Some(color) = color {
			property(css, name, &color_css(color));
		}
	}

	if let Some(widths) = props.border_widths {
		// Per-edge border props paint inside the box too, one inset shadow
		// per drawn edge.
		let shadows = [
			(widths.top, 0.0, 1.0),
			(widths.right, -1.0, 0.0),
			(widths.bottom, 0.0, -1.0),
			(widths.left, 1.0, 0.0),
		]
		.into_iter()
		.filter(|(width, ..)| *width > 0.0)
		.map(|(width, x, y)| {
			format!(
				"inset {}px {}px 0 0 var(--lk-border-color)",
				x * width,
				y * width
			)
		})
		.collect::<Vec<_>>();

		if !shadows.is_empty() {
			property(css, "box-shadow", &shadows.join(","));
		}
	} else if let Some(width) = props.border_width {
		// Legacy border props paint inside the box without affecting layout.
		write!(
			css,
			"box-shadow:inset 0 0 0 {width}px var(--lk-border-color);"
		)
		.expect("write to String");
	}

	if let Some(italic) = props.italic {
		property(css, "font-style", keyword::font_style(italic));
	}

	if let Some(radius) = props.corner_radius {
		write!(css, "border-radius:{radius}px;").expect("write to String");

		property(
			css,
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
		property(css, "font-weight", keyword::font_weight(weight));
	}

	if let Some(alignment) = props.text_align {
		let value = keyword::text_align(alignment);
		property(css, "text-align", value);
		property(css, "--lk-text-align", value);
	} else if element.kind == NativeKind::Button {
		property(css, "text-align", "var(--lk-text-align,center)");
	}

	if element.kind == NativeKind::Image {
		property(css, "object-fit", "contain");
	}

	if props.clip == Some(true) && element.kind != NativeKind::ScrollView {
		property(css, "overflow", "hidden");
	}

	if props.hidden == Some(true) {
		property(css, "display", "none");
	}
}

/// Control decoration stays separate from utility declarations, so shared
/// geometry and colors can still be styled through the usual base props.
///
/// A control without `active:` utilities dims while held, as native
/// controls without a platform highlight do. Browser buttons here never draw
/// native chrome, so every such button dims. The filter multiplies any
/// opacity the element already has, and no utility sets `filter`.
pub(super) fn control(selector: &str, kind: NativeKind, styles_active: bool) -> String {
	let pressed = if crate::element::is_pressable(kind) && !styles_active {
		format!(
			":where({selector}:active:not([aria-disabled=true])){{filter:opacity({});}}",
			crate::element::DEFAULT_PRESSED_OPACITY
		)
	} else {
		String::new()
	};

	match kind {
		NativeKind::Button => pressed,
		NativeKind::Pressable => format!("{selector} *{{pointer-events:none;}}{pressed}"),
		// The drawing covers the element's box and may paint past it, as
		// native drawings do; the element itself stays the hit target.
		NativeKind::Vector => {
			format!(
				"{selector}>svg{{position:absolute;inset:0;width:100%;height:100%;overflow:\
				 visible;pointer-events:none;}}"
			)
		}
		NativeKind::Switch => {
			format!(
				":where({selector}:checked){{background-color:var(--lk-switch-active,#2563eb);\
				 background-position:right 2px \
				 center;}}:where({selector}:focus-visible){{outline:2px solid \
				 Highlight;outline-offset:2px;}}:where({selector}:disabled){{opacity:.5;}}"
			)
		}
		NativeKind::ActivityIndicator => {
			format!(
				"{selector}::before{{content:\"\";box-sizing:border-box;width:100%;height:100%;\
				 border:2px solid \
				 currentColor;border-right-color:transparent;border-radius:50%;\
				 }}{selector}[data-lk-running=false]{{visibility:hidden;\
				 }}{selector}[data-lk-running=true]::before{{animation:lk-spinner .8s linear \
				 infinite;}}@keyframes \
				 lk-spinner{{to{{transform:rotate(360deg);}}}}@media(prefers-reduced-motion:\
				 reduce){{{selector}[data-lk-running=true]::before{{animation-duration:2s;}}}}"
			)
		}
		_ => String::new(),
	}
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

pub(super) fn color_css(color: Color) -> String {
	format!("#{:08x}", color.0)
}

fn direction(value: Direction) -> &'static str {
	match value {
		Direction::Ltr => "ltr",
		Direction::Rtl => "rtl",
	}
}

/// Scroll views and scrolling overflow use `auto`, so scrollbars appear only
/// once the content overflows; hidden overflow clips even a scroll view.
fn overflow(kind: NativeKind, value: Overflow) -> &'static str {
	match (kind, value) {
		(_, Overflow::Hidden) => "hidden",
		(NativeKind::ScrollView, _) | (_, Overflow::Scroll) => "auto",
		_ => "visible",
	}
}

/// The inline style of one styled run of a `Text`.
pub(super) fn span(span: &crate::TextSpan) -> String {
	let mut css = String::new();

	if let Some(color) = span.color {
		property(&mut css, "color", &color_css(color));
	}

	if let Some(background) = span.background {
		property(&mut css, "background-color", &color_css(background));
	}

	if let Some(weight) = span.font_weight {
		property(&mut css, "font-weight", keyword::font_weight(weight));
	}

	if let Some(italic) = span.italic {
		property(&mut css, "font-style", keyword::font_style(italic));
	}

	css
}
