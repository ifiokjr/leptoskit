//! The display list: plain values describing what to draw, in order.

/// A colour in sRGB with straight (not premultiplied) alpha, each channel
/// from 0 to 1, the way CSS writes `rgba()`. The renderer premultiplies it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Color {
	pub r: f32,
	pub g: f32,
	pub b: f32,
	pub a: f32,
}

impl Color {
	pub const BLACK: Self = Self::rgba(0.0, 0.0, 0.0, 1.0);
	pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
	pub const WHITE: Self = Self::rgba(1.0, 1.0, 1.0, 1.0);

	/// A colour from channels between 0 and 1.
	pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
		Self { r, g, b, a }
	}

	/// An opaque colour from 8-bit channels, as in `#rrggbb`.
	pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
		Self::rgba8(r, g, b, 255)
	}

	/// A colour from 8-bit channels, alpha included.
	pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
		Self::rgba(
			r as f32 / 255.0,
			g as f32 / 255.0,
			b as f32 / 255.0,
			a as f32 / 255.0,
		)
	}

	/// The same colour with its alpha multiplied by `opacity`.
	#[must_use]
	pub fn with_opacity(self, opacity: f32) -> Self {
		Self {
			a: self.a * opacity,
			..self
		}
	}

	/// The colour with its channels multiplied by alpha, as the GPU blends it.
	pub fn premultiplied(self) -> [f32; 4] {
		[self.r * self.a, self.g * self.a, self.b * self.a, self.a]
	}
}

/// A point in the canvas's coordinate space, in logical points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Point {
	pub x: f32,
	pub y: f32,
}

impl Point {
	pub const fn new(x: f32, y: f32) -> Self {
		Self { x, y }
	}
}

/// An axis-aligned rectangle: its top-left corner and its size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
	pub x: f32,
	pub y: f32,
	pub width: f32,
	pub height: f32,
}

impl Rect {
	pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
		Self {
			x,
			y,
			width,
			height,
		}
	}
}

/// The radius of each corner of a rounded rectangle. Radii larger than half
/// the shorter side are reduced to it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CornerRadii {
	pub top_left: f32,
	pub top_right: f32,
	pub bottom_right: f32,
	pub bottom_left: f32,
}

impl CornerRadii {
	/// The same radius on every corner.
	pub const fn all(radius: f32) -> Self {
		Self {
			top_left: radius,
			top_right: radius,
			bottom_right: radius,
			bottom_left: radius,
		}
	}

	#[cfg(feature = "renderer")]
	pub(crate) fn to_array(self) -> [f32; 4] {
		[
			self.top_left,
			self.top_right,
			self.bottom_right,
			self.bottom_left,
		]
	}
}

impl From<f32> for CornerRadii {
	fn from(radius: f32) -> Self {
		Self::all(radius)
	}
}

/// A 2D affine transform, laid out like the canvas `setTransform(a, b, c, d,
/// e, f)`: a point `(x, y)` maps to `(a·x + c·y + e, b·x + d·y + f)`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Affine(pub [f32; 6]);

impl Default for Affine {
	fn default() -> Self {
		Self::IDENTITY
	}
}

impl Affine {
	pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

	/// Moves points by `(x, y)`.
	pub const fn translate(x: f32, y: f32) -> Self {
		Self([1.0, 0.0, 0.0, 1.0, x, y])
	}

	/// Scales points away from the origin.
	pub const fn scale(x: f32, y: f32) -> Self {
		Self([x, 0.0, 0.0, y, 0.0, 0.0])
	}

	/// Turns points about the origin, clockwise on screen, by `radians`.
	pub fn rotate(radians: f32) -> Self {
		let (sin, cos) = radians.sin_cos();
		Self([cos, sin, -sin, cos, 0.0, 0.0])
	}

