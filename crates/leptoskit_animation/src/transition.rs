//! Transitions: when a property changes, move to the new value instead of
//! jumping, as CSS `transition` and Reanimated's animated styles do.

use std::ops::BitOr;

use crate::curve::Curve;
use crate::curve::Easing;
use crate::curve::Spring;
use crate::curve::sanitize_ms;

/// The properties a [`Transition`] animates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Properties(u8);

impl Properties {
	/// Every property a transition can animate.
	pub const ALL: Self = Self(Self::OPACITY.0 | Self::TRANSFORM.0 | Self::COLORS.0);
	/// The background and border colors. Text color changes at once.
	pub const COLORS: Self = Self(1 << 2);
	/// No property.
	pub const NONE: Self = Self(0);
	/// Opacity.
	pub const OPACITY: Self = Self(1 << 0);
	/// The transform: movement, scale, and rotation.
	pub const TRANSFORM: Self = Self(1 << 1);

	/// The set with these bits, ignoring unknown ones.
	#[must_use]
	pub const fn from_bits(bits: u8) -> Self {
		Self(bits & Self::ALL.0)
	}

	/// The bits of this set.
	#[must_use]
	pub const fn bits(self) -> u8 {
		self.0
	}

	/// Whether every property in `other` is in this set.
	#[must_use]
	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}

	/// Whether the set is empty.
	#[must_use]
	pub const fn is_empty(self) -> bool {
		self.0 == 0
	}

	/// The CSS `transition-property` value.
	#[must_use]
	pub fn css(self) -> String {
		let mut names = Vec::new();

		if self.contains(Self::COLORS) {
			names.extend(["background-color", "border-color"]);
		}

		if self.contains(Self::OPACITY) {
			names.push("opacity");
		}

		if self.contains(Self::TRANSFORM) {
			names.extend(["translate", "scale", "rotate"]);
		}

		if names.is_empty() {
			"none".to_owned()
		} else {
			names.join(",")
		}
	}
}

impl BitOr for Properties {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}
}

/// Animates changes to some of a widget's properties.
///
/// When a commit changes an animated property, the platform moves it from
/// what is on screen to the new value along [`curve`](Self::curve), on its
/// compositor: the guest commits once and does no work per frame. A change
/// that arrives mid-animation starts from wherever the value is.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transition {
	/// The properties that animate.
	pub properties: Properties,
	/// How they move.
	pub curve: Curve,
	/// How long to wait before moving, in milliseconds.
	#[cfg_attr(feature = "serde", serde(default))]
	pub delay_ms: f32,
}

impl Default for Transition {
	/// Tailwind's `transition`: colors, opacity, and transforms over 150 ms,
	/// easing in and out.
	fn default() -> Self {
		Self::new(Properties::ALL)
	}
}

impl Transition {
	/// Animates `properties` with Tailwind's default curve.
	#[must_use]
	pub const fn new(properties: Properties) -> Self {
		Self {
			properties,
			curve: Curve::DEFAULT,
			delay_ms: 0.0,
		}
	}

	/// Animates every property on `spring`.
	#[must_use]
	pub const fn spring(spring: Spring) -> Self {
		Self::new(Properties::ALL).curve(Curve::spring(spring))
	}

	/// Animates every property over `duration_ms` with `easing`.
	#[must_use]
	pub const fn timing(duration_ms: f32, easing: Easing) -> Self {
		Self::new(Properties::ALL).curve(Curve::timing(duration_ms, easing))
	}

	/// Animates only `properties`.
	#[must_use]
	pub const fn properties(mut self, properties: Properties) -> Self {
		self.properties = properties;
		self
	}

	/// Moves along `curve`.
	#[must_use]
	pub const fn curve(mut self, curve: Curve) -> Self {
		self.curve = curve;
		self
	}

	/// Waits `delay_ms` before moving.
	#[must_use]
	pub const fn delay(mut self, delay_ms: f32) -> Self {
		self.delay_ms = delay_ms;
		self
	}

	/// The transition with every value finite and in range.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		Self {
			properties: Properties::from_bits(self.properties.bits()),
			curve: self.curve.sanitized(),
			delay_ms: sanitize_ms(self.delay_ms),
		}
	}

	/// The CSS `transition-*` declarations, each ending in `;`.
	#[must_use]
	pub fn css(&self) -> String {
		let transition = self.sanitized();
		let (duration, easing) = transition.curve.css();
		format!(
			"transition-property:{};transition-duration:{duration}ms;transition-timing-function:\
			 {easing};transition-delay:{}ms;",
			transition.properties.css(),
			transition.delay_ms
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_default_is_tailwinds_transition() {
		assert_eq!(
			Transition::default().css(),
			"transition-property:background-color,border-color,opacity,translate,scale,rotate;\
			 transition-duration:150ms;transition-timing-function:cubic-bezier(0.4,0,0.2,1);\
			 transition-delay:0ms;"
		);
	}

	#[test]
	fn unknown_property_bits_are_dropped() {
		assert_eq!(Properties::from_bits(0xff), Properties::ALL);
		assert!(Properties::ALL.contains(Properties::OPACITY | Properties::TRANSFORM));
		assert!(!Properties::OPACITY.contains(Properties::TRANSFORM));
	}
}
