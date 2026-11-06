//! Declarative motion: how an element enters, how it follows scrolling, and
//! how a container staggers its children.
//!
//! A [`Motion`] is a list of [`Pose`]s at points between `0.0` and `1.0`,
//! and what drives the movement between them:
//!
//! - time, once, when the element first appears ([`Motion::enter`]);
//! - the element's own passage through the viewport ([`Motion::in_view`]);
//! - a named ancestor's passage through the viewport ([`Motion::follow`] with
//!   [`Element::timeline`](crate::Element::timeline)).
//!
//! Poses describe only what can move without moving anything else: opacity,
//! offset, scale, rotation, clipping, and blur. Layout never changes, so
//! motion cannot shift the content around it.
//!
//! The web renders motion as CSS animations, scroll-driven animations where
//! scroll drives them, and `linear()` easing for springs, so nothing runs in
//! script. Every animation sits behind `prefers-reduced-motion:
//! no-preference`, and scroll-driven animations also behind `@supports`:
//! people who ask for less motion, and browsers without scroll timelines, see
//! each element at rest at once. Native platforms show the resting pose too:
//! motion is a web feature today, and native hosts never receive it.

use std::time::Duration;

/// How an element looks at one point of a motion.
///
/// Each value a pose sets stands in for the element's own value of that
/// property while the motion runs; [`Motion::enter`] ends on the element's
/// own style. Unset fields leave that property to the rest of the motion,
/// and a property no pose sets never animates. The two offsets move
/// together: a pose that sets only `y` keeps `x` at zero.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
	/// The opacity, from `0.0` (transparent) to `1.0` (opaque).
	pub opacity: Option<f32>,
	/// The horizontal offset, in layout units; positive moves right.
	pub x: Option<f32>,
	/// The vertical offset, in layout units; positive moves down.
	pub y: Option<f32>,
	/// The scale, where `1.0` is the resting size.
	pub scale: Option<f32>,
	/// The rotation, in degrees clockwise.
	pub rotate: Option<f32>,
	/// How much of the element is clipped away from each edge.
	pub clip: Option<Inset>,
	/// A blur radius, in layout units.
	pub blur: Option<f32>,
}

impl Pose {
	/// The resting pose: opaque, in place, at full size, upright, unclipped,
	/// and sharp. Use it between other poses to hold an element at rest: it
	/// sets only the properties the other poses move. Any other pose moves
	/// every property it sets, resting values included.
	pub const REST: Self = Self {
		opacity: Some(1.0),
		x: Some(0.0),
		y: Some(0.0),
		scale: Some(1.0),
		rotate: Some(0.0),
		clip: Some(Inset::NONE),
		blur: Some(0.0),
	};

	/// A pose that sets nothing yet.
	#[must_use]
	pub const fn new() -> Self {
		Self {
			opacity: None,
			x: None,
			y: None,
			scale: None,
			rotate: None,
			clip: None,
			blur: None,
		}
	}

	/// Sets the opacity.
	#[must_use]
	pub const fn opacity(mut self, opacity: f32) -> Self {
		self.opacity = Some(opacity);
		self
	}

	/// Sets the horizontal offset, in layout units.
	#[must_use]
	pub const fn x(mut self, x: f32) -> Self {
		self.x = Some(x);
		self
	}

	/// Sets the vertical offset, in layout units.
	#[must_use]
	pub const fn y(mut self, y: f32) -> Self {
		self.y = Some(y);
		self
	}

	/// Sets the scale.
	#[must_use]
	pub const fn scale(mut self, scale: f32) -> Self {
		self.scale = Some(scale);
		self
	}

	/// Sets the rotation, in degrees clockwise.
	#[must_use]
	pub const fn rotate(mut self, degrees: f32) -> Self {
		self.rotate = Some(degrees);
		self
	}

	/// Clips the element, as [`Inset`] describes.
	#[must_use]
	pub const fn clip(mut self, clip: Inset) -> Self {
		self.clip = Some(clip);
		self
	}

	/// Sets a blur radius, in layout units.
	#[must_use]
	pub const fn blur(mut self, radius: f32) -> Self {
		self.blur = Some(radius);
		self
	}
}

/// How much of an element is clipped away from each edge, as fractions of
/// its size. Clipping hides part of the element without moving it, so a
/// motion from one inset to another reads as the element being drawn in.
///
/// While a clip plays, anything the element paints outside its own box is
/// clipped too.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Inset {
	/// The fraction clipped from the top edge.
	pub top: f32,
	/// The fraction clipped from the right edge.
	pub right: f32,
	/// The fraction clipped from the bottom edge.
	pub bottom: f32,
	/// The fraction clipped from the left edge.
	pub left: f32,
}

