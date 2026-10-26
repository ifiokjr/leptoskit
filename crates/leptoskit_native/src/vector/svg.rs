//! SVG documents imported into drawings.
//!
//! The importer reads the static subset of SVG 1.1 that icons, illustrations,
//! and design tool exports use for shapes: `<svg>`, `<g>`, `<path>`,
//! `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, and
//! `<polygon>`, with transforms, presentation attributes, `style`
//! declarations, CSS colors, opacity, fill rules, and every stroke property.
//! Group transforms are applied to the geometry here, so backends still only
//! see absolute paths. Anything that would change how the document looks but
//! is not supported, such as gradients, text, clipping, or a `<style>`
//! sheet, is an error naming it.

use std::collections::BTreeMap;
use std::fmt;

use super::Affine;
use super::Drawing;
use super::FillRule;
use super::LineCap;
use super::LineJoin;
use super::Paint;
use super::PathData;
use super::Point;
use super::Stroke;
use super::StrokeScaling;
use super::VectorAlign;
use super::VectorAlignment;
use super::VectorError;
use super::VectorPath;
use super::VectorScaling;
use super::ViewBox;
use crate::props::Color;

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// Why an SVG document could not become a drawing.
#[derive(Debug, Clone, PartialEq)]
pub enum SvgError {
	/// The document is not well-formed XML.
	Xml(String),
	/// The root element is not an SVG `<svg>`.
	NotSvg,
	/// The root has neither a `viewBox` nor a numeric `width` and `height`.
	MissingViewBox,
	/// An element that would draw something unsupported, such as `<text>`,
	/// `<use>`, or a gradient outside `<defs>`.
	UnsupportedElement(String),
	/// A property whose value changes rendering in an unsupported way, such
	/// as `clip-path`, `mask`, `filter`, markers, or a gradient paint.
	Unsupported {
		/// The property or attribute name.
		name: String,
		/// Its value.
		value: String,
	},
	/// A value that is not valid for its property.
	Invalid {
		/// The property or attribute name.
		name: String,
		/// Its value.
		value: String,
	},
	/// Geometry that the vector types reject.
	Vector(VectorError),
}

impl fmt::Display for SvgError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Xml(message) => write!(formatter, "the SVG is not well-formed XML: {message}"),
			Self::NotSvg => formatter.write_str("the document's root element is not an SVG <svg>"),
			Self::MissingViewBox => {
				formatter.write_str("the <svg> needs a viewBox, or a numeric width and height")
			}
			Self::UnsupportedElement(name) => {
				write!(formatter, "<{name}> is not supported in vector drawings")
			}
			Self::Unsupported { name, value } => {
				write!(
					formatter,
					"`{name}: {value}` is not supported in vector drawings"
				)
			}
			Self::Invalid { name, value } => write!(formatter, "`{value}` is not a valid {name}"),
			Self::Vector(error) => error.fmt(formatter),
		}
	}
}

impl std::error::Error for SvgError {}

impl From<VectorError> for SvgError {
	fn from(error: VectorError) -> Self {
		Self::Vector(error)
	}
}

/// The inherited painting properties at one element.
#[derive(Clone)]
struct Style {
	fill: Option<Paint>,
	fill_opacity: f32,
	fill_rule: FillRule,
	stroke: Option<Paint>,
	stroke_opacity: f32,
	stroke_width: f32,
	cap: LineCap,
	join: LineJoin,
	miter_limit: f32,
	dashes: Vec<f32>,
	dash_offset: f32,
	/// The `color` property, which `currentColor` refers to. Unset means the
	/// element's own text color decides, at render time.
	color: Option<Color>,
	visible: bool,
	/// The product of every ancestor's `opacity`, which is not inherited but
	/// composes down the tree.
	opacity: f32,
	/// The user-space-to-view-box transform.
	transform: Affine,
}

impl Default for Style {
	fn default() -> Self {
		Self {
			fill: Some(Paint::Color(Color::rgba(0, 0, 0, 255))),
			fill_opacity: 1.0,
			fill_rule: FillRule::Nonzero,
			stroke: None,
			stroke_opacity: 1.0,
			stroke_width: 1.0,
			cap: LineCap::Butt,
			join: LineJoin::Miter,
			miter_limit: Stroke::MITER_LIMIT,
			dashes: Vec::new(),
			dash_offset: 0.0,
			color: None,
			visible: true,
			opacity: 1.0,
			transform: Affine::IDENTITY,
		}
	}
}

