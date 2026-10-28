//! Path geometry: absolute commands, the SVG path data parser, basic shapes,
//! and affine transforms.

use std::f64::consts::FRAC_PI_2;
use std::fmt;
use std::sync::Arc;
use std::sync::OnceLock;

use super::Transform;
use super::VectorError;

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

	pub(super) fn is_finite(self) -> bool {
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
/// Path data contains only these five commands: relative commands, the `H`,
/// `V`, `S`, and `T` shorthands, and elliptical arcs are resolved while
/// parsing, and every subpath begins with [`MoveTo`](Self::MoveTo), including
/// one that continues after a [`Close`](Self::Close).
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
		self.map(|point| transform.apply(point))
	}

	fn map(self, map: impl Fn(Point) -> Point) -> Self {
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

	pub(super) fn points(&self) -> impl Iterator<Item = Point> {
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

/// The shared, immutable body of a [`PathData`].
#[derive(Debug, Default)]
struct PathBody {
	commands: Box<[PathCommand]>,
	/// The canonical text, formatted once on first use: the web renderer and
	/// the wire format both need it on every render.
	canonical: OnceLock<Box<str>>,
}

/// Validated path geometry in absolute [`PathCommand`]s.
///
/// Build it from SVG path data with [`parse`](Self::parse), from basic shapes
/// such as [`circle`](Self::circle), or from generated geometry with
/// [`from_commands`](Self::from_commands). It displays and serializes as
/// canonical path data: absolute `M`, `L`, `Q`, `C`, and `Z` commands with
/// every token separated by one space, such as `M 0 0 L 10 10 Z`, so hosts
/// outside Rust can split it on spaces instead of parsing SVG.
///
/// Path data is immutable and shared: cloning it copies a pointer, so parse an
/// icon once and reuse it across elements and renders. Comparing two clones of
/// the same path is a pointer comparison.
#[derive(Debug, Clone, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "String", into = "String")
)]
pub struct PathData {
	body: Arc<PathBody>,
}

impl PartialEq for PathData {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.body, &other.body) || self.body.commands == other.body.commands
	}
}

impl PathData {
	/// Parses SVG path data.
	///
	/// Every SVG command is accepted, in absolute and relative forms: `M`,
	/// `L`, `H`, `V`, `C`, `S`, `Q`, `T`, `A`, and `Z`. Elliptical arcs become
	/// cubic Béziers of at most a quarter turn each, following the SVG
	/// implementation notes, including scaling up radii that are too small to
	/// reach the end point. Numbers may be separated by whitespace or commas,
	/// arc flags may be written without separators, and repeated coordinates
	/// repeat the previous command. Empty data is a valid path that draws
	/// nothing.
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

