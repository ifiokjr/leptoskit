//! Turns a sanitized scene into GPU-ready buffers and an ordered list of
//! draws, split into render passes wherever a layer starts or ends.

use std::ops::Range;

use bytemuck::Pod;
use bytemuck::Zeroable;
use lyon_tessellation::BuffersBuilder;
use lyon_tessellation::FillOptions;
use lyon_tessellation::FillTessellator;
use lyon_tessellation::FillVertex;
use lyon_tessellation::StrokeOptions;
use lyon_tessellation::StrokeTessellator;
use lyon_tessellation::StrokeVertex;
use lyon_tessellation::VertexBuffers;
use lyon_tessellation::math::Box2D;
use lyon_tessellation::math::point;
use lyon_tessellation::path::Winding;
use lyon_tessellation::path::builder::BorderRadii;

use crate::model::*;

/// The most path vertices one frame keeps; paths past it are skipped.
pub(crate) const MAX_VERTICES: usize = 2_000_000;
/// Texels in one gradient ramp.
pub(crate) const RAMP_WIDTH: usize = 256;

/// One rounded rectangle or box shadow; see `QuadIn` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub(crate) struct Instance {
	pub rect: [f32; 4],
	pub radii: [f32; 4],
	pub m0: [f32; 4],
	pub m1: [f32; 4],
	pub params: [f32; 4],
	pub color: [f32; 4],
	pub paint: [u32; 4],
}

/// One tessellated path vertex; see `MeshIn` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub(crate) struct MeshVertex {
	pub pos: [f32; 2],
	pub local: [f32; 2],
	pub paint: u32,
}

/// One fill; see `Paint` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub(crate) struct PaintRecord {
	pub kind: u32,
	pub row: u32,
	pub pad: [u32; 2],
	pub color: [f32; 4],
	pub geom: [f32; 4],
}

const KIND_FILL: f32 = 0.0;
const KIND_SHADOW: f32 = 1.0;

/// One draw call.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Draw {
	/// Instances of the quad pipeline.
	Quads(Range<u32>),
	/// Indices of a path mesh.
	Mesh(Range<u32>),
	/// Indices of a clip shape that raises the stencil inside it.
	ClipPush(Range<u32>),
	/// Indices of a clip shape that lowers the stencil inside it again.
	ClipPop(Range<u32>),
	/// Blends the finished layer at `level` into its parent, with the
	/// opacity at `instance`.
	Composite { level: usize, instance: u32 },
}

/// A draw and the stencil value it tests against: how many clips are open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Step {
	pub draw: Draw,
	pub stencil: u32,
}

/// One render pass: into the target (level 0) or a layer's texture,
/// cleared first or continuing what was drawn before.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Segment {
	pub level: usize,
	pub clear: bool,
	pub steps: Range<usize>,
}

/// Everything one scene uploads and draws.
#[derive(Default)]
pub(crate) struct Frame {
	pub instances: Vec<Instance>,
	pub mesh: VertexBuffers<MeshVertex, u32>,
	pub paints: Vec<PaintRecord>,
	/// Premultiplied RGBA8 ramps, [`RAMP_WIDTH`] texels a row.
	pub ramps: Vec<u8>,
	pub opacities: Vec<f32>,
	pub steps: Vec<Step>,
	pub segments: Vec<Segment>,
	/// How many render targets the frame needs: the target and one per
	/// nested layer.
	pub levels: usize,
}

impl Frame {
	pub(crate) fn ramp_rows(&self) -> usize {
		self.ramps.len() / (RAMP_WIDTH * 4)
	}
}

/// What a push opened.
enum Open {
	Clip(Range<u32>),
	Layer { opacity: f32, depth: u32 },
	Transform(Affine),
}

struct Encoder {
	frame: Frame,
	width: f32,
	height: f32,
	scale: f32,
	transform: Affine,
	depth: u32,
	level: usize,
	segment: Segment,
	stack: Vec<Open>,
}

/// Encodes a scene that has already been sanitized, for a target `width` by
/// `height` device pixels with `scale` pixels per point.
pub(crate) fn encode(scene: &Scene, width: u32, height: u32, scale: f32) -> Frame {
	let mut encoder = Encoder {
		frame: Frame {
			levels: 1,
			..Frame::default()
		},
		width: width as f32,
		height: height as f32,
		scale,
		transform: Affine::IDENTITY,
		depth: 0,
		level: 0,
		segment: Segment {
			level: 0,
			clear: true,
			steps: 0..0,
		},
		stack: Vec::new(),
	};
	for command in &scene.commands {
		encoder.command(command);
	}
	encoder.end_segment();
	if encoder.frame.paints.is_empty() {
		encoder.frame.paints.push(PaintRecord::zeroed());
	}
	encoder.frame
}

