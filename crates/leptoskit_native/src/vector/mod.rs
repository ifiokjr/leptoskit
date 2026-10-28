//! Vector drawings: filled and stroked SVG-style paths that every backend
//! draws natively.
//!
//! A [`Drawing`] holds paths in its own coordinate space, the [`ViewBox`], and
//! maps that space onto the element's laid-out frame. [`VectorScaling::Fit`]
//! scales uniformly so the whole drawing shows, [`VectorScaling::Fill`]
//! scales uniformly so the drawing covers the frame, and
//! [`VectorScaling::Stretch`] scales each axis independently. The first two
//! place the drawing by its [`VectorAlign`]ment, like SVG's
//! `preserveAspectRatio`.
//!
//! Path data is parsed once, here, into absolute commands. Backends never parse
//! SVG: they ask the drawing for its [`Transform`] onto the frame and replay
//! the transformed [`PathCommand`]s through the platform's path API. Paths and
//! drawings are immutable and shared, so cloning them, diffing an unchanged
//! drawing, and re-rendering a screen that reuses a parsed icon cost a pointer
//! copy or comparison rather than a walk over every point.
//!
//! ```
//! use leptoskit_native::Color;
//! use leptoskit_native::vector::Drawing;
//! use leptoskit_native::vector::PathData;
//! use leptoskit_native::vector::Stroke;
//! use leptoskit_native::vector::VectorPath;
//! use leptoskit_native::vector::ViewBox;
//!
//! let ink = Color::rgba(17, 17, 17, 255);
//! let drawing = Drawing::new(ViewBox::new(0.0, 0.0, 24.0, 24.0)?)
//! 	.path(
//! 		VectorPath::new(PathData::parse("M4 12 C8 4 16 20 20 12")?)
//! 			.stroke(Stroke::new(ink, 2.0)?.round().dashes(&[4.0, 2.0], 0.0)?),
//! 	)
//! 	.path(VectorPath::new(PathData::circle(12.0, 12.0, 3.0)?).fill(ink));
//!
//! assert_eq!(drawing.paths().len(), 2);
//! # Ok::<(), leptoskit_native::vector::VectorError>(())
//! ```

use std::fmt;
use std::sync::Arc;

use crate::props::Color;

mod path;
#[cfg(feature = "svg")]
mod svg;

pub use path::Affine;
pub use path::PathCommand;
pub use path::PathData;
pub use path::Point;
#[cfg(feature = "svg")]
pub use svg::SvgError;

/// Invalid vector drawing input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorError {
	/// A view box needs finite coordinates and a positive, finite size.
	ViewBox,
	/// A stroke width must be finite and greater than zero.
	StrokeWidth,
	/// A miter limit must be finite and at least one.
	MiterLimit,
	/// Dash lengths and the dash offset must be finite, and lengths must not
	/// be negative.
	DashArray,
	/// A path coordinate must be finite.
	NonFiniteCoordinate,
	/// A shape needs finite coordinates and finite, non-negative sizes.
	ShapeSize,
	/// A subpath must begin with a move-to command.
	MissingMoveTo,
	/// Path data is malformed at this byte offset.
	PathData {
		/// The byte offset of the first character that could not be read.
		offset: usize,
	},
	/// A transform list is not valid SVG.
	Transform,
}

impl fmt::Display for VectorError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::ViewBox => {
				formatter.write_str(
					"a view box needs finite coordinates and a positive, finite width and height",
				)
			}
			Self::StrokeWidth => formatter.write_str("a stroke width must be finite and positive"),
			Self::MiterLimit => formatter.write_str("a miter limit must be finite and at least 1"),
			Self::DashArray => {
				formatter.write_str(
					"dash lengths must be finite and non-negative, and the dash offset finite",
				)
			}
			Self::NonFiniteCoordinate => formatter.write_str("path coordinates must be finite"),
			Self::ShapeSize => {
				formatter.write_str(
					"shapes need finite coordinates and finite, non-negative sizes and radii",
				)
			}
			Self::MissingMoveTo => formatter.write_str("path data must start with a move-to (M)"),
			Self::PathData { offset } => write!(formatter, "invalid path data at byte {offset}"),
			Self::Transform => {
				formatter.write_str(
					"a transform list must use matrix, translate, scale, rotate, skewX, or skewY \
					 with finite arguments",
				)
			}
		}
	}
}