		Ok(builder.finish())
	}

	/// A rectangle with its top-left corner at `(x, y)`.
	pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Result<Self, VectorError> {
		Self::rounded_rect(x, y, width, height, 0.0, 0.0)
	}

	/// A rectangle with elliptical corners, by SVG's `<rect>` rules: each
	/// radius is clamped to half the side it runs along, and a zero radius
	/// gives square corners.
	pub fn rounded_rect(
		x: f32,
		y: f32,
		width: f32,
		height: f32,
		rx: f32,
		ry: f32,
	) -> Result<Self, VectorError> {
		shape_sizes(&[x, y], &[width, height, rx, ry])?;
		let rx = rx.min(width / 2.0);
		let ry = ry.min(height / 2.0);
		let (right, bottom) = (x + width, y + height);
		let mut builder = Builder::default();

		if rx == 0.0 || ry == 0.0 {
			builder.move_to(Point::new(x, y));
			builder.line_to(Point::new(right, y));
			builder.line_to(Point::new(right, bottom));
			builder.line_to(Point::new(x, bottom));
		} else {
			let radii = Point::new(rx, ry);
			builder.move_to(Point::new(x + rx, y));
			builder.line_to(Point::new(right - rx, y));
			builder.arc(radii, 0.0, false, true, Point::new(right, y + ry));
			builder.line_to(Point::new(right, bottom - ry));
			builder.arc(radii, 0.0, false, true, Point::new(right - rx, bottom));
			builder.line_to(Point::new(x + rx, bottom));
			builder.arc(radii, 0.0, false, true, Point::new(x, bottom - ry));
			builder.line_to(Point::new(x, y + ry));
			builder.arc(radii, 0.0, false, true, Point::new(x + rx, y));
		}

		builder.close();
		Ok(builder.finish())
	}

	/// A circle centred on `(cx, cy)`.
	pub fn circle(cx: f32, cy: f32, radius: f32) -> Result<Self, VectorError> {
		Self::ellipse(cx, cy, radius, radius)
	}

	/// An axis-aligned ellipse centred on `(cx, cy)`, drawn clockwise from
	/// its rightmost point as four quarter arcs.
	pub fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Result<Self, VectorError> {
		shape_sizes(&[cx, cy], &[rx, ry])?;
		let radii = Point::new(rx, ry);
		let mut builder = Builder::default();
		builder.move_to(Point::new(cx + rx, cy));

		for to in [
			Point::new(cx, cy + ry),
			Point::new(cx - rx, cy),
			Point::new(cx, cy - ry),
			Point::new(cx + rx, cy),
		] {
			builder.arc(radii, 0.0, false, true, to);
		}

		builder.close();
		Ok(builder.finish())
	}

	/// A straight line between two points.
	pub fn line(from: Point, to: Point) -> Result<Self, VectorError> {
		Self::polyline([from, to])
	}

	/// Straight lines joining the points in order, left open.
	pub fn polyline(points: impl IntoIterator<Item = Point>) -> Result<Self, VectorError> {
		let mut points = points.into_iter();
		let Some(first) = points.next() else {
			return Ok(Self::default());
		};

		Self::from_commands(
			std::iter::once(PathCommand::MoveTo(first)).chain(points.map(PathCommand::LineTo)),
		)
	}

	/// Straight lines joining the points in order, closed back to the first.
	pub fn polygon(points: impl IntoIterator<Item = Point>) -> Result<Self, VectorError> {
		let open = Self::polyline(points)?;

		if open.is_empty() {
			return Ok(open);
		}

		Self::from_commands(open.commands().iter().copied().chain([PathCommand::Close]))
	}

	/// The absolute commands, in drawing order.
	#[must_use]
	pub fn commands(&self) -> &[PathCommand] {
		&self.body.commands
	}

	/// Whether the path has no commands.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.body.commands.is_empty()
	}

	/// The canonical path data, formatted once and then reused.
	#[must_use]
	pub fn canonical(&self) -> &str {
		self.body
			.canonical
			.get_or_init(|| format_canonical(&self.body.commands).into_boxed_str())
	}

	/// The commands mapped onto a frame through `transform`.
	pub fn transformed(&self, transform: Transform) -> impl Iterator<Item = PathCommand> + '_ {
		self.body
			.commands
			.iter()
			.map(move |command| command.transformed(transform))
	}

	/// A copy of this path with every point mapped through `affine`.
	///
	/// Béziers stay exact under affine maps, so rotated, skewed, and
	/// translated geometry needs no approximation.
	#[must_use]
	pub fn transform(&self, affine: Affine) -> Self {
		Self::from_trusted(
			self.body
				.commands
				.iter()
				.map(|command| command.map(|point| affine.apply(point)))
				.collect(),
		)
	}

	/// Wraps commands that already satisfy the invariants.
	fn from_trusted(commands: Box<[PathCommand]>) -> Self {
		Self {
			body: Arc::new(PathBody {
				commands,
				canonical: OnceLock::new(),
			}),
		}
	}
}