impl Encoder {
	fn command(&mut self, command: &Command) {
		match command {
			Command::FillRect {
				rect,
				radii,
				paint,
				border,
			} => self.fill_rect(*rect, *radii, paint, *border),
			Command::BoxShadow {
				rect,
				radii,
				shadow,
			} => self.box_shadow(*rect, *radii, *shadow),
			Command::FillPath { path, rule, paint } => {
				let Some(tessellation) = self.tessellation(path, 0.0) else {
					return;
				};
				let paint = self.paint(paint);
				if let Some(range) = self.fill(path, *rule, paint, tessellation) {
					self.step(Draw::Mesh(range));
				}
			}
			Command::StrokePath {
				path,
				stroke,
				paint,
			} => {
				if stroke.width <= 0.0 {
					return;
				}
				let reach = stroke.width * 0.5 * stroke.miter_limit.max(1.5);
				let Some(tessellation) = self.tessellation(path, reach) else {
					return;
				};
				let paint = self.paint(paint);
				if let Some(range) = self.stroke(path, *stroke, paint, tessellation) {
					self.step(Draw::Mesh(range));
				}
			}
			Command::PushClip { rect, radii } => {
				let path = rounded_rect(*rect, *radii);
				let range = self.clip_shape(&path, FillRule::NonZero);
				self.push_clip(range);
			}
			Command::PushClipPath { path, rule } => {
				let range = self.clip_shape(path, *rule);
				self.push_clip(range);
			}
			Command::PushLayer { opacity } => {
				self.stack.push(Open::Layer {
					opacity: *opacity,
					depth: self.depth,
				});
				self.end_segment();
				self.level += 1;
				self.frame.levels = self.frame.levels.max(self.level + 1);
				self.depth = 0;
				self.begin_segment(true);
			}
			Command::PushTransform(transform) => {
				self.stack.push(Open::Transform(self.transform));
				self.transform = transform.then(self.transform);
			}
			Command::PopClip | Command::PopLayer | Command::PopTransform => self.pop(),
		}
	}

	/// Closes the innermost push; the scene is sanitized, so it is the one
	/// the pop names.
	fn pop(&mut self) {
		match self.stack.pop() {
			Some(Open::Clip(range)) => {
				self.step(Draw::ClipPop(range));
				self.depth -= 1;
			}
			Some(Open::Layer { opacity, depth }) => {
				self.end_segment();
				let level = self.level;
				self.level -= 1;
				self.depth = depth;
				self.begin_segment(false);
				let instance = self.frame.opacities.len() as u32;
				self.frame.opacities.push(opacity);
				self.step(Draw::Composite { level, instance });
			}
			Some(Open::Transform(transform)) => self.transform = transform,
			None => {}
		}
	}

	fn push_clip(&mut self, range: Range<u32>) {
		self.step(Draw::ClipPush(range.clone()));
		self.depth += 1;
		self.stack.push(Open::Clip(range));
	}

	fn end_segment(&mut self) {
		self.segment.steps.end = self.frame.steps.len();
		self.frame.segments.push(self.segment.clone());
	}

	fn begin_segment(&mut self, clear: bool) {
		let start = self.frame.steps.len();
		self.segment = Segment {
			level: self.level,
			clear,
			steps: start..start,
		};
	}

	/// Adds a draw, merging it into the previous one when both are quads or
	/// both are path meshes that follow on, at the same stencil value.
	fn step(&mut self, draw: Draw) {
		let stencil = self.depth;
		if self.frame.steps.len() > self.segment.steps.start
			&& let Some(last) = self.frame.steps.last_mut()
			&& last.stencil == stencil
		{
			match (&mut last.draw, &draw) {
				(Draw::Quads(a), Draw::Quads(b)) | (Draw::Mesh(a), Draw::Mesh(b))
					if a.end == b.start =>
				{
					a.end = b.end;
					return;
				}
				_ => {}
			}
		}
		self.frame.steps.push(Step { draw, stencil });
	}

	/// The current transform into device pixels.
	fn device(&self) -> Affine {
		self.transform.then(Affine::scale(self.scale, self.scale))
	}

	/// How many device pixels one unit of the current space covers, or
	/// `None` when the transform flattens everything to nothing.
	fn pixels_per_unit(&self) -> Option<f32> {
		let [a, b, c, d, ..] = self.device().0;
		let px = (a * d - b * c).abs().sqrt();
		(px.is_finite() && px > 1e-6).then_some(px)
	}

	/// Whether the local box from `lo` to `hi` lands anywhere on the target.
	fn visible(&self, lo: Point, hi: Point) -> bool {
		let device = self.device();
		let corners = [
			device.apply(lo),
			device.apply(Point::new(hi.x, lo.y)),
			device.apply(Point::new(lo.x, hi.y)),
			device.apply(hi),
		];
		let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
		for corner in corners {
			min_x = min_x.min(corner.x);
			min_y = min_y.min(corner.y);
			max_x = max_x.max(corner.x);
			max_y = max_y.max(corner.y);
		}
		max_x > 0.0 && max_y > 0.0 && min_x < self.width && min_y < self.height
	}

	fn quad(
		&mut self,
		rect: Rect,
		radii: CornerRadii,
		margin: f32,
		params: [f32; 4],
		color: [f32; 4],
		paint: u32,
	) {
		let Some(px) = self.pixels_per_unit() else {
			return;
		};
		let margin = margin + 1.0 / px;
		let lo = Point::new(rect.x - margin, rect.y - margin);
		let hi = Point::new(rect.x + rect.width + margin, rect.y + rect.height + margin);
		if !self.visible(lo, hi) {
			return;
		}
		let Affine(m) = self.device();
		let index = self.frame.instances.len() as u32;
		self.frame.instances.push(Instance {
			rect: [rect.x, rect.y, rect.width, rect.height],
			radii: radii.to_array(),
			m0: [m[0], m[1], m[2], m[3]],
			m1: [m[4], m[5], px, margin],
			params,
			color,
			paint: [paint, 0, 0, 0],
		});
		self.step(Draw::Quads(index..index + 1));
	}