impl std::error::Error for VectorError {}

/// What a fill or stroke paints with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
	/// A fixed color.
	Color(Color),
	/// The element's text color, as SVG's `currentColor`: a `Vector` styled
	/// with `text-indigo-600` paints its `CurrentColor` paths indigo, and
	/// follows the color it inherits from its parents.
	CurrentColor,
}

impl Paint {
	/// The color to paint with: a fixed color as given, and `CurrentColor`
	/// as CSS's initial black when no text color resolved it.
	#[must_use]
	pub const fn color(self) -> Color {
		match self {
			Self::Color(color) => color,
			Self::CurrentColor => BLACK,
		}
	}

	fn resolved(self, current: Color) -> Self {
		match self {
			Self::CurrentColor => Self::Color(current),
			fixed @ Self::Color(_) => fixed,
		}
	}
}

impl From<Color> for Paint {
	fn from(color: Color) -> Self {
		Self::Color(color)
	}
}

/// The wire form: a packed `0xRRGGBBAA` number, or `"current_color"`.
#[cfg(feature = "serde")]
impl serde::Serialize for Paint {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			Self::Color(color) => serializer.serialize_u32(color.0),
			Self::CurrentColor => serializer.serialize_str("current_color"),
		}
	}
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Paint {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(serde::Deserialize)]
		#[serde(untagged)]
		enum Wire {
			Color(u32),
			Keyword(String),
		}

		match Wire::deserialize(deserializer)? {
			Wire::Color(packed) => Ok(Self::Color(Color(packed))),
			Wire::Keyword(keyword) if keyword == "current_color" => Ok(Self::CurrentColor),
			Wire::Keyword(other) => {
				Err(serde::de::Error::custom(format!(
					"unknown paint `{other}`: expected a packed color or `current_color`"
				)))
			}
		}
	}
}

const BLACK: Color = Color::rgba(0, 0, 0, 255);

/// Which regions of a self-intersecting or nested path count as inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum FillRule {
	/// Inside wherever the outline winds around a point a nonzero number of
	/// times; SVG's default.
	#[default]
	Nonzero,
	/// Inside wherever a ray from a point crosses the outline an odd number
	/// of times, so nested shapes punch holes whatever their direction.
	Evenodd,
}

/// The rectangle of drawing coordinates mapped onto the element's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "ViewBoxFields", into = "ViewBoxFields")
)]
pub struct ViewBox {
	min_x: f32,
	min_y: f32,
	width: f32,
	height: f32,
}

impl ViewBox {
	/// The unit square, `0 0 1 1`.
	pub const UNIT: Self = Self {
		min_x: 0.0,
		min_y: 0.0,
		width: 1.0,
		height: 1.0,
	};

	/// Validates a view box, written in SVG's `min-x min-y width height`
	/// order.
	pub fn new(min_x: f32, min_y: f32, width: f32, height: f32) -> Result<Self, VectorError> {
		if ![min_x, min_y, width, height]
			.into_iter()
			.all(f32::is_finite)
			|| width <= 0.0
			|| height <= 0.0
		{
			return Err(VectorError::ViewBox);
		}

		Ok(Self {
			min_x,
			min_y,
			width,
			height,
		})
	}

	/// The left edge.
	pub const fn min_x(self) -> f32 {
		self.min_x
	}

	/// The top edge.
	pub const fn min_y(self) -> f32 {
		self.min_y
	}

	/// The positive width.
	pub const fn width(self) -> f32 {
		self.width
	}

	/// The positive height.
	pub const fn height(self) -> f32 {
		self.height
	}
}

impl Default for ViewBox {
	fn default() -> Self {
		Self::UNIT
	}
}

impl fmt::Display for ViewBox {
	/// The SVG `viewBox` attribute value.
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			formatter,
			"{} {} {} {}",
			self.min_x, self.min_y, self.width, self.height
		)
	}
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct ViewBoxFields {
	min_x: f32,
	min_y: f32,
	width: f32,
	height: f32,
}

