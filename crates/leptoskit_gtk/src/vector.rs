//! Filled and stroked vector paths painted with cairo.
//!
//! The view box is mapped onto the allocation before the cairo path is built,
//! so stroke widths and dashes are set in frame units: fixed strokes keep
//! their layout size at every scale and scaling strokes follow the drawing.
//! The cairo node covers the painted extent rather than the allocation: like
//! the other platforms, a drawing is clipped only when its frame clips.
//!
//! GTK keeps the render node a snapshot produced until the widget queues a
//! draw or its allocation changes, so an unchanged drawing is never repainted.

use std::cell::RefCell;

use gtk::cairo;
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use leptoskit_native::Color;
use leptoskit_native::Drawing;
use leptoskit_native::FillRule;
use leptoskit_native::LineCap;
use leptoskit_native::LineJoin;
use leptoskit_native::Stroke;
use leptoskit_native::vector::PathCommand;
use leptoskit_native::vector::Point;
use leptoskit_native::vector::Transform;

mod imp {
	use super::*;

	#[derive(Default)]
	pub struct Vector {
		pub(super) drawing: RefCell<Option<Drawing>>,
		/// Reused frame-unit dash lengths, so repaints do not allocate.
		pub(super) dashes: RefCell<Vec<f64>>,
	}

	#[glib::object_subclass]
	impl ObjectSubclass for Vector {
		type ParentType = gtk::Widget;
		type Type = super::Vector;

		const NAME: &'static str = "LeptoskitVector";
	}

	impl ObjectImpl for Vector {}

	impl WidgetImpl for Vector {
		fn snapshot(&self, snapshot: &gtk::Snapshot) {
			let drawing = self.drawing.borrow();
			let Some(drawing) = drawing.as_ref() else {
				return;
			};
			let obj = self.obj();
			let transform = drawing.transform(obj.width() as f32, obj.height() as f32);
			let Some(bounds) = paint_bounds(drawing, transform) else {
				return;
			};
			let context = snapshot.append_cairo(&bounds);

			if let Err(error) = paint(&context, drawing, transform, &mut self.dashes.borrow_mut()) {
				glib::g_critical!("leptoskit", "painting a Vector failed: {error}");
			}
		}
	}
}

