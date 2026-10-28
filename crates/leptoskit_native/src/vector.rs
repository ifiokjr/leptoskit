//! Vector drawings: filled and stroked SVG-style paths that every backend
//! draws natively.
//!
//! A [`Drawing`] holds paths in its own coordinate space, the [`ViewBox`], and
//! maps that space onto the element's laid-out frame. [`VectorScaling::Fit`]
//! scales uniformly and centres the drawing, like SVG's default
//! `preserveAspectRatio`; [`VectorScaling::Stretch`] scales each axis
//! independently to fill the frame. Stroke widths are always in layout units
//! and never scale, so a stretched outline keeps an even line.
//!
//! Path data is parsed once, here, into absolute commands. Backends never parse
//! SVG: they ask the drawing for its [`Transform`] onto the frame and replay
//! the transformed [`PathCommand`]s through the platform's path API.
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
//! 			.stroke(Stroke::new(ink, 2.0)?.round()),
//! 	)
//! 	.path(VectorPath::new(PathData::parse("M10 10 h4 v4 h-4 Z")?).fill(ink));
//!
//! assert_eq!(drawing.paths().len(), 2);
//! # Ok::<(), leptoskit_native::vector::VectorError>(())
//! ```

use std::fmt;

use crate::props::Color;

/// Invalid vector drawing input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorError {
	/// A view box needs finite coordinates and a positive, finite size.
	ViewBox,
	/// A stroke width must be finite and greater than zero.
	StrokeWidth,
	/// A path coordinate must be finite.
	NonFiniteCoordinate,
	/// A subpath must begin with a move-to command.
	MissingMoveTo,
	/// Path data is malformed at this byte offset.
	PathData {
		/// The byte offset of the first character that could not be read.
		offset: usize,
	},
	/// Path data uses a command leptoskit does not draw, such as the
	/// elliptical arc `A`.
	UnsupportedCommand {
		/// The command letter.
		command: char,
		/// The byte offset of the command.
		offset: usize,
	},
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
			Self::NonFiniteCoordinate => formatter.write_str("path coordinates must be finite"),
			Self::MissingMoveTo => formatter.write_str("path data must start with a move-to (M)"),
			Self::PathData { offset } => write!(formatter, "invalid path data at byte {offset}"),
			Self::UnsupportedCommand { command, offset } => {
				write!(
					formatter,
					"path command `{command}` at byte {offset} is not supported: use M, L, H, V, \
					 C, S, Q, T, and Z"
				)
			}
		}
	}
}

impl std::error::Error for VectorError {}

/// A point in a drawing's coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Point {
	/// The horizontal coordinate.
	pub x: f32,
	/// The vertical coordinate.
	pub y: f32,
}

impl Point {
	/// The point at `(x, y)`.
	pub const fn new(x: f32, y: f32) -> Self {
		Self { x, y }
	}

	fn is_finite(self) -> bool {
		self.x.is_finite() && self.y.is_finite()
	}

	fn offset(self, by: Self) -> Self {
		Self::new(self.x + by.x, self.y + by.y)
	}

	/// This point mirrored through `center`, for smooth curve shorthands.
	fn reflected(self, center: Self) -> Self {
		Self::new(
			2.0f32.mul_add(center.x, -self.x),
			2.0f32.mul_add(center.y, -self.y),
		)
	}
}

/// One absolute path command.
///
/// Parsed path data contains only these five commands: relative commands and
/// the `H`, `V`, `S`, and `T` shorthands are resolved while parsing, and every
/// subpath begins with [`MoveTo`](Self::MoveTo), including one that continues
/// after a [`Close`](Self::Close).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
	/// Starts a new subpath at a point.
	MoveTo(Point),
	/// A straight line to a point.
	LineTo(Point),
	/// A quadratic Bézier curve.
	QuadTo {
		/// The control point.
		control: Point,
		/// The end point.
		to: Point,
	},
	/// A cubic Bézier curve.
	CubicTo {
		/// The first control point.
		control1: Point,
		/// The second control point.
		control2: Point,
		/// The end point.
		to: Point,
	},
	/// Closes the current subpath with a line back to its start.
	Close,
}

