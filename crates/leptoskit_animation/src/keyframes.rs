//! Keyframe animations: a sequence of looks a widget moves through, once, a
//! few times, or for as long as it is on screen, like Tailwind's
//! `animate-spin` and CSS `@keyframes`.

use std::fmt::Write;

use crate::curve::Easing;
use crate::curve::finite;
use crate::curve::sanitize_ms;
use crate::transform::Transform;
use crate::transform::TransformParts;
use crate::transform::Visual;
use crate::transform::lerp;

/// The most keyframes one animation may have. Interfaces need a handful;
/// the cap keeps a hostile guest from handing platforms huge animations.
pub const MAX_KEYFRAMES: usize = 16;

/// One look in a [`Keyframes`] sequence.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Keyframe {
	/// Where in the sequence this look applies, from `0.0` (start) to `1.0`
	/// (end).
	pub offset: f32,
	/// The look. Unset parts are interpolated from the nearest keyframes
	/// that set them, or the widget's own value at the ends, as in CSS.
	#[cfg_attr(feature = "serde", serde(default, flatten))]
	pub visual: Visual,
	/// How the motion from this keyframe to the next is eased, instead of
	/// the sequence's own easing.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub easing: Option<Easing>,
}

impl Keyframe {
	/// A keyframe at `offset` with no look set yet.
	#[must_use]
	pub const fn at(offset: f32) -> Self {
		Self {
			offset,
			visual: Visual::NONE,
			easing: None,
		}
	}

	/// Sets the opacity.
	#[must_use]
	pub const fn opacity(mut self, opacity: f32) -> Self {
		self.visual.opacity = Some(opacity);
		self
	}

	/// Sets the transform.
	#[must_use]
	pub const fn transform(mut self, transform: Transform) -> Self {
		self.visual.transform = Some(transform);
		self
	}

	/// Eases the motion from this keyframe to the next with `easing`.
	#[must_use]
	pub const fn easing(mut self, easing: Easing) -> Self {
		self.easing = Some(easing);
		self
	}
}

/// How many times a keyframe sequence plays.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Iterations {
	/// A number of times; fractions stop part of the way through.
	Count(f32),
	/// For as long as the widget is shown.
	Infinite,
}

impl Iterations {
	/// The count, with `f32::INFINITY` for [`Self::Infinite`].
	#[must_use]
	pub fn count(self) -> f32 {
		match self {
			Self::Count(count) => finite(count, 1.0).clamp(0.0, 1_000_000.0),
			Self::Infinite => f32::INFINITY,
		}
	}
}

/// A keyframe animation, played by the platform's compositor: Core
/// Animation's `CAKeyframeAnimation` on Apple platforms, `ObjectAnimator`
/// keyframes on Android, and CSS `@keyframes` on the web.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Keyframes {
	/// The looks, in increasing offset order.
	pub frames: Vec<Keyframe>,
	/// How long one pass through the sequence takes, in milliseconds.
	pub duration_ms: f32,
	/// How long to wait before the first pass, in milliseconds.
	#[cfg_attr(feature = "serde", serde(default))]
	pub delay_ms: f32,
	/// How many passes to play.
	pub iterations: Iterations,
	/// Whether every second pass plays backward, as CSS `alternate`.
	#[cfg_attr(feature = "serde", serde(default))]
	pub alternate: bool,
	/// How each step between keyframes is eased, unless the keyframe sets
	/// its own. As in CSS, easing applies to each step, not the whole pass.
	pub easing: Easing,
}

impl Keyframes {
	/// A sequence through `frames`, once, over `duration_ms`, easing each
	/// step in and out.
	#[must_use]
	pub fn new(duration_ms: f32, frames: impl IntoIterator<Item = Keyframe>) -> Self {
		Self {
			frames: frames.into_iter().collect(),
			duration_ms,
			delay_ms: 0.0,
			iterations: Iterations::Count(1.0),
			alternate: false,
			easing: Easing::EASE_IN_OUT,
		}
	}

	/// Tailwind's `animate-spin`: a full clockwise turn every second, for
	/// loading indicators.
	#[must_use]
	pub fn spin() -> Self {
		Self::new(
			1000.0,
			[
				Keyframe::at(0.0).transform(Transform::rotate(0.0)),
				Keyframe::at(1.0).transform(Transform::rotate(360.0)),
			],
		)
		.forever()
		.easing(Easing::Linear)
	}

