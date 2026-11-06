//! Bounds for scenes that come from code the host does not trust, such as a
//! Wasm guest. The renderer sanitizes every scene before drawing it.

use crate::model::*;

/// The most commands a scene keeps; later ones are dropped.
pub const MAX_COMMANDS: usize = 10_000;
/// The most commands one path keeps.
pub const MAX_PATH_COMMANDS: usize = 10_000;
/// The most path commands a whole scene keeps; paths past it are emptied.
pub const MAX_SCENE_PATH_COMMANDS: usize = 100_000;
/// The most stops a gradient keeps.
pub const MAX_GRADIENT_STOPS: usize = 16;
/// The most gradients a scene keeps; later ones become their first stop's
/// colour.
pub const MAX_GRADIENTS: usize = 1024;
/// The largest coordinate or size, in points, either way from the origin.
pub const MAX_COORDINATE: f32 = 1.0e6;
/// The largest scale or shear a single transform may apply.
pub const MAX_TRANSFORM_SCALE: f32 = 1.0e4;
/// The widest shadow blur, in points.
pub const MAX_BLUR: f32 = 1_000.0;
/// The widest stroke, in points.
pub const MAX_STROKE_WIDTH: f32 = 10_000.0;
/// The largest stroke miter limit.
pub const MAX_MITER_LIMIT: f32 = 100.0;
/// The most clips, layers, and transforms open at once; deeper pushes are
/// dropped with their pops.
pub const MAX_DEPTH: usize = 32;
/// The most layers open at once; each one holds a texture the size of the
/// canvas.
pub const MAX_LAYER_DEPTH: usize = 8;

/// A finite value clamped to `[min, max]`; an infinity or NaN becomes the
/// bound closest to zero.
fn bounded(value: f32, min: f32, max: f32) -> f32 {
	if value.is_finite() {
		value.clamp(min, max)
	} else {
		0.0_f32.clamp(min, max)
	}
}

fn coordinate(value: f32) -> f32 {
	bounded(value, -MAX_COORDINATE, MAX_COORDINATE)
}

fn length(value: f32, max: f32) -> f32 {
	bounded(value, 0.0, max)
}

fn unit(value: f32) -> f32 {
	bounded(value, 0.0, 1.0)
}

impl Color {
	/// Channels finite and between 0 and 1.
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self::rgba(unit(self.r), unit(self.g), unit(self.b), unit(self.a))
	}
}

impl Point {
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self::new(coordinate(self.x), coordinate(self.y))
	}
}

impl Rect {
	/// Finite, with a size that is not negative.
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self::new(
			coordinate(self.x),
			coordinate(self.y),
			length(self.width, MAX_COORDINATE),
			length(self.height, MAX_COORDINATE),
		)
	}
}

impl CornerRadii {
	/// Each radius finite, not negative, and at most half of `rect`'s
	/// shorter side.
	#[must_use]
	pub fn sanitized(self, rect: Rect) -> Self {
		let max = rect.width.min(rect.height) * 0.5;
		let radius = |value: f32| length(value, max);
		Self {
			top_left: radius(self.top_left),
			top_right: radius(self.top_right),
			bottom_right: radius(self.bottom_right),
			bottom_left: radius(self.bottom_left),
		}
	}
}

impl Affine {
	/// Scales and shears within [`MAX_TRANSFORM_SCALE`], translations
	/// within [`MAX_COORDINATE`].
	#[must_use]
	pub fn sanitized(self) -> Self {
		let scale = |value: f32| bounded(value, -MAX_TRANSFORM_SCALE, MAX_TRANSFORM_SCALE);
		let Self(m) = self;
		Self([
			scale(m[0]),
			scale(m[1]),
			scale(m[2]),
			scale(m[3]),
			coordinate(m[4]),
			coordinate(m[5]),
		])
	}
}

/// Stops limited to [`MAX_GRADIENT_STOPS`], offsets between 0 and 1 and
/// never going backwards, as CSS fixes them up.
fn stops(stops: &[GradientStop]) -> Vec<GradientStop> {
	let mut last = 0.0_f32;
	stops
		.iter()
		.take(MAX_GRADIENT_STOPS)
		.map(|stop| {
			last = unit(stop.offset).max(last);
			GradientStop::new(last, stop.color.sanitized())
		})
		.collect()
}