impl PathCommand {
	/// This command with every point mapped through `transform`.
	#[must_use]
	pub fn transformed(self, transform: Transform) -> Self {
		let map = |point| transform.apply(point);

		match self {
			Self::MoveTo(point) => Self::MoveTo(map(point)),
			Self::LineTo(point) => Self::LineTo(map(point)),
			Self::QuadTo { control, to } => {
				Self::QuadTo {
					control: map(control),
					to: map(to),
				}
			}
			Self::CubicTo {
				control1,
				control2,
				to,
			} => {
				Self::CubicTo {
					control1: map(control1),
					control2: map(control2),
					to: map(to),
				}
			}
			Self::Close => Self::Close,
		}
	}

	fn points(&self) -> impl Iterator<Item = Point> {
		let points: [Option<Point>; 3] = match *self {
			Self::MoveTo(point) | Self::LineTo(point) => [Some(point), None, None],
			Self::QuadTo { control, to } => [Some(control), Some(to), None],
			Self::CubicTo {
				control1,
				control2,
				to,
			} => [Some(control1), Some(control2), Some(to)],
			Self::Close => [None, None, None],
		};
		points.into_iter().flatten()
	}
}

/// Validated path geometry in absolute [`PathCommand`]s.
///
/// Build it from SVG path data with [`parse`](Self::parse), or from generated
/// geometry with [`from_commands`](Self::from_commands). It displays and
/// serializes as canonical path data: absolute `M`, `L`, `Q`, `C`, and `Z`
/// commands with every token separated by one space, such as
/// `M 0 0 L 10 10 Z`, so hosts outside Rust can split it on spaces instead of
/// parsing SVG.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "String", into = "String")
)]
pub struct PathData {
	commands: Vec<PathCommand>,
}

impl PathData {
	/// Parses SVG path data.
	///
	/// Every command except the elliptical arc is accepted, in absolute and
	/// relative forms: `M`, `L`, `H`, `V`, `C`, `S`, `Q`, `T`, and `Z`. Numbers
	/// may be separated by whitespace or commas, and repeated coordinates
	/// repeat the previous command, as in SVG. Empty data is a valid path that
	/// draws nothing.
	pub fn parse(data: &str) -> Result<Self, VectorError> {
		Parser::new(data).parse()
	}

	/// Validates absolute commands: every coordinate must be finite and the
	/// first command must be a [`PathCommand::MoveTo`]. A subpath that
	/// continues after a [`PathCommand::Close`] without moving first starts
	/// where the closed subpath started, as in SVG.
	pub fn from_commands(
		commands: impl IntoIterator<Item = PathCommand>,
	) -> Result<Self, VectorError> {
		let mut builder = Builder::default();

		for command in commands {
			builder.push(command)?;
		}

		Ok(Self {
			commands: builder.commands,
		})
	}

	/// The absolute commands, in drawing order.
	#[must_use]
	pub fn commands(&self) -> &[PathCommand] {
		&self.commands
	}

	/// Whether the path has no commands.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.commands.is_empty()
	}

	/// The commands mapped onto a frame through `transform`.
	pub fn transformed(&self, transform: Transform) -> impl Iterator<Item = PathCommand> + '_ {
		self.commands
			.iter()
			.map(move |command| command.transformed(transform))
	}
}

impl fmt::Display for PathData {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut separator = "";

		for command in &self.commands {
			let letter = match command {
				PathCommand::MoveTo(_) => 'M',
				PathCommand::LineTo(_) => 'L',
				PathCommand::QuadTo { .. } => 'Q',
				PathCommand::CubicTo { .. } => 'C',
				PathCommand::Close => 'Z',
			};
			write!(formatter, "{separator}{letter}")?;
			separator = " ";

			for point in command.points() {
				write!(formatter, " {} {}", point.x, point.y)?;
			}
		}

		Ok(())
	}
}

impl std::str::FromStr for PathData {
	type Err = VectorError;

	fn from_str(data: &str) -> Result<Self, Self::Err> {
		Self::parse(data)
	}
}

impl TryFrom<String> for PathData {
	type Error = VectorError;

	fn try_from(data: String) -> Result<Self, Self::Error> {
		Self::parse(&data)
	}
}

impl From<PathData> for String {
	fn from(data: PathData) -> Self {
		data.to_string()
	}
}

/// Accumulates validated commands and keeps the every-subpath-moves-first
/// invariant.
#[derive(Default)]
struct Builder {
	commands: Vec<PathCommand>,
	start: Point,
	closed: bool,
}

