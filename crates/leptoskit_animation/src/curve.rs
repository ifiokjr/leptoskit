//! How a value moves from where it is to where it is going: a timing curve
//! with a fixed duration, or a spring that settles when its physics says so.

/// How progress through a timed animation maps to progress of the value.
///
/// The named curves are Tailwind's `ease-*` values, so a class and the Rust
/// API produce the same motion.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Easing {
	/// Constant speed.
	Linear,
	/// A CSS `cubic-bezier(x1, y1, x2, y2)` curve.
	CubicBezier([f32; 4]),
}

impl Easing {
	/// Tailwind's `ease-in`: starts slowly.
	pub const EASE_IN: Self = Self::CubicBezier([0.4, 0.0, 1.0, 1.0]);
	/// Tailwind's `ease-in-out` and the default for `transition`: slow at
	/// both ends.
	pub const EASE_IN_OUT: Self = Self::CubicBezier([0.4, 0.0, 0.2, 1.0]);
	/// Tailwind's `ease-out`: ends slowly.
	pub const EASE_OUT: Self = Self::CubicBezier([0.0, 0.0, 0.2, 1.0]);

	/// The eased progress at `x`, both from `0.0` to `1.0`. A curve whose
	/// control points leave that range can overshoot it.
	#[must_use]
	pub fn sample(self, x: f32) -> f32 {
		let x = x.clamp(0.0, 1.0);

		match self {
			Self::Linear => x,
			Self::CubicBezier([x1, y1, x2, y2]) => {
				let t = solve_bezier_x(x, x1, x2);
				bezier(t, y1, y2)
			}
		}
	}

	/// The curve clamped to values every platform accepts: control point x
	/// coordinates within `0.0..=1.0`, as CSS requires, and finite y.
	#[must_use]
	pub fn sanitized(self) -> Self {
		match self {
			Self::Linear => Self::Linear,
			Self::CubicBezier([x1, y1, x2, y2]) => {
				Self::CubicBezier([
					finite(x1, 0.0).clamp(0.0, 1.0),
					finite(y1, 0.0).clamp(-10.0, 10.0),
					finite(x2, 1.0).clamp(0.0, 1.0),
					finite(y2, 1.0).clamp(-10.0, 10.0),
				])
			}
		}
	}

	/// The CSS `<easing-function>` for this curve.
	#[must_use]
	pub fn css(self) -> String {
		match self {
			Self::Linear => "linear".to_owned(),
			Self::CubicBezier([x1, y1, x2, y2]) => format!("cubic-bezier({x1},{y1},{x2},{y2})"),
		}
	}
}

/// One coordinate of a cubic Bézier from `(0, 0)` to `(1, 1)` with control
/// coordinates `a` and `b`.
fn bezier(t: f32, a: f32, b: f32) -> f32 {
	let inverse = 1.0 - t;
	3.0 * inverse * inverse * t * a + 3.0 * inverse * t * t * b + t * t * t
}

/// The parameter at which the curve's x coordinate reaches `x`. Newton's
/// method converges in a few steps on ordinary curves; bisection covers the
/// flat ones, as browsers do.
fn solve_bezier_x(x: f32, x1: f32, x2: f32) -> f32 {
	let mut t = x;

	for _ in 0..8 {
		let error = bezier(t, x1, x2) - x;

		if error.abs() < 1e-6 {
			return t;
		}

		let inverse = 1.0 - t;
		let slope =
			3.0 * inverse * inverse * x1 + 6.0 * inverse * t * (x2 - x1) + 3.0 * t * t * (1.0 - x2);

		if slope.abs() < 1e-6 {
			break;
		}

		t -= error / slope;
	}

	let (mut low, mut high) = (0.0_f32, 1.0_f32);
	t = x;

	for _ in 0..32 {
		let value = bezier(t, x1, x2);

		if (value - x).abs() < 1e-6 {
			break;
		}

		if value < x {
			low = t;
		} else {
			high = t;
		}

		t = f32::midpoint(low, high);
	}

	t
}