/// The viewport that percentages resolve against.
#[derive(Clone, Copy)]
struct Viewport {
	width: f32,
	height: f32,
}

impl Viewport {
	/// SVG's normalized diagonal, for percentages of radii and stroke widths.
	fn diagonal(self) -> f32 {
		(self.width.hypot(self.height)) / std::f32::consts::SQRT_2
	}
}

pub(super) fn parse(document: &str) -> Result<Drawing, SvgError> {
	let tree =
		roxmltree::Document::parse(document).map_err(|error| SvgError::Xml(error.to_string()))?;
	let root = tree.root_element();

	if root.tag_name().namespace() != Some(SVG_NAMESPACE) || root.tag_name().name() != "svg" {
		return Err(SvgError::NotSvg);
	}

	let view_box = view_box(root)?;
	let viewport = Viewport {
		width: view_box.width(),
		height: view_box.height(),
	};
	let (scaling, alignment) = match root.attribute("preserveAspectRatio") {
		Some(value) => aspect_ratio(value)?,
		None => (VectorScaling::Fit, VectorAlignment::CENTER),
	};
	let mut drawing = Drawing::new(view_box).scaling(scaling).align(alignment);
	let style = element_style(root, &Style::default(), viewport)?;
	children(root, &style, viewport, &mut drawing)?;
	Ok(drawing)
}

fn view_box(root: roxmltree::Node<'_, '_>) -> Result<ViewBox, SvgError> {
	if let Some(value) = root.attribute("viewBox") {
		let numbers = numbers("viewBox", value)?;
		let &[min_x, min_y, width, height] = numbers.as_slice() else {
			return Err(invalid("viewBox", value));
		};
		return Ok(ViewBox::new(min_x, min_y, width, height)?);
	}

	let size = |name| {
		root.attribute(name)
			.and_then(|value| absolute_length(value.trim()))
	};

	match (size("width"), size("height")) {
		(Some(width), Some(height)) => Ok(ViewBox::new(0.0, 0.0, width, height)?),
		_ => Err(SvgError::MissingViewBox),
	}
}

fn aspect_ratio(value: &str) -> Result<(VectorScaling, VectorAlignment), SvgError> {
	let mut words = value.split_ascii_whitespace();
	let align = words.next().unwrap_or("xMidYMid");

	if align == "none" {
		return Ok((VectorScaling::Stretch, VectorAlignment::CENTER));
	}

	let axis = |text: &str| {
		match text {
			"Min" => Some(VectorAlign::Start),
			"Mid" => Some(VectorAlign::Center),
			"Max" => Some(VectorAlign::End),
			_ => None,
		}
	};
	let alignment = align
		.strip_prefix('x')
		.filter(|rest| rest.len() == 7 && rest.as_bytes()[3] == b'Y')
		.and_then(|rest| Some(VectorAlignment::new(axis(&rest[..3])?, axis(&rest[4..])?)))
		.ok_or_else(|| invalid("preserveAspectRatio", value))?;
	let scaling = match words.next() {
		None | Some("meet") => VectorScaling::Fit,
		Some("slice") => VectorScaling::Fill,
		Some(_) => return Err(invalid("preserveAspectRatio", value)),
	};

	Ok((scaling, alignment))
}

fn children(
	parent: roxmltree::Node<'_, '_>,
	style: &Style,
	viewport: Viewport,
	drawing: &mut Drawing,
) -> Result<(), SvgError> {
	for node in parent.children().filter(roxmltree::Node::is_element) {
		// Editor metadata, such as Inkscape's, lives in other namespaces and
		// draws nothing.
		if node.tag_name().namespace() != Some(SVG_NAMESPACE) {
			continue;
		}

		let name = node.tag_name().name();

		if matches!(name, "defs" | "title" | "desc" | "metadata") {
			continue;
		}

		if property(node, "display").as_deref() == Some("none") {
			continue;
		}

		let style = element_style(node, style, viewport)?;

		match name {
			"g" => children(node, &style, viewport, drawing)?,
			"path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => {
				if let Some(path) = shape(node, &style, viewport)? {
					drawing.push(path);
				}
			}
			other => return Err(SvgError::UnsupportedElement(other.to_owned())),
		}
	}

	Ok(())
}