	fn fill_rect(&mut self, rect: Rect, radii: CornerRadii, paint: &Paint, border: Option<Border>) {
		if rect.width <= 0.0 || rect.height <= 0.0 {
			return;
		}
		let border = border.unwrap_or_default();
		let paint = self.paint(paint);
		self.quad(
			rect,
			radii,
			0.0,
			[KIND_FILL, border.width, 0.0, 0.0],
			border.color.premultiplied(),
			paint,
		);
	}

	fn box_shadow(&mut self, rect: Rect, radii: CornerRadii, shadow: BoxShadow) {
		let spread = shadow.spread;
		let rect = Rect::new(
			rect.x + shadow.offset_x - spread,
			rect.y + shadow.offset_y - spread,
			rect.width + spread * 2.0,
			rect.height + spread * 2.0,
		);
		if rect.width <= 0.0 || rect.height <= 0.0 || shadow.color.a <= 0.0 {
			return;
		}
		let grow = |radius: f32| {
			if radius > 0.0 {
				(radius + spread).max(0.0)
			} else {
				0.0
			}
		};
		let radii = CornerRadii {
			top_left: grow(radii.top_left),
			top_right: grow(radii.top_right),
			bottom_right: grow(radii.bottom_right),
			bottom_left: grow(radii.bottom_left),
		};
		let sigma = shadow.blur * 0.5;
		self.quad(
			rect,
			radii,
			sigma * 3.0,
			[KIND_SHADOW, 0.0, sigma, 0.0],
			shadow.color.premultiplied(),
			0,
		);
	}

	/// Registers a fill and returns its index.
	fn paint(&mut self, paint: &Paint) -> u32 {
		let record = match paint {
			Paint::Solid(color) => solid(*color),
			Paint::LinearGradient { start, end, stops } => {
				let (dx, dy) = (end.x - start.x, end.y - start.y);
				let length = dx * dx + dy * dy;
				if length <= 1e-12 || stops.is_empty() {
					solid(last_color(stops))
				} else {
					PaintRecord {
						kind: 1,
						row: self.ramp(stops),
						pad: [0; 2],
						color: [0.0; 4],
						geom: [start.x, start.y, dx / length, dy / length],
					}
				}
			}
			Paint::RadialGradient {
				center,
				radius,
				stops,
			} => {
				if *radius <= 1e-6 || stops.is_empty() {
					solid(last_color(stops))
				} else {
					PaintRecord {
						kind: 2,
						row: self.ramp(stops),
						pad: [0; 2],
						color: [0.0; 4],
						geom: [center.x, center.y, 1.0 / radius, 0.0],
					}
				}
			}
		};
		let index = self.frame.paints.len() as u32;
		self.frame.paints.push(record);
		index
	}

	/// Bakes gradient stops into a ramp row and returns the row.
	fn ramp(&mut self, stops: &[GradientStop]) -> u32 {
		let row = self.frame.ramp_rows() as u32;
		for texel in 0..RAMP_WIDTH {
			let color = ramp_color(stops, texel as f32 / (RAMP_WIDTH - 1) as f32);
			self.frame
				.ramps
				.extend(color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8));
		}
		row
	}

	/// The flattening tolerance for a path in the current space, or `None`
	/// when it would land off the target. `reach` is how far a stroke
	/// extends past the outline.
	fn tessellation(&self, path: &Path, reach: f32) -> Option<f32> {
		let px = self.pixels_per_unit()?;
		if self.frame.mesh.vertices.len() >= MAX_VERTICES {
			return None;
		}
		let (lo, hi) = bounds(path)?;
		let lo = Point::new(lo.x - reach, lo.y - reach);
		let hi = Point::new(hi.x + reach, hi.y + reach);
		if !self.visible(lo, hi) {
			return None;
		}
		// A quarter pixel, but never finer than 1/4096 of the path, which
		// bounds how many vertices one curve can become at any zoom.
		let extent = (hi.x - lo.x).max(hi.y - lo.y);
		Some((0.25 / px).max(extent / 4096.0).max(1e-4))
	}

	/// Keeps the vertices tessellation added if it succeeded within budget,
	/// returning their index range.
	fn commit(&mut self, start: (usize, usize), ok: bool) -> Option<Range<u32>> {
		let mesh = &mut self.frame.mesh;
		if !ok || mesh.vertices.len() > MAX_VERTICES {
			mesh.vertices.truncate(start.0);
			mesh.indices.truncate(start.1);
			return None;
		}
		(mesh.indices.len() > start.1).then_some(start.1 as u32..mesh.indices.len() as u32)
	}

	fn fill(
		&mut self,
		path: &Path,
		rule: FillRule,
		paint: u32,
		tolerance: f32,
	) -> Option<Range<u32>> {
		let lyon = to_lyon(path);
		let device = self.device();
		let start = (
			self.frame.mesh.vertices.len(),
			self.frame.mesh.indices.len(),
		);
		let options = FillOptions::tolerance(tolerance).with_fill_rule(match rule {
			FillRule::NonZero => lyon_tessellation::FillRule::NonZero,
			FillRule::EvenOdd => lyon_tessellation::FillRule::EvenOdd,
		});
		let ok = FillTessellator::new()
			.tessellate_path(
				&lyon,
				&options,
				&mut BuffersBuilder::new(&mut self.frame.mesh, |vertex: FillVertex| {
					mesh_vertex(device, vertex.position(), paint)
				}),
			)
			.is_ok();
		self.commit(start, ok)
	}

	fn stroke(
		&mut self,
		path: &Path,
		stroke: Stroke,
		paint: u32,
		tolerance: f32,
	) -> Option<Range<u32>> {
		let lyon = to_lyon(path);
		let device = self.device();
		let start = (
			self.frame.mesh.vertices.len(),
			self.frame.mesh.indices.len(),
		);
		let options = StrokeOptions::tolerance(tolerance)
			.with_line_width(stroke.width)
			.with_miter_limit(stroke.miter_limit)
			.with_line_cap(match stroke.cap {
				LineCap::Butt => lyon_tessellation::LineCap::Butt,
				LineCap::Round => lyon_tessellation::LineCap::Round,
				LineCap::Square => lyon_tessellation::LineCap::Square,
			})
			.with_line_join(match stroke.join {
				LineJoin::Miter => lyon_tessellation::LineJoin::Miter,
				LineJoin::Round => lyon_tessellation::LineJoin::Round,
				LineJoin::Bevel => lyon_tessellation::LineJoin::Bevel,
			});
		let ok = StrokeTessellator::new()
			.tessellate_path(
				&lyon,
				&options,
				&mut BuffersBuilder::new(&mut self.frame.mesh, |vertex: StrokeVertex| {
					mesh_vertex(device, vertex.position(), paint)
				}),
			)
			.is_ok();
		self.commit(start, ok)
	}

	/// Tessellates a clip shape. One that lands off the target, or cannot be
	/// tessellated, is empty and so hides everything it clips.
	fn clip_shape(&mut self, path: &Path, rule: FillRule) -> Range<u32> {
		let empty = self.frame.mesh.indices.len() as u32;
		self.tessellation(path, 0.0)
			.and_then(|tolerance| self.fill(path, rule, 0, tolerance))
			.unwrap_or(empty..empty)
	}
}

