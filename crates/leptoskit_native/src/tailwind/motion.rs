//! Transform and animation utilities: `translate-*`, `scale-*`, `rotate-*`,
//! `transition-*`, `duration-*`, `ease-*`, `delay-*`, and `animate-*`.
//!
//! Native resolves them to [`Transform`] and [`Animation`](crate::Animation)
//! props that platforms run on their compositor. The web gets the CSS
//! Tailwind 4 generates: the individual `translate`, `scale`, and `rotate`
//! properties, transitions that read `--tw-duration` and `--tw-ease` so the
//! classes combine in any order, and `@keyframes` for `animate-*`.

use std::fmt::Write;

use leptoskit_animation::Curve;
use leptoskit_animation::Easing;
use leptoskit_animation::Keyframes;
use leptoskit_animation::Offset;
use leptoskit_animation::Properties;
use leptoskit_animation::Transform;
use leptoskit_animation::Transition;

use super::StyleContext;
use super::declaration::Axis;
use super::declaration::Declaration;
use super::declaration::write;
use super::value::Length;
use super::value::arbitrary;
use super::value::finite;
use crate::NativeProps;

/// One transform or animation declaration.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Motion {
	/// Movement along one axis. A percentage is of the widget's own size.
	Translate(Axis, Length),
	/// Scale on one axis, or both.
	Scale(Option<Axis>, f32),
	/// Clockwise rotation, in degrees.
	Rotate(f32),
	/// The properties a transition animates.
	Transition(Properties),
	/// The transition's duration, in milliseconds.
	Duration(f32),
	/// The transition's easing.
	Ease(Easing),
	/// The transition's delay, in milliseconds.
	Delay(f32),
	/// A keyframe preset, or none.
	Animate(Option<Preset>),
}

/// Tailwind's built-in `animate-*` keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Preset {
	Spin,
	Ping,
	Pulse,
	Bounce,
}

impl Preset {
	fn keyframes(self) -> Keyframes {
		match self {
			Self::Spin => Keyframes::spin(),
			Self::Ping => Keyframes::ping(),
			Self::Pulse => Keyframes::pulse(),
			Self::Bounce => Keyframes::bounce(),
		}
	}

	fn name(self) -> &'static str {
		match self {
			Self::Spin => "lk-spin",
			Self::Ping => "lk-ping",
			Self::Pulse => "lk-pulse",
			Self::Bounce => "lk-bounce",
		}
	}
}

type Parsed = Result<(u16, Vec<Declaration>), String>;

/// Parses a motion utility, or returns `None` when `utility` is not one.
pub(super) fn parse(utility: &str) -> Option<Parsed> {
	let negative = utility.starts_with('-');
	let body = utility.strip_prefix('-').unwrap_or(utility);

	if let Some(result) = transform(body, negative) {
		return Some(result);
	}

	if negative {
		return None;
	}

	let motion = match body {
		"transition" | "transition-all" => Ok(Motion::Transition(Properties::ALL)),
		"transition-colors" => Ok(Motion::Transition(Properties::COLORS)),
		"transition-opacity" => Ok(Motion::Transition(Properties::OPACITY)),
		"transition-transform" => Ok(Motion::Transition(Properties::TRANSFORM)),
		"transition-none" => Ok(Motion::Transition(Properties::NONE)),
		"transition-shadow" => Err("shadows are not supported yet".to_owned()),
		"ease-linear" => Ok(Motion::Ease(Easing::Linear)),
		"ease-in" => Ok(Motion::Ease(Easing::EASE_IN)),
		"ease-out" => Ok(Motion::Ease(Easing::EASE_OUT)),
		"ease-in-out" => Ok(Motion::Ease(Easing::EASE_IN_OUT)),
		"animate-spin" => Ok(Motion::Animate(Some(Preset::Spin))),
		"animate-ping" => Ok(Motion::Animate(Some(Preset::Ping))),
		"animate-pulse" => Ok(Motion::Animate(Some(Preset::Pulse))),
		"animate-bounce" => Ok(Motion::Animate(Some(Preset::Bounce))),
		"animate-none" => Ok(Motion::Animate(None)),
		_ => {
			if let Some(value) = body.strip_prefix("duration-") {
				milliseconds(value).map(Motion::Duration)
			} else if let Some(value) = body.strip_prefix("delay-") {
				milliseconds(value).map(Motion::Delay)
			} else if let Some(value) = body.strip_prefix("ease-") {
				cubic_bezier(value).map(Motion::Ease)
			} else if body.starts_with("animate-") {
				Err(
					"use animate-spin, animate-ping, animate-pulse, animate-bounce, or \
					 animate-none"
						.to_owned(),
				)
			} else {
				return None;
			}
		}
	};

	Some(motion.map(|motion| (300, vec![Declaration::Motion(motion)])))
}