impl Paint {
	/// Every value bounded. A gradient with no stops is transparent.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		match self {
			Self::Solid(color) => Self::Solid(color.sanitized()),
			Self::LinearGradient {
				start,
				end,
				stops: list,
			} if !list.is_empty() => {
				Self::LinearGradient {
					start: start.sanitized(),
					end: end.sanitized(),
					stops: stops(list),
				}
			}
			Self::RadialGradient {
				center,
				radius,
				stops: list,
			} if !list.is_empty() => {
				Self::RadialGradient {
					center: center.sanitized(),
					radius: length(*radius, MAX_COORDINATE),
					stops: stops(list),
				}
			}
			Self::LinearGradient { .. } | Self::RadialGradient { .. } => {
				Self::Solid(Color::TRANSPARENT)
			}
		}
	}

	fn is_gradient(&self) -> bool {
		!matches!(self, Self::Solid(_))
	}

	/// The paint's first colour, which stands in for a gradient past
	/// [`MAX_GRADIENTS`].
	fn first_color(&self) -> Color {
		match self {
			Self::Solid(color) => *color,
			Self::LinearGradient { stops, .. } | Self::RadialGradient { stops, .. } => {
				stops.first().map_or(Color::TRANSPARENT, |stop| stop.color)
			}
		}
	}
}

impl Stroke {
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self {
			width: length(self.width, MAX_STROKE_WIDTH),
			miter_limit: bounded(self.miter_limit, 1.0, MAX_MITER_LIMIT),
			..self
		}
	}
}

impl BoxShadow {
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self {
			offset_x: coordinate(self.offset_x),
			offset_y: coordinate(self.offset_y),
			blur: length(self.blur, MAX_BLUR),
			spread: coordinate(self.spread),
			color: self.color.sanitized(),
		}
	}
}

impl Border {
	/// A width between zero and half of `rect`'s shorter side.
	#[must_use]
	pub fn sanitized(self, rect: Rect) -> Self {
		Self {
			width: length(self.width, rect.width.min(rect.height) * 0.5),
			color: self.color.sanitized(),
		}
	}
}

impl Path {
	/// Points bounded and at most `budget` commands, capped at
	/// [`MAX_PATH_COMMANDS`].
	#[must_use]
	pub fn sanitized(&self, budget: usize) -> Self {
		Self {
			commands: self
				.commands
				.iter()
				.take(budget.min(MAX_PATH_COMMANDS))
				.map(|command| {
					match *command {
						PathCommand::MoveTo(to) => PathCommand::MoveTo(to.sanitized()),
						PathCommand::LineTo(to) => PathCommand::LineTo(to.sanitized()),
						PathCommand::QuadTo(control, to) => {
							PathCommand::QuadTo(control.sanitized(), to.sanitized())
						}
						PathCommand::CubicTo(c1, c2, to) => {
							PathCommand::CubicTo(c1.sanitized(), c2.sanitized(), to.sanitized())
						}
						PathCommand::Close => PathCommand::Close,
					}
				})
				.collect(),
		}
	}
}

/// What a push opened, so its pop can be matched.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
	Clip,
	Layer,
	Transform,
}

/// One entry on the stack of open pushes: what it opened and whether it
/// was kept or dropped for going too deep.
struct Frame {
	open: Open,
	kept: bool,
}

/// Running totals while a scene is sanitized.
#[derive(Default)]
struct Budget {
	path_commands: usize,
	gradients: usize,
	stack: Vec<Frame>,
	depth: usize,
	layers: usize,
}

impl Budget {
	fn path(&mut self, path: &Path) -> Path {
		let path = path.sanitized(MAX_SCENE_PATH_COMMANDS - self.path_commands);
		self.path_commands += path.commands.len();
		path
	}

	fn paint(&mut self, paint: &Paint) -> Paint {
		let paint = paint.sanitized();
		if !paint.is_gradient() {
			return paint;
		}
		self.gradients += 1;
		if self.gradients > MAX_GRADIENTS {
			Paint::Solid(paint.first_color())
		} else {
			paint
		}
	}