fn solid(color: Color) -> PaintRecord {
	PaintRecord {
		kind: 0,
		row: 0,
		pad: [0; 2],
		color: color.premultiplied(),
		geom: [0.0; 4],
	}
}

fn last_color(stops: &[GradientStop]) -> Color {
	stops.last().map_or(Color::TRANSPARENT, |stop| stop.color)
}

/// The premultiplied colour a gradient has at `t`, mixing neighbouring stops
/// with premultiplied alpha.
pub(crate) fn ramp_color(stops: &[GradientStop], t: f32) -> [f32; 4] {
	let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
		return [0.0; 4];
	};
	if t <= first.offset {
		return first.color.premultiplied();
	}
	if t >= last.offset {
		return last.color.premultiplied();
	}
	let next = stops
		.iter()
		.position(|stop| stop.offset > t)
		.unwrap_or(stops.len() - 1);
	let (a, b) = (stops[next.saturating_sub(1)], stops[next]);
	let span = b.offset - a.offset;
	let mix = if span > 0.0 {
		(t - a.offset) / span
	} else {
		1.0
	};
	let (a, b) = (a.color.premultiplied(), b.color.premultiplied());
	std::array::from_fn(|channel| a[channel] + (b[channel] - a[channel]) * mix)
}

fn mesh_vertex(device: Affine, position: lyon_tessellation::math::Point, paint: u32) -> MeshVertex {
	let local = Point::new(position.x, position.y);
	let pos = device.apply(local);
	MeshVertex {
		pos: [pos.x, pos.y],
		local: [local.x, local.y],
		paint,
	}
}

/// The box around every point of a path, control points included.
fn bounds(path: &Path) -> Option<(Point, Point)> {
	let mut points = path.commands.iter().flat_map(|command| {
		match *command {
			PathCommand::MoveTo(p) | PathCommand::LineTo(p) => [Some(p), None, None],
			PathCommand::QuadTo(c, p) => [Some(c), Some(p), None],
			PathCommand::CubicTo(c1, c2, p) => [Some(c1), Some(c2), Some(p)],
			PathCommand::Close => [None, None, None],
		}
	});
	let first = points.find_map(|point| point)?;
	Some(points.flatten().fold((first, first), |(lo, hi), p| {
		(
			Point::new(lo.x.min(p.x), lo.y.min(p.y)),
			Point::new(hi.x.max(p.x), hi.y.max(p.y)),
		)
	}))
}

/// A rounded rectangle as a path, for clips.
fn rounded_rect(rect: Rect, radii: CornerRadii) -> Path {
	let mut builder = lyon_tessellation::path::Path::builder();
	builder.add_rounded_rectangle(
		&Box2D::new(
			point(rect.x, rect.y),
			point(rect.x + rect.width, rect.y + rect.height),
		),
		&BorderRadii {
			top_left: radii.top_left,
			top_right: radii.top_right,
			bottom_left: radii.bottom_left,
			bottom_right: radii.bottom_right,
		},
		Winding::Positive,
	);
	from_lyon(&builder.build())
}

/// Converts lyon's path back to ours, so clip rectangles share the path
/// code.
fn from_lyon(path: &lyon_tessellation::path::Path) -> Path {
	use lyon_tessellation::path::Event;
	let at = |p: lyon_tessellation::math::Point| Point::new(p.x, p.y);
	let mut commands = Vec::new();
	for event in path {
		match event {
			Event::Begin { at: to } => commands.push(PathCommand::MoveTo(at(to))),
			Event::Line { to, .. } => commands.push(PathCommand::LineTo(at(to))),
			Event::Quadratic { ctrl, to, .. } => {
				commands.push(PathCommand::QuadTo(at(ctrl), at(to)));
			}
			Event::Cubic {
				ctrl1, ctrl2, to, ..
			} => {
				commands.push(PathCommand::CubicTo(at(ctrl1), at(ctrl2), at(to)));
			}
			Event::End { close, .. } => {
				if close {
					commands.push(PathCommand::Close);
				}
			}
		}
	}
	Path { commands }
}