	/// Tailwind's `animate-ping`: grows to twice its size while fading out,
	/// like a radar ping, for notification badges.
	#[must_use]
	pub fn ping() -> Self {
		let out = Keyframe::at(0.75)
			.transform(Transform::scale(2.0))
			.opacity(0.0);
		Self::new(1000.0, [out, Keyframe { offset: 1.0, ..out }])
			.forever()
			.easing(Easing::CubicBezier([0.0, 0.0, 0.2, 1.0]))
	}

	/// Tailwind's `animate-pulse`: fades to half opacity and back, for
	/// loading placeholders.
	#[must_use]
	pub fn pulse() -> Self {
		Self::new(2000.0, [Keyframe::at(0.5).opacity(0.5)])
			.forever()
			.easing(Easing::CubicBezier([0.4, 0.0, 0.6, 1.0]))
	}

	/// Tailwind's `animate-bounce`: hops up by a quarter of its height and
	/// lands, for "scroll down" hints.
	#[must_use]
	pub fn bounce() -> Self {
		let up = Keyframe::at(0.0)
			.transform(
				Transform::IDENTITY.translated(crate::Offset::ZERO, crate::Offset::percent(-25.0)),
			)
			.easing(Easing::CubicBezier([0.8, 0.0, 1.0, 1.0]));
		Self::new(
			1000.0,
			[
				up,
				Keyframe::at(0.5)
					.transform(Transform::IDENTITY)
					.easing(Easing::CubicBezier([0.0, 0.0, 0.2, 1.0])),
				Keyframe { offset: 1.0, ..up },
			],
		)
		.forever()
	}

	/// Plays for as long as the widget is shown.
	#[must_use]
	pub const fn forever(mut self) -> Self {
		self.iterations = Iterations::Infinite;
		self
	}

	/// Plays `count` times.
	#[must_use]
	pub const fn repeat(mut self, count: f32) -> Self {
		self.iterations = Iterations::Count(count);
		self
	}

	/// Plays every second pass backward.
	#[must_use]
	pub const fn alternate(mut self) -> Self {
		self.alternate = true;
		self
	}

	/// Waits `delay_ms` before the first pass.
	#[must_use]
	pub const fn delay(mut self, delay_ms: f32) -> Self {
		self.delay_ms = delay_ms;
		self
	}

	/// Sets how long one pass takes.
	#[must_use]
	pub const fn duration(mut self, duration_ms: f32) -> Self {
		self.duration_ms = duration_ms;
		self
	}

	/// Eases each step with `easing`, unless a keyframe sets its own.
	#[must_use]
	pub const fn easing(mut self, easing: Easing) -> Self {
		self.easing = easing;
		self
	}

	/// The sequence with at most [`MAX_KEYFRAMES`] keyframes, offsets
	/// within `0.0..=1.0` in increasing order, and every value finite and in
	/// range.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		let mut frames: Vec<Keyframe> = self
			.frames
			.iter()
			.take(MAX_KEYFRAMES)
			.map(|frame| {
				Keyframe {
					offset: finite(frame.offset, 0.0).clamp(0.0, 1.0),
					visual: frame.visual.sanitized(),
					easing: frame.easing.map(Easing::sanitized),
				}
			})
			.collect();
		frames.sort_by(|left, right| left.offset.total_cmp(&right.offset));