#[cfg(feature = "serde")]
impl TryFrom<ViewBoxFields> for ViewBox {
	type Error = VectorError;

	fn try_from(fields: ViewBoxFields) -> Result<Self, Self::Error> {
		Self::new(fields.min_x, fields.min_y, fields.width, fields.height)
	}
}

#[cfg(feature = "serde")]
impl From<ViewBox> for ViewBoxFields {
	fn from(view_box: ViewBox) -> Self {
		Self {
			min_x: view_box.min_x,
			min_y: view_box.min_y,
			width: view_box.width,
			height: view_box.height,
		}
	}
}

/// How a drawing's view box maps onto the element's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum VectorScaling {
	/// Scales both axes by the same factor, as large as fits, so the whole
	/// view box shows: SVG's `meet`.
	#[default]
	Fit,
	/// Scales both axes by the same factor, as small as covers the frame, so
	/// the drawing fills it and its overflow is cropped when the element
	/// clips: SVG's `slice`.
	Fill,
	/// Scales each axis independently so the view box fills the frame
	/// exactly: SVG's `preserveAspectRatio="none"`. Alignment does not apply.
	Stretch,
}

/// Where a fitted or filled drawing sits along one axis of its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum VectorAlign {
	/// Against the left or top edge: SVG's `xMin` or `YMin`.
	Start,
	/// Centred: SVG's `xMid` or `YMid`.
	#[default]
	Center,
	/// Against the right or bottom edge: SVG's `xMax` or `YMax`.
	End,
}

impl VectorAlign {
	/// The share of the spare space placed before the drawing.
	#[must_use]
	pub const fn factor(self) -> f32 {
		match self {
			Self::Start => 0.0,
			Self::Center => 0.5,
			Self::End => 1.0,
		}
	}
}

/// The alignment of a drawing on both axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VectorAlignment {
	/// The horizontal alignment.
	pub x: VectorAlign,
	/// The vertical alignment.
	pub y: VectorAlign,
}

impl VectorAlignment {
	/// Centred on both axes, SVG's default `xMidYMid`.
	pub const CENTER: Self = Self::new(VectorAlign::Center, VectorAlign::Center);

	/// An alignment from its horizontal and vertical parts.
	#[must_use]
	pub const fn new(x: VectorAlign, y: VectorAlign) -> Self {
		Self { x, y }
	}

	/// The SVG `preserveAspectRatio` alignment keyword, such as `xMinYMax`.
	#[must_use]
	pub const fn svg_keyword(self) -> &'static str {
		use VectorAlign::Center;
		use VectorAlign::End;
		use VectorAlign::Start;

		match (self.x, self.y) {
			(Start, Start) => "xMinYMin",
			(Start, Center) => "xMinYMid",
			(Start, End) => "xMinYMax",
			(Center, Start) => "xMidYMin",
			(Center, Center) => "xMidYMid",
			(Center, End) => "xMidYMax",
			(End, Start) => "xMaxYMin",
			(End, Center) => "xMaxYMid",
			(End, End) => "xMaxYMax",
		}
	}

	#[cfg(feature = "serde")]
	#[expect(
		clippy::trivially_copy_pass_by_ref,
		reason = "serde passes a reference"
	)]
	fn is_center(&self) -> bool {
		*self == Self::CENTER
	}
}

/// The shape at the open ends of a stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum LineCap {
	/// The stroke stops square at the end point, SVG's default.
	#[default]
	Butt,
	/// A half-circle of the stroke's width past the end point.
	Round,
	/// A half-square of the stroke's width past the end point.
	Square,
}

/// The shape where two stroked segments meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum LineJoin {
	/// A sharp corner, beveled once it exceeds the stroke's miter limit;
	/// SVG's default.
	#[default]
	Miter,
	/// A rounded corner.
	Round,
	/// A corner cut flat.
	Bevel,
}

/// Whether a stroke's width follows the drawing's scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum StrokeScaling {
	/// The width and dashes are in layout units and never scale, like SVG's
	/// `vector-effect="non-scaling-stroke"`. A stretched outline keeps an
	/// even line.
	#[default]
	Fixed,
	/// The width and dashes are in view box units and scale with the
	/// drawing, as SVG strokes do by default. Under uneven stretching they
	/// scale by the square root of the area scale; the web distorts them
	/// with the stretch instead.
	Drawing,
}