/// `translate-*`, `scale-*`, and `rotate-*`, any of which may be negated.
fn transform(body: &str, negative: bool) -> Option<Parsed> {
	let sign = if negative { -1.0 } else { 1.0 };
	let one = |motion| Ok((300, vec![Declaration::Motion(motion)]));
	let axes = |rank, motions: Vec<Motion>| {
		Ok((rank, motions.into_iter().map(Declaration::Motion).collect()))
	};

	if body == "translate-none" && !negative {
		let zero = || Length::Px(0.0);
		return Some(axes(
			300,
			vec![
				Motion::Translate(Axis::X, zero()),
				Motion::Translate(Axis::Y, zero()),
			],
		));
	}

	for (prefix, targets, rank) in [
		("translate-x-", &[Axis::X][..], 301),
		("translate-y-", &[Axis::Y][..], 301),
		("translate-", &[Axis::X, Axis::Y][..], 300),
	] {
		if let Some(value) = body.strip_prefix(prefix) {
			let result = Length::spacing(value, true, false, true).and_then(|length| {
				if matches!(length, Length::Auto) {
					return Err("translations cannot be auto".to_owned());
				}

				if negative {
					length.negated()
				} else {
					Ok(length)
				}
			});

			return Some(result.and_then(|length| {
				axes(
					rank,
					targets
						.iter()
						.map(|axis| Motion::Translate(*axis, length.clone()))
						.collect(),
				)
			}));
		}
	}

	if body == "scale-none" && !negative {
		return Some(one(Motion::Scale(None, 1.0)));
	}

	for (prefix, axis, rank) in [
		("scale-x-", Some(Axis::X), 301),
		("scale-y-", Some(Axis::Y), 301),
		("scale-", None, 300),
	] {
		if let Some(value) = body.strip_prefix(prefix) {
			return Some(
				scale(value).and_then(|scale| axes(rank, vec![Motion::Scale(axis, sign * scale)])),
			);
		}
	}

	if body == "rotate-none" && !negative {
		return Some(one(Motion::Rotate(0.0)));
	}

	if let Some(value) = body.strip_prefix("rotate-") {
		return Some(degrees(value).and_then(|degrees| one(Motion::Rotate(sign * degrees))));
	}

	None
}

/// `95` is 95%; `[1.15]` is a factor and `[115%]` a percentage.
fn scale(value: &str) -> Result<f32, String> {
	match arbitrary(value) {
		Some(value) => {
			value.strip_suffix('%').map_or_else(
				|| finite(value),
				|percent| finite(percent).map(|percent| percent / 100.0),
			)
		}
		None => finite(value).map(|percent| percent / 100.0),
	}
	.and_then(|scale| {
		if scale < 0.0 {
			Err("use the negative prefix, as in -scale-100, to flip".to_owned())
		} else {
			Ok(scale)
		}
	})
}

/// `45` is 45 degrees; `[17deg]` and `[0.25turn]` are arbitrary angles.
fn degrees(value: &str) -> Result<f32, String> {
	let Some(value) = arbitrary(value) else {
		return finite(value);
	};

	if let Some(turns) = value.strip_suffix("turn") {
		return finite(turns).map(|turns| turns * 360.0);
	}

	value
		.strip_suffix("deg")
		.ok_or_else(|| "use an angle in deg or turn, as in rotate-[17deg]".to_owned())
		.and_then(finite)
}

/// `300` is 300 ms; `[400ms]` and `[0.4s]` are arbitrary durations.
fn milliseconds(value: &str) -> Result<f32, String> {
	let milliseconds = match arbitrary(value) {
		Some(value) => {
			if let Some(milliseconds) = value.strip_suffix("ms") {
				finite(milliseconds)
			} else if let Some(seconds) = value.strip_suffix('s') {
				finite(seconds).map(|seconds| seconds * 1000.0)
			} else {
				Err("use a time in ms or s, as in duration-[400ms]".to_owned())
			}
		}
		None => finite(value),
	}?;

	if (0.0..=60_000.0).contains(&milliseconds) {
		Ok(milliseconds)
	} else {
		Err("times must be between 0 and 60000 ms".to_owned())
	}
}

/// `[cubic-bezier(0.95,0.05,0.795,0.035)]`.
fn cubic_bezier(value: &str) -> Result<Easing, String> {
	let points = arbitrary(value)
		.and_then(|value| value.strip_prefix("cubic-bezier("))
		.and_then(|value| value.strip_suffix(')'))
		.ok_or_else(|| {
			"use ease-linear, ease-in, ease-out, ease-in-out, or ease-[cubic-bezier(…)]".to_owned()
		})?;
	let numbers = points
		.split(',')
		.map(|number| finite(number.trim()))
		.collect::<Result<Vec<_>, _>>()?;
	let [x1, y1, x2, y2] = numbers[..] else {
		return Err("cubic-bezier() takes four numbers".to_owned());
	};

	if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
		return Err("cubic-bezier() x values must be between 0 and 1".to_owned());
	}

	Ok(Easing::CubicBezier([x1, y1, x2, y2]))
}

