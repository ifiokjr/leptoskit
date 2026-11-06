//! Animations that run on the platform, not in your app's code.
//!
//! A leptoskit app's interface runs as a Wasm guest in an interpreter. If the
//! guest computed every frame of an animation, a busy guest would stutter
//! the animation and the animation would keep the guest busy. Instead, the
//! guest describes an animation once, as part of a widget's properties, and
//! the platform runs it on its compositor: Core Animation on Apple
//! platforms, the Android animation framework, and CSS transitions and
//! animations on the web. This is the model React Native Reanimated uses
//! when it runs animations on the UI thread instead of in JavaScript.
//!
//! This crate is that description, with the presets and the math every
//! platform shares:
//!
//! - [`Curve`]: how a value moves. [`Curve::timing`] runs for a fixed duration
//!   along an [`Easing`], as Reanimated's `withTiming`; [`Curve::spring`] runs
//!   a [`Spring`] until it settles, as `withSpring`.
//! - [`Transition`]: when a property changes, animate to the new value instead
//!   of jumping. Tailwind's `transition`, `duration-*`, and `ease-*` classes
//!   build one.
//! - [`Transform`]: move, scale, and rotate a widget's drawing without laying
//!   anything out again.
//! - [`Keyframes`]: a looping or one-off sequence, as `withRepeat` and
//!   `withSequence`, or Tailwind's `animate-spin`.
//! - [`Presence`]: how a widget enters when it is added and exits when it is
//!   removed, as Reanimated's `entering` and `exiting` animations.
//! - [`Animation`]: all of these for one widget.
//!
//! | Reanimated                        | leptoskit                                     |
//! | --------------------------------- | --------------------------------------------- |
//! | `withTiming(v, { duration })`     | `Transition::timing(duration, easing)`        |
//! | `withSpring(v, config)`           | `Transition::spring(Spring::new(..))`         |
//! | `withRepeat(withSequence(..))`    | `Keyframes::new(..).forever()`                |
//! | `entering={FadeIn.duration(300)}` | `entering(Presence::fade().duration(300.0))`  |
//! | `exiting={SlideOutLeft}`          | `exiting(Presence::slide(Edge::Left))`        |
//! | `layout={LinearTransition}`       | `layout_transition(Curve::DEFAULT)`           |
//!
//! Every value can come from an untrusted guest, so each type has a
//! `sanitized` form that platforms use: finite numbers, bounded durations,
//! and at most [`MAX_KEYFRAMES`] keyframes.

mod curve;
mod keyframes;
mod presence;
mod transform;
mod transition;

pub use self::curve::Curve;
pub use self::curve::Easing;
pub use self::curve::Spring;
pub use self::keyframes::Iterations;
pub use self::keyframes::Keyframe;
pub use self::keyframes::Keyframes;
pub use self::keyframes::MAX_KEYFRAMES;
pub use self::presence::Edge;
pub use self::presence::Presence;
pub use self::transform::Offset;
pub use self::transform::Transform;
pub use self::transform::TransformParts;
pub use self::transform::Visual;
pub use self::transition::Properties;
pub use self::transition::Transition;

/// Everything that animates one widget. Each part is optional; an empty
/// `Animation` changes nothing.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Animation {
	/// Animates changes to properties.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub transition: Option<Transition>,
	/// Plays a keyframe sequence while the widget is shown.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub keyframes: Option<Keyframes>,
	/// How the widget appears when it is added.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub entering: Option<Presence>,
	/// How the widget leaves when it is removed.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub exiting: Option<Presence>,
	/// Animates changes to the widget's position and size after layout,
	/// as Reanimated's `LinearTransition`.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub layout: Option<Curve>,
}

impl Animation {
	/// No animation.
	pub const NONE: Self = Self {
		transition: None,
		keyframes: None,
		entering: None,
		exiting: None,
		layout: None,
	};

	/// Whether nothing animates.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		*self == Self::NONE
	}

	/// The animation with every value finite and in range, and a transition
	/// with no properties dropped.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		Self {
			transition: self
				.transition
				.map(|transition| transition.sanitized())
				.filter(|transition| !transition.properties.is_empty()),
			keyframes: self.keyframes.as_ref().map(Keyframes::sanitized),
			entering: self.entering.map(|presence| presence.sanitized()),
			exiting: self.exiting.map(|presence| presence.sanitized()),
			layout: self.layout.map(Curve::sanitized),
		}
	}
}