impl Builder {
	fn push(&mut self, command: PathCommand) -> Result<(), VectorError> {
		if !command.points().all(Point::is_finite) {
			return Err(VectorError::NonFiniteCoordinate);
		}

		match command {
			PathCommand::MoveTo(point) => {
				self.start = point;
			}
			_ if self.commands.is_empty() => return Err(VectorError::MissingMoveTo),
			// A closed subpath that keeps drawing restarts at its start point;
			// spelling the move out spares every backend that rule.
			_ if self.closed => self.commands.push(PathCommand::MoveTo(self.start)),
			_ => {}
		}

		self.closed = command == PathCommand::Close;
		self.commands.push(command);
		Ok(())
	}
}

/// Reads SVG path data into absolute commands.
struct Parser<'a> {
	bytes: &'a [u8],
	position: usize,
	builder: Builder,
	current: Point,
	/// The previous command's second control point, for `S` and `T`.
	previous_control: Option<(Point, bool)>,
}

impl<'a> Parser<'a> {
	fn new(data: &'a str) -> Self {
		Self {
			bytes: data.as_bytes(),
			position: 0,
			builder: Builder::default(),
			current: Point::default(),
			previous_control: None,
		}
	}

	fn parse(mut self) -> Result<PathData, VectorError> {
		let mut command: Option<u8> = None;

		loop {
			self.skip_separators();
			let Some(&byte) = self.bytes.get(self.position) else {
				break;
			};
			let offset = self.position;

			if byte.is_ascii_alphabetic() {
				self.position += 1;
				command = Some(byte);
			} else {
				// Coordinates without a letter repeat the previous command;
				// a move-to repeats as a line-to.
				command = match command {
					Some(b'M') => Some(b'L'),
					Some(b'm') => Some(b'l'),
					Some(b'Z' | b'z') | None => return Err(VectorError::PathData { offset }),
					repeated => repeated,
				};
			}

			let letter = command.expect("a command was read or repeated above");
			self.command(letter, offset)?;
		}

		Ok(PathData {
			commands: self.builder.commands,
		})
	}

	fn command(&mut self, letter: u8, offset: usize) -> Result<(), VectorError> {
		let relative = letter.is_ascii_lowercase();
		let origin = if relative {
			self.current
		} else {
			Point::default()
		};
		let mut control = None;

		let command = match letter.to_ascii_uppercase() {
			b'M' => PathCommand::MoveTo(self.point()?.offset(origin)),
			b'L' => PathCommand::LineTo(self.point()?.offset(origin)),
			b'H' => {
				let x = self.number()? + origin.x;
				PathCommand::LineTo(Point::new(x, self.current.y))
			}
			b'V' => {
				let y = self.number()? + origin.y;
				PathCommand::LineTo(Point::new(self.current.x, y))
			}
			b'C' => {
				let control1 = self.point()?.offset(origin);
				let control2 = self.point()?.offset(origin);
				let to = self.point()?.offset(origin);
				control = Some((control2, true));
				PathCommand::CubicTo {
					control1,
					control2,
					to,
				}
			}
			b'S' => {
				let control1 = self.reflected_control(true);
				let control2 = self.point()?.offset(origin);
				let to = self.point()?.offset(origin);
				control = Some((control2, true));
				PathCommand::CubicTo {
					control1,
					control2,
					to,
				}
			}
			b'Q' => {
				let quad = self.point()?.offset(origin);
				let to = self.point()?.offset(origin);
				control = Some((quad, false));
				PathCommand::QuadTo { control: quad, to }
			}
			b'T' => {
				let quad = self.reflected_control(false);
				let to = self.point()?.offset(origin);
				control = Some((quad, false));
				PathCommand::QuadTo { control: quad, to }
			}
			b'Z' => PathCommand::Close,
			b'A' => {
				return Err(VectorError::UnsupportedCommand {
					command: char::from(letter),
					offset,
				});
			}
			_ => return Err(VectorError::PathData { offset }),
		};

		self.builder.push(command)?;
		self.previous_control = control;
		self.current = match command {
			PathCommand::MoveTo(point) | PathCommand::LineTo(point) => point,
			PathCommand::QuadTo { to, .. } | PathCommand::CubicTo { to, .. } => to,
			PathCommand::Close => self.builder.start,
		};
		Ok(())
	}