/// A damped spring, the motion React Native Reanimated's `withSpring` and
/// `SwiftUI`'s springs use. It has no fixed duration: it runs until the value
/// settles, and a spring with little damping overshoots and bounces back.
///
/// Platforms run it natively: Core Animation's `CASpringAnimation` on Apple
/// platforms, and the same curve, sampled, on Android and the web.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Spring {
	/// The mass on the spring. Heavier springs move more slowly.
	pub mass: f32,
	/// How hard the spring pulls toward its target.
	pub stiffness: f32,
	/// How quickly motion dies away. Less damping bounces more.
	pub damping: f32,
}

impl Default for Spring {
	fn default() -> Self {
		Self::DEFAULT
	}
}

impl Spring {
	/// React Native Reanimated's `withSpring` default: lively, with a visible
	/// bounce.
	pub const DEFAULT: Self = Self::new(100.0, 10.0);
	/// Slow and soft, with a small bounce.
	pub const GENTLE: Self = Self::new(120.0, 14.0);
	/// Quick and firm, without a bounce: a good default for interface
	/// feedback.
	pub const SNAPPY: Self = Self::new(400.0, 40.0);
	/// Fast, with a bounce that settles quickly.
	pub const STIFF: Self = Self::new(210.0, 20.0);
	/// Bouncy.
	pub const WOBBLY: Self = Self::new(180.0, 12.0);

	/// A spring with a mass of one.
	#[must_use]
	pub const fn new(stiffness: f32, damping: f32) -> Self {
		Self {
			mass: 1.0,
			stiffness,
			damping,
		}
	}

	/// A spring described the way `SwiftUI` describes one: roughly how long it
	/// takes, and how much it bounces, from `0.0` (none) to just under `1.0`.
	#[must_use]
	pub fn with_duration(duration_ms: f32, bounce: f32) -> Self {
		let seconds = finite(duration_ms, 500.0).max(1.0) / 1000.0;
		let bounce = finite(bounce, 0.0).clamp(0.0, 0.99);
		let stiffness = (std::f32::consts::TAU / seconds).powi(2);
		let damping = 4.0 * std::f32::consts::PI * (1.0 - bounce) / seconds;

		Self::new(stiffness, damping)
	}

	/// Sets the mass.
	#[must_use]
	pub const fn mass(mut self, mass: f32) -> Self {
		self.mass = mass;
		self
	}