	/// This transform followed by `next`.
	#[must_use]
	pub fn then(self, next: Self) -> Self {
		let Self(m) = self;
		let Self(n) = next;
		Self([
			n[0] * m[0] + n[2] * m[1],
			n[1] * m[0] + n[3] * m[1],
			n[0] * m[2] + n[2] * m[3],
			n[1] * m[2] + n[3] * m[3],
			n[0] * m[4] + n[2] * m[5] + n[4],
			n[1] * m[4] + n[3] * m[5] + n[5],
		])
	}

	/// Where the transform puts `point`.
	pub fn apply(self, point: Point) -> Point {
		let Self(m) = self;
		Point::new(
			m[0] * point.x + m[2] * point.y + m[4],
			m[1] * point.x + m[3] * point.y + m[5],
		)
	}
}

/// One colour on a gradient, at `offset` between 0 (the start) and 1 (the
/// end).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GradientStop {
	pub offset: f32,
	pub color: Color,
}

impl GradientStop {
	pub const fn new(offset: f32, color: Color) -> Self {
		Self { offset, color }
	}
}

/// What fills a shape. Gradients are positioned in the shape's own
/// coordinate space, and colours between stops are mixed with premultiplied
/// alpha, as CSS gradients are.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Paint {
	Solid(Color),
	/// Runs from `start` to `end`; beyond either end the nearest stop's
	/// colour continues.
	LinearGradient {
		start: Point,
		end: Point,
		stops: Vec<GradientStop>,
	},
	/// Runs from `center` out to `radius`; beyond it the last stop's colour
	/// continues.
	RadialGradient {
		center: Point,
		radius: f32,
		stops: Vec<GradientStop>,
	},
}

impl Default for Paint {
	fn default() -> Self {
		Self::Solid(Color::BLACK)
	}
}

impl From<Color> for Paint {
	fn from(color: Color) -> Self {
		Self::Solid(color)
	}
}

/// Which areas of a path count as inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum FillRule {
	/// Inside wherever the outline winds around the point at all, so a
	/// shape drawn twice in the same direction stays filled.
	#[default]
	NonZero,
	/// Inside wherever the outline crosses an odd number of times, so a
	/// shape inside another cuts a hole.
	EvenOdd,
}

/// How the ends of an open stroke look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum LineCap {
	#[default]
	Butt,
	Round,
	Square,
}

/// How a stroke turns a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum LineJoin {
	#[default]
	Miter,
	Round,
	Bevel,
}

/// The outline drawn along a path.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Stroke {
	pub width: f32,
	pub cap: LineCap,
	pub join: LineJoin,
	/// How far a miter join may reach, as a multiple of half the width,
	/// before it is cut to a bevel. The canvas default is 10.
	pub miter_limit: f32,
}

impl Default for Stroke {
	fn default() -> Self {
		Self {
			width: 1.0,
			cap: LineCap::Butt,
			join: LineJoin::Miter,
			miter_limit: 10.0,
		}
	}
}

impl Stroke {
	/// A stroke `width` points wide with the canvas defaults.
	pub fn new(width: f32) -> Self {
		Self {
			width,
			..Self::default()
		}
	}

	#[must_use]
	pub fn cap(mut self, cap: LineCap) -> Self {
		self.cap = cap;
		self
	}

	#[must_use]
	pub fn join(mut self, join: LineJoin) -> Self {
		self.join = join;
		self
	}

	#[must_use]
	pub fn miter_limit(mut self, miter_limit: f32) -> Self {
		self.miter_limit = miter_limit;
		self
	}
}

/// One step of a path outline.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PathCommand {
	/// Starts a new outline at the point.
	MoveTo(Point),
	/// A straight line to the point.
	LineTo(Point),
	/// A quadratic Bézier curve through one control point.
	QuadTo(Point, Point),
	/// A cubic Bézier curve through two control points.
	CubicTo(Point, Point, Point),
	/// A straight line back to where the outline started.
	Close,
}

/// A vector outline made of lines and Bézier curves, in the style of SVG
/// path data. A line or curve with no outline started begins one where the
/// last outline began, or at the origin.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Path {
	pub commands: Vec<PathCommand>,
}