impl Inset {
	/// Nothing clipped.
	pub const NONE: Self = Self::new(0.0, 0.0, 0.0, 0.0);

	/// Clips the given fraction from each edge.
	#[must_use]
	pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
		Self {
			top,
			right,
			bottom,
			left,
		}
	}

	/// Shows the `shown` fraction nearest the left edge: moving from `0.0`
	/// to `1.0` draws the element in from the left.
	#[must_use]
	pub const fn from_left(shown: f32) -> Self {
		Self::new(0.0, 1.0 - shown, 0.0, 0.0)
	}

	/// Shows the `shown` fraction nearest the right edge.
	#[must_use]
	pub const fn from_right(shown: f32) -> Self {
		Self::new(0.0, 0.0, 0.0, 1.0 - shown)
	}

	/// Shows the `shown` fraction nearest the top edge.
	#[must_use]
	pub const fn from_top(shown: f32) -> Self {
		Self::new(0.0, 0.0, 1.0 - shown, 0.0)
	}

	/// Shows the `shown` fraction nearest the bottom edge.
	#[must_use]
	pub const fn from_bottom(shown: f32) -> Self {
		Self::new(1.0 - shown, 0.0, 0.0, 0.0)
	}
}

/// How a motion's progress speeds up and slows down between poses.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Easing {
	/// An even pace from start to end.
	Linear,
	/// Leaves quickly and settles gently: for things arriving.
	#[default]
	Out,
	/// Starts gently and leaves quickly: for things departing.
	In,
	/// Gentle at both ends: for moves between two resting places.
	InOut,
	/// A cubic Bézier curve through `(x1, y1)` and `(x2, y2)`, as CSS
	/// `cubic-bezier()` takes it. The x values are clamped between `0.0` and
	/// `1.0`.
	Bezier(f32, f32, f32, f32),
	/// Jumps through this many even steps, the way type appears on a
	/// terminal. Zero steps is one step.
	Steps(u16),
	/// A physical spring, which may overshoot and settle. A motion with a
	/// spring and no duration takes as long as the spring takes to settle.
	Spring(Spring),
}

/// A damped spring that carries a motion from its first pose to its last.
///
/// Stiffer springs are faster, more damping means less bounce, and more
/// mass is slower and heavier. Non-finite or non-positive stiffness and mass
/// settle at once; negative damping counts as none.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
	/// How strongly the spring pulls toward rest.
	pub stiffness: f32,
	/// How strongly motion is resisted.
	pub damping: f32,
	/// The weight being moved.
	pub mass: f32,
}

impl Spring {
	/// A visible but small overshoot, for things that land with character.
	pub const BOUNCY: Self = Self::new(260.0, 12.0, 1.0);
	/// Settles without overshooting, for calm arrivals.
	pub const GENTLE: Self = Self::new(120.0, 22.0, 1.0);
	/// The furthest a spring may be from rest when it counts as settled.
	const SETTLED: f32 = 0.001;
	/// The longest a spring may take to settle.
	const SETTLE_LIMIT: Duration = Duration::from_secs(5);
	/// Quick, with the slightest overshoot, for responses to a press.
	pub const SNAPPY: Self = Self::new(420.0, 30.0, 1.0);

	/// A spring with the given stiffness, damping, and mass.
	#[must_use]
	pub const fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
		Self {
			stiffness,
			damping,
			mass,
		}
	}

	/// Where the spring is `seconds` after it is released: `0.0` at the
	/// start and `1.0` at rest, passing `1.0` when it overshoots.
	#[must_use]
	pub fn position(self, seconds: f32) -> f32 {
		let valid = |value: f32| value.is_finite() && value > 0.0;

		if !valid(self.stiffness) || !valid(self.mass) || !seconds.is_finite() {
			return 1.0;
		}

		let t = f64::from(seconds.max(0.0));
		let damping = if self.damping.is_finite() {
			f64::from(self.damping.max(0.0))
		} else {
			0.0
		};
		let natural = (f64::from(self.stiffness) / f64::from(self.mass)).sqrt();
		let ratio = damping / (2.0 * (f64::from(self.stiffness) * f64::from(self.mass)).sqrt());

		let displacement = if ratio < 1.0 {
			let damped = natural * (1.0 - ratio * ratio).sqrt();
			(-ratio * natural * t).exp()
				* ((damped * t).cos() + ratio * natural / damped * (damped * t).sin())
		} else if (ratio - 1.0).abs() < f64::EPSILON {
			(-natural * t).exp() * (1.0 + natural * t)
		} else {
			let spread = (ratio * ratio - 1.0).sqrt();
			let fast = -natural * (ratio + spread);
			let slow = -natural * (ratio - spread);
			(fast * (slow * t).exp() - slow * (fast * t).exp()) / (fast - slow)
		};

		(1.0 - displacement) as f32
	}

	/// How long the spring takes to stay within a thousandth of rest, at
	/// most five seconds.
	#[must_use]
	pub fn settle_duration(self) -> Duration {
		let step = Duration::from_millis(1);
		let mut settled = Duration::ZERO;
		let mut at = Duration::ZERO;

		while at < Self::SETTLE_LIMIT {
			if (1.0 - self.position(at.as_secs_f32())).abs() > Self::SETTLED {
				settled = at + step;
			}
			at += step;
		}

		settled
	}
}