/// Reads a property from the `style` attribute, which wins, or from the
/// presentation attribute of the same name.
fn property(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
	let declared = node.attribute("style").and_then(|style| {
		style
			.split(';')
			.filter_map(|declaration| declaration.split_once(':'))
			.filter(|(property, _)| property.trim() == name)
			.map(|(_, value)| {
				value
					.trim()
					.trim_end_matches("!important")
					.trim()
					.to_owned()
			})
			.next_back()
	});

	declared.or_else(|| node.attribute(name).map(|value| value.trim().to_owned()))
}

/// The style an element computes from its parent's and its own properties.
fn element_style(
	node: roxmltree::Node<'_, '_>,
	parent: &Style,
	viewport: Viewport,
) -> Result<Style, SvgError> {
	let mut style = parent.clone();
	let read = |name: &str| property(node, name).filter(|value| value != "inherit");

	for name in ["clip-path", "mask", "filter"] {
		if let Some(value) = read(name).filter(|value| value != "none") {
			return Err(SvgError::Unsupported {
				name: name.to_owned(),
				value,
			});
		}
	}

	for name in ["marker-start", "marker-mid", "marker-end", "marker"] {
		if let Some(value) = read(name).filter(|value| value != "none") {
			return Err(SvgError::Unsupported {
				name: name.to_owned(),
				value,
			});
		}
	}

	for (name, normal) in [("paint-order", "normal"), ("mix-blend-mode", "normal")] {
		if let Some(value) = read(name).filter(|value| value != normal) {
			return Err(SvgError::Unsupported {
				name: name.to_owned(),
				value,
			});
		}
	}

	if let Some(value) = read("color") {
		style.color = Some(color("color", &value)?);
	}

	if let Some(value) = read("fill") {
		style.fill = paint("fill", &value, style.color)?;
	}

	if let Some(value) = read("stroke") {
		style.stroke = paint("stroke", &value, style.color)?;
	}

	if let Some(value) = read("fill-opacity") {
		style.fill_opacity = opacity("fill-opacity", &value)?;
	}

	if let Some(value) = read("stroke-opacity") {
		style.stroke_opacity = opacity("stroke-opacity", &value)?;
	}

	if let Some(value) = read("opacity") {
		style.opacity *= opacity("opacity", &value)?;
	}

	if let Some(value) = read("fill-rule") {
		style.fill_rule = match value.as_str() {
			"nonzero" => FillRule::Nonzero,
			"evenodd" => FillRule::Evenodd,
			_ => return Err(invalid("fill-rule", &value)),
		};
	}

	if let Some(value) = read("stroke-width") {
		style.stroke_width = length("stroke-width", &value, viewport.diagonal())?;
	}

	if let Some(value) = read("stroke-linecap") {
		style.cap = match value.as_str() {
			"butt" => LineCap::Butt,
			"round" => LineCap::Round,
			"square" => LineCap::Square,
			_ => return Err(invalid("stroke-linecap", &value)),
		};
	}

	if let Some(value) = read("stroke-linejoin") {
		style.join = match value.as_str() {
			"miter" | "miter-clip" => LineJoin::Miter,
			"round" => LineJoin::Round,
			"bevel" => LineJoin::Bevel,
			_ => return Err(invalid("stroke-linejoin", &value)),
		};
	}

	if let Some(value) = read("stroke-miterlimit") {
		style.miter_limit = number("stroke-miterlimit", &value)?;
	}

	if let Some(value) = read("stroke-dasharray") {
		style.dashes = if value == "none" {
			Vec::new()
		} else {
			value
				.split(|c: char| c == ',' || c.is_ascii_whitespace())
				.filter(|part| !part.is_empty())
				.map(|part| length("stroke-dasharray", part, viewport.diagonal()))
				.collect::<Result<_, _>>()?
		};
	}

	if let Some(value) = read("stroke-dashoffset") {
		style.dash_offset = length("stroke-dashoffset", &value, viewport.diagonal())?;
	}

	if let Some(value) = read("visibility") {
		style.visible = match value.as_str() {
			"visible" => true,
			"hidden" | "collapse" => false,
			_ => return Err(invalid("visibility", &value)),
		};
	}

	if let Some(value) = node.attribute("transform") {
		style.transform = parent.transform.then(Affine::parse(value)?);
	}

	Ok(style)
}

