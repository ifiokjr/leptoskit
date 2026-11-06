//! Moving, scaling, and rotating a widget's drawing without changing its
//! layout, and the visual state animations move between.

use std::fmt::Write;

use crate::curve::finite;

/// A distance along one axis: a fixed length plus a percentage of the
/// widget's own size on that axis, as CSS `translate: calc(4px + 50%)`.
///
/// The percentage is resolved by the platform against the widget's current
/// frame, so `translate-x-full` keeps sliding a widget by its own width as
/// that width changes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Offset {
	/// The fixed part, in layout units.
	#[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_zero"))]
	pub points: f32,
	/// The part relative to the widget's own size, where `100.0` is its
	/// whole width or height.
	#[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_zero"))]
	pub percent: f32,
}

#[cfg(feature = "serde")]
#[expect(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde passes skipped fields by reference"
)]
fn is_zero(value: &f32) -> bool {
	*value == 0.0
}

#[cfg(feature = "serde")]
#[expect(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde passes skipped fields by reference"
)]
#[expect(clippy::float_cmp, reason = "only exactly one is the default")]
fn is_one(value: &f32) -> bool {
	*value == 1.0
}

#[cfg(feature = "serde")]
#[expect(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde passes skipped fields by reference"
)]
fn offset_is_zero(offset: &Offset) -> bool {
	offset.is_zero()
}

#[cfg(feature = "serde")]
const fn one() -> f32 {
	1.0
}

impl Offset {
	/// No offset.
	pub const ZERO: Self = Self {
		points: 0.0,
		percent: 0.0,
	};

	/// A fixed length in layout units.
	#[must_use]
	pub const fn points(points: f32) -> Self {
		Self {
			points,
			percent: 0.0,
		}
	}

	/// A percentage of the widget's own size.
	#[must_use]
	pub const fn percent(percent: f32) -> Self {
		Self {
			points: 0.0,
			percent,
		}
	}

	/// The distance for a widget whose size on this axis is `size`.
	#[must_use]
	pub fn resolve(self, size: f32) -> f32 {
		self.points + self.percent / 100.0 * size
	}

	/// Whether this is no offset at all.
	#[must_use]
	pub fn is_zero(self) -> bool {
		self.points == 0.0 && self.percent == 0.0
	}

	fn lerp(self, to: Self, t: f32) -> Self {
		Self {
			points: lerp(self.points, to.points, t),
			percent: lerp(self.percent, to.percent, t),
		}
	}

	fn sanitized(self) -> Self {
		Self {
			points: finite(self.points, 0.0).clamp(-1_000_000.0, 1_000_000.0),
			percent: finite(self.percent, 0.0).clamp(-100_000.0, 100_000.0),
		}
	}
}

/// Moves, scales, and rotates a widget's drawing around its center, without
/// changing its layout or its neighbors'.
///
/// The parts apply in CSS's order for the individual `translate`, `rotate`,
/// and `scale` properties: the widget is scaled, then rotated, then moved.
/// Platforms apply transforms on their compositor, so changing one, or
/// animating it, never lays anything out again.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transform {
	/// Horizontal movement; positive is right.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "offset_is_zero")
	)]
	pub translate_x: Offset,
	/// Vertical movement; positive is down.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "offset_is_zero")
	)]
	pub translate_y: Offset,
	/// Horizontal scale, where `1.0` is the widget's own size.
	#[cfg_attr(
		feature = "serde",
		serde(default = "one", skip_serializing_if = "is_one")
	)]
	pub scale_x: f32,
	/// Vertical scale, where `1.0` is the widget's own size.
	#[cfg_attr(
		feature = "serde",
		serde(default = "one", skip_serializing_if = "is_one")
	)]
	pub scale_y: f32,
	/// Clockwise rotation, in degrees.
	#[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_zero"))]
	pub rotate: f32,
}

impl Default for Transform {
	fn default() -> Self {
		Self::IDENTITY
	}
}

impl Transform {
	/// No transform.
	pub const IDENTITY: Self = Self {
		translate_x: Offset::ZERO,
		translate_y: Offset::ZERO,
		scale_x: 1.0,
		scale_y: 1.0,
		rotate: 0.0,
	};

	/// A movement by fixed lengths.
	#[must_use]
	pub const fn translate(x: f32, y: f32) -> Self {
		Self::IDENTITY.translated(Offset::points(x), Offset::points(y))
	}

	/// A uniform scale.
	#[must_use]
	pub const fn scale(scale: f32) -> Self {
		Self::IDENTITY.scaled(scale, scale)
	}