	/// The spring with every parameter finite and in a range every platform
	/// can run without stalling or diverging.
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self {
			mass: finite(self.mass, 1.0).clamp(0.01, 1000.0),
			stiffness: finite(self.stiffness, 100.0).clamp(0.01, 100_000.0),
			damping: finite(self.damping, 10.0).clamp(0.0, 10_000.0),
		}
	}

	/// The damping ratio: below `1.0` the spring overshoots, at `1.0` and
	/// above it does not.
	#[must_use]
	pub fn damping_ratio(self) -> f32 {
		let spring = self.sanitized();
		spring.damping / (2.0 * (spring.stiffness * spring.mass).sqrt())
	}

	/// Progress toward the target `elapsed_ms` after starting at rest: `0.0`
	/// at the start and `1.0` once settled, overshooting past `1.0` while it
	/// bounces.
	#[must_use]
	pub fn progress(self, elapsed_ms: f32) -> f32 {
		1.0 - self.displacement(finite(elapsed_ms, 0.0).max(0.0) / 1000.0)
	}

	/// How far the spring is from its target `t` seconds after being
	/// released from a displacement of one, at rest.
	fn displacement(self, t: f32) -> f32 {
		let spring = self.sanitized();
		let natural = (spring.stiffness / spring.mass).sqrt();
		let ratio = self.damping_ratio();

		if ratio < 1.0 {
			let damped = natural * (1.0 - ratio * ratio).sqrt();
			let decay = (-ratio * natural * t).exp();
			decay * ((damped * t).cos() + (ratio * natural / damped) * (damped * t).sin())
		} else if ratio - 1.0 < 1.0e-4 {
			// Critically damped, or so close that the overdamped form would
			// divide by almost nothing.
			(-natural * t).exp() * (1.0 + natural * t)
		} else {
			let root = (ratio * ratio - 1.0).sqrt();
			let fast = -natural * (ratio + root);
			let slow = -natural * (ratio - root);
			(slow * (fast * t).exp() - fast * (slow * t).exp()) / (slow - fast)
		}
	}

	/// How long the spring takes to come within a thousandth of its target
	/// and stay there, in milliseconds, capped at a minute. Platforms
	/// without a native spring run the sampled curve for this long.
	#[must_use]
	pub fn settle_ms(self) -> f32 {
		const THRESHOLD: f32 = 0.001;
		const LIMIT_MS: f32 = 60_000.0;
		let spring = self.sanitized();
		let natural = (spring.stiffness / spring.mass).sqrt();
		let ratio = self.damping_ratio();

		if ratio < 1.0 {
			// The oscillation stays inside its decaying envelope.
			let damped = natural * (1.0 - ratio * ratio).sqrt();
			let amplitude = (1.0 + (ratio * natural / damped).powi(2)).sqrt();
			let decay = ratio * natural;

			if decay <= 0.0 {
				return LIMIT_MS;
			}

			return ((amplitude / THRESHOLD).ln() / decay * 1000.0).clamp(0.0, LIMIT_MS);
		}

		// Without overshoot the displacement falls monotonically.
		let (mut low, mut high) = (0.0_f32, LIMIT_MS / 1000.0);

		if self.displacement(high) > THRESHOLD {
			return LIMIT_MS;
		}

		for _ in 0..48 {
			let middle = f32::midpoint(low, high);

			if self.displacement(middle) > THRESHOLD {
				low = middle;
			} else {
				high = middle;
			}
		}

		high * 1000.0
	}
}

/// How a value moves: along a timing curve for a fixed duration, or on a
/// spring.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Curve {
	/// A fixed duration along an easing curve, as `withTiming` and CSS
	/// transitions run.
	Timing {
		/// How long the motion takes, in milliseconds.
		duration_ms: f32,
		/// How progress is distributed over that time.
		easing: Easing,
	},
	/// A spring, which takes as long as its physics needs.
	Spring(Spring),
}

impl Default for Curve {
	fn default() -> Self {
		Self::DEFAULT
	}
}

impl Curve {
	/// Tailwind's default transition: 150 ms, easing in and out.
	pub const DEFAULT: Self = Self::timing(150.0, Easing::EASE_IN_OUT);

	/// A fixed-duration curve.
	#[must_use]
	pub const fn timing(duration_ms: f32, easing: Easing) -> Self {
		Self::Timing {
			duration_ms,
			easing,
		}
	}

	/// A spring.
	#[must_use]
	pub const fn spring(spring: Spring) -> Self {
		Self::Spring(spring)
	}

	/// How long the motion runs, in milliseconds.
	#[must_use]
	pub fn duration_ms(self) -> f32 {
		match self.sanitized() {
			Self::Timing { duration_ms, .. } => duration_ms,
			Self::Spring(spring) => spring.settle_ms(),
		}
	}

	/// Progress at `elapsed_ms`: `0.0` at the start and `1.0` at the end. A
	/// spring overshoots past `1.0` while it bounces.
	#[must_use]
	pub fn progress(self, elapsed_ms: f32) -> f32 {
		match self.sanitized() {
			Self::Timing {
				duration_ms,
				easing,
			} => {
				if duration_ms <= 0.0 {
					return 1.0;
				}

				easing.sample(elapsed_ms / duration_ms)
			}
			Self::Spring(spring) => spring.progress(elapsed_ms),
		}
	}

	/// The curve with every value finite and within the ranges platforms
	/// accept. Durations are capped at a minute.
	#[must_use]
	pub fn sanitized(self) -> Self {
		match self {
			Self::Timing {
				duration_ms,
				easing,
			} => {
				Self::Timing {
					duration_ms: sanitize_ms(duration_ms),
					easing: easing.sanitized(),
				}
			}
			Self::Spring(spring) => Self::Spring(spring.sanitized()),
		}
	}