/// One shape element as a painted path, or `None` when it draws nothing.
fn shape(
	node: roxmltree::Node<'_, '_>,
	style: &Style,
	viewport: Viewport,
) -> Result<Option<VectorPath>, SvgError> {
	let size = |name: &str, basis: f32| -> Result<f32, SvgError> {
		node.attribute(name)
			.map_or(Ok(0.0), |value| length(name, value, basis))
	};
	let (width, height, diagonal) = (viewport.width, viewport.height, viewport.diagonal());

	let data = match node.tag_name().name() {
		"path" => PathData::parse(node.attribute("d").unwrap_or_default())?,
		"rect" => {
			let (x, y) = (size("x", width)?, size("y", height)?);
			let (w, h) = (size("width", width)?, size("height", height)?);
			let rx = node.attribute("rx").map(|value| length("rx", value, width));
			let ry = node
				.attribute("ry")
				.map(|value| length("ry", value, height));
			// A missing radius takes the other's value, as in SVG.
			let (rx, ry) = match (rx.transpose()?, ry.transpose()?) {
				(Some(rx), Some(ry)) => (rx, ry),
				(Some(radius), None) | (None, Some(radius)) => (radius, radius),
				(None, None) => (0.0, 0.0),
			};

			if w == 0.0 || h == 0.0 {
				return Ok(None);
			}

			PathData::rounded_rect(x, y, w, h, rx, ry)?
		}
		"circle" => {
			let r = size("r", diagonal)?;

			if r == 0.0 {
				return Ok(None);
			}

			PathData::circle(size("cx", width)?, size("cy", height)?, r)?
		}
		"ellipse" => {
			let (rx, ry) = (size("rx", width)?, size("ry", height)?);

			if rx == 0.0 || ry == 0.0 {
				return Ok(None);
			}

			PathData::ellipse(size("cx", width)?, size("cy", height)?, rx, ry)?
		}
		"line" => {
			PathData::line(
				Point::new(size("x1", width)?, size("y1", height)?),
				Point::new(size("x2", width)?, size("y2", height)?),
			)?
		}
		polygon @ ("polyline" | "polygon") => {
			let value = node.attribute("points").unwrap_or_default();
			let numbers = numbers("points", value)?;
			// An odd trailing coordinate is ignored, as SVG specifies.
			let points = numbers
				.chunks_exact(2)
				.map(|pair| Point::new(pair[0], pair[1]));

			if polygon == "polygon" {
				PathData::polygon(points)?
			} else {
				PathData::polyline(points)?
			}
		}
		other => unreachable!("<{other}> is not a shape element"),
	};

	if data.is_empty() || !style.visible {
		return Ok(None);
	}

	let data = if style.transform == Affine::IDENTITY {
		data
	} else {
		data.transform(style.transform)
	};
	let mut path = VectorPath::new(data).fill_rule(style.fill_rule);

	if let Some(fill) = style.fill {
		path = path.fill(faded(fill, style.fill_opacity * style.opacity));
	}

	if let Some(stroke) = style.stroke.filter(|_| style.stroke_width > 0.0) {
		let non_scaling = property(node, "vector-effect").as_deref() == Some("non-scaling-stroke");
		// A scaling stroke's width is in the shape's own user space; the
		// drawing scales it from view box units onwards.
		let scale = if non_scaling {
			1.0
		} else {
			style.transform.length_scale()
		};
		let dashes = style
			.dashes
			.iter()
			.map(|length| length * scale)
			.collect::<Vec<_>>();
		path = path.stroke(
			Stroke::new(
				faded(stroke, style.stroke_opacity * style.opacity),
				style.stroke_width * scale,
			)?
			.cap(style.cap)
			.join(style.join)
			.miter_limit(style.miter_limit)?
			.dashes(&dashes, style.dash_offset * scale)?
			.scaling(if non_scaling {
				StrokeScaling::Fixed
			} else {
				StrokeScaling::Drawing
			}),
		);
	}

	Ok(Some(path))
}

/// Folds an opacity into a paint's alpha. `currentColor` keeps following
/// the element's color, so only a fixed color can fade.
fn faded(paint: Paint, opacity: f32) -> Paint {
	match paint {
		Paint::Color(color) if opacity < 1.0 => {
			let alpha = (f32::from(color.alpha()) * opacity).round() as u8;
			Paint::Color(Color::rgba(color.red(), color.green(), color.blue(), alpha))
		}
		other => other,
	}
}

