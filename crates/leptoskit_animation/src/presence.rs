//! Entering and exiting: how a widget appears when it is added and leaves
//! when it is removed, like Reanimated's `entering` and `exiting` layout
//! animations.

use crate::curve::Curve;
use crate::curve::Easing;
use crate::curve::Spring;
use crate::curve::sanitize_ms;
use crate::transform::Offset;
use crate::transform::Transform;
use crate::transform::Visual;

/// An edge of a widget or the screen, for sliding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
	/// The left edge.
	Left,
	/// The right edge.
	Right,
	/// The top edge.
	Top,
	/// The bottom edge.
	Bottom,
}

/// How a widget enters or exits.
///
/// As an entering animation, the widget starts at [`visual`](Self::visual)
/// and moves to its own look. As an exiting one, the platform keeps drawing
/// the removed widget while it moves from its own look to `visual`, then
/// removes it. Unset parts of `visual` stay at the widget's own value.
///
/// ```
/// use leptoskit_animation::Edge;
/// use leptoskit_animation::Presence;
/// use leptoskit_animation::Spring;
///
/// // Fade in while rising from a little below.
/// let entering = Presence::fade().slide_by(0.0, 16.0);
/// // Slide off to the left on a spring.
/// let exiting = Presence::slide(Edge::Left).spring(Spring::SNAPPY);
/// # let _ = (entering, exiting);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Presence {
	/// The look the widget enters from or exits to.
	pub visual: Visual,
	/// How it moves.
	pub curve: Curve,
	/// How long to wait before moving, in milliseconds.
	#[cfg_attr(feature = "serde", serde(default))]
	pub delay_ms: f32,
}

impl Presence {
	/// The curve presets use unless told otherwise: 300 ms, easing in and
	/// out, Reanimated's default duration.
	pub const DEFAULT_CURVE: Curve = Curve::timing(300.0, Easing::EASE_IN_OUT);

	/// Moves from or to `visual` on the default curve.
	#[must_use]
	pub const fn new(visual: Visual) -> Self {
		Self {
			visual,
			curve: Self::DEFAULT_CURVE,
			delay_ms: 0.0,
		}
	}

	/// Fades in from, or out to, transparent. Reanimated's `FadeIn` and
	/// `FadeOut`.
	#[must_use]
	pub const fn fade() -> Self {
		Self::new(Visual::NONE).opacity(0.0)
	}

	/// Slides in from, or out to, `edge`, by the widget's own size.
	/// Reanimated's `SlideInLeft`, `SlideOutRight`, and the like, measured
	/// by the widget rather than the screen.
	#[must_use]
	pub const fn slide(edge: Edge) -> Self {
		let (x, y) = match edge {
			Edge::Left => (Offset::percent(-100.0), Offset::ZERO),
			Edge::Right => (Offset::percent(100.0), Offset::ZERO),
			Edge::Top => (Offset::ZERO, Offset::percent(-100.0)),
			Edge::Bottom => (Offset::ZERO, Offset::percent(100.0)),
		};

		Self::new(Visual::NONE).translate(x, y)
	}

	/// Grows from, or shrinks to, nothing. Reanimated's `ZoomIn` and
	/// `ZoomOut`.
	#[must_use]
	pub const fn zoom() -> Self {
		Self::new(Visual::NONE).scale(0.0)
	}

	/// Sets the opacity entered from or exited to.
	#[must_use]
	pub const fn opacity(mut self, opacity: f32) -> Self {
		self.visual.opacity = Some(opacity);
		self
	}

	/// Sets the uniform scale entered from or exited to.
	#[must_use]
	pub const fn scale(mut self, scale: f32) -> Self {
		let transform = self.transform();
		self.visual.transform = Some(transform.scaled(scale, scale));
		self
	}

	/// Sets the movement entered from or exited to.
	#[must_use]
	pub const fn translate(mut self, x: Offset, y: Offset) -> Self {
		let transform = self.transform();
		self.visual.transform = Some(transform.translated(x, y));
		self
	}

	/// Sets the movement entered from or exited to, in layout units.
	#[must_use]
	pub const fn slide_by(self, x: f32, y: f32) -> Self {
		self.translate(Offset::points(x), Offset::points(y))
	}

	/// Sets the rotation entered from or exited to, in degrees.
	#[must_use]
	pub const fn rotate(mut self, degrees: f32) -> Self {
		let transform = self.transform();
		self.visual.transform = Some(transform.rotated(degrees));
		self
	}

	/// Moves over `duration_ms`, keeping the easing, or easing in and out
	/// when the curve was a spring.
	#[must_use]
	pub const fn duration(mut self, duration_ms: f32) -> Self {
		let easing = match self.curve {
			Curve::Timing { easing, .. } => easing,
			Curve::Spring(_) => Easing::EASE_IN_OUT,
		};
		self.curve = Curve::timing(duration_ms, easing);
		self
	}

	/// Eases the motion with `easing`, keeping the duration, or 300 ms when
	/// the curve was a spring.
	#[must_use]
	pub const fn easing(mut self, easing: Easing) -> Self {
		let duration_ms = match self.curve {
			Curve::Timing { duration_ms, .. } => duration_ms,
			Curve::Spring(_) => 300.0,
		};
		self.curve = Curve::timing(duration_ms, easing);
		self
	}

	/// Moves on `spring`, as Reanimated's `.springify()`.
	#[must_use]
	pub const fn spring(mut self, spring: Spring) -> Self {
		self.curve = Curve::spring(spring);
		self
	}

	/// Waits `delay_ms` before moving, for staggering a list's rows.
	#[must_use]
	pub const fn delay(mut self, delay_ms: f32) -> Self {
		self.delay_ms = delay_ms;
		self
	}

	/// The presence with every value finite and in range.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		Self {
			visual: self.visual.sanitized(),
			curve: self.curve.sanitized(),
			delay_ms: sanitize_ms(self.delay_ms),
		}
	}

	const fn transform(&self) -> Transform {
		match self.visual.transform {
			Some(transform) => transform,
			None => Transform::IDENTITY,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn presets_combine() {
		let presence = Presence::fade().scale(0.95).slide_by(0.0, 8.0).delay(50.0);
		assert_eq!(presence.visual.opacity, Some(0.0));
		let transform = presence.visual.transform.unwrap();
		assert!((transform.scale_x - 0.95).abs() < f32::EPSILON);
		assert!((transform.translate_y.points - 8.0).abs() < f32::EPSILON);
		assert!((presence.delay_ms - 50.0).abs() < f32::EPSILON);
	}

	#[test]
	fn sliding_moves_by_the_widgets_own_size() {
		let left = Presence::slide(Edge::Left).visual.transform.unwrap();
		assert!((left.translate_x.percent + 100.0).abs() < f32::EPSILON);
		let bottom = Presence::slide(Edge::Bottom).visual.transform.unwrap();
		assert!((bottom.translate_y.percent - 100.0).abs() < f32::EPSILON);
	}

	#[test]
	fn duration_and_easing_keep_each_other() {
		let presence = Presence::zoom().easing(Easing::Linear).duration(120.0);
		assert_eq!(presence.curve, Curve::timing(120.0, Easing::Linear));
	}
}
