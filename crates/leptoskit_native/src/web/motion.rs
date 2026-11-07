//! Motion as CSS: keyframes, timed and scroll-driven animations, named view
//! timelines, and springs as `linear()` easing.
//!
//! Every animation sits behind `prefers-reduced-motion: no-preference`, and
//! scroll-driven ones also behind `@supports (animation-timeline: view())`,
//! so people who ask for less motion, and browsers without scroll timelines,
//! see every element at rest. Poses animate only opacity, the individual
//! transform properties, `clip-path`, and `filter`, which never move other
//! content.

use std::fmt::Write;
use std::time::Duration;

use sha2::Digest;
use sha2::Sha256;

use crate::motion::Easing;
use crate::motion::Motion;
use crate::motion::MotionProps;
use crate::motion::Pose;
use crate::motion::Spring;
use crate::motion::Trigger;
use crate::motion::ViewPhase;
use crate::motion::ViewRange;

/// Stands for the element's selector in [`MotionCss::rules`] until the
/// element's style scope is known. It cannot appear in generated CSS.
pub(super) const SELECTOR: char = '\u{1}';

/// The CSS one element's motion settings need.
#[derive(Debug, Default, PartialEq)]
pub(super) struct MotionCss {
	/// Rules for the element, with [`SELECTOR`] standing for its selector.
	/// Part of the element's style scope, so equal motions share a sheet.
	pub(super) rules: String,
	/// The keyframes the rules name: their name and their CSS.
	pub(super) keyframes: Option<(String, String)>,
}

/// The rules for `props`, with `offset` added to a timed motion's delay by a
/// staggering parent.
pub(super) fn css(props: &MotionProps, offset: Duration) -> MotionCss {
	let mut out = MotionCss::default();

	if let Some(name) = &props.timeline {
		write!(
			out.rules,
			"@supports (animation-timeline:view()){{{SELECTOR}{{view-timeline:{} block}}}}",
			timeline_name(name)
		)
		.expect("write to String");
	}

	let Some(motion) = &props.motion else {
		return out;
	};
	let Some((name, keyframes)) = keyframes(motion) else {
		return out;
	};

	let easing = easing(motion.easing_curve());
	// An entrance ends on the element's own style, so once it has played the
	// animation lets go. Holding its last keyframe would keep a reveal's
	// `inset(0%)` clipping whatever the element paints outside its box, such
	// as a heading's descenders.
	let fill = if motion.holds_last_pose() {
		"both"
	} else {
		"backwards"
	};
	match motion.trigger() {
		Trigger::Appear => {
			write!(
				out.rules,
				"@media (prefers-reduced-motion:no-preference){{{SELECTOR}{{animation:{name} {}ms \
				 {easing} {}ms 1 normal {fill}}}}}",
				motion.resolved_duration().as_millis(),
				(motion.start_delay() + offset).as_millis(),
			)
			.expect("write to String");
		}
		Trigger::InView(range) => {
			scrolled(&mut out.rules, &name, &easing, fill, "view()", *range);
		}
		Trigger::Timeline {
			name: timeline,
			range,
		} => {
			scrolled(
				&mut out.rules,
				&name,
				&easing,
				fill,
				&timeline_name(timeline),
				*range,
			);
		}
	}

	out.keyframes = Some((name, keyframes));
	out
}

/// A scroll-driven animation on `timeline` over `range`, filling `fill`.
fn scrolled(
	rules: &mut String,
	name: &str,
	easing: &str,
	fill: &str,
	timeline: &str,
	range: ViewRange,
) {
	write!(
		rules,
		"@supports (animation-timeline:view()){{@media \
		 (prefers-reduced-motion:no-preference){{{SELECTOR}{{animation:{name} {easing} \
		 {fill};animation-timeline:{timeline};animation-range:{} {}% {} {}%}}}}}}",
		phase(range.start),
		number(fraction(range.start_at) * 100.0),
		phase(range.end),
		number(fraction(range.end_at) * 100.0),
	)
	.expect("write to String");
}