fn paint(name: &str, value: &str, color: Option<Color>) -> Result<Option<Paint>, SvgError> {
	match value {
		"none" => Ok(None),
		"currentColor" | "currentcolor" => {
			Ok(Some(color.map_or(Paint::CurrentColor, Paint::Color)))
		}
		_ if value.starts_with("url(") => {
			Err(SvgError::Unsupported {
				name: name.to_owned(),
				value: value.to_owned(),
			})
		}
		_ => Ok(Some(Paint::Color(self::color(name, value)?))),
	}
}

fn invalid(name: &str, value: &str) -> SvgError {
	SvgError::Invalid {
		name: name.to_owned(),
		value: value.to_owned(),
	}
}

fn number(name: &str, value: &str) -> Result<f32, SvgError> {
	value
		.trim()
		.parse::<f32>()
		.ok()
		.filter(|number| number.is_finite())
		.ok_or_else(|| invalid(name, value))
}

fn numbers(name: &str, value: &str) -> Result<Vec<f32>, SvgError> {
	value
		.split(|c: char| c == ',' || c.is_ascii_whitespace())
		.filter(|part| !part.is_empty())
		.map(|part| number(name, part))
		.collect()
}

fn opacity(name: &str, value: &str) -> Result<f32, SvgError> {
	let opacity = match value.strip_suffix('%') {
		Some(percent) => number(name, percent)? / 100.0,
		None => number(name, value)?,
	};
	Ok(opacity.clamp(0.0, 1.0))
}

/// A length in user units: a number, an absolute unit at CSS's 96 pixels
/// per inch, or a percentage of `basis`. Font-relative units have no font to
/// resolve against, so they are rejected.
fn length(name: &str, value: &str, basis: f32) -> Result<f32, SvgError> {
	let value = value.trim();

	if let Some(percent) = value.strip_suffix('%') {
		return Ok(number(name, percent)? / 100.0 * basis);
	}

	absolute_length(value).ok_or_else(|| invalid(name, value))
}