/// Builds lyon's path from ours. A line or curve with no outline open starts
/// one where the last outline began, as SVG does after a close.
fn to_lyon(path: &Path) -> lyon_tessellation::path::Path {
	let mut builder = lyon_tessellation::path::Path::builder();
	let mut open = false;
	let mut start = point(0.0, 0.0);
	let to = |p: Point| point(p.x, p.y);
	for command in &path.commands {
		if !open && !matches!(command, PathCommand::MoveTo(_) | PathCommand::Close) {
			builder.begin(start);
			open = true;
		}
		match *command {
			PathCommand::MoveTo(p) => {
				if open {
					builder.end(false);
				}
				start = to(p);
				builder.begin(start);
				open = true;
			}
			PathCommand::LineTo(p) => {
				builder.line_to(to(p));
			}
			PathCommand::QuadTo(c, p) => {
				builder.quadratic_bezier_to(to(c), to(p));
			}
			PathCommand::CubicTo(c1, c2, p) => {
				builder.cubic_bezier_to(to(c1), to(c2), to(p));
			}
			PathCommand::Close => {
				if open {
					builder.end(true);
					open = false;
				}
			}
		}
	}
	if open {
		builder.end(false);
	}
	builder.build()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn layers_split_passes_and_restore_the_stencil() {
		let scene = Scene::new()
			.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0), 0.0)
			.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK)
			.push_layer(0.5)
			.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE)
			.pop_layer()
			.pop_clip()
			.sanitized();
		let frame = encode(&scene, 10, 10, 1.0);
		assert_eq!(frame.levels, 2);
		let levels: Vec<_> = frame
			.segments
			.iter()
			.map(|segment| (segment.level, segment.clear))
			.collect();
		assert_eq!(levels, [(0, true), (1, true), (0, false)]);
		let composite = &frame.steps[frame.segments[2].steps.start];
		assert_eq!(
			composite.draw,
			Draw::Composite {
				level: 1,
				instance: 0
			}
		);
		assert_eq!(composite.stencil, 1);
		assert_eq!(frame.steps.last().map(|step| step.stencil), Some(1));
	}

	#[test]
	fn neighbouring_quads_share_one_draw() {
		let scene = Scene::new()
			.fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), Color::BLACK)
			.fill_rect(Rect::new(4.0, 0.0, 4.0, 4.0), Color::WHITE)
			.fill_rect(Rect::new(400.0, 0.0, 4.0, 4.0), Color::WHITE);
		let frame = encode(&scene, 10, 10, 1.0);
		assert_eq!(frame.steps.len(), 1);
		assert_eq!(frame.steps[0].draw, Draw::Quads(0..2));
	}

	#[test]
	fn ramps_mix_premultiplied_and_hold_past_the_ends() {
		let stops = [
			GradientStop::new(0.25, Color::rgba(1.0, 0.0, 0.0, 1.0)),
			GradientStop::new(0.75, Color::rgba(0.0, 0.0, 1.0, 0.0)),
		];
		assert_eq!(ramp_color(&stops, 0.0), [1.0, 0.0, 0.0, 1.0]);
		assert_eq!(ramp_color(&stops, 1.0), [0.0, 0.0, 0.0, 0.0]);
		assert_eq!(ramp_color(&stops, 0.5), [0.5, 0.0, 0.0, 0.5]);
		assert_eq!(ramp_color(&[], 0.5), [0.0; 4], "no stops, no colour");
	}

	#[test]
	fn huge_zoom_keeps_tessellation_bounded() {
		let scene = Scene::new()
			.push_transform(Affine::scale(1.0e4, 1.0e4))
			.fill_path(
				Path::new().circle(0.0, 0.0, 1.0e3),
				FillRule::NonZero,
				Color::BLACK,
			)
			.sanitized();
		let frame = encode(&scene, 100, 100, 1.0);
		assert!(
			frame.mesh.vertices.len() < 10_000,
			"{}",
			frame.mesh.vertices.len()
		);
	}

	#[test]
	fn an_empty_scene_still_produces_a_default_paint() {
		let frame = encode(&Scene::new(), 10, 10, 1.0);
		assert!(frame.instances.is_empty());
		assert!(frame.mesh.vertices.is_empty());
		assert_eq!(frame.paints.len(), 1, "the shader needs one paint to bind");
		assert_eq!(frame.paints[0].kind, 0);
		assert_eq!(frame.paints[0].color, [0.0; 4]);
		assert_eq!(frame.ramp_rows(), 0);
		assert_eq!(frame.steps, [] as [Step; 0]);
		assert_eq!(
			frame.segments,
			[Segment {
				level: 0,
				clear: true,
				steps: 0..0,
			}],
		);
		assert_eq!(frame.levels, 1);
	}

	#[test]
	fn fill_instances_carry_geometry_border_and_paint() {
		let scene = Scene::new()
			.with(Command::FillRect {
				rect: Rect::new(0.0, 0.0, 20.0, 10.0),
				radii: CornerRadii {
					top_left: 4.0,
					top_right: 3.0,
					bottom_right: 2.0,
					bottom_left: 1.0,
				},
				paint: Color::rgba(1.0, 0.0, 0.0, 0.5).into(),
				border: Some(Border {
					width: 2.0,
					color: Color::BLACK,
				}),
			})
			.sanitized();
		let frame = encode(&scene, 20, 10, 1.0);
		assert_eq!(frame.steps.len(), 1);
		assert_eq!(frame.instances.len(), 1);
		let quad = &frame.instances[0];
		assert_eq!(quad.rect, [0.0, 0.0, 20.0, 10.0]);
		assert_eq!(quad.radii, [4.0, 3.0, 2.0, 1.0]);
		// One pixel per point: the device matrix is the identity, padded by
		// the one-pixel anti-aliasing margin.
		assert_eq!(quad.m0, [1.0, 0.0, 0.0, 1.0]);
		assert_eq!(quad.m1, [0.0, 0.0, 1.0, 1.0]);
		assert_eq!(quad.params[0], KIND_FILL);
		assert_eq!(quad.params[1], 2.0, "the border width rides in params");
		assert_eq!(
			quad.color,
			[0.0, 0.0, 0.0, 1.0],
			"the border colour rides in color"
		);
		assert_eq!(quad.paint, [0, 0, 0, 0]);
		assert_eq!(
			frame.paints[0].color,
			[0.5, 0.0, 0.0, 0.5],
			"the fill colour is premultiplied"
		);
	}

	#[test]
	fn box_shadows_move_grow_and_skip_the_invisible() {
		let scene = Scene::new()
			.box_shadow(
				Rect::new(10.0, 10.0, 20.0, 20.0),
				4.0,
				BoxShadow {
					offset_x: 5.0,
					offset_y: 3.0,
					blur: 6.0,
					spread: 2.0,
					color: Color::rgba(0.0, 0.0, 0.0, 0.5),
				},
			)
			// A fully transparent shadow draws nothing.
			.box_shadow(
				Rect::new(0.0, 0.0, 10.0, 10.0),
				0.0,
				BoxShadow {
					color: Color::TRANSPARENT,
					..BoxShadow::default()
				},
			)
			// A negative spread can shrink the shadow past its corners and
			// a corner with no radius stays square.
			.box_shadow(
				Rect::new(10.0, 10.0, 4.0, 4.0),
				0.0,
				BoxShadow {
					spread: -1.0,
					color: Color::BLACK,
					..BoxShadow::default()
				},
			)
			.sanitized();
		let frame = encode(&scene, 50, 50, 1.0);
		assert_eq!(frame.instances.len(), 2);
		let shadow = &frame.instances[0];
		assert_eq!(
			shadow.rect,
			[13.0, 11.0, 24.0, 24.0],
			"the offset moves it and the spread grows it twice"
		);
		assert_eq!(shadow.radii, [6.0; 4], "the spread grows the corners too");
		assert_eq!(shadow.params[0], KIND_SHADOW);
		assert_eq!(shadow.params[2], 3.0, "sigma is half the blur");
		assert_eq!(shadow.color, [0.0, 0.0, 0.0, 0.5]);
		assert_eq!(shadow.paint, [0, 0, 0, 0], "shadows need no paint");
		let shrunk = &frame.instances[1];
		assert_eq!(shrunk.rect, [11.0, 11.0, 2.0, 2.0]);
		assert_eq!(
			shrunk.radii, [0.0; 4],
			"a corner with no radius stays square"
		);
		assert_eq!(frame.steps.len(), 1, "the two shadow quads merge");
	}

	#[test]
	fn strokes_encode_every_cap_and_join() {
		let line = || Path::new().move_to(5.0, 5.0).line_to(35.0, 5.0);
		for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
			for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
				let scene = Scene::new()
					.stroke_path(line(), Stroke::new(4.0).cap(cap).join(join), Color::BLACK)
					.sanitized();
				let frame = encode(&scene, 40, 10, 1.0);
				assert_eq!(frame.steps.len(), 1, "{cap:?} {join:?}");
				let Draw::Mesh(range) = &frame.steps[0].draw else {
					panic!("a mesh draw");
				};
				assert!(!range.is_empty(), "{cap:?} {join:?}");
			}
		}
	}

	#[test]
	fn strokes_that_cannot_draw_are_dropped() {
		// A zero width draws nothing; a stroke whose outline, padded by how
		// far its miter could reach, lands off the target never tessellates.
		let scene = Scene::new()
			.stroke_path(
				Path::new().move_to(0.0, 0.0).line_to(5.0, 0.0),
				Stroke::new(0.0),
				Color::BLACK,
			)
			.stroke_path(
				Path::new().move_to(-100.0, 0.0).line_to(-60.0, 0.0),
				Stroke::new(2.0),
				Color::BLACK,
			)
			.sanitized();
		let frame = encode(&scene, 40, 10, 1.0);
		assert_eq!(frame.steps, [] as [Step; 0]);
		assert!(frame.mesh.vertices.is_empty());
	}

	#[test]
	fn paths_without_leading_moves_and_inner_moves_fill() {
		// A line with no outline open starts one where the last outline
		// began, as SVG does after a close: here at the origin.
		let scene = Scene::new()
			.fill_path(
				Path::new().line_to(20.0, 0.0).line_to(20.0, 20.0).close(),
				FillRule::NonZero,
				Color::BLACK,
			)
			.sanitized();
		let frame = encode(&scene, 40, 40, 1.0);
		assert_eq!(frame.steps.len(), 1);

		// A MoveTo inside an open outline ends it and starts another; the
		// outline still open at the end is finished for the tessellator.
		let twin = Path::new()
			.move_to(0.0, 0.0)
			.line_to(8.0, 0.0)
			.line_to(8.0, 8.0)
			.move_to(16.0, 0.0)
			.line_to(24.0, 0.0)
			.line_to(24.0, 8.0)
			.line_to(16.0, 8.0);
		let scene = Scene::new()
			.fill_path(twin, FillRule::NonZero, Color::BLACK)
			.sanitized();
		let frame = encode(&scene, 40, 40, 1.0);
		assert_eq!(frame.steps.len(), 1);
	}

	#[test]
	fn quadratic_and_cubic_commands_fill() {
		let curve = Path::new()
			.move_to(0.0, 30.0)
			.quad_to(20.0, 0.0, 40.0, 30.0)
			.cubic_to(
				Point::new(60.0, 0.0),
				Point::new(80.0, 60.0),
				Point::new(100.0, 30.0),
			)
			.line_to(100.0, 40.0)
			.line_to(0.0, 40.0)
			.close();
		let scene = Scene::new()
			.fill_path(curve, FillRule::EvenOdd, Color::BLACK)
			.sanitized();
		let frame = encode(&scene, 100, 40, 1.0);
		assert_eq!(frame.steps.len(), 1);
		let Draw::Mesh(range) = &frame.steps[0].draw else {
			panic!("a mesh draw");
		};
		assert!(!range.is_empty());
	}

	#[test]
	fn clip_paths_push_and_their_pops_restore_the_stencil() {
		let rings = Path::new()
			.rect(Rect::new(0.0, 0.0, 20.0, 20.0))
			.rect(Rect::new(4.0, 4.0, 12.0, 12.0));
		let scene = Scene::new()
			.push_clip_path(rings, FillRule::EvenOdd)
			.fill_rect(Rect::new(0.0, 0.0, 20.0, 20.0), Color::BLACK)
			.pop_clip()
			.sanitized();
		let frame = encode(&scene, 20, 20, 1.0);
		assert_eq!(frame.steps.len(), 3);
		let Draw::ClipPush(push) = &frame.steps[0].draw else {
			panic!("a clip push");
		};
		assert!(!push.is_empty(), "the rings tessellate to real geometry");
		assert_eq!(frame.steps[0].stencil, 0);
		assert!(matches!(&frame.steps[1].draw, Draw::Quads(_)));
		assert_eq!(frame.steps[1].stencil, 1);
		assert_eq!(frame.steps[2].draw, Draw::ClipPop(push.clone()));
		assert_eq!(frame.steps[2].stencil, 1);
	}

	#[test]
	fn rounded_clips_round_trip_through_lyon() {
		let scene = Scene::new()
			.push_clip(Rect::new(0.0, 0.0, 20.0, 20.0), CornerRadii::all(10.0))
			.fill_rect(Rect::new(0.0, 0.0, 20.0, 20.0), Color::BLACK)
			.pop_clip()
			.sanitized();
		let frame = encode(&scene, 20, 20, 1.0);
		let Draw::ClipPush(range) = &frame.steps[0].draw else {
			panic!("a clip push");
		};
		assert!(
			range.len() >= 16,
			"four cubic corners tessellate to real geometry: {range:?}"
		);
	}

	#[test]
	fn an_off_target_clip_pushes_an_empty_shape() {
		let scene = Scene::new()
			.push_clip(Rect::new(-40.0, 0.0, 10.0, 10.0), 0.0)
			.fill_rect(Rect::new(0.0, 0.0, 20.0, 20.0), Color::BLACK)
			.pop_clip()
			.sanitized();
		let frame = encode(&scene, 20, 20, 1.0);
		// The empty clip still pushes and pops so the stencil stays
		// balanced; what it clips draws on, unhindered.
		assert_eq!(
			frame.steps,
			[
				Step {
					draw: Draw::ClipPush(0..0),
					stencil: 0,
				},
				Step {
					draw: Draw::Quads(0..1),
					stencil: 1,
				},
				Step {
					draw: Draw::ClipPop(0..0),
					stencil: 1,
				},
			],
		);
	}

	#[test]
	fn empty_rects_and_off_target_geometry_are_dropped() {
		let scene = Scene::new()
			.fill_rect(Rect::new(0.0, 0.0, 0.0, 10.0), Color::BLACK)
			.fill_rect(Rect::new(0.0, 0.0, 10.0, -0.0), Color::BLACK)
			.fill_rect(Rect::new(-30.0, 0.0, 10.0, 10.0), Color::BLACK)
			.fill_rect(Rect::new(0.0, 50.0, 10.0, 10.0), Color::BLACK)
			.fill_path(
				Path::new().circle(-100.0, 5.0, 4.0),
				FillRule::NonZero,
				Color::BLACK,
			)
			// A point has no inside: the tessellation succeeds with no
			// triangles and nothing is drawn.
			.fill_path(
				Path::new().move_to(5.0, 5.0).close(),
				FillRule::NonZero,
				Color::BLACK,
			)
			.stroke_path(
				Path::new().move_to(5.0, 5.0).close(),
				Stroke::new(2.0),
				Color::BLACK,
			)
			.sanitized();
		let frame = encode(&scene, 10, 10, 1.0);
		assert!(frame.instances.is_empty());
		assert_eq!(frame.steps, [] as [Step; 0]);
		assert!(frame.mesh.vertices.is_empty());
	}

	#[test]
	fn gradients_register_paints_and_bake_ramps() {
		let linear = Paint::LinearGradient {
			start: Point::new(0.0, 0.0),
			end: Point::new(100.0, 0.0),
			stops: vec![
				GradientStop::new(0.0, Color::rgba(1.0, 0.0, 0.0, 1.0)),
				GradientStop::new(1.0, Color::rgba(0.0, 0.0, 1.0, 1.0)),
			],
		};
		let radial = Paint::RadialGradient {
			center: Point::new(50.0, 5.0),
			radius: 5.0,
			stops: vec![
				GradientStop::new(0.0, Color::WHITE),
				GradientStop::new(1.0, Color::BLACK),
			],
		};
		let scene = Scene::new()
			.fill_rect(Rect::new(0.0, 0.0, 100.0, 10.0), linear)
			.fill_rect(Rect::new(0.0, 0.0, 100.0, 10.0), radial)
			.fill_rect(Rect::new(0.0, 0.0, 100.0, 10.0), Color::BLACK)
			.sanitized();
		let frame = encode(&scene, 100, 10, 1.0);
		assert_eq!(frame.paints.len(), 3);
		assert_eq!(frame.paints[0].kind, 1, "linear");
		assert_eq!(frame.paints[0].row, 0);
		assert_eq!(frame.paints[1].kind, 2, "radial");
		assert_eq!(frame.paints[1].row, 1);
		assert_eq!(frame.paints[2].kind, 0, "solid");
		assert_eq!(frame.paints[2].color, [0.0, 0.0, 0.0, 1.0]);
		assert_eq!(frame.ramp_rows(), 2);
		assert_eq!(frame.ramps.len(), 2 * RAMP_WIDTH * 4);
		assert_eq!(&frame.ramps[..4], &[255, 0, 0, 255], "the first stop");
		assert_eq!(
			&frame.ramps[(RAMP_WIDTH - 1) * 4..RAMP_WIDTH * 4],
			&[0, 0, 255, 255],
			"the last stop"
		);
	}

	#[test]
	fn degenerate_gradients_fall_back_to_solids() {
		let scene = Scene::new()
			.fill_rect(
				Rect::new(0.0, 0.0, 10.0, 10.0),
				Paint::LinearGradient {
					start: Point::new(3.0, 3.0),
					end: Point::new(3.0, 3.0),
					stops: vec![
						GradientStop::new(0.0, Color::WHITE),
						GradientStop::new(1.0, Color::BLACK),
					],
				},
			)
			.fill_rect(
				Rect::new(0.0, 0.0, 10.0, 10.0),
				Paint::RadialGradient {
					center: Point::new(5.0, 5.0),
					radius: 0.0,
					stops: vec![
						GradientStop::new(0.0, Color::WHITE),
						GradientStop::new(0.5, Color::BLACK),
					],
				},
			)
			.sanitized();
		let frame = encode(&scene, 10, 10, 1.0);
		assert_eq!(frame.paints.len(), 2);
		assert!(
			frame.paints.iter().all(|paint| paint.kind == 0),
			"both gradients collapsed to solids"
		);
		assert_eq!(frame.paints[0].color, [0.0, 0.0, 0.0, 1.0]);
		assert_eq!(frame.paints[1].color, [0.0, 0.0, 0.0, 1.0]);
		assert!(frame.ramps.is_empty(), "no ramps were baked");
	}

	#[test]
	fn raw_scenes_encode_defensively() {
		// encode draws a scene as given: an empty gradient paints nothing
		// and a pop with nothing open is ignored.
		let scene = Scene::new()
			.pop_clip()
			.pop_layer()
			.pop_transform()
			.with(Command::FillRect {
				rect: Rect::new(0.0, 0.0, 10.0, 10.0),
				radii: CornerRadii::default(),
				paint: Paint::LinearGradient {
					start: Point::new(0.0, 0.0),
					end: Point::new(0.0, 0.0),
					stops: Vec::new(),
				},
				border: None,
			});
		let frame = encode(&scene, 10, 10, 1.0);
		assert_eq!(frame.paints.len(), 1);
		assert_eq!(frame.paints[0].color, [0.0; 4]);
		assert_eq!(frame.steps.len(), 1);
	}

	#[test]
	fn transforms_compose_and_a_flat_transform_drops_geometry() {
		let scene = Scene::new()
			.push_transform(Affine::translate(10.0, 0.0))
			.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK)
			.push_transform(Affine::rotate(std::f32::consts::FRAC_PI_2))
			.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK)
			.fill_path(
				Path::new().rect(Rect::new(0.0, 0.0, 8.0, 8.0)),
				FillRule::NonZero,
				Color::BLACK,
			)
			.stroke_path(
				Path::new().move_to(0.0, 0.0).line_to(6.0, 0.0),
				Stroke::new(1.0),
				Color::BLACK,
			)
			.pop_transform()
			.pop_transform()
			.push_transform(Affine::scale(0.0, 0.0))
			.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK)
			.fill_path(
				Path::new().circle(5.0, 5.0, 4.0),
				FillRule::NonZero,
				Color::BLACK,
			)
			.stroke_path(
				Path::new().move_to(0.0, 0.0).line_to(6.0, 0.0),
				Stroke::new(1.0),
				Color::BLACK,
			)
			.pop_transform()
			.sanitized();
		let frame = encode(&scene, 100, 100, 1.0);
		// The two translated rects, then one merged mesh for the rotated
		// fill and stroke; everything under the flat transform collapses.
		assert_eq!(frame.instances.len(), 2);
		assert_eq!(frame.steps.len(), 2);
		assert_eq!(frame.steps[0].draw, Draw::Quads(0..2));
		let Draw::Mesh(range) = &frame.steps[1].draw else {
			panic!("a mesh draw");
		};
		assert!(!range.is_empty());
	}
}