impl Motion {
	pub(super) fn apply(&self, props: &mut NativeProps, context: &StyleContext) {
		match self {
			Self::Translate(axis, length) => {
				let offset = match length {
					Length::Percent(fraction) => Offset::percent(fraction * 100.0),
					length => Offset::points(length.points(context)),
				};
				let transform = props.transform.get_or_insert(Transform::IDENTITY);

				match axis {
					Axis::X => transform.translate_x = offset,
					Axis::Y => transform.translate_y = offset,
				}
			}
			Self::Scale(axis, scale) => {
				let transform = props.transform.get_or_insert(Transform::IDENTITY);

				if *axis != Some(Axis::Y) {
					transform.scale_x = *scale;
				}

				if *axis != Some(Axis::X) {
					transform.scale_y = *scale;
				}
			}
			Self::Rotate(degrees) => {
				props.transform.get_or_insert(Transform::IDENTITY).rotate = *degrees;
			}
			Self::Transition(properties) => transition(props).properties = *properties,
			Self::Duration(duration_ms) => {
				let transition = transition(props);
				let easing = match transition.curve {
					Curve::Timing { easing, .. } => easing,
					Curve::Spring(_) => Easing::EASE_IN_OUT,
				};
				transition.curve = Curve::timing(*duration_ms, easing);
			}
			Self::Ease(easing) => {
				let transition = transition(props);
				let duration_ms = match transition.curve {
					Curve::Timing { duration_ms, .. } => duration_ms,
					Curve::Spring(_) => 150.0,
				};
				transition.curve = Curve::timing(duration_ms, *easing);
			}
			Self::Delay(delay_ms) => transition(props).delay_ms = *delay_ms,
			Self::Animate(preset) => {
				props.animation_mut().keyframes = preset.map(Preset::keyframes);
			}
		}
	}

	pub(super) fn css(&self, output: &mut String, important: bool) {
		match self {
			Self::Translate(axis, length) => {
				write(
					output,
					important,
					axis_var("--tw-translate", *axis),
					&length.css(),
				);
				write(
					output,
					important,
					"translate",
					"var(--tw-translate-x,0) var(--tw-translate-y,0)",
				);
			}
			Self::Scale(axis, scale) => {
				let scale = scale.to_string();

				for each in [Axis::X, Axis::Y] {
					if axis.is_none_or(|axis| axis == each) {
						write(output, important, axis_var("--tw-scale", each), &scale);
					}
				}

				write(
					output,
					important,
					"scale",
					"var(--tw-scale-x,1) var(--tw-scale-y,1)",
				);
			}
			Self::Rotate(degrees) => write(output, important, "rotate", &format!("{degrees}deg")),
			Self::Transition(properties) => {
				write(output, important, "transition-property", &properties.css());

				if !properties.is_empty() {
					write(
						output,
						important,
						"transition-timing-function",
						&format!("var(--tw-ease,{})", Easing::EASE_IN_OUT.css()),
					);
					write(
						output,
						important,
						"transition-duration",
						"var(--tw-duration,150ms)",
					);
				}
			}
			Self::Duration(duration_ms) => {
				let duration = format!("{duration_ms}ms");
				write(output, important, "--tw-duration", &duration);
				write(output, important, "transition-duration", &duration);
			}
			Self::Ease(easing) => {
				let easing = easing.css();
				write(output, important, "--tw-ease", &easing);
				write(output, important, "transition-timing-function", &easing);
			}
			Self::Delay(delay_ms) => {
				write(
					output,
					important,
					"transition-delay",
					&format!("{delay_ms}ms"),
				);
			}
			Self::Animate(None) => write(output, important, "animation", "none"),
			Self::Animate(Some(preset)) => {
				let (_, animation) = preset.keyframes().css(preset.name());
				write(output, important, "animation", &animation);
			}
		}
	}

	/// The `@keyframes` rule an `animate-*` utility plays, written after its
	/// selector's block.
	pub(super) fn keyframes_css(&self, output: &mut String) {
		if let Self::Animate(Some(preset)) = self {
			let (rule, _) = preset.keyframes().css(preset.name());
			let _ = write!(output, "{rule}");
		}
	}
}

fn transition(props: &mut NativeProps) -> &mut Transition {
	props
		.animation_mut()
		.transition
		.get_or_insert(Transition::new(Properties::NONE))
}

fn axis_var(prefix: &'static str, axis: Axis) -> &'static str {
	match (prefix, axis) {
		("--tw-translate", Axis::X) => "--tw-translate-x",
		("--tw-translate", Axis::Y) => "--tw-translate-y",
		(_, Axis::X) => "--tw-scale-x",
		(_, Axis::Y) => "--tw-scale-y",
	}
}