	/// A clockwise rotation in degrees.
	#[must_use]
	pub const fn rotate(degrees: f32) -> Self {
		Self::IDENTITY.rotated(degrees)
	}

	/// This transform, moved by `x` and `y` instead.
	#[must_use]
	pub const fn translated(mut self, x: Offset, y: Offset) -> Self {
		self.translate_x = x;
		self.translate_y = y;
		self
	}

	/// This transform, scaled by `x` and `y` instead.
	#[must_use]
	pub const fn scaled(mut self, x: f32, y: f32) -> Self {
		self.scale_x = x;
		self.scale_y = y;
		self
	}

	/// This transform, rotated by `degrees` instead.
	#[must_use]
	pub const fn rotated(mut self, degrees: f32) -> Self {
		self.rotate = degrees;
		self
	}

	/// Whether this transform leaves the widget where it is.
	#[must_use]
	pub fn is_identity(&self) -> bool {
		*self == Self::IDENTITY
	}

	/// The transform `t` of the way from `self` to `to`, part by part, as
	/// CSS interpolates the individual transform properties.
	#[must_use]
	pub fn lerp(&self, to: &Self, t: f32) -> Self {
		Self {
			translate_x: self.translate_x.lerp(to.translate_x, t),
			translate_y: self.translate_y.lerp(to.translate_y, t),
			scale_x: lerp(self.scale_x, to.scale_x, t),
			scale_y: lerp(self.scale_y, to.scale_y, t),
			rotate: lerp(self.rotate, to.rotate, t),
		}
	}

	/// The 2D affine matrix `[a, b, c, d, tx, ty]` for a widget of `width`
	/// by `height`, about its center: a point `(x, y)` measured from the
	/// center maps to `(a·x + c·y + tx, b·x + d·y + ty)`.
	#[must_use]
	pub fn matrix(&self, width: f32, height: f32) -> [f32; 6] {
		let (sin, cos) = self.rotate.to_radians().sin_cos();
		[
			cos * self.scale_x,
			sin * self.scale_x,
			-sin * self.scale_y,
			cos * self.scale_y,
			self.translate_x.resolve(width),
			self.translate_y.resolve(height),
		]
	}

	/// The transform with every part finite and bounded.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		Self {
			translate_x: self.translate_x.sanitized(),
			translate_y: self.translate_y.sanitized(),
			scale_x: finite(self.scale_x, 1.0).clamp(-10_000.0, 10_000.0),
			scale_y: finite(self.scale_y, 1.0).clamp(-10_000.0, 10_000.0),
			rotate: finite(self.rotate, 0.0).clamp(-1_000_000.0, 1_000_000.0),
		}
	}

	/// The CSS individual transform properties for this transform, each
	/// ending in `;`.
	#[must_use]
	pub fn css(&self) -> String {
		self.css_parts(TransformParts::ALL)
	}

	/// The CSS individual transform properties for only `parts` of this
	/// transform, each ending in `;`.
	#[must_use]
	pub fn css_parts(&self, parts: TransformParts) -> String {
		let mut css = String::new();

		if parts.translate {
			let _ = write!(
				css,
				"translate:{} {};",
				offset_css(self.translate_x),
				offset_css(self.translate_y)
			);
		}

		if parts.rotate {
			let _ = write!(css, "rotate:{}deg;", self.rotate);
		}

		if parts.scale {
			let _ = write!(css, "scale:{} {};", self.scale_x, self.scale_y);
		}

		css
	}

	/// This transform with `parts` taken from `other`.
	#[must_use]
	pub fn with_parts(mut self, other: &Self, parts: TransformParts) -> Self {
		if parts.translate {
			self.translate_x = other.translate_x;
			self.translate_y = other.translate_y;
		}

		if parts.scale {
			self.scale_x = other.scale_x;
			self.scale_y = other.scale_y;
		}

		if parts.rotate {
			self.rotate = other.rotate;
		}

		self
	}

	/// The parts of this transform that differ from no transform.
	#[must_use]
	#[expect(clippy::float_cmp, reason = "only exactly one scales nothing")]
	pub fn parts(&self) -> TransformParts {
		TransformParts {
			translate: !self.translate_x.is_zero() || !self.translate_y.is_zero(),
			scale: self.scale_x != 1.0 || self.scale_y != 1.0,
			rotate: self.rotate != 0.0,
		}
	}
}