/// A timeline's CSS name. Characters a dashed identifier cannot hold become
/// `-`, so any name the app gives stays one valid identifier.
fn timeline_name(name: &str) -> String {
	let name: String = name
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
				character
			} else {
				'-'
			}
		})
		.collect();
	format!("--lk-{name}")
}

fn phase(phase: ViewPhase) -> &'static str {
	match phase {
		ViewPhase::Cover => "cover",
		ViewPhase::Contain => "contain",
		ViewPhase::Entry => "entry",
		ViewPhase::Exit => "exit",
	}
}

/// The motion's `@keyframes` and their name, which hashes their content, so
/// equal motions share one rule. A replay key joins the name: a new name
/// restarts a running animation. `None` when no pose sets anything that
/// moves.
fn keyframes(motion: &Motion) -> Option<(String, String)> {
	let animated = Animated::of(motion.poses());

	if !animated.any() {
		return None;
	}

	let mut body = String::new();
	for (at, pose) in motion.poses() {
		let declarations = declarations(pose, animated);

		if !declarations.is_empty() {
			write!(body, "{}%{{{declarations}}}", number(at * 100.0)).expect("write to String");
		}
	}

	// `inset()` cannot interpolate with an element's own `clip-path: none`,
	// so a clip with no pose at either end would jump there halfway. Ending
	// on a clip that shows everything keeps the reveal smooth.
	if animated.clip {
		let clipped = |at_end: fn(f32) -> bool| {
			motion
				.poses()
				.iter()
				.any(|(at, pose)| at_end(*at) && pose.clip.is_some())
		};
		let shown = "clip-path:inset(0% 0% 0% 0%);";

		if !clipped(|at| at <= 0.0) {
			body.insert_str(0, &format!("0%{{{shown}}}"));
		}

		if !clipped(|at| at >= 1.0) {
			write!(body, "100%{{{shown}}}").expect("write to String");
		}
	}

	let digest = Sha256::digest(body.as_bytes());
	let mut name = String::from("lk-m-");
	for byte in &digest[..6] {
		write!(name, "{byte:02x}").expect("write to String");
	}

	if let Some(key) = motion.replay_key() {
		write!(name, "-{key}").expect("write to String");
	}

	let css = format!("@keyframes {name}{{{body}}}");
	Some((name, css))
}

/// Which properties a motion moves: those some pose other than
/// [`Pose::REST`] sets. The resting pose holds only these, so holding at
/// rest never clips or blurs an element that nothing clips or blurs, while
/// a pose that sets a resting value on purpose, such as `rotate(0.0)` on an
/// element that is rotated by its own style, still moves it.
#[expect(
	clippy::struct_excessive_bools,
	reason = "one independent flag per animatable property, never a state machine"
)]
#[derive(Debug, Clone, Copy, Default)]
struct Animated {
	opacity: bool,
	translate: bool,
	scale: bool,
	rotate: bool,
	clip: bool,
	blur: bool,
}

impl Animated {
	fn of(poses: &[(f32, Pose)]) -> Self {
		let mut animated = Self::default();

		for (_, pose) in poses.iter().filter(|(_, pose)| *pose != Pose::REST) {
			animated.opacity |= pose.opacity.is_some();
			animated.translate |= pose.x.is_some() || pose.y.is_some();
			animated.scale |= pose.scale.is_some();
			animated.rotate |= pose.rotate.is_some();
			animated.clip |= pose.clip.is_some();
			animated.blur |= pose.blur.is_some();
		}

		animated
	}

	fn any(self) -> bool {
		self.opacity || self.translate || self.scale || self.rotate || self.clip || self.blur
	}
}

