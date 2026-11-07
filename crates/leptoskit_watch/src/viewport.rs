//! Physical constraints of a watch display.

use crate::WatchKind;

/// Outline of the watch face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchShape {
	/// Circular face (most Wear OS and Apple Watch models).
	Round,
	/// Sharp rectangle (legacy).
	Square,
	/// Rounded rectangle (modern Apple Watch, many Wear faces).
	RoundedRect,
}

/// Coarse size class for padding and type scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WatchSizeClass {
	/// Roughly 38–41 mm.
	Small,
	/// Roughly 42–45 mm.
	Medium,
	/// Roughly 46 mm and larger.
	Large,
}

/// The constrained viewport a watch host reports to the shared layout pass.
///
/// Phone viewports (390×844 and friends) are the wrong starting point. Hosts
/// should report the real watch bounds, including a safe inset for the crown
/// and rounded edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WatchViewport {
	/// Wear OS or watchOS.
	pub kind: WatchKind,
	/// Face outline.
	pub shape: WatchShape,
	/// Coarse size class.
	pub size_class: WatchSizeClass,
	/// Logical width in density-independent pixels.
	pub width_dp: f32,
	/// Logical height in density-independent pixels.
	pub height_dp: f32,
	/// Inset from each edge that content should stay clear of.
	pub safe_inset_dp: f32,
}

impl WatchViewport {
	/// A representative Wear OS round medium face (~1.2" / 384×384 logical).
	#[must_use]
	pub const fn wear_os_medium() -> Self {
		Self {
			kind: WatchKind::WearOs,
			shape: WatchShape::Round,
			size_class: WatchSizeClass::Medium,
			width_dp: 384.0,
			height_dp: 384.0,
			safe_inset_dp: 24.0,
		}
	}

	/// A representative Apple Watch 45 mm face.
	#[must_use]
	pub const fn watch_os_medium() -> Self {
		Self {
			kind: WatchKind::WatchOs,
			shape: WatchShape::RoundedRect,
			size_class: WatchSizeClass::Medium,
			width_dp: 242.0,
			height_dp: 297.0,
			safe_inset_dp: 16.0,
		}
	}

	/// Content width after the safe inset on both sides.
	#[must_use]
	pub fn content_width(self) -> f32 {
		(self.width_dp - self.safe_inset_dp * 2.0).max(0.0)
	}

	/// Content height after the safe inset on both sides.
	#[must_use]
	pub fn content_height(self) -> f32 {
		(self.height_dp - self.safe_inset_dp * 2.0).max(0.0)
	}
}