	/// The previous curve's control point mirrored through the current point,
	/// or the current point when the previous command was another kind.
	fn reflected_control(&self, cubic: bool) -> Point {
		match self.previous_control {
			Some((control, was_cubic)) if was_cubic == cubic => control.reflected(self.current),
			_ => self.current,
		}
	}

	fn point(&mut self) -> Result<Point, VectorError> {
		let x = self.number()?;
		let y = self.number()?;
		Ok(Point::new(x, y))
	}

	fn number(&mut self) -> Result<f32, VectorError> {
		self.skip_separators();
		let start = self.position;
		let error = VectorError::PathData { offset: start };

		if matches!(self.peek(), Some(b'+' | b'-')) {
			self.position += 1;
		}

		let integer = self.digits();
		let fraction = if self.peek() == Some(b'.') {
			self.position += 1;
			self.digits()
		} else {
			0
		};

		if integer == 0 && fraction == 0 {
			return Err(error);
		}

		if matches!(self.peek(), Some(b'e' | b'E')) {
			let exponent = self.position;
			self.position += 1;

			if matches!(self.peek(), Some(b'+' | b'-')) {
				self.position += 1;
			}

			// An `e` without digits belongs to whatever follows the number.
			if self.digits() == 0 {
				self.position = exponent;
			}
		}

		let text = std::str::from_utf8(&self.bytes[start..self.position])
			.expect("number tokens contain only ASCII");
		let value = text.parse::<f32>().map_err(|_| error)?;

		if !value.is_finite() {
			return Err(VectorError::NonFiniteCoordinate);
		}

		Ok(value)
	}

	fn digits(&mut self) -> usize {
		let start = self.position;

		while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
			self.position += 1;
		}

		self.position - start
	}

	fn peek(&self) -> Option<u8> {
		self.bytes.get(self.position).copied()
	}

	fn skip_separators(&mut self) {
		while self
			.peek()
			.is_some_and(|byte| byte == b',' || byte.is_ascii_whitespace())
		{
			self.position += 1;
		}
	}
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
	/// Scales both axes by the same factor, as large as fits, and centres the
	/// drawing: SVG's `preserveAspectRatio="xMidYMid meet"`.
	#[default]
	Fit,
	/// Scales each axis independently so the view box fills the frame
	/// exactly: SVG's `preserveAspectRatio="none"`.
	Stretch,
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
	/// A sharp corner, beveled once it exceeds [`Stroke::MITER_LIMIT`]; SVG's
	/// default.
	#[default]
	Miter,
	/// A rounded corner.
	Round,
	/// A corner cut flat.
	Bevel,
}

/// How a path's outline is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "StrokeFields", into = "StrokeFields")
)]
pub struct Stroke {
	color: Color,
	width: f32,
	cap: LineCap,
	join: LineJoin,
}

impl Stroke {
	/// The ratio of miter length to stroke width beyond which a miter join is
	/// beveled. It is SVG's default, which every backend sets explicitly
	/// because platform defaults differ.
	pub const MITER_LIMIT: f32 = 4.0;

	/// A stroke of `color`, `width` layout units wide, with SVG's default
	/// butt caps and miter joins.
	///
	/// The width never scales with the drawing.
	pub fn new(color: Color, width: f32) -> Result<Self, VectorError> {
		if !width.is_finite() || width <= 0.0 {
			return Err(VectorError::StrokeWidth);
		}

		Ok(Self {
			color,
			width,
			cap: LineCap::default(),
			join: LineJoin::default(),
		})
	}

	/// Sets the end cap.
	#[must_use]
	pub const fn cap(mut self, cap: LineCap) -> Self {
		self.cap = cap;
		self
	}

	/// Sets the corner join.
	#[must_use]
	pub const fn join(mut self, join: LineJoin) -> Self {
		self.join = join;
		self
	}

	/// Rounds both the caps and the joins, the usual choice for hand-drawn
	/// lines.
	#[must_use]
	pub const fn round(self) -> Self {
		self.cap(LineCap::Round).join(LineJoin::Round)
	}

	/// The stroke color.
	pub const fn color(self) -> Color {
		self.color
	}

	/// The width, in layout units.
	pub const fn width(self) -> f32 {
		self.width
	}

	/// The end cap.
	pub const fn line_cap(self) -> LineCap {
		self.cap
	}