		Self {
			frames,
			duration_ms: sanitize_ms(self.duration_ms),
			delay_ms: sanitize_ms(self.delay_ms),
			iterations: match self.iterations {
				Iterations::Infinite => Iterations::Infinite,
				Iterations::Count(_) => Iterations::Count(self.iterations.count()),
			},
			alternate: self.alternate,
			easing: self.easing.sanitized(),
		}
	}

	/// The look `elapsed_ms` after the animation started, for a widget whose
	/// own look is `base`. Before the delay and after the last pass, the
	/// widget shows its own look, as a CSS animation without a fill mode.
	#[must_use]
	pub fn sample(&self, elapsed_ms: f32, base: Visual) -> Visual {
		let elapsed = elapsed_ms - self.delay_ms;

		if elapsed < 0.0 || self.duration_ms <= 0.0 {
			return base;
		}

		let passes = elapsed / self.duration_ms;

		if passes >= self.iterations.count() {
			return base;
		}

		let pass = passes.floor();
		let mut progress = passes - pass;

		#[expect(
			clippy::cast_possible_truncation,
			clippy::cast_sign_loss,
			reason = "only the parity of the pass matters"
		)]
		let odd = (pass as u64) % 2 == 1;

		if self.alternate && odd {
			progress = 1.0 - progress;
		}

		let own = base.transform.unwrap_or_default();
		let parts = self.animated_parts();
		let transform = if parts.is_empty() {
			base.transform
		} else {
			let tracked = self.track(
				progress,
				own,
				|visual| visual.transform,
				|from, to, t| from.lerp(&to, t),
			);
			Some(own.with_parts(&tracked, parts))
		};

		Visual {
			opacity: base
				.opacity
				.or(Some(1.0))
				.map(|own| self.track(progress, own, |visual| visual.opacity, lerp)),
			transform,
		}
	}

	/// The transform parts some keyframe moves away from no transform. Only
	/// these animate; the widget keeps its own value for the others.
	#[must_use]
	pub fn animated_parts(&self) -> TransformParts {
		self.frames
			.iter()
			.filter_map(|frame| frame.visual.transform)
			.fold(TransformParts::NONE, |parts, transform| {
				parts.union(transform.parts())
			})
	}

	/// Whether any keyframe sets the opacity.
	#[must_use]
	pub fn animates_opacity(&self) -> bool {
		self.frames
			.iter()
			.any(|frame| frame.visual.opacity.is_some())
	}

	/// One property's value at `progress` through a pass, interpolated
	/// between the keyframes that set it, with the widget's own value at
	/// any end no keyframe sets.
	fn track<T: Copy>(
		&self,
		progress: f32,
		own: T,
		read: impl Fn(&Visual) -> Option<T>,
		mix: impl Fn(T, T, f32) -> T,
	) -> T {
		let mut previous = (0.0, own, self.easing);

		for frame in &self.frames {
			let Some(value) = read(&frame.visual) else {
				continue;
			};
			let easing = frame.easing.unwrap_or(self.easing);

			if frame.offset >= progress {
				let span = frame.offset - previous.0;

				if span <= 0.0 {
					return value;
				}

				let t = previous.2.sample((progress - previous.0) / span);
				return mix(previous.1, value, t);
			}

			previous = (frame.offset, value, easing);
		}

		let span = 1.0 - previous.0;

		if span <= 0.0 {
			return previous.1;
		}

		mix(
			previous.1,
			own,
			previous.2.sample((progress - previous.0) / span),
		)
	}

	/// The CSS `@keyframes` rule named `name` and the `animation` value that
	/// plays it.
	#[must_use]
	pub fn css(&self, name: &str) -> (String, String) {
		let sequence = self.sanitized();
		let mut rule = format!("@keyframes {name}{{");

		let parts = sequence.animated_parts();

		for frame in &sequence.frames {
			let _ = write!(rule, "{}%{{", frame.offset * 100.0);

			if let Some(opacity) = frame.visual.opacity {
				let _ = write!(rule, "opacity:{opacity};");
			}

			if let Some(transform) = frame.visual.transform {
				rule.push_str(&transform.css_parts(parts));
			}

			if let Some(easing) = frame.easing {
				let _ = write!(rule, "animation-timing-function:{};", easing.css());
			}

			rule.push('}');
		}

		rule.push('}');

		let iterations = match sequence.iterations {
			Iterations::Infinite => "infinite".to_owned(),
			Iterations::Count(count) => count.to_string(),
		};
		let animation = format!(
			"{name} {}ms {} {}ms {iterations}{}",
			sequence.duration_ms,
			sequence.easing.css(),
			sequence.delay_ms,
			if sequence.alternate { " alternate" } else { "" }
		);

		(rule, animation)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn spin_turns_linearly_and_forever() {
		let spin = Keyframes::spin();
		let quarter = spin.sample(250.0, Visual::NONE);
		assert!((quarter.transform.unwrap().rotate - 90.0).abs() < 0.01);
		let later = spin.sample(10_250.0, Visual::NONE);
		assert!((later.transform.unwrap().rotate - 90.0).abs() < 0.01);
	}

	#[test]
	fn unset_ends_use_the_widgets_own_look() {
		let pulse = Keyframes::pulse();
		let own = Visual {
			opacity: Some(0.8),
			transform: None,
		};
		assert!((pulse.sample(0.0, own).opacity.unwrap() - 0.8).abs() < 1e-4);
		assert!((pulse.sample(1000.0, own).opacity.unwrap() - 0.5).abs() < 1e-4);
		assert!((pulse.sample(1999.0, own).opacity.unwrap() - 0.8).abs() < 0.01);
	}

	#[test]
	fn a_finished_animation_shows_the_widgets_own_look() {
		let once = Keyframes::new(100.0, [Keyframe::at(1.0).opacity(0.0)]);
		let own = Visual {
			opacity: Some(1.0),
			transform: None,
		};
		assert_eq!(once.sample(150.0, own), own);
		assert!(once.sample(50.0, own).opacity.unwrap() < 1.0);
	}

	#[test]
	fn keyframes_leave_the_parts_they_do_not_vary_alone() {
		let own = Visual {
			opacity: None,
			transform: Some(
				Transform::scale(2.0).translated(crate::Offset::points(10.0), crate::Offset::ZERO),
			),
		};
		let turned = Keyframes::spin().sample(500.0, own).transform.unwrap();
		assert!((turned.rotate - 180.0).abs() < 0.01);
		assert!((turned.scale_x - 2.0).abs() < f32::EPSILON);
		assert!((turned.translate_x.points - 10.0).abs() < f32::EPSILON);
	}

	#[test]
	fn alternate_passes_play_backward() {
		let sequence = Keyframes::new(100.0, [Keyframe::at(1.0).opacity(0.0)])
			.easing(Easing::Linear)
			.repeat(2.0)
			.alternate();
		let own = Visual {
			opacity: Some(1.0),
			transform: None,
		};
		assert!((sequence.sample(25.0, own).opacity.unwrap() - 0.75).abs() < 1e-4);
		assert!((sequence.sample(125.0, own).opacity.unwrap() - 0.25).abs() < 1e-4);
	}

	#[test]
	fn sanitizing_caps_sorts_and_bounds_frames() {
		let frames = (0..40)
			.rev()
			.map(|index| Keyframe::at(index as f32 / 39.0).opacity(2.0));
		let sequence = Keyframes::new(f32::INFINITY, frames)
			.repeat(f32::NAN)
			.sanitized();
		assert_eq!(sequence.frames.len(), MAX_KEYFRAMES);
		assert!(
			sequence
				.frames
				.windows(2)
				.all(|pair| pair[0].offset <= pair[1].offset)
		);
		assert!(
			sequence
				.frames
				.iter()
				.all(|frame| frame.visual.opacity == Some(1.0))
		);
		assert!((sequence.duration_ms - 0.0).abs() < f32::EPSILON);
		assert_eq!(sequence.iterations, Iterations::Count(1.0));
	}

	#[test]
	fn css_matches_tailwinds_spin() {
		let (rule, animation) = Keyframes::spin().css("lk-spin");
		assert_eq!(
			rule,
			"@keyframes lk-spin{0%{rotate:0deg;}100%{rotate:360deg;}}"
		);
		assert_eq!(animation, "lk-spin 1000ms linear 0ms infinite");
	}

	#[test]
	fn ping_grows_and_fades_then_holds() {
		let ping = Keyframes::ping();
		assert!(ping.animates_opacity());
		let own = Visual {
			opacity: Some(1.0),
			transform: None,
		};
		let held = ping.sample(900.0, own);
		assert_eq!(held.opacity, Some(0.0));
		assert!((held.transform.unwrap().scale_x - 2.0).abs() < 1e-4);

		let (rule, animation) = ping.css("lk-ping");
		assert_eq!(
			rule,
			"@keyframes lk-ping{75%{opacity:0;scale:2 2;}100%{opacity:0;scale:2 2;}}"
		);
		assert_eq!(
			animation,
			"lk-ping 1000ms cubic-bezier(0,0,0.2,1) 0ms infinite"
		);
	}

	#[test]
	fn bounce_eases_each_step_with_its_own_curve() {
		let (rule, _) = Keyframes::bounce().css("lk-bounce");
		assert!(
			rule.starts_with(
				"@keyframes lk-bounce{0%{translate:0px \
				 -25%;animation-timing-function:cubic-bezier(0.8,0,1,1);}"
			),
			"{rule}"
		);
		assert!(!Keyframes::bounce().animates_opacity());
	}

	#[test]
	fn delay_duration_and_count_shape_the_timeline() {
		let sequence = Keyframes::new(100.0, [Keyframe::at(1.0).opacity(0.0)])
			.easing(Easing::Linear)
			.duration(200.0)
			.delay(50.0)
			.repeat(3.0);
		let own = Visual {
			opacity: Some(1.0),
			transform: None,
		};
		assert_eq!(sequence.sample(25.0, own), own);
		assert!((sequence.sample(150.0, own).opacity.unwrap() - 0.5).abs() < 1e-4);
		assert_eq!(sequence.css("lk-fade").1, "lk-fade 200ms linear 50ms 3");
	}

	#[test]
	fn a_keyframe_at_the_start_applies_at_once() {
		let sequence = Keyframes::new(
			100.0,
			[
				Keyframe::at(0.0).opacity(0.3),
				Keyframe::at(1.0).opacity(0.6),
			],
		);
		let own = Visual {
			opacity: Some(1.0),
			transform: None,
		};
		assert!((sequence.sample(0.0, own).opacity.unwrap() - 0.3).abs() < 1e-5);
	}
}