impl Path {
	pub fn new() -> Self {
		Self::default()
	}

	#[must_use]
	pub fn move_to(mut self, x: f32, y: f32) -> Self {
		self.commands.push(PathCommand::MoveTo(Point::new(x, y)));
		self
	}

	#[must_use]
	pub fn line_to(mut self, x: f32, y: f32) -> Self {
		self.commands.push(PathCommand::LineTo(Point::new(x, y)));
		self
	}

	#[must_use]
	pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
		self.commands
			.push(PathCommand::QuadTo(Point::new(cx, cy), Point::new(x, y)));
		self
	}

	#[must_use]
	pub fn cubic_to(mut self, c1: Point, c2: Point, to: Point) -> Self {
		self.commands.push(PathCommand::CubicTo(c1, c2, to));
		self
	}

	#[must_use]
	pub fn close(mut self) -> Self {
		self.commands.push(PathCommand::Close);
		self
	}

	/// A closed rectangle, clockwise on screen.
	#[must_use]
	pub fn rect(self, rect: Rect) -> Self {
		self.move_to(rect.x, rect.y)
			.line_to(rect.x + rect.width, rect.y)
			.line_to(rect.x + rect.width, rect.y + rect.height)
			.line_to(rect.x, rect.y + rect.height)
			.close()
	}

	/// A closed circle made of four cubic curves.
	#[must_use]
	pub fn circle(self, cx: f32, cy: f32, r: f32) -> Self {
		// The control-point distance that best approximates a quarter circle.
		const K: f32 = 0.552_284_8;
		let k = r * K;
		self.move_to(cx + r, cy)
			.cubic_to(
				Point::new(cx + r, cy + k),
				Point::new(cx + k, cy + r),
				Point::new(cx, cy + r),
			)
			.cubic_to(
				Point::new(cx - k, cy + r),
				Point::new(cx - r, cy + k),
				Point::new(cx - r, cy),
			)
			.cubic_to(
				Point::new(cx - r, cy - k),
				Point::new(cx - k, cy - r),
				Point::new(cx, cy - r),
			)
			.cubic_to(
				Point::new(cx + k, cy - r),
				Point::new(cx + r, cy - k),
				Point::new(cx + r, cy),
			)
			.close()
	}
}

/// A border drawn inside a rectangle's edge, as CSS draws one with
/// `box-sizing: border-box`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Border {
	pub width: f32,
	pub color: Color,
}

/// A blurred shadow cast by a rounded rectangle, as CSS `box-shadow`
/// describes one: moved by the offset, grown by `spread`, and blurred by
/// `blur` (a Gaussian whose standard deviation is half of it).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxShadow {
	pub offset_x: f32,
	pub offset_y: f32,
	pub blur: f32,
	pub spread: f32,
	pub color: Color,
}

/// One drawing instruction. Commands draw in order, each over the ones
/// before it. Every push must be closed by the matching pop, innermost
/// first, the way the canvas pairs `save` and `restore`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
	feature = "serde",
	serde(
		tag = "type",
		rename_all = "snake_case",
		rename_all_fields = "snake_case"
	)
)]
pub enum Command {
	/// Fills a rectangle with optionally rounded corners, with an optional
	/// border inside its edge.
	FillRect {
		rect: Rect,
		radii: CornerRadii,
		paint: Paint,
		border: Option<Border>,
	},
	/// Draws a rounded rectangle's shadow. Draw it before the rectangle: the
	/// shadow covers the area under the rectangle too.
	BoxShadow {
		rect: Rect,
		radii: CornerRadii,
		shadow: BoxShadow,
	},
	/// Fills the inside of a path.
	FillPath {
		path: Path,
		rule: FillRule,
		paint: Paint,
	},
	/// Draws a path's outline.
	StrokePath {
		path: Path,
		stroke: Stroke,
		paint: Paint,
	},
	/// Limits drawing to a rectangle, optionally rounded, until `PopClip`.
	/// Nested clips intersect.
	PushClip {
		rect: Rect,
		radii: CornerRadii,
	},
	/// Limits drawing to the inside of a path until `PopClip`.
	PushClipPath {
		path: Path,
		rule: FillRule,
	},
	PopClip,
	/// Draws everything until `PopLayer` into a layer of its own, then
	/// blends the layer at `opacity`, so overlapping shapes fade as one.
	PushLayer {
		opacity: f32,
	},
	PopLayer,
	/// Transforms everything until `PopTransform`, after the transforms
	/// already open.
	PushTransform(Affine),
	PopTransform,
}