/// A part of the viewport an element passes through while it scrolls, as
/// CSS scroll-driven animations name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPhase {
	/// From the element's first edge entering the viewport to its last edge
	/// leaving.
	Cover,
	/// While the element is entirely inside the viewport, or, for an element
	/// taller than the viewport, while it fills it.
	Contain,
	/// While the element enters the viewport.
	Entry,
	/// While the element leaves the viewport.
	Exit,
}

/// Where a scroll-driven motion starts and ends: a fraction of one
/// [`ViewPhase`] to a fraction of another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewRange {
	/// The phase the motion starts in.
	pub start: ViewPhase,
	/// How far through `start` the motion starts, from `0.0` to `1.0`.
	pub start_at: f32,
	/// The phase the motion ends in.
	pub end: ViewPhase,
	/// How far through `end` the motion ends, from `0.0` to `1.0`.
	pub end_at: f32,
}

impl ViewRange {
	/// From a fraction of one phase to a fraction of another.
	#[must_use]
	pub const fn new(start: ViewPhase, start_at: f32, end: ViewPhase, end_at: f32) -> Self {
		Self {
			start,
			start_at,
			end,
			end_at,
		}
	}

	/// The part of one phase between two fractions of it.
	#[must_use]
	pub const fn during(phase: ViewPhase, from: f32, to: f32) -> Self {
		Self::new(phase, from, phase, to)
	}

	/// The whole time the element enters the viewport: the motion is
	/// finished once the element is fully in view.
	#[must_use]
	pub const fn entry() -> Self {
		Self::during(ViewPhase::Entry, 0.0, 1.0)
	}

	/// The whole time the element leaves the viewport.
	#[must_use]
	pub const fn exit() -> Self {
		Self::during(ViewPhase::Exit, 0.0, 1.0)
	}

	/// The whole time any part of the element is in view.
	#[must_use]
	pub const fn cover() -> Self {
		Self::during(ViewPhase::Cover, 0.0, 1.0)
	}

	/// The whole time the element is fully in view, or fills it.
	#[must_use]
	pub const fn contain() -> Self {
		Self::during(ViewPhase::Contain, 0.0, 1.0)
	}
}

impl Default for ViewRange {
	fn default() -> Self {
		Self::entry()
	}
}

/// What drives a [`Motion`].
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Trigger {
	/// Time, once, starting when the element appears.
	#[default]
	Appear,
	/// The element's own passage through the viewport.
	InView(ViewRange),
	/// The passage through the viewport of the ancestor that named this
	/// timeline with [`Element::timeline`](crate::Element::timeline).
	Timeline {
		/// The ancestor's timeline name.
		name: String,
		/// The part of the ancestor's passage the motion spans.
		range: ViewRange,
	},
}

/// A choreographed change between poses: what an element looks like at each
/// point, and what moves it from one point to the next.
///
/// ```
/// use std::time::Duration;
///
/// use leptoskit_native::motion::Motion;
/// use leptoskit_native::motion::Pose;
/// use leptoskit_native::motion::Spring;
///
/// // Rises into place with a little bounce, a beat after the page loads.
/// let rise = Motion::enter(Pose::new().y(24.0).opacity(0.0))
/// 	.spring(Spring::BOUNCY)
/// 	.delay(Duration::from_millis(120));
/// assert_eq!(rise.poses().len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
	keyframes: Vec<(f32, Pose)>,
	trigger: Trigger,
	duration: Option<Duration>,
	delay: Duration,
	easing: Easing,
	key: Option<u64>,
}

impl Motion {
	/// How long a motion lasts when neither it nor a spring says otherwise.
	pub const DEFAULT_DURATION: Duration = Duration::from_millis(600);