/// How a path's outline is drawn.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "StrokeFields", into = "StrokeFields")
)]
pub struct Stroke {
	paint: Paint,
	width: f32,
	cap: LineCap,
	join: LineJoin,
	miter_limit: f32,
	/// Normalized: empty for a solid line, else an even number of lengths.
	dashes: Arc<[f32]>,
	dash_offset: f32,
	scaling: StrokeScaling,
}

impl Stroke {
	/// SVG's default ratio of miter length to stroke width beyond which a
	/// miter join is beveled. Every backend sets the limit explicitly,
	/// because platform defaults differ.
	pub const MITER_LIMIT: f32 = 4.0;

	/// A solid stroke `width` units wide, with SVG's default butt caps,
	/// miter joins, and miter limit, whose width never scales.
	pub fn new(paint: impl Into<Paint>, width: f32) -> Result<Self, VectorError> {
		if !width.is_finite() || width <= 0.0 {
			return Err(VectorError::StrokeWidth);
		}

		Ok(Self {
			paint: paint.into(),
			width,
			cap: LineCap::default(),
			join: LineJoin::default(),
			miter_limit: Self::MITER_LIMIT,
			dashes: Arc::new([]),
			dash_offset: 0.0,
			scaling: StrokeScaling::default(),
		})
	}

	/// Sets the end cap.
	#[must_use]
	pub fn cap(mut self, cap: LineCap) -> Self {
		self.cap = cap;
		self
	}

	/// Sets the corner join.
	#[must_use]
	pub fn join(mut self, join: LineJoin) -> Self {
		self.join = join;
		self
	}

	/// Rounds both the caps and the joins, the usual choice for hand-drawn
	/// lines.
	#[must_use]
	pub fn round(self) -> Self {
		self.cap(LineCap::Round).join(LineJoin::Round)
	}

	/// Sets the miter limit, which must be at least one.
	pub fn miter_limit(mut self, limit: f32) -> Result<Self, VectorError> {
		if !limit.is_finite() || limit < 1.0 {
			return Err(VectorError::MiterLimit);
		}

		self.miter_limit = limit;
		Ok(self)
	}

	/// Dashes the stroke with alternating dash and gap `lengths`, starting
	/// `offset` units into the pattern, as SVG's `stroke-dasharray` and
	/// `stroke-dashoffset`. An odd list repeats to make it even, and an empty
	/// or all-zero list draws a solid line.
	pub fn dashes(mut self, lengths: &[f32], offset: f32) -> Result<Self, VectorError> {
		if !offset.is_finite()
			|| lengths
				.iter()
				.any(|length| !length.is_finite() || *length < 0.0)
		{
			return Err(VectorError::DashArray);
		}

		self.dashes = if lengths.iter().all(|length| *length == 0.0) {
			Arc::new([])
		} else if lengths.len() % 2 == 1 {
			lengths.iter().chain(lengths).copied().collect()
		} else {
			lengths.into()
		};
		self.dash_offset = if self.dashes.is_empty() { 0.0 } else { offset };
		Ok(self)
	}

	/// Sets whether the width and dashes scale with the drawing.
	#[must_use]
	pub fn scaling(mut self, scaling: StrokeScaling) -> Self {
		self.scaling = scaling;
		self
	}

	/// What the stroke paints with.
	pub const fn paint(&self) -> Paint {
		self.paint
	}

	/// The color to paint with; see [`Paint::color`].
	pub const fn color(&self) -> Color {
		self.paint.color()
	}

	/// The width, in layout units for a fixed stroke and view box units for
	/// a scaling one.
	pub const fn width(&self) -> f32 {
		self.width
	}

	/// The end cap.
	pub const fn line_cap(&self) -> LineCap {
		self.cap
	}

	/// The corner join.
	pub const fn line_join(&self) -> LineJoin {
		self.join
	}

	/// The miter limit.
	pub const fn miter(&self) -> f32 {
		self.miter_limit
	}