/// A display list: the commands that draw one canvas, in logical points
/// with the origin at the top left and y growing downwards.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Scene {
	pub commands: Vec<Command>,
}

impl Scene {
	pub fn new() -> Self {
		Self::default()
	}

	/// Appends a command.
	pub fn push(&mut self, command: Command) {
		self.commands.push(command);
	}

	/// The scene with `command` appended.
	#[must_use]
	pub fn with(mut self, command: Command) -> Self {
		self.push(command);
		self
	}

	/// Fills a rectangle.
	#[must_use]
	pub fn fill_rect(self, rect: Rect, paint: impl Into<Paint>) -> Self {
		self.fill_rounded_rect(rect, CornerRadii::default(), paint)
	}

	/// Fills a rectangle with rounded corners.
	#[must_use]
	pub fn fill_rounded_rect(
		self,
		rect: Rect,
		radii: impl Into<CornerRadii>,
		paint: impl Into<Paint>,
	) -> Self {
		self.with(Command::FillRect {
			rect,
			radii: radii.into(),
			paint: paint.into(),
			border: None,
		})
	}

	/// Draws a box shadow under where a rounded rectangle will go.
	#[must_use]
	pub fn box_shadow(self, rect: Rect, radii: impl Into<CornerRadii>, shadow: BoxShadow) -> Self {
		self.with(Command::BoxShadow {
			rect,
			radii: radii.into(),
			shadow,
		})
	}

	/// Fills a path.
	#[must_use]
	pub fn fill_path(self, path: Path, rule: FillRule, paint: impl Into<Paint>) -> Self {
		self.with(Command::FillPath {
			path,
			rule,
			paint: paint.into(),
		})
	}

	/// Strokes a path's outline.
	#[must_use]
	pub fn stroke_path(self, path: Path, stroke: Stroke, paint: impl Into<Paint>) -> Self {
		self.with(Command::StrokePath {
			path,
			stroke,
			paint: paint.into(),
		})
	}

	/// Clips what follows to a rectangle, optionally rounded, until
	/// `pop_clip`.
	#[must_use]
	pub fn push_clip(self, rect: Rect, radii: impl Into<CornerRadii>) -> Self {
		self.with(Command::PushClip {
			rect,
			radii: radii.into(),
		})
	}

	/// Clips what follows to a path, until `pop_clip`.
	#[must_use]
	pub fn push_clip_path(self, path: Path, rule: FillRule) -> Self {
		self.with(Command::PushClipPath { path, rule })
	}

	#[must_use]
	pub fn pop_clip(self) -> Self {
		self.with(Command::PopClip)
	}

	/// Groups what follows, until `pop_layer`, into one layer blended at
	/// `opacity`.
	#[must_use]
	pub fn push_layer(self, opacity: f32) -> Self {
		self.with(Command::PushLayer { opacity })
	}

	#[must_use]
	pub fn pop_layer(self) -> Self {
		self.with(Command::PopLayer)
	}

	/// Transforms what follows, until `pop_transform`.
	#[must_use]
	pub fn push_transform(self, transform: Affine) -> Self {
		self.with(Command::PushTransform(transform))
	}

	#[must_use]
	pub fn pop_transform(self) -> Self {
		self.with(Command::PopTransform)
	}
}