	/// Opens a push, returning whether it is kept.
	fn open(&mut self, open: Open) -> bool {
		let kept = self.depth < MAX_DEPTH && (open != Open::Layer || self.layers < MAX_LAYER_DEPTH);
		if kept {
			self.depth += 1;
			self.layers += usize::from(open == Open::Layer);
		}
		self.stack.push(Frame { open, kept });
		kept
	}

	/// Closes the innermost push if it is `open`, returning whether the pop
	/// is kept. A pop that matches nothing is dropped.
	fn close(&mut self, open: Open) -> bool {
		if self.stack.last().is_none_or(|frame| frame.open != open) {
			return false;
		}
		let frame = self.stack.pop().unwrap_or(Frame { open, kept: false });
		if frame.kept {
			self.depth -= 1;
			self.layers -= usize::from(open == Open::Layer);
		}
		frame.kept
	}
}

fn pop(open: Open) -> Command {
	match open {
		Open::Clip => Command::PopClip,
		Open::Layer => Command::PopLayer,
		Open::Transform => Command::PopTransform,
	}
}

impl Command {
	fn sanitized(&self, budget: &mut Budget) -> Option<Self> {
		Some(match self {
			Self::FillRect {
				rect,
				radii,
				paint,
				border,
			} => {
				let rect = rect.sanitized();
				Self::FillRect {
					rect,
					radii: radii.sanitized(rect),
					paint: budget.paint(paint),
					border: border.map(|border| border.sanitized(rect)),
				}
			}
			Self::BoxShadow {
				rect,
				radii,
				shadow,
			} => {
				let rect = rect.sanitized();
				Self::BoxShadow {
					rect,
					radii: radii.sanitized(rect),
					shadow: shadow.sanitized(),
				}
			}
			Self::FillPath { path, rule, paint } => {
				Self::FillPath {
					path: budget.path(path),
					rule: *rule,
					paint: budget.paint(paint),
				}
			}
			Self::StrokePath {
				path,
				stroke,
				paint,
			} => {
				Self::StrokePath {
					path: budget.path(path),
					stroke: stroke.sanitized(),
					paint: budget.paint(paint),
				}
			}
			Self::PushClip { rect, radii } => {
				let rect = rect.sanitized();
				budget.open(Open::Clip).then(|| {
					Self::PushClip {
						rect,
						radii: radii.sanitized(rect),
					}
				})?
			}
			Self::PushClipPath { path, rule } => {
				if !budget.open(Open::Clip) {
					return None;
				}
				Self::PushClipPath {
					path: budget.path(path),
					rule: *rule,
				}
			}
			Self::PushLayer { opacity } => {
				budget.open(Open::Layer).then(|| {
					Self::PushLayer {
						opacity: unit(*opacity),
					}
				})?
			}
			Self::PushTransform(transform) => {
				budget
					.open(Open::Transform)
					.then(|| Self::PushTransform(transform.sanitized()))?
			}
			Self::PopClip => budget.close(Open::Clip).then_some(Self::PopClip)?,
			Self::PopLayer => budget.close(Open::Layer).then_some(Self::PopLayer)?,
			Self::PopTransform => {
				budget
					.close(Open::Transform)
					.then_some(Self::PopTransform)?
			}
		})
	}
}

impl Scene {
	/// The scene made safe to draw whatever sent it: its first
	/// [`MAX_COMMANDS`] commands, every number finite and bounded, gradient
	/// stops in order, and pushes and pops balanced. A pop that does not
	/// match the innermost open push is dropped, pushes deeper than
	/// [`MAX_DEPTH`] (or [`MAX_LAYER_DEPTH`] layers) are dropped with their
	/// pops, and pushes still open at the end are closed.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		let mut budget = Budget::default();
		let mut commands: Vec<Command> = self
			.commands
			.iter()
			.take(MAX_COMMANDS)
			.filter_map(|command| command.sanitized(&mut budget))
			.collect();
		while let Some(frame) = budget.stack.pop() {
			if frame.kept {
				commands.push(pop(frame.open));
			}
		}
		Self { commands }
	}
}