glib::wrapper! {
	/// The native control of a `Vector` element.
	pub struct Vector(ObjectSubclass<imp::Vector>)
		@extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Vector {
	/// A drawing surface with a fixed accessible role. GTK cannot change a
	/// role once assistive technology has seen the widget, so the backend
	/// replaces the control when the role must change.
	pub fn new(role: gtk::AccessibleRole) -> Self {
		glib::Object::builder()
			.property("accessible-role", role)
			.build()
	}

	/// Replaces the drawing, repainting only when it changed. A new
	/// allocation repaints without this call.
	pub fn set_drawing(&self, drawing: Option<&Drawing>) {
		let mut current = self.imp().drawing.borrow_mut();

		if current.as_ref() == drawing {
			return;
		}
		*current = drawing.cloned();
		drop(current);
		self.queue_draw();
	}
}

/// The accessible role of a `Vector` control: an explicit ARIA role, else an
/// image when labelled and decorative otherwise.
pub(crate) fn role(props: &leptoskit_native::NativeProps) -> gtk::AccessibleRole {
	match props.aria().role {
		Some(role) => crate::accessibility::role(role),

		None if props.accessibility_label.is_some() => gtk::AccessibleRole::Img,
		None => gtk::AccessibleRole::None,
	}
}

/// The frame-space rectangle the painted paths can reach, or `None` when
/// nothing paints.
///
/// A Bézier curve lies within its control points' hull, so the points bound
/// every outline. Strokes extend that by their outset. One more unit leaves
/// room for antialiasing.
fn paint_bounds(drawing: &Drawing, transform: Transform) -> Option<graphene::Rect> {
	let mut extent: Option<(Point, Point)> = None;
	let mut outset: f32 = 0.0;

	for path in drawing.paths() {
		if path.fill_color().is_none() && path.stroke_style().is_none() {
			continue;
		}

		if let Some(stroke) = path.stroke_style() {
			outset = outset.max(stroke_outset(stroke, transform));
		}

		for command in path.data().transformed(transform) {
			for point in points(command) {
				let (min, max) = extent.get_or_insert((point, point));
				min.x = min.x.min(point.x);
				min.y = min.y.min(point.y);
				max.x = max.x.max(point.x);
				max.y = max.y.max(point.y);
			}
		}
	}

	let (min, max) = extent?;

	let outset = outset + 1.0;
	Some(graphene::Rect::new(
		min.x - outset,
		min.y - outset,
		max.x - min.x + outset * 2.0,
		max.y - min.y + outset * 2.0,
	))
}

/// How far past its outline a stroke can paint, in frame units.
///
/// Half the width on either side, except at corners and ends: a miter join
/// reaches half the miter length, which the miter limit caps at `limit`
/// half-widths, and a square cap's corners reach √2 half-widths. Dashes only
/// remove paint; every dash end is capped like a line end, which the cap
/// factor already covers.
fn stroke_outset(stroke: &Stroke, transform: Transform) -> f32 {
	let join = match stroke.line_join() {
		LineJoin::Miter => stroke.miter(),
		LineJoin::Round | LineJoin::Bevel => 1.0,
	};

	let cap = match stroke.line_cap() {
		LineCap::Square => std::f32::consts::SQRT_2,
		LineCap::Butt | LineCap::Round => 1.0,
	};

	stroke.frame_width(transform) / 2.0 * join.max(cap)
}

fn points(command: PathCommand) -> impl Iterator<Item = Point> {
	let points = match command {
		PathCommand::MoveTo(point) | PathCommand::LineTo(point) => [Some(point), None, None],
		PathCommand::QuadTo { control, to } => [Some(control), Some(to), None],
		PathCommand::CubicTo {
			control1,
			control2,
			to,
		} => [Some(control1), Some(control2), Some(to)],
		PathCommand::Close => [None, None, None],
	};

	points.into_iter().flatten()
}

/// Paints every path in order, each filled with its rule and then stroked.
///
/// `dashes` is scratch space for the frame-unit dash pattern.
fn paint(
	context: &cairo::Context,
	drawing: &Drawing,
	transform: Transform,
	dashes: &mut Vec<f64>,
) -> Result<(), cairo::Error> {
	for path in drawing.paths() {
		let stroke = path
			.stroke_style()
			.filter(|stroke| stroke.frame_width(transform) > 0.0);

		if path.fill_color().is_none() && stroke.is_none() {
			continue;
		}

		trace(context, path.data().transformed(transform));

		if let Some(color) = path.fill_color() {
			context.set_fill_rule(match path.rule() {
				FillRule::Nonzero => cairo::FillRule::Winding,
				FillRule::Evenodd => cairo::FillRule::EvenOdd,
			});

			source(context, color);
			context.fill_preserve()?;
		}

		match stroke {
			Some(stroke) => {
				apply_stroke(context, stroke, transform, dashes);
				context.stroke()?;
			}
			None => context.new_path(),
		}
	}

	Ok(())
}

/// Sets every stroke parameter, so no state leaks from the previous path.
///
/// A drawing-scaled stroke under a zero scale has zero width and is skipped
/// before this, which also keeps cairo from rejecting an all-zero dash
/// pattern.
fn apply_stroke(
	context: &cairo::Context,
	stroke: &Stroke,
	transform: Transform,
	dashes: &mut Vec<f64>,
) {
	source(context, stroke.color());
	context.set_line_width(f64::from(stroke.frame_width(transform)));
	context.set_line_cap(match stroke.line_cap() {
		LineCap::Butt => cairo::LineCap::Butt,
		LineCap::Round => cairo::LineCap::Round,
		LineCap::Square => cairo::LineCap::Square,
	});
	context.set_line_join(match stroke.line_join() {
		LineJoin::Miter => cairo::LineJoin::Miter,
		LineJoin::Round => cairo::LineJoin::Round,
		LineJoin::Bevel => cairo::LineJoin::Bevel,
	});

	context.set_miter_limit(f64::from(stroke.miter()));
	dashes.clear();
	dashes.extend(stroke.frame_dashes(transform).map(f64::from));
	context.set_dash(dashes, f64::from(stroke.frame_dash_offset(transform)));
}

/// Replaces the context's path with frame-space commands. Cairo has no
/// quadratic curve, so each one is raised to the equivalent cubic.
fn trace(context: &cairo::Context, commands: impl Iterator<Item = PathCommand>) {
	context.new_path();
	let mut start = Point::default();
	let mut current = Point::default();

	for command in commands {
		match command {
			PathCommand::MoveTo(point) => {
				context.move_to(point.x.into(), point.y.into());
				start = point;
				current = point;
			}
			PathCommand::LineTo(point) => {
				context.line_to(point.x.into(), point.y.into());
				current = point;
			}
			PathCommand::QuadTo { control, to } => {
				let toward = |from: Point| {
					Point::new(
						from.x + (control.x - from.x) * 2.0 / 3.0,
						from.y + (control.y - from.y) * 2.0 / 3.0,
					)
				};
				let (control1, control2) = (toward(current), toward(to));
				context.curve_to(
					control1.x.into(),
					control1.y.into(),
					control2.x.into(),
					control2.y.into(),
					to.x.into(),
					to.y.into(),
				);
				current = to;
			}
			PathCommand::CubicTo {
				control1,
				control2,
				to,
			} => {
				context.curve_to(
					control1.x.into(),
					control1.y.into(),
					control2.x.into(),
					control2.y.into(),
					to.x.into(),
					to.y.into(),
				);
				current = to;
			}
			PathCommand::Close => {
				context.close_path();
				current = start;
			}
		}
	}
}

fn source(context: &cairo::Context, color: Color) {
	context.set_source_rgba(
		f64::from(color.red()) / 255.0,
		f64::from(color.green()) / 255.0,
		f64::from(color.blue()) / 255.0,
		f64::from(color.alpha()) / 255.0,
	);
}