fn format_canonical(commands: &[PathCommand]) -> String {
	use std::fmt::Write;

	let mut text = String::with_capacity(commands.len() * 16);

	for (index, command) in commands.iter().enumerate() {
		let letter = match command {
			PathCommand::MoveTo(_) => 'M',
			PathCommand::LineTo(_) => 'L',
			PathCommand::QuadTo { .. } => 'Q',
			PathCommand::CubicTo { .. } => 'C',
			PathCommand::Close => 'Z',
		};

		if index > 0 {
			text.push(' ');
		}
		text.push(letter);

		for point in command.points() {
			write!(text, " {} {}", point.x, point.y).expect("write to String");
		}
	}

	text
}

/// Rejects non-finite positions and negative or non-finite sizes.
fn shape_sizes(positions: &[f32], sizes: &[f32]) -> Result<(), VectorError> {
	if positions.iter().all(|value| value.is_finite())
		&& sizes.iter().all(|value| value.is_finite() && *value >= 0.0)
	{
		Ok(())
	} else {
		Err(VectorError::ShapeSize)
	}
}

impl fmt::Display for PathData {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.canonical())
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
		data.canonical().to_owned()
	}
}

/// Accumulates validated commands and keeps the every-subpath-moves-first
/// invariant.
#[derive(Default)]
struct Builder {
	commands: Vec<PathCommand>,
	start: Point,
	current: Point,
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
		self.current = match command {
			PathCommand::MoveTo(point) | PathCommand::LineTo(point) => point,
			PathCommand::QuadTo { to, .. } | PathCommand::CubicTo { to, .. } => to,
			PathCommand::Close => self.start,
		};
		self.commands.push(command);
		Ok(())
	}

	/// Pushes geometry the builder itself derived from finite input.
	fn push_trusted(&mut self, command: PathCommand) {
		self.push(command)
			.expect("geometry derived from validated input is finite and starts with a move");
	}

	fn move_to(&mut self, point: Point) {
		self.push_trusted(PathCommand::MoveTo(point));
	}

	fn line_to(&mut self, point: Point) {
		self.push_trusted(PathCommand::LineTo(point));
	}

	fn close(&mut self) {
		self.push_trusted(PathCommand::Close);
	}

	/// An elliptical arc from the current point, as cubic Béziers.
	fn arc(&mut self, radii: Point, rotation: f32, large: bool, sweep: bool, to: Point) {
		for command in arc_to_cubics(self.current, radii, rotation, large, sweep, to) {
			self.push_trusted(command);
		}
	}

	fn finish(self) -> PathData {
		PathData::from_trusted(self.commands.into_boxed_slice())
	}
}