	/// The dash pattern: empty for a solid stroke, else an even number of
	/// alternating dash and gap lengths.
	pub fn dash_lengths(&self) -> &[f32] {
		&self.dashes
	}

	/// How far into the dash pattern the stroke starts.
	pub const fn dash_offset(&self) -> f32 {
		self.dash_offset
	}

	/// Whether the width and dashes scale with the drawing.
	pub const fn stroke_scaling(&self) -> StrokeScaling {
		self.scaling
	}

	/// The factor from this stroke's units to frame units under `transform`.
	#[must_use]
	pub fn frame_scale(&self, transform: Transform) -> f32 {
		match self.scaling {
			StrokeScaling::Fixed => 1.0,
			StrokeScaling::Drawing => transform.stroke_scale(),
		}
	}

	/// The width in frame units under `transform`.
	#[must_use]
	pub fn frame_width(&self, transform: Transform) -> f32 {
		self.width * self.frame_scale(transform)
	}

	/// The dash lengths in frame units under `transform`.
	pub fn frame_dashes(&self, transform: Transform) -> impl Iterator<Item = f32> + '_ {
		let scale = self.frame_scale(transform);
		self.dashes.iter().map(move |length| length * scale)
	}

	/// The dash offset in frame units under `transform`.
	#[must_use]
	pub fn frame_dash_offset(&self, transform: Transform) -> f32 {
		self.dash_offset * self.frame_scale(transform)
	}
}

/// The wire form, which omits every field still at its default.
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct StrokeFields {
	color: Paint,
	width: f32,
	#[serde(default)]
	cap: LineCap,
	#[serde(default)]
	join: LineJoin,
	#[serde(default = "default_miter", skip_serializing_if = "is_default_miter")]
	miter_limit: f32,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	dashes: Vec<f32>,
	#[serde(default, skip_serializing_if = "is_zero")]
	dash_offset: f32,
	#[serde(default, skip_serializing_if = "is_fixed")]
	scaling: StrokeScaling,
}

#[cfg(feature = "serde")]
const fn default_miter() -> f32 {
	Stroke::MITER_LIMIT
}

#[cfg(feature = "serde")]
#[expect(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde passes a reference"
)]
fn is_default_miter(limit: &f32) -> bool {
	*limit == Stroke::MITER_LIMIT
}

#[cfg(feature = "serde")]
#[expect(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde passes a reference"
)]
fn is_zero(value: &f32) -> bool {
	*value == 0.0
}

#[cfg(feature = "serde")]
#[expect(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde passes a reference"
)]
fn is_fixed(scaling: &StrokeScaling) -> bool {
	*scaling == StrokeScaling::Fixed
}

#[cfg(feature = "serde")]
impl TryFrom<StrokeFields> for Stroke {
	type Error = VectorError;

	fn try_from(fields: StrokeFields) -> Result<Self, Self::Error> {
		Self::new(fields.color, fields.width)?
			.cap(fields.cap)
			.join(fields.join)
			.scaling(fields.scaling)
			.miter_limit(fields.miter_limit)?
			.dashes(&fields.dashes, fields.dash_offset)
	}
}

#[cfg(feature = "serde")]
impl From<Stroke> for StrokeFields {
	fn from(stroke: Stroke) -> Self {
		Self {
			color: stroke.paint,
			width: stroke.width,
			cap: stroke.cap,
			join: stroke.join,
			miter_limit: stroke.miter_limit,
			dashes: stroke.dashes.to_vec(),
			dash_offset: stroke.dash_offset,
			scaling: stroke.scaling,
		}
	}
}

/// One path of a drawing and how it is painted.
///
/// The fill is painted first and the stroke on top, centred on the outline.
/// A path with neither a fill nor a stroke draws nothing.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VectorPath {
	data: PathData,
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	fill: Option<Paint>,
	#[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_nonzero"))]
	fill_rule: FillRule,
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	stroke: Option<Stroke>,
}

#[cfg(feature = "serde")]
#[expect(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde passes a reference"
)]
fn is_nonzero(rule: &FillRule) -> bool {
	*rule == FillRule::Nonzero
}