/// Which of a transform's movement, scale, and rotation something touches.
/// A keyframe sequence animates only the parts its keyframes vary, so
/// `animate-spin` turns a widget without undoing its own movement or scale,
/// as CSS's separate `translate`, `scale`, and `rotate` properties behave.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TransformParts {
	/// Movement.
	pub translate: bool,
	/// Scale.
	pub scale: bool,
	/// Rotation.
	pub rotate: bool,
}

impl TransformParts {
	/// Every part.
	pub const ALL: Self = Self {
		translate: true,
		scale: true,
		rotate: true,
	};
	/// No part.
	pub const NONE: Self = Self {
		translate: false,
		scale: false,
		rotate: false,
	};

	/// Whether no part is touched.
	#[must_use]
	pub const fn is_empty(self) -> bool {
		!self.translate && !self.scale && !self.rotate
	}

	/// The parts touched by either.
	#[must_use]
	pub const fn union(self, other: Self) -> Self {
		Self {
			translate: self.translate || other.translate,
			scale: self.scale || other.scale,
			rotate: self.rotate || other.rotate,
		}
	}
}

fn offset_css(offset: Offset) -> String {
	match (offset.points, offset.percent) {
		(points, 0.0) => format!("{points}px"),
		(0.0, percent) => format!("{percent}%"),
		(points, percent) => format!("calc({points}px + {percent}%)"),
	}
}

/// The parts of a widget's look an animation can move between: how opaque
/// it is and how it is transformed. Unset parts keep the widget's own value.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Visual {
	/// Opacity, from `0.0` (transparent) to `1.0` (opaque).
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub opacity: Option<f32>,
	/// The transform.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub transform: Option<Transform>,
}

impl Visual {
	/// Neither opacity nor transform.
	pub const NONE: Self = Self {
		opacity: None,
		transform: None,
	};

	/// Whether no part is set.
	#[must_use]
	pub const fn is_empty(&self) -> bool {
		self.opacity.is_none() && self.transform.is_none()
	}

	/// The visual with every value finite and in range.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		Self {
			opacity: self.opacity.map(|value| finite(value, 1.0).clamp(0.0, 1.0)),
			transform: self.transform.map(|transform| transform.sanitized()),
		}
	}

	/// The CSS declarations for the set parts, each ending in `;`.
	#[must_use]
	pub fn css(&self) -> String {
		let mut css = String::new();

		if let Some(opacity) = self.opacity {
			let _ = write!(css, "opacity:{opacity};");
		}

		if let Some(transform) = self.transform {
			css.push_str(&transform.css());
		}

		css
	}
}

pub(crate) fn lerp(from: f32, to: f32, t: f32) -> f32 {
	from + (to - from) * t
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn percentages_resolve_against_the_widgets_size() {
		let offset = Offset {
			points: 4.0,
			percent: -50.0,
		};
		assert!((offset.resolve(200.0) - -96.0).abs() < f32::EPSILON);
	}

	#[test]
	fn the_matrix_scales_then_rotates_then_moves() {
		let transform = Transform::IDENTITY
			.scaled(2.0, 2.0)
			.rotated(90.0)
			.translated(Offset::points(10.0), Offset::percent(50.0));
		let [a, b, c, d, tx, ty] = transform.matrix(100.0, 40.0);
		// (1, 0) scales to (2, 0) and rotates clockwise to (0, 2).
		assert!(a.abs() < 1e-5 && (b - 2.0).abs() < 1e-5);
		assert!((c + 2.0).abs() < 1e-5 && d.abs() < 1e-5);
		assert!((tx - 10.0).abs() < 1e-5 && (ty - 20.0).abs() < 1e-5);
	}

	#[test]
	fn css_writes_the_individual_transform_properties() {
		let transform = Transform::IDENTITY
			.translated(
				Offset {
					points: 4.0,
					percent: 50.0,
				},
				Offset::percent(-100.0),
			)
			.rotated(45.0)
			.scaled(0.95, 0.95);
		assert_eq!(
			transform.css(),
			"translate:calc(4px + 50%) -100%;rotate:45deg;scale:0.95 0.95;"
		);
	}

	#[test]
	fn sanitizing_clamps_opacity_and_drops_non_finite_parts() {
		let visual = Visual {
			opacity: Some(4.0),
			transform: Some(Transform::scale(f32::NAN)),
		}
		.sanitized();
		assert_eq!(visual.opacity, Some(1.0));
		assert_eq!(visual.transform, Some(Transform::IDENTITY));
	}
}