/// Converts an SVG elliptical arc into cubic Béziers of at most a quarter
/// turn each, following the SVG implementation notes (F.6.5 and F.6.6).
///
/// Coincident end points draw nothing and a zero radius draws a straight
/// line. The arithmetic runs in `f64`, and the last segment ends exactly at
/// `to`, so consecutive arcs never drift apart. Names follow the
/// specification's notation (`x1′` is `x1p`), so the code reads against it.
#[expect(
	clippy::similar_names,
	reason = "the SVG specification names these quantities x1, x1′, cx′, and so on"
)]
fn arc_to_cubics(
	from: Point,
	radii: Point,
	rotation: f32,
	large: bool,
	sweep: bool,
	to: Point,
) -> Vec<PathCommand> {
	if from == to {
		return Vec::new();
	}

	let (mut rx, mut ry) = (f64::from(radii.x).abs(), f64::from(radii.y).abs());

	if rx == 0.0 || ry == 0.0 {
		return vec![PathCommand::LineTo(to)];
	}

	let (sin, cos) = f64::from(rotation).to_radians().sin_cos();
	let (x1, y1, x2, y2) = (
		f64::from(from.x),
		f64::from(from.y),
		f64::from(to.x),
		f64::from(to.y),
	);
	let (half_dx, half_dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
	let x1p = cos.mul_add(half_dx, sin * half_dy);
	let y1p = (-sin).mul_add(half_dx, cos * half_dy);

	// Radii too small to span the end points scale up uniformly until they
	// just do.
	let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
	if lambda > 1.0 {
		rx *= lambda.sqrt();
		ry *= lambda.sqrt();
	}

	let numerator = (rx * ry).powi(2) - (rx * y1p).powi(2) - (ry * x1p).powi(2);
	let denominator = (rx * y1p).powi(2) + (ry * x1p).powi(2);
	let sign = if large == sweep { -1.0 } else { 1.0 };
	let coefficient = sign * (numerator / denominator).max(0.0).sqrt();
	let cxp = coefficient * rx * y1p / ry;
	let cyp = coefficient * -ry * x1p / rx;
	let cx = cos.mul_add(cxp, -sin * cyp) + f64::midpoint(x1, x2);
	let cy = sin.mul_add(cxp, cos * cyp) + f64::midpoint(y1, y2);

	let angle = |ux: f64, uy: f64, vx: f64, vy: f64| {
		ux.mul_add(vy, -uy * vx).atan2(ux.mul_add(vx, uy * vy))
	};
	let start = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
	let mut delta = angle(
		(x1p - cxp) / rx,
		(y1p - cyp) / ry,
		(-x1p - cxp) / rx,
		(-y1p - cyp) / ry,
	);

	if !sweep && delta > 0.0 {
		delta -= std::f64::consts::TAU;
	} else if sweep && delta < 0.0 {
		delta += std::f64::consts::TAU;
	}

	let segments = (delta.abs() / FRAC_PI_2).ceil().max(1.0);
	let step = delta / segments;
	let handle = 4.0 / 3.0 * (step / 4.0).tan();
	// Maps a point on the unit circle onto the rotated ellipse.
	let map = |ux: f64, uy: f64| {
		Point::new(
			(cx + (rx * ux).mul_add(cos, -(ry * uy) * sin)) as f32,
			(cy + (rx * ux).mul_add(sin, (ry * uy) * cos)) as f32,
		)
	};
	let count = segments as usize;

	(0..count)
		.map(|index| {
			let from_angle = (index as f64).mul_add(step, start);
			let to_angle = from_angle + step;
			let (sin1, cos1) = from_angle.sin_cos();
			let (sin2, cos2) = to_angle.sin_cos();
			PathCommand::CubicTo {
				control1: map(handle.mul_add(-sin1, cos1), handle.mul_add(cos1, sin1)),
				control2: map(handle.mul_add(sin2, cos2), handle.mul_add(-cos2, sin2)),
				to: if index + 1 == count {
					to
				} else {
					map(cos2, sin2)
				},
			}
		})
		.collect()
}

/// Reads SVG path data into absolute commands.
struct Parser<'a> {
	bytes: &'a [u8],
	position: usize,
	builder: Builder,
	/// The previous command's last control point, and whether it was a
	/// cubic, for `S` and `T`.
	previous_control: Option<(Point, bool)>,
}