	/// The corner join.
	pub const fn line_join(self) -> LineJoin {
		self.join
	}
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct StrokeFields {
	color: Color,
	width: f32,
	#[serde(default)]
	cap: LineCap,
	#[serde(default)]
	join: LineJoin,
}

#[cfg(feature = "serde")]
impl TryFrom<StrokeFields> for Stroke {
	type Error = VectorError;

	fn try_from(fields: StrokeFields) -> Result<Self, Self::Error> {
		Ok(Self::new(fields.color, fields.width)?
			.cap(fields.cap)
			.join(fields.join))
	}
}

#[cfg(feature = "serde")]
impl From<Stroke> for StrokeFields {
	fn from(stroke: Stroke) -> Self {
		Self {
			color: stroke.color,
			width: stroke.width,
			cap: stroke.cap,
			join: stroke.join,
		}
	}
}

/// One path of a drawing and how it is painted.
///
/// The fill is painted first and the stroke on top, centred on the outline.
/// Fills use the nonzero winding rule. A path with neither a fill nor a stroke
/// draws nothing.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VectorPath {
	data: PathData,
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	fill: Option<Color>,
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	stroke: Option<Stroke>,
}

impl VectorPath {
	/// An unpainted path; add a [`fill`](Self::fill), a
	/// [`stroke`](Self::stroke), or both.
	pub fn new(data: PathData) -> Self {
		Self {
			data,
			fill: None,
			stroke: None,
		}
	}

	/// Fills the path's interior.
	#[must_use]
	pub fn fill(mut self, color: Color) -> Self {
		self.fill = Some(color);
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

	/// The fill color, if the path is filled.
	pub fn fill_color(&self) -> Option<Color> {
		self.fill
	}

	/// The stroke, if the path is stroked.
	pub fn stroke_style(&self) -> Option<Stroke> {
		self.stroke
	}
}

/// The paths of a `Vector` element, in their own coordinate space.
///
/// Paths paint in order, so later paths draw over earlier ones. The drawing is
/// not clipped to the element's frame: strokes and overshooting curves may
/// paint outside it unless the element clips its content.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Drawing {
	view_box: ViewBox,
	#[cfg_attr(feature = "serde", serde(default))]
	scaling: VectorScaling,
	#[cfg_attr(feature = "serde", serde(default))]
	paths: Vec<VectorPath>,
}

impl Drawing {
	/// An empty drawing over `view_box`, scaled to fit.
	pub fn new(view_box: ViewBox) -> Self {
		Self {
			view_box,
			scaling: VectorScaling::default(),
			paths: Vec::new(),
		}
	}

	/// Sets how the view box maps onto the frame.
	#[must_use]
	pub fn scaling(mut self, scaling: VectorScaling) -> Self {
		self.scaling = scaling;
		self
	}

	/// Appends a path, painted over the existing ones.
	#[must_use]
	pub fn path(mut self, path: VectorPath) -> Self {
		self.paths.push(path);
		self
	}

	/// Appends a path in place.
	pub fn push(&mut self, path: VectorPath) {
		self.paths.push(path);
	}

	/// Replaces the view box in place.
	pub fn set_view_box(&mut self, view_box: ViewBox) {
		self.view_box = view_box;
	}

	/// Replaces the scaling in place.
	pub fn set_scaling(&mut self, scaling: VectorScaling) {
		self.scaling = scaling;
	}

	/// The coordinate space.
	pub fn view_box(&self) -> ViewBox {
		self.view_box
	}

	/// How the view box maps onto the frame.
	pub fn vector_scaling(&self) -> VectorScaling {
		self.scaling
	}

	/// The paths, in painting order.
	pub fn paths(&self) -> &[VectorPath] {
		&self.paths
	}

	/// The mapping from drawing coordinates to a `width` by `height` frame,
	/// with the origin at the frame's top-left corner.
	#[must_use]
	pub fn transform(&self, width: f32, height: f32) -> Transform {
		let view_box = self.view_box;
		let scale_x = width.max(0.0) / view_box.width;
		let scale_y = height.max(0.0) / view_box.height;

		let (scale_x, scale_y, offset_x, offset_y) = match self.scaling {
			VectorScaling::Stretch => (scale_x, scale_y, 0.0, 0.0),
			VectorScaling::Fit => {
				let scale = scale_x.min(scale_y);
				(
					scale,
					scale,
					view_box.width.mul_add(-scale, width.max(0.0)) / 2.0,
					view_box.height.mul_add(-scale, height.max(0.0)) / 2.0,
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
}