/// One pose's declarations, for the properties the motion moves. Non-finite
/// values are left out, so the browser carries the property through them.
fn declarations(pose: &Pose, animated: Animated) -> String {
	let mut css = String::new();
	let finite = |value: Option<f32>| value.filter(|value| value.is_finite());

	if animated.opacity
		&& let Some(opacity) = finite(pose.opacity)
	{
		write!(css, "opacity:{};", number(opacity.clamp(0.0, 1.0))).expect("write to String");
	}

	if animated.translate && (pose.x.is_some() || pose.y.is_some()) {
		let x = finite(pose.x).unwrap_or(0.0);
		let y = finite(pose.y).unwrap_or(0.0);
		write!(css, "translate:{}px {}px;", number(x), number(y)).expect("write to String");
	}

	if animated.scale
		&& let Some(scale) = finite(pose.scale)
	{
		write!(css, "scale:{};", number(scale)).expect("write to String");
	}

	if animated.rotate
		&& let Some(rotate) = finite(pose.rotate)
	{
		write!(css, "rotate:{}deg;", number(rotate)).expect("write to String");
	}

	if animated.clip
		&& let Some(clip) = pose.clip
	{
		write!(
			css,
			"clip-path:inset({}% {}% {}% {}%);",
			number(fraction(clip.top) * 100.0),
			number(fraction(clip.right) * 100.0),
			number(fraction(clip.bottom) * 100.0),
			number(fraction(clip.left) * 100.0),
		)
		.expect("write to String");
	}

	if animated.blur
		&& let Some(blur) = finite(pose.blur)
	{
		write!(css, "filter:blur({}px);", number(blur.max(0.0))).expect("write to String");
	}

	css
}

/// The CSS easing function for `easing`.
fn easing(easing: Easing) -> String {
	match easing {
		Easing::Linear => "linear".to_owned(),
		Easing::Out => "cubic-bezier(.2,.8,.2,1)".to_owned(),
		Easing::In => "cubic-bezier(.5,0,.9,.3)".to_owned(),
		Easing::InOut => "cubic-bezier(.65,0,.35,1)".to_owned(),
		Easing::Bezier(x1, y1, x2, y2) => {
			let y = |value: f32| if value.is_finite() { value } else { 0.0 };
			format!(
				"cubic-bezier({},{},{},{})",
				number(fraction(x1)),
				number(y(y1)),
				number(fraction(x2)),
				number(y(y2)),
			)
		}
		Easing::Steps(steps) => format!("steps({})", steps.max(1)),
		Easing::Spring(spring) => spring_easing(spring),
	}
}

/// How many points a spring's `linear()` curve samples.
const SPRING_SAMPLES: u32 = 48;

/// A spring as `linear()` points over its settling time, so the browser
/// plays the same curve the spring's equations give, whatever duration the
/// motion runs for.
fn spring_easing(spring: Spring) -> String {
	let settle = spring.settle_duration().as_secs_f32();

	if settle <= 0.0 {
		return "linear".to_owned();
	}

	let mut points = String::from("linear(0");
	for sample in 1..SPRING_SAMPLES {
		let at = settle * sample as f32 / SPRING_SAMPLES as f32;
		write!(points, ",{}", number(spring.position(at))).expect("write to String");
	}

	points.push_str(",1)");
	points
}

/// Clamps a fraction to `0.0..=1.0`, treating non-finite values as zero.
fn fraction(value: f32) -> f32 {
	if value.is_finite() {
		value.clamp(0.0, 1.0)
	} else {
		0.0
	}
}