impl<'a> Parser<'a> {
	fn new(data: &'a str) -> Self {
		Self {
			bytes: data.as_bytes(),
			position: 0,
			builder: Builder::default(),
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

		Ok(self.builder.finish())
	}

	fn command(&mut self, letter: u8, offset: usize) -> Result<(), VectorError> {
		let current = self.builder.current;
		let origin = if letter.is_ascii_lowercase() {
			current
		} else {
			Point::default()
		};
		let mut control = None;

		let commands = match letter.to_ascii_uppercase() {
			b'M' => vec![PathCommand::MoveTo(self.point()?.offset(origin))],
			b'L' => vec![PathCommand::LineTo(self.point()?.offset(origin))],
			b'H' => {
				let x = self.number()? + origin.x;
				vec![PathCommand::LineTo(Point::new(x, current.y))]
			}
			b'V' => {
				let y = self.number()? + origin.y;
				vec![PathCommand::LineTo(Point::new(current.x, y))]
			}
			b'C' => {
				let control1 = self.point()?.offset(origin);
				let control2 = self.point()?.offset(origin);
				let to = self.point()?.offset(origin);
				control = Some((control2, true));
				vec![PathCommand::CubicTo {
					control1,
					control2,
					to,
				}]
			}
			b'S' => {
				let control1 = self.reflected_control(true);
				let control2 = self.point()?.offset(origin);
				let to = self.point()?.offset(origin);
				control = Some((control2, true));
				vec![PathCommand::CubicTo {
					control1,
					control2,
					to,
				}]
			}
			b'Q' => {
				let quad = self.point()?.offset(origin);
				let to = self.point()?.offset(origin);
				control = Some((quad, false));
				vec![PathCommand::QuadTo { control: quad, to }]
			}
			b'T' => {
				let quad = self.reflected_control(false);
				let to = self.point()?.offset(origin);
				control = Some((quad, false));
				vec![PathCommand::QuadTo { control: quad, to }]
			}
			b'A' => {
				let radii = self.point()?;
				let rotation = self.number()?;
				let large = self.flag()?;
				let sweep = self.flag()?;
				let to = self.point()?.offset(origin);

				if !radii.is_finite() || !rotation.is_finite() {
					return Err(VectorError::NonFiniteCoordinate);
				}

				// The arc still needs a subpath to start from.
				if self.builder.commands.is_empty() {
					return Err(VectorError::MissingMoveTo);
				}

				arc_to_cubics(current, radii, rotation, large, sweep, to)
			}
			b'Z' => vec![PathCommand::Close],
			_ => return Err(VectorError::PathData { offset }),
		};

		for command in commands {
			self.builder.push(command)?;
		}

		self.previous_control = control;
		Ok(())
	}

	/// The previous curve's control point mirrored through the current point,
	/// or the current point when the previous command was another kind.
	fn reflected_control(&self, cubic: bool) -> Point {
		let current = self.builder.current;

		match self.previous_control {
			Some((control, was_cubic)) if was_cubic == cubic => control.reflected(current),
			_ => current,
		}
	}

	fn point(&mut self) -> Result<Point, VectorError> {
		let x = self.number()?;
		let y = self.number()?;
		Ok(Point::new(x, y))
	}

	/// An arc flag: a single `0` or `1`, which needs no separator after it.
	fn flag(&mut self) -> Result<bool, VectorError> {
		self.skip_separators();
		let offset = self.position;
		let flag = match self.peek() {
			Some(b'0') => false,
			Some(b'1') => true,
			_ => return Err(VectorError::PathData { offset }),
		};
		self.position += 1;
		Ok(flag)
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

/// A 2D affine transform in SVG's `matrix(a b c d e f)` form: a point maps
/// to `(a·x + c·y + e, b·x + d·y + f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
	/// The horizontal scale component.
	pub a: f32,
	/// The vertical skew component.
	pub b: f32,
	/// The horizontal skew component.
	pub c: f32,
	/// The vertical scale component.
	pub d: f32,
	/// The horizontal translation.
	pub e: f32,
	/// The vertical translation.
	pub f: f32,
}

impl Default for Affine {
	fn default() -> Self {
		Self::IDENTITY
	}
}

impl Affine {
	/// The transform that changes nothing.
	pub const IDENTITY: Self = Self {
		a: 1.0,
		b: 0.0,
		c: 0.0,
		d: 1.0,
		e: 0.0,
		f: 0.0,
	};

	/// A translation.
	#[must_use]
	pub const fn translate(x: f32, y: f32) -> Self {
		Self {
			e: x,
			f: y,
			..Self::IDENTITY
		}
	}

	/// A scale about the origin.
	#[must_use]
	pub const fn scale(x: f32, y: f32) -> Self {
		Self {
			a: x,
			d: y,
			..Self::IDENTITY
		}
	}

	/// A clockwise rotation about the origin, in degrees, in a y-down space.
	#[must_use]
	pub fn rotate(degrees: f32) -> Self {
		let (sin, cos) = degrees.to_radians().sin_cos();
		Self {
			a: cos,
			b: sin,
			c: -sin,
			d: cos,
			e: 0.0,
			f: 0.0,
		}
	}

	/// A horizontal skew, in degrees.
	#[must_use]
	pub fn skew_x(degrees: f32) -> Self {
		Self {
			c: degrees.to_radians().tan(),
			..Self::IDENTITY
		}
	}

	/// A vertical skew, in degrees.
	#[must_use]
	pub fn skew_y(degrees: f32) -> Self {
		Self {
			b: degrees.to_radians().tan(),
			..Self::IDENTITY
		}
	}

	/// This transform applied after `inner`: `self.then(inner)` maps a point
	/// through `inner` first, as SVG's `transform="self inner"` does.
	#[must_use]
	pub fn then(self, inner: Self) -> Self {
		Self {
			a: self.a.mul_add(inner.a, self.c * inner.b),
			b: self.b.mul_add(inner.a, self.d * inner.b),
			c: self.a.mul_add(inner.c, self.c * inner.d),
			d: self.b.mul_add(inner.c, self.d * inner.d),
			e: self.a.mul_add(inner.e, self.c.mul_add(inner.f, self.e)),
			f: self.b.mul_add(inner.e, self.d.mul_add(inner.f, self.f)),
		}
	}

	/// Maps a point.
	#[must_use]
	pub fn apply(self, point: Point) -> Point {
		Point::new(
			self.a.mul_add(point.x, self.c.mul_add(point.y, self.e)),
			self.b.mul_add(point.x, self.d.mul_add(point.y, self.f)),
		)
	}

	/// The factor this transform scales lengths by on average: the square
	/// root of its area scale. Exact for uniform scales and rotations.
	#[must_use]
	pub fn length_scale(self) -> f32 {
		self.a.mul_add(self.d, -self.b * self.c).abs().sqrt()
	}

	/// Parses an SVG transform list, such as
	/// `translate(10 20) rotate(45, 12, 12) scale(2)`. Transforms apply right
	/// to left, as in SVG.
	#[expect(
		clippy::many_single_char_names,
		reason = "matrix(a b c d e f) is SVG's own notation"
	)]
	pub fn parse(list: &str) -> Result<Self, VectorError> {
		let error = || VectorError::Transform;
		let mut result = Self::IDENTITY;
		let mut rest = list.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');

		while !rest.is_empty() {
			let open = rest.find('(').ok_or_else(error)?;
			let close = rest.find(')').ok_or_else(error)?;
			let name = rest[..open].trim();
			let arguments = rest[open + 1..close]
				.split(|c: char| c == ',' || c.is_ascii_whitespace())
				.filter(|argument| !argument.is_empty())
				.map(|argument| {
					argument
						.parse::<f32>()
						.ok()
						.filter(|value| value.is_finite())
				})
				.collect::<Option<Vec<_>>>()
				.ok_or_else(error)?;
			let next = match (name, arguments.as_slice()) {
				("matrix", &[a, b, c, d, e, f]) => Self { a, b, c, d, e, f },
				("translate", &[x]) => Self::translate(x, 0.0),
				("translate", &[x, y]) => Self::translate(x, y),
				("scale", &[x]) => Self::scale(x, x),
				("scale", &[x, y]) => Self::scale(x, y),
				("rotate", &[angle]) => Self::rotate(angle),
				("rotate", &[angle, x, y]) => {
					Self::translate(x, y)
						.then(Self::rotate(angle))
						.then(Self::translate(-x, -y))
				}
				("skewX", &[angle]) => Self::skew_x(angle),
				("skewY", &[angle]) => Self::skew_y(angle),
				_ => return Err(error()),
			};
			result = result.then(next);
			rest =
				rest[close + 1..].trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
		}

		Ok(result)
	}
}