	/// The CSS duration and `<easing-function>` that reproduce this curve.
	/// A spring becomes a `linear()` function sampled along its motion,
	/// lasting until it settles.
	#[must_use]
	pub fn css(self) -> (f32, String) {
		match self.sanitized() {
			Self::Timing {
				duration_ms,
				easing,
			} => (duration_ms, easing.css()),
			Self::Spring(spring) => {
				let duration = spring.settle_ms().max(1.0);
				// About one point per 16 ms frame, at most 64, which keeps
				// the bounce's peaks without a large stylesheet.
				#[expect(
					clippy::cast_possible_truncation,
					clippy::cast_sign_loss,
					reason = "the point count is a small positive number"
				)]
				let points = ((duration / 16.0).ceil() as usize).clamp(8, 64);
				let mut css = String::from("linear(0");

				for index in 1..=points {
					#[expect(clippy::cast_precision_loss, reason = "the point count is at most 64")]
					let fraction = index as f32 / points as f32;
					let value = if index == points {
						1.0
					} else {
						spring.progress(fraction * duration)
					};
					css.push(',');
					css.push_str(&format_fraction(value));
				}

				css.push(')');
				(duration, css)
			}
		}
	}
}

/// A fraction to four decimal places, without trailing zeros.
fn format_fraction(value: f32) -> String {
	let text = format!("{value:.4}");
	let text = text.trim_end_matches('0').trim_end_matches('.');

	if text.is_empty() || text == "-" || text == "-0" {
		"0".to_owned()
	} else {
		text.to_owned()
	}
}

/// `value`, or `fallback` when it is not finite.
pub(crate) fn finite(value: f32, fallback: f32) -> f32 {
	if value.is_finite() { value } else { fallback }
}