	/// Moves the element from `from` to its resting style when it appears.
	///
	/// The last pose is the element's own style, whatever its classes set,
	/// so an entrance always ends exactly where the element rests.
	#[must_use]
	pub fn enter(from: Pose) -> Self {
		Self::keyframes([(0.0, from)])
	}

	/// Moves the element through `poses`, each at a point from `0.0` to
	/// `1.0`. Points outside that range are clamped, poses are sorted by
	/// point, and non-finite points are dropped.
	///
	/// Leaving out the pose at `0.0` or `1.0` starts or ends at the
	/// element's own style. A motion that ends with a pose of its own holds
	/// it once finished.
	#[must_use]
	pub fn keyframes(poses: impl IntoIterator<Item = (f32, Pose)>) -> Self {
		let mut keyframes: Vec<(f32, Pose)> = poses
			.into_iter()
			.filter(|(at, _)| at.is_finite())
			.map(|(at, pose)| (at.clamp(0.0, 1.0), pose))
			.collect();
		keyframes.sort_by(|left, right| left.0.total_cmp(&right.0));

		Self {
			keyframes,
			trigger: Trigger::Appear,
			duration: None,
			delay: Duration::ZERO,
			easing: Easing::Out,
			key: None,
		}
	}

	/// Sets how long the motion lasts when time drives it.
	#[must_use]
	pub const fn duration(mut self, duration: Duration) -> Self {
		self.duration = Some(duration);
		self
	}

	/// Waits before a motion that time drives starts. The element holds its
	/// first pose while it waits.
	#[must_use]
	pub const fn delay(mut self, delay: Duration) -> Self {
		self.delay = delay;
		self
	}

	/// Sets how the motion speeds up and slows down.
	#[must_use]
	pub const fn easing(mut self, easing: Easing) -> Self {
		self.easing = easing;
		self
	}

	/// Carries the motion with a spring; the same as
	/// `easing(Easing::Spring(spring))`.
	#[must_use]
	pub const fn spring(self, spring: Spring) -> Self {
		self.easing(Easing::Spring(spring))
	}

	/// Drives the motion with the element's own passage through `range` of
	/// the viewport instead of time: scrolling forward plays it, and
	/// scrolling back reverses it.
	#[must_use]
	pub fn in_view(mut self, range: ViewRange) -> Self {
		self.trigger = Trigger::InView(range);
		self
	}

	/// Drives the motion with the passage of an ancestor that named
	/// `timeline` with [`Element::timeline`](crate::Element::timeline)
	/// through `range` of the viewport. Elements that follow one timeline
	/// with different ranges move as one choreography.
	#[must_use]
	pub fn follow(mut self, timeline: impl Into<String>, range: ViewRange) -> Self {
		self.trigger = Trigger::Timeline {
			name: timeline.into(),
			range,
		};
		self
	}

	/// Plays the motion again whenever `key` changes, such as a count that
	/// pops each time it grows. Only motions that time drives replay.
	#[must_use]
	pub const fn key(mut self, key: u64) -> Self {
		self.key = Some(key);
		self
	}

	/// The poses, sorted by their points from `0.0` to `1.0`.
	#[must_use]
	pub fn poses(&self) -> &[(f32, Pose)] {
		&self.keyframes
	}

	/// What drives the motion.
	#[must_use]
	pub fn trigger(&self) -> &Trigger {
		&self.trigger
	}

	/// How long the motion lasts when time drives it: the set duration,
	/// else the spring's settling time, else
	/// [`DEFAULT_DURATION`](Self::DEFAULT_DURATION).
	#[must_use]
	pub fn resolved_duration(&self) -> Duration {
		match (self.duration, self.easing) {
			(Some(duration), _) => duration,
			(None, Easing::Spring(spring)) => spring.settle_duration(),
			(None, _) => Self::DEFAULT_DURATION,
		}
	}

	/// How long a motion that time drives waits before it starts.
	#[must_use]
	pub fn start_delay(&self) -> Duration {
		self.delay
	}

	/// How the motion speeds up and slows down.
	#[must_use]
	pub fn easing_curve(&self) -> Easing {
		self.easing
	}

	/// The key that replays the motion when it changes.
	#[must_use]
	pub fn replay_key(&self) -> Option<u64> {
		self.key
	}

	/// Whether the motion holds its last pose once finished, because that
	/// pose is its own rather than the element's style.
	#[must_use]
	pub fn holds_last_pose(&self) -> bool {
		self.keyframes.last().is_some_and(|(at, _)| *at >= 1.0)
	}
}