fn absolute_length(value: &str) -> Option<f32> {
	let split = value
		.find(|c: char| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
		.unwrap_or(value.len());
	let (number, unit) = value.split_at(split);
	let number = number.trim().parse::<f32>().ok()?;
	let scale = match unit {
		"" | "px" => 1.0,
		"pt" => 96.0 / 72.0,
		"pc" => 16.0,
		"in" => 96.0,
		"cm" => 96.0 / 2.54,
		"mm" => 96.0 / 25.4,
		"q" | "Q" => 96.0 / 101.6,
		_ => return None,
	};
	Some(number * scale).filter(|length| length.is_finite())
}

/// A CSS color: hex, `rgb()`, `rgba()`, `hsl()`, `hsla()`, `transparent`, or
/// one of the 148 named colors.
fn color(name: &str, value: &str) -> Result<Color, SvgError> {
	parse_color(value.trim()).ok_or_else(|| invalid(name, value))
}

fn parse_color(value: &str) -> Option<Color> {
	if let Some(hex) = value.strip_prefix('#') {
		let digit = |index: usize| u8::from_str_radix(hex.get(index..=index)?, 16).ok();
		let pair = |index: usize| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok();
		return match hex.len() {
			3 | 4 => {
				let channel = |index| digit(index).map(|value| value * 17);
				let alpha = if hex.len() == 4 { channel(3)? } else { 255 };
				Some(Color::rgba(channel(0)?, channel(1)?, channel(2)?, alpha))
			}
			6 | 8 => {
				let alpha = if hex.len() == 8 { pair(6)? } else { 255 };
				Some(Color::rgba(pair(0)?, pair(2)?, pair(4)?, alpha))
			}
			_ => None,
		};
	}

	let lower = value.to_ascii_lowercase();

	if let Some((function, arguments)) = lower
		.strip_suffix(')')
		.and_then(|value| value.split_once('('))
	{
		return function_color(function.trim(), arguments);
	}

	if lower == "transparent" {
		return Some(Color::rgba(0, 0, 0, 0));
	}

	named_colors()
		.get(lower.as_str())
		.map(|&packed| Color((packed << 8) | 0xff))
}

/// `rgb()`, `rgba()`, `hsl()`, and `hsla()`, in comma or space syntax with an
/// optional `/ alpha`.
fn function_color(function: &str, arguments: &str) -> Option<Color> {
	let parts = arguments
		.split(|c: char| c == ',' || c == '/' || c.is_ascii_whitespace())
		.filter(|part| !part.is_empty())
		.collect::<Vec<_>>();
	let (channels, alpha) = match parts.as_slice() {
		[a, b, c] => ([*a, *b, *c], None),
		[a, b, c, alpha] => ([*a, *b, *c], Some(*alpha)),
		_ => return None,
	};
	let fraction = |part: &str| -> Option<f32> {
		match part.strip_suffix('%') {
			Some(percent) => percent.parse::<f32>().ok().map(|value| value / 100.0),
			None => part.parse::<f32>().ok(),
		}
	};
	let alpha = alpha.map_or(Some(1.0), fraction)?.clamp(0.0, 1.0);
	let byte = |unit: f32| (unit.clamp(0.0, 1.0) * 255.0).round() as u8;

	let [red, green, blue] = match function {
		"rgb" | "rgba" => {
			let channel = |part: &str| -> Option<f32> {
				match part.strip_suffix('%') {
					Some(percent) => percent.parse::<f32>().ok().map(|value| value / 100.0),
					None => part.parse::<f32>().ok().map(|value| value / 255.0),
				}
			};
			[
				channel(channels[0])?,
				channel(channels[1])?,
				channel(channels[2])?,
			]
		}
		"hsl" | "hsla" => {
			let hue = channels[0].trim_end_matches("deg").parse::<f32>().ok()?;
			hsl_to_rgb(
				hue.rem_euclid(360.0) / 360.0,
				fraction(channels[1])?.clamp(0.0, 1.0),
				fraction(channels[2])?.clamp(0.0, 1.0),
			)
		}
		_ => return None,
	};

	[red, green, blue, alpha]
		.iter()
		.all(|value| value.is_finite())
		.then(|| Color::rgba(byte(red), byte(green), byte(blue), byte(alpha)))
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
	let chroma = (1.0 - 2.0f32.mul_add(lightness, -1.0).abs()) * saturation;
	let channel = |offset: f32| {
		let k = (offset + hue * 12.0) % 12.0;
		lightness - chroma / 2.0 * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
	};
	[channel(0.0), channel(8.0), channel(4.0)]
}

/// CSS's named colors as packed `0xRRGGBB`.
fn named_colors() -> &'static BTreeMap<&'static str, u32> {
	static COLORS: std::sync::OnceLock<BTreeMap<&'static str, u32>> = std::sync::OnceLock::new();
	COLORS.get_or_init(|| NAMED_COLORS.iter().copied().collect())
}

#[expect(
	clippy::unreadable_literal,
	reason = "CSS writes these colors as six hex digits, unseparated"
)]
const NAMED_COLORS: &[(&str, u32)] = &[
	("aliceblue", 0xf0f8ff),
	("antiquewhite", 0xfaebd7),
	("aqua", 0x00ffff),
	("aquamarine", 0x7fffd4),
	("azure", 0xf0ffff),
	("beige", 0xf5f5dc),
	("bisque", 0xffe4c4),
	("black", 0x000000),
	("blanchedalmond", 0xffebcd),
	("blue", 0x0000ff),
	("blueviolet", 0x8a2be2),
	("brown", 0xa52a2a),
	("burlywood", 0xdeb887),
	("cadetblue", 0x5f9ea0),
	("chartreuse", 0x7fff00),
	("chocolate", 0xd2691e),
	("coral", 0xff7f50),
	("cornflowerblue", 0x6495ed),
	("cornsilk", 0xfff8dc),
	("crimson", 0xdc143c),
	("cyan", 0x00ffff),
	("darkblue", 0x00008b),
	("darkcyan", 0x008b8b),
	("darkgoldenrod", 0xb8860b),
	("darkgray", 0xa9a9a9),
	("darkgreen", 0x006400),
	("darkgrey", 0xa9a9a9),
	("darkkhaki", 0xbdb76b),
	("darkmagenta", 0x8b008b),
	("darkolivegreen", 0x556b2f),
	("darkorange", 0xff8c00),
	("darkorchid", 0x9932cc),
	("darkred", 0x8b0000),
	("darksalmon", 0xe9967a),
	("darkseagreen", 0x8fbc8f),
	("darkslateblue", 0x483d8b),
	("darkslategray", 0x2f4f4f),
	("darkslategrey", 0x2f4f4f),
	("darkturquoise", 0x00ced1),
	("darkviolet", 0x9400d3),
	("deeppink", 0xff1493),
	("deepskyblue", 0x00bfff),
	("dimgray", 0x696969),
	("dimgrey", 0x696969),
	("dodgerblue", 0x1e90ff),
	("firebrick", 0xb22222),
	("floralwhite", 0xfffaf0),
	("forestgreen", 0x228b22),
	("fuchsia", 0xff00ff),
	("gainsboro", 0xdcdcdc),
	("ghostwhite", 0xf8f8ff),
	("gold", 0xffd700),
	("goldenrod", 0xdaa520),
	("gray", 0x808080),
	("green", 0x008000),
	("greenyellow", 0xadff2f),
	("grey", 0x808080),
	("honeydew", 0xf0fff0),
	("hotpink", 0xff69b4),
	("indianred", 0xcd5c5c),
	("indigo", 0x4b0082),
	("ivory", 0xfffff0),
	("khaki", 0xf0e68c),
	("lavender", 0xe6e6fa),
	("lavenderblush", 0xfff0f5),
	("lawngreen", 0x7cfc00),
	("lemonchiffon", 0xfffacd),
	("lightblue", 0xadd8e6),
	("lightcoral", 0xf08080),
	("lightcyan", 0xe0ffff),
	("lightgoldenrodyellow", 0xfafad2),
	("lightgray", 0xd3d3d3),
	("lightgreen", 0x90ee90),
	("lightgrey", 0xd3d3d3),
	("lightpink", 0xffb6c1),
	("lightsalmon", 0xffa07a),
	("lightseagreen", 0x20b2aa),
	("lightskyblue", 0x87cefa),
	("lightslategray", 0x778899),
	("lightslategrey", 0x778899),
	("lightsteelblue", 0xb0c4de),
	("lightyellow", 0xffffe0),
	("lime", 0x00ff00),
	("limegreen", 0x32cd32),
	("linen", 0xfaf0e6),
	("magenta", 0xff00ff),
	("maroon", 0x800000),
	("mediumaquamarine", 0x66cdaa),
	("mediumblue", 0x0000cd),
	("mediumorchid", 0xba55d3),
	("mediumpurple", 0x9370db),
	("mediumseagreen", 0x3cb371),
	("mediumslateblue", 0x7b68ee),
	("mediumspringgreen", 0x00fa9a),
	("mediumturquoise", 0x48d1cc),
	("mediumvioletred", 0xc71585),
	("midnightblue", 0x191970),
	("mintcream", 0xf5fffa),
	("mistyrose", 0xffe4e1),
	("moccasin", 0xffe4b5),
	("navajowhite", 0xffdead),
	("navy", 0x000080),
	("oldlace", 0xfdf5e6),
	("olive", 0x808000),
	("olivedrab", 0x6b8e23),
	("orange", 0xffa500),
	("orangered", 0xff4500),
	("orchid", 0xda70d6),
	("palegoldenrod", 0xeee8aa),
	("palegreen", 0x98fb98),
	("paleturquoise", 0xafeeee),
	("palevioletred", 0xdb7093),
	("papayawhip", 0xffefd5),
	("peachpuff", 0xffdab9),
	("peru", 0xcd853f),
	("pink", 0xffc0cb),
	("plum", 0xdda0dd),
	("powderblue", 0xb0e0e6),
	("purple", 0x800080),
	("rebeccapurple", 0x663399),
	("red", 0xff0000),
	("rosybrown", 0xbc8f8f),
	("royalblue", 0x4169e1),
	("saddlebrown", 0x8b4513),
	("salmon", 0xfa8072),
	("sandybrown", 0xf4a460),
	("seagreen", 0x2e8b57),
	("seashell", 0xfff5ee),
	("sienna", 0xa0522d),
	("silver", 0xc0c0c0),
	("skyblue", 0x87ceeb),
	("slateblue", 0x6a5acd),
	("slategray", 0x708090),
	("slategrey", 0x708090),
	("snow", 0xfffafa),
	("springgreen", 0x00ff7f),
	("steelblue", 0x4682b4),
	("tan", 0xd2b48c),
	("teal", 0x008080),
	("thistle", 0xd8bfd8),
	("tomato", 0xff6347),
	("turquoise", 0x40e0d0),
	("violet", 0xee82ee),
	("wheat", 0xf5deb3),
	("white", 0xffffff),
	("whitesmoke", 0xf5f5f5),
	("yellow", 0xffff00),
	("yellowgreen", 0x9acd32),
];