/// A duration or delay in milliseconds, finite and within a minute.
pub(crate) fn sanitize_ms(value: f32) -> f32 {
	finite(value, 0.0).clamp(0.0, 60_000.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn named_easings_hit_their_ends() {
		for easing in [
			Easing::Linear,
			Easing::EASE_IN,
			Easing::EASE_OUT,
			Easing::EASE_IN_OUT,
		] {
			assert!(easing.sample(0.0).abs() < 1e-4);
			assert!((easing.sample(1.0) - 1.0).abs() < 1e-4);
		}

		// Ease-out is ahead of linear halfway through; ease-in behind.
		assert!(Easing::EASE_OUT.sample(0.5) > 0.5);
		assert!(Easing::EASE_IN.sample(0.5) < 0.5);
	}

	#[test]
	fn an_underdamped_spring_overshoots_then_settles() {
		let spring = Spring::DEFAULT;
		assert!(spring.damping_ratio() < 1.0);
		let peak = (0..2000)
			.map(|ms| spring.progress(ms as f32))
			.fold(0.0_f32, f32::max);
		assert!(peak > 1.1, "peak {peak}");

		let settle = spring.settle_ms();
		assert!(settle > 500.0 && settle < 3000.0, "settle {settle}");

		for ms in [settle, settle + 100.0, settle + 1000.0] {
			assert!((spring.progress(ms) - 1.0).abs() <= 0.0011);
		}
	}

	#[test]
	fn critically_and_over_damped_springs_never_overshoot() {
		for spring in [Spring::new(100.0, 20.0), Spring::new(100.0, 60.0)] {
			let mut last = 0.0;

			for ms in 0..3000 {
				let progress = spring.progress(ms as f32);
				assert!(progress <= 1.0 + 1e-5);
				assert!(progress + 1e-5 >= last);
				last = progress;
			}

			assert!(spring.settle_ms() < 60_000.0);
		}
	}

	#[test]
	fn a_spring_from_duration_and_bounce_takes_about_that_long() {
		let spring = Spring::with_duration(500.0, 0.0);
		assert!((spring.damping_ratio() - 1.0).abs() < 1e-3);
		let settle = spring.settle_ms();
		assert!(settle > 300.0 && settle < 1200.0, "settle {settle}");
	}

	#[test]
	fn hostile_values_are_tamed() {
		let spring = Spring {
			mass: f32::NAN,
			stiffness: f32::INFINITY,
			damping: -5.0,
		}
		.sanitized();
		assert!(spring.mass.is_finite() && spring.stiffness.is_finite());
		assert!(spring.damping >= 0.0);
		assert!(Curve::Spring(spring).duration_ms() <= 60_000.0);

		let curve = Curve::timing(f32::NAN, Easing::CubicBezier([5.0, 0.0, -3.0, 1.0])).sanitized();
		assert_eq!(
			curve,
			Curve::timing(0.0, Easing::CubicBezier([1.0, 0.0, 0.0, 1.0]))
		);
		assert!((curve.progress(10.0) - 1.0).abs() < f32::EPSILON);
	}

	#[test]
	fn css_reproduces_timing_and_spring_curves() {
		assert_eq!(
			Curve::DEFAULT.css(),
			(150.0, "cubic-bezier(0.4,0,0.2,1)".to_owned())
		);

		let (duration, css) = Curve::spring(Spring::DEFAULT).css();
		assert!((duration - Spring::DEFAULT.settle_ms()).abs() < 1.0);
		assert!(css.starts_with("linear(0,"));
		assert!(css.ends_with(",1)"));
	}

	#[test]
	fn flat_curves_fall_back_to_bisection() {
		// The slope is zero halfway, where Newton's method gives up.
		let flat = Easing::CubicBezier([1.0, 0.0, 0.0, 1.0]);
		let mut last = 0.0;

		for step in 0..=100 {
			let value = flat.sample(step as f32 / 100.0);
			assert!(value + 1e-4 >= last, "{step}: {value} < {last}");
			last = value;
		}

		for x in [0.3, 0.45, 0.49, 0.51, 0.7] {
			let t = solve_bezier_x(x, 1.0, 0.0);
			assert!((bezier(t, 1.0, 0.0) - x).abs() < 1e-3, "x {x}, t {t}");
		}

		assert_eq!(Easing::Linear.css(), "linear");
	}

	#[test]
	fn defaults_and_builders() {
		assert_eq!(Spring::default(), Spring::DEFAULT);
		assert_eq!(Curve::default(), Curve::DEFAULT);
		let heavy = Spring::DEFAULT.mass(4.0);
		assert!((heavy.mass - 4.0).abs() < f32::EPSILON);
		assert!(heavy.settle_ms() > Spring::DEFAULT.settle_ms());
	}

	#[test]
	fn timing_curves_report_their_duration_and_progress() {
		let curve = Curve::timing(200.0, Easing::Linear);
		assert!((curve.duration_ms() - 200.0).abs() < f32::EPSILON);
		assert!((curve.progress(50.0) - 0.25).abs() < 1e-5);
		assert!((curve.progress(400.0) - 1.0).abs() < f32::EPSILON);

		let spring = Curve::spring(Spring::SNAPPY);
		assert!((spring.progress(10_000.0) - 1.0).abs() < 0.001);
		assert!((spring.duration_ms() - Spring::SNAPPY.settle_ms()).abs() < f32::EPSILON);
	}

	#[test]
	fn a_spring_too_slow_to_settle_is_capped_at_a_minute() {
		let sluggish = Spring::new(0.01, 10_000.0);
		assert!(sluggish.damping_ratio() > 1.0);
		assert!((sluggish.settle_ms() - 60_000.0).abs() < f32::EPSILON);
	}

	#[test]
	fn fractions_lose_trailing_zeros_and_negative_zero() {
		assert_eq!(format_fraction(0.5), "0.5");
		assert_eq!(format_fraction(1.0), "1");
		assert_eq!(format_fraction(0.0), "0");
		assert_eq!(format_fraction(-0.000_01), "0");
		assert_eq!(format_fraction(-0.25), "-0.25");
	}
}