impl VectorPath {
	/// An unpainted path; add a [`fill`](Self::fill), a
	/// [`stroke`](Self::stroke), or both.
	pub fn new(data: PathData) -> Self {
		Self {
			data,
			fill: None,
			fill_rule: FillRule::default(),
			stroke: None,
		}
	}

	/// Fills the path's interior with a color or the current text color.
	#[must_use]
	pub fn fill(mut self, paint: impl Into<Paint>) -> Self {
		self.fill = Some(paint.into());
		self
	}

	/// Sets which regions the fill covers.
	#[must_use]
	pub fn fill_rule(mut self, rule: FillRule) -> Self {
		self.fill_rule = rule;
		self
	}

	/// Strokes the path's outline.
	#[must_use]
	pub fn stroke(mut self, stroke: Stroke) -> Self {
		self.stroke = Some(stroke);
		self
	}

	/// The geometry.
	pub fn data(&self) -> &PathData {
		&self.data
	}

	/// The fill, if the path is filled.
	pub fn fill_paint(&self) -> Option<Paint> {
		self.fill
	}

	/// The fill color, if the path is filled; see [`Paint::color`].
	pub fn fill_color(&self) -> Option<Color> {
		self.fill.map(Paint::color)
	}

	/// Which regions the fill covers.
	pub fn rule(&self) -> FillRule {
		self.fill_rule
	}

	/// The stroke, if the path is stroked.
	pub fn stroke_style(&self) -> Option<&Stroke> {
		self.stroke.as_ref()
	}

	fn uses_current_color(&self) -> bool {
		self.fill == Some(Paint::CurrentColor)
			|| self
				.stroke
				.as_ref()
				.is_some_and(|stroke| stroke.paint == Paint::CurrentColor)
	}

	fn resolve_current_color(&mut self, current: Color) {
		self.fill = self.fill.map(|paint| paint.resolved(current));

		if let Some(stroke) = &mut self.stroke {
			stroke.paint = stroke.paint.resolved(current);
		}
	}
}

/// The shared body of a [`Drawing`].
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct DrawingBody {
	view_box: ViewBox,
	#[cfg_attr(feature = "serde", serde(default))]
	scaling: VectorScaling,
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "VectorAlignment::is_center")
	)]
	align: VectorAlignment,
	#[cfg_attr(feature = "serde", serde(default))]
	paths: Vec<VectorPath>,
}

/// The paths of a `Vector` element, in their own coordinate space.
///
/// Paths paint in order, so later paths draw over earlier ones. The drawing is
/// not clipped to the element's frame: strokes and overshooting curves may
/// paint outside it unless the element clips its content.
///
/// A drawing is immutable and shared once built: cloning it copies a
/// pointer, and an unchanged drawing compares equal by pointer, so diffing a
/// screen that keeps reusing it is free.
#[derive(Debug, Clone, Default)]
pub struct Drawing {
	body: Arc<DrawingBody>,
}

impl PartialEq for Drawing {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.body, &other.body) || self.body == other.body
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for Drawing {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		self.body.serialize(serializer)
	}
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Drawing {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		DrawingBody::deserialize(deserializer).map(|body| {
			Self {
				body: Arc::new(body),
			}
		})
	}
}

impl Drawing {
	/// An empty drawing over `view_box`, scaled to fit and centred.
	pub fn new(view_box: ViewBox) -> Self {
		Self {
			body: Arc::new(DrawingBody {
				view_box,
				..DrawingBody::default()
			}),
		}
	}

	/// Parses an SVG document into a drawing.
	///
	/// See the [`svg`](crate::vector#svg-documents) notes for what is
	/// supported; anything else is an error naming it, never a silently
	/// incomplete drawing.
	#[cfg(feature = "svg")]
	pub fn parse_svg(document: &str) -> Result<Self, SvgError> {
		svg::parse(document)
	}

	/// Sets how the view box maps onto the frame.
	#[must_use]
	pub fn scaling(mut self, scaling: VectorScaling) -> Self {
		self.set_scaling(scaling);
		self
	}

	/// Sets where a fitted or filled drawing sits in its frame.
	#[must_use]
	pub fn align(mut self, alignment: VectorAlignment) -> Self {
		self.set_alignment(alignment);
		self
	}