/// The motion settings one element carries: its own [`Motion`], the
/// timeline it names for its descendants, and how it staggers its children.
///
/// Set them with [`Element::motion`](crate::Element::motion),
/// [`Element::timeline`](crate::Element::timeline), and
/// [`Element::stagger`](crate::Element::stagger).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MotionProps {
	/// The element's own motion.
	pub motion: Option<Motion>,
	/// The timeline name descendants follow with [`Motion::follow`].
	pub timeline: Option<String>,
	/// How much later each child's timed motion starts than the one before.
	pub stagger: Option<Duration>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn springs_start_at_zero_and_come_to_rest() {
		for spring in [Spring::BOUNCY, Spring::GENTLE, Spring::SNAPPY] {
			assert!(spring.position(0.0).abs() < 1e-6, "{spring:?}");
			let settle = spring.settle_duration();
			assert!(settle > Duration::ZERO && settle < Duration::from_secs(5));
			assert!((spring.position(settle.as_secs_f32()) - 1.0).abs() <= 0.001);
		}
	}

	#[test]
	fn only_underdamped_springs_overshoot() {
		let peak = |spring: Spring| {
			(0..2000)
				.map(|ms| spring.position(ms as f32 / 1000.0))
				.fold(0.0f32, f32::max)
		};
		assert!(peak(Spring::BOUNCY) > 1.05);
		assert!(peak(Spring::GENTLE) <= 1.0 + 1e-4);
		// Critical and heavy damping both approach rest from below.
		assert!(peak(Spring::new(100.0, 20.0, 1.0)) <= 1.0 + 1e-4);
		assert!(peak(Spring::new(100.0, 60.0, 1.0)) <= 1.0 + 1e-4);
	}

	#[test]
	fn invalid_springs_settle_at_once() {
		for spring in [
			Spring::new(0.0, 10.0, 1.0),
			Spring::new(f32::NAN, 10.0, 1.0),
			Spring::new(100.0, 10.0, -1.0),
		] {
			assert!((spring.position(0.0) - 1.0).abs() < f32::EPSILON);
			assert_eq!(spring.settle_duration(), Duration::ZERO);
		}

		// An undamped spring never settles, so it stops at the limit.
		assert_eq!(
			Spring::new(100.0, -4.0, 1.0).settle_duration(),
			Duration::from_secs(5)
		);
	}

	#[test]
	fn keyframes_are_clamped_sorted_and_finite() {
		let motion = Motion::keyframes([
			(2.0, Pose::new().opacity(1.0)),
			(f32::NAN, Pose::new().opacity(0.5)),
			(-1.0, Pose::new().opacity(0.0)),
			(0.5, Pose::REST),
		]);
		let points: Vec<f32> = motion.poses().iter().map(|(at, _)| *at).collect();
		assert_eq!(points, [0.0, 0.5, 1.0]);
		assert!(motion.holds_last_pose());
		assert!(!Motion::enter(Pose::new().y(10.0)).holds_last_pose());
	}

	#[test]
	fn durations_come_from_the_motion_the_spring_or_the_default() {
		let enter = Motion::enter(Pose::new().opacity(0.0));
		assert_eq!(enter.resolved_duration(), Motion::DEFAULT_DURATION);
		assert_eq!(
			enter.clone().spring(Spring::SNAPPY).resolved_duration(),
			Spring::SNAPPY.settle_duration()
		);
		assert_eq!(
			enter
				.spring(Spring::SNAPPY)
				.duration(Duration::from_millis(90))
				.resolved_duration(),
			Duration::from_millis(90)
		);
	}

	#[test]
	fn triggers_follow_the_last_builder_call() {
		let motion = Motion::enter(Pose::new().x(-20.0)).in_view(ViewRange::cover());
		assert_eq!(motion.trigger(), &Trigger::InView(ViewRange::cover()));
		let motion = motion.follow("stage", ViewRange::during(ViewPhase::Contain, 0.2, 0.4));
		assert_eq!(
			motion.trigger(),
			&Trigger::Timeline {
				name: "stage".into(),
				range: ViewRange::new(ViewPhase::Contain, 0.2, ViewPhase::Contain, 0.4),
			}
		);
	}

	#[test]
	fn insets_show_the_fraction_nearest_their_edge() {
		assert_eq!(Inset::from_left(0.25), Inset::new(0.0, 0.75, 0.0, 0.0));
		assert_eq!(Inset::from_right(0.0), Inset::new(0.0, 0.0, 0.0, 1.0));
		assert_eq!(Inset::from_top(1.0), Inset::NONE);
		assert_eq!(Inset::from_bottom(0.5), Inset::new(0.5, 0.0, 0.0, 0.0));
	}
}