/// A number for CSS: at most three decimals, without trailing zeros or a
/// negative zero.
fn number(value: f32) -> String {
	let rounded = (value * 1000.0).round() / 1000.0;
	let text = format!("{rounded:.3}");
	let text = text.trim_end_matches('0').trim_end_matches('.');

	if text == "-0" {
		"0".to_owned()
	} else {
		text.to_owned()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::motion::Inset;
	use crate::motion::Spring;

	fn rules(props: &MotionProps) -> String {
		css(props, Duration::ZERO).rules.replace(SELECTOR, "#el")
	}

	fn of(motion: Motion) -> MotionProps {
		MotionProps {
			motion: Some(motion),
			..MotionProps::default()
		}
	}

	#[test]
	fn entrances_play_once_behind_reduced_motion_and_end_at_rest() {
		let props =
			of(Motion::enter(Pose::new().y(24.0).opacity(0.0)).delay(Duration::from_millis(100)));
		let out = css(&props, Duration::from_millis(80));
		let (name, keyframes) = out.keyframes.unwrap();

		assert_eq!(
			keyframes,
			format!("@keyframes {name}{{0%{{opacity:0;translate:0px 24px;}}}}")
		);
		assert_eq!(
			out.rules.replace(SELECTOR, "#el"),
			format!(
				"@media (prefers-reduced-motion:no-preference){{#el{{animation:{name} 600ms \
				 cubic-bezier(.2,.8,.2,1) 180ms 1 normal backwards}}}}"
			)
		);
	}

	#[test]
	fn motions_that_end_on_their_own_pose_hold_it() {
		let rules = rules(&of(Motion::keyframes([
			(0.0, Pose::new().scale(0.5)),
			(1.0, Pose::new().scale(1.5)),
		])));
		assert!(rules.ends_with(" 1 normal both}}"), "{rules}");
	}

	#[test]
	fn scroll_driven_motion_needs_scroll_timelines_and_motion() {
		let rules = rules(&of(Motion::enter(Pose::new().clip(Inset::from_left(0.0)))
			.in_view(ViewRange::new(ViewPhase::Entry, 0.1, ViewPhase::Cover, 0.5))));
		assert!(rules.starts_with(
			"@supports (animation-timeline:view()){@media \
			 (prefers-reduced-motion:no-preference){#el{"
		));
		assert!(
			rules.contains(
				"backwards;animation-timeline:view();animation-range:entry 10% cover 50%}"
			)
		);
	}

	#[test]
	fn scroll_driven_entrances_rest_on_the_elements_own_style() {
		// Past its range, a reveal lets go of its `inset(0%)`, so nothing the
		// element paints outside its box stays clipped.
		let reveal = rules(&of(Motion::enter(
			Pose::new().clip(Inset::from_bottom(0.0)),
		)
		.in_view(ViewRange::entry())));
		assert!(
			reveal.contains(" backwards;animation-timeline:view();"),
			"{reveal}"
		);

		// A motion that ends on a pose of its own keeps holding it.
		let held = rules(&of(Motion::keyframes([
			(0.0, Pose::new().clip(Inset::from_left(0.0))),
			(1.0, Pose::new().clip(Inset::NONE)),
		])
		.follow("stage", ViewRange::contain())));
		assert!(
			held.contains(" both;animation-timeline:--lk-stage;"),
			"{held}"
		);
	}

	#[test]
	fn timelines_are_named_for_followers_with_safe_identifiers() {
		let props = MotionProps {
			motion: Some(Motion::enter(Pose::new().opacity(0.0)).follow(
				"the stage!",
				ViewRange::during(ViewPhase::Contain, 0.0, 0.25),
			)),
			timeline: Some("the stage!".into()),
			stagger: None,
		};
		let rules = rules(&props);
		assert!(rules.starts_with(
			"@supports (animation-timeline:view()){#el{view-timeline:--lk-the-stage- block}}"
		));
		assert!(
			rules.contains(
				"animation-timeline:--lk-the-stage-;animation-range:contain 0% contain 25%"
			)
		);
	}

	#[test]
	fn rest_values_are_written_only_for_moving_properties() {
		let (_, css) = keyframes(&Motion::keyframes([
			(0.0, Pose::new().opacity(0.0)),
			(0.5, Pose::REST),
			(1.0, Pose::new().opacity(0.0)),
		]))
		.unwrap();
		assert!(css.contains("50%{opacity:1;}"), "{css}");
		assert!(!css.contains("clip-path") && !css.contains("filter"));
		assert!(keyframes(&Motion::enter(Pose::REST)).is_none());
		assert!(keyframes(&Motion::enter(Pose::new())).is_none());
	}

	#[test]
	fn a_resting_value_set_on_purpose_still_moves() {
		// The element's own style turns it; the motion swings it there from
		// upright.
		let (_, css) = keyframes(&Motion::enter(Pose::new().rotate(0.0).scale(0.9))).unwrap();
		assert!(css.contains("0%{scale:0.9;rotate:0deg;}"), "{css}");
	}

	#[test]
	fn clips_end_shown_instead_of_jumping_to_none() {
		let (_, reveal) =
			keyframes(&Motion::enter(Pose::new().clip(Inset::from_left(0.0)))).unwrap();
		assert!(
			reveal.ends_with(
				"{0%{clip-path:inset(0% 100% 0% 0%);}100%{clip-path:inset(0% 0% 0% 0%);}}"
			),
			"{reveal}"
		);

		let (_, late) = keyframes(&Motion::keyframes([
			(0.5, Pose::new().clip(Inset::from_top(0.0))),
			(1.0, Pose::new().clip(Inset::from_top(0.5))),
		]))
		.unwrap();
		assert!(
			late.contains("{0%{clip-path:inset(0% 0% 0% 0%);}50%{"),
			"{late}"
		);
		assert!(
			!late.contains("100%{clip-path:inset(0% 0% 0% 0%)"),
			"{late}"
		);
	}

	#[test]
	fn replay_keys_rename_the_keyframes() {
		let pop = Motion::enter(Pose::new().scale(1.4));
		let (plain, _) = keyframes(&pop).unwrap();
		let (first, _) = keyframes(&pop.clone().key(1)).unwrap();
		let (second, _) = keyframes(&pop.key(2)).unwrap();
		assert_eq!(first, format!("{plain}-1"));
		assert_ne!(first, second);
	}

	#[test]
	fn every_pose_property_has_its_css() {
		let pose = Pose::new()
			.opacity(2.0)
			.x(-3.5)
			.scale(0.9)
			.rotate(-6.0)
			.clip(Inset::new(0.0, 0.25, f32::NAN, 2.0))
			.blur(-1.0);
		let animated = Animated::of(&[(0.0, pose), (1.0, Pose::REST)]);
		assert_eq!(
			declarations(&pose, animated),
			"opacity:1;translate:-3.5px 0px;scale:0.9;rotate:-6deg;clip-path:inset(0% 25% 0% \
			 100%);filter:blur(0px);"
		);
		assert_eq!(
			declarations(&Pose::new().y(f32::INFINITY), animated),
			"translate:0px 0px;"
		);
	}

	#[test]
	fn springs_become_linear_curves_that_end_at_rest() {
		let curve = easing(Easing::Spring(Spring::BOUNCY));
		assert!(curve.starts_with("linear(0,") && curve.ends_with(",1)"));
		assert_eq!(curve.matches(',').count() as u32, SPRING_SAMPLES);
		let overshoot = curve
			.trim_start_matches("linear(")
			.trim_end_matches(')')
			.split(',')
			.map(|point| point.parse::<f32>().unwrap())
			.fold(0.0f32, f32::max);
		assert!(overshoot > 1.0, "a bouncy spring overshoots: {curve}");
		assert_eq!(easing(Easing::Spring(Spring::new(0.0, 1.0, 1.0))), "linear");
	}

	#[test]
	fn easings_name_their_css_functions() {
		assert_eq!(easing(Easing::Linear), "linear");
		assert_eq!(easing(Easing::Steps(0)), "steps(1)");
		assert_eq!(
			easing(Easing::Bezier(-1.0, 1.5, 2.0, f32::NAN)),
			"cubic-bezier(0,1.5,1,0)"
		);
	}

	#[test]
	fn numbers_are_short_and_never_negative_zero() {
		assert_eq!(number(0.0), "0");
		assert_eq!(number(-0.0001), "0");
		assert_eq!(number(12.5), "12.5");
		assert_eq!(number(1.0 / 3.0), "0.333");
	}
}