	/// Appends a path, painted over the existing ones.
	#[must_use]
	pub fn path(mut self, path: VectorPath) -> Self {
		self.push(path);
		self
	}

	/// Appends a path in place.
	pub fn push(&mut self, path: VectorPath) {
		Arc::make_mut(&mut self.body).paths.push(path);
	}

	/// Replaces the view box in place.
	pub fn set_view_box(&mut self, view_box: ViewBox) {
		Arc::make_mut(&mut self.body).view_box = view_box;
	}

	/// Replaces the scaling in place.
	pub fn set_scaling(&mut self, scaling: VectorScaling) {
		Arc::make_mut(&mut self.body).scaling = scaling;
	}

	/// Replaces the alignment in place.
	pub fn set_alignment(&mut self, alignment: VectorAlignment) {
		Arc::make_mut(&mut self.body).align = alignment;
	}

	/// The coordinate space.
	pub fn view_box(&self) -> ViewBox {
		self.body.view_box
	}

	/// How the view box maps onto the frame.
	pub fn vector_scaling(&self) -> VectorScaling {
		self.body.scaling
	}

	/// Where a fitted or filled drawing sits in its frame.
	pub fn alignment(&self) -> VectorAlignment {
		self.body.align
	}

	/// The paths, in painting order.
	pub fn paths(&self) -> &[VectorPath] {
		&self.body.paths
	}

	/// Whether any path paints with [`Paint::CurrentColor`].
	pub fn uses_current_color(&self) -> bool {
		self.body.paths.iter().any(VectorPath::uses_current_color)
	}

	/// Replaces every [`Paint::CurrentColor`] with `current`, copying the
	/// drawing only when it has one.
	pub fn resolve_current_color(&mut self, current: Color) {
		if !self.uses_current_color() {
			return;
		}

		for path in &mut Arc::make_mut(&mut self.body).paths {
			path.resolve_current_color(current);
		}
	}

	/// The mapping from drawing coordinates to a `width` by `height` frame,
	/// with the origin at the frame's top-left corner.
	#[must_use]
	pub fn transform(&self, width: f32, height: f32) -> Transform {
		let DrawingBody {
			view_box,
			scaling,
			align,
			..
		} = *self.body;
		let (width, height) = (width.max(0.0), height.max(0.0));
		let scale_x = width / view_box.width;
		let scale_y = height / view_box.height;

		let (scale_x, scale_y, offset_x, offset_y) = match scaling {
			VectorScaling::Stretch => (scale_x, scale_y, 0.0, 0.0),
			VectorScaling::Fit | VectorScaling::Fill => {
				let scale = if scaling == VectorScaling::Fit {
					scale_x.min(scale_y)
				} else {
					scale_x.max(scale_y)
				};
				(
					scale,
					scale,
					view_box.width.mul_add(-scale, width) * align.x.factor(),
					view_box.height.mul_add(-scale, height) * align.y.factor(),
				)
			}
		};

		Transform {
			scale_x,
			scale_y,
			translate_x: view_box.min_x.mul_add(-scale_x, offset_x),
			translate_y: view_box.min_y.mul_add(-scale_y, offset_y),
		}
	}
}

/// An axis-aligned scale and translation from drawing coordinates to frame
/// coordinates: `frame = drawing * scale + translate` on each axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
	/// The horizontal scale.
	pub scale_x: f32,
	/// The vertical scale.
	pub scale_y: f32,
	/// The horizontal offset, in frame units.
	pub translate_x: f32,
	/// The vertical offset, in frame units.
	pub translate_y: f32,
}

impl Transform {
	/// Maps a drawing point into the frame.
	#[must_use]
	pub fn apply(self, point: Point) -> Point {
		Point::new(
			point.x.mul_add(self.scale_x, self.translate_x),
			point.y.mul_add(self.scale_y, self.translate_y),
		)
	}

	/// The factor a scaling stroke's width grows by: the square root of the
	/// area scale, which is the scale itself whenever it is uniform.
	#[must_use]
	pub fn stroke_scale(self) -> f32 {
		(self.scale_x * self.scale_y).abs().sqrt()
	}
}
