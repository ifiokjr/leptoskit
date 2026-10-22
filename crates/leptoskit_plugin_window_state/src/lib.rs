//! Persist a window's normal bounds and restore them onto available displays.
//!
//! Coordinates use the host's logical screen coordinate system, never physical
//! pixels. Persist [`WindowState`] with the application's store; restore
//! through [`placement`] after enumerating current work areas. The crate owns
//! no global window, storage directory, or event loop. Native hosts must
//! explicitly pass the window they own. AppKit helpers are in `macos` on
//! macOS.

use serde::Deserialize;
use serde::Serialize;

#[cfg(target_os = "macos")]
pub mod macos;

/// Invalid or unavailable window state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
	/// Bounds contain nonfinite coordinates, nonpositive sizes, or overflow.
	#[error("invalid window geometry")]
	InvalidGeometry,
	/// No display is currently available.
	#[error("no display is available")]
	NoDisplay,
	/// The display identifier is empty or too long.
	#[error("invalid display identifier")]
	InvalidDisplay,
	/// The host must supply the last normal bounds for a zoomed window.
	#[error("normal window bounds are required while maximized")]
	MissingNormalBounds,
	/// Fullscreen restoration needs host-specific asynchronous lifecycle work.
	#[error("fullscreen window restoration is unavailable")]
	Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBounds {
	x: f64,
	y: f64,
	width: f64,
	height: f64,
}

/// Finite, positive bounds, validated even when deserialized from storage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawBounds", into = "RawBounds")]
pub struct Bounds {
	x: f64,
	y: f64,
	width: f64,
	height: f64,
}

impl Bounds {
	/// Constructs bounds in logical screen units.
	pub fn new(x: f64, y: f64, width: f64, height: f64) -> Result<Self, Error> {
		if ![x, y, width, height, x + width, y + height]
			.into_iter()
			.all(f64::is_finite)
			|| width <= 0.0
			|| height <= 0.0
		{
			return Err(Error::InvalidGeometry);
		}
		Ok(Self {
			x,
			y,
			width,
			height,
		})
	}

	/// Horizontal origin.
	pub fn x(self) -> f64 {
		self.x
	}

	/// Vertical origin.
	pub fn y(self) -> f64 {
		self.y
	}

	/// Width.
	pub fn width(self) -> f64 {
		self.width
	}

	/// Height.
	pub fn height(self) -> f64 {
		self.height
	}

	fn overlap(self, other: Self) -> f64 {
		let width = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
		let height = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
		width.max(0.0) * height.max(0.0)
	}
}

impl TryFrom<RawBounds> for Bounds {
	type Error = Error;

	fn try_from(value: RawBounds) -> Result<Self, Self::Error> {
		Self::new(value.x, value.y, value.width, value.height)
	}
}

impl From<Bounds> for RawBounds {
	fn from(value: Bounds) -> Self {
		Self {
			x: value.x,
			y: value.y,
			width: value.width,
			height: value.height,
		}
	}
}

/// A monitor's usable area, excluding the taskbar, menu bar, or dock.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
	id: String,
	work_area: Bounds,
}

impl Display {
	/// `id` should be stable across launches. Put the primary display first.
	pub fn new(id: String, work_area: Bounds) -> Result<Self, Error> {
		if id.is_empty() || id.len() > 256 {
			return Err(Error::InvalidDisplay);
		}
		Ok(Self { id, work_area })
	}

	/// Host-provided stable display identifier.
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Usable logical screen bounds.
	pub fn work_area(&self) -> Bounds {
		self.work_area
	}
}

/// Whether the native window was zoomed when captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowMode {
	/// Restore the saved normal bounds.
	Normal,
	/// Restore normal bounds first, then ask the platform to maximize.
	Maximized,
}

/// Host-persisted state. Normal bounds must be tracked before maximizing.
///
/// The host decides when to save and how to persist this record. This avoids
/// synchronous disk access in move/resize callbacks and lets applications use
/// the same durable store and window identifiers as the rest of their state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowState {
	/// Bounds before maximizing or minimizing.
	pub normal_bounds: Bounds,
	/// Preferred monitor; a disconnected monitor is safely replaced.
	pub display_id: Option<String>,
	/// Restore mode.
	pub mode: WindowMode,
	/// Whether the host should restore the window minimized.
	pub minimized: bool,
}

/// A placement validated against the displays currently connected.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
	/// Bounds wholly contained within the chosen display's work area.
	pub bounds: Bounds,
	/// Chosen display.
	pub display_id: String,
	/// Native mode to apply after restoring normal bounds.
	pub mode: WindowMode,
	/// Native minimized state.
	pub minimized: bool,
}

/// Selects the saved monitor, then the greatest overlap, then the first
/// display.
///
/// Oversized windows shrink to the work area. Both edges are clamped, including
/// negative monitor coordinates. Logical dimensions remain unchanged across
/// scale-factor changes when they still fit, so no physical-pixel scaling is
/// accidentally applied twice.
pub fn placement(state: &WindowState, displays: &[Display]) -> Result<Placement, Error> {
	let first = displays.first().ok_or(Error::NoDisplay)?;
	let selected = state
		.display_id
		.as_deref()
		.and_then(|id| displays.iter().find(|display| display.id == id))
		.unwrap_or_else(|| {
			displays.iter().fold(first, |best, display| {
				if state.normal_bounds.overlap(display.work_area)
					> state.normal_bounds.overlap(best.work_area)
				{
					display
				} else {
					best
				}
			})
		});
	let area = selected.work_area;
	let width = state.normal_bounds.width.min(area.width);
	let height = state.normal_bounds.height.min(area.height);
	let bounds = Bounds::new(
		state
			.normal_bounds
			.x
			.clamp(area.x, area.x + (area.width - width)),
		state
			.normal_bounds
			.y
			.clamp(area.y, area.y + (area.height - height)),
		width,
		height,
	)?;
	Ok(Placement {
		bounds,
		display_id: selected.id.clone(),
		mode: state.mode,
		minimized: state.minimized,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn state(bounds: Bounds, display_id: Option<&str>) -> WindowState {
		WindowState {
			normal_bounds: bounds,
			display_id: display_id.map(str::to_owned),
			mode: WindowMode::Normal,
			minimized: false,
		}
	}

	#[test]
	fn disconnected_display_restores_visible_bounds_on_primary() {
		let saved = state(
			Bounds::new(4000.0, -2000.0, 2000.0, 1000.0).unwrap(),
			Some("removed"),
		);
		let primary = Display::new(
			"main".into(),
			Bounds::new(0.0, 24.0, 1280.0, 696.0).unwrap(),
		)
		.unwrap();
		let restored = placement(&saved, &[primary]).unwrap();
		assert_eq!(
			restored.bounds,
			Bounds::new(0.0, 24.0, 1280.0, 696.0).unwrap()
		);
		assert_eq!(restored.display_id, "main");
	}

	#[test]
	fn selects_overlapping_negative_monitor_and_preserves_logical_size() {
		let displays = [
			Display::new(
				"main".into(),
				Bounds::new(0.0, 0.0, 1920.0, 1080.0).unwrap(),
			)
			.unwrap(),
			Display::new(
				"left".into(),
				Bounds::new(-1920.0, 0.0, 1920.0, 1080.0).unwrap(),
			)
			.unwrap(),
		];
		let saved = state(Bounds::new(-1800.0, 200.0, 800.0, 600.0).unwrap(), None);
		let restored = placement(&saved, &displays).unwrap();
		assert_eq!(restored.bounds, saved.normal_bounds);
		assert_eq!(restored.display_id, "left");
		assert_eq!(placement(&saved, &[]), Err(Error::NoDisplay));
	}

	#[test]
	fn stored_geometry_is_validated_and_roundtrips() {
		let saved = state(
			Bounds::new(-10.0, 20.0, 300.0, 400.0).unwrap(),
			Some("screen"),
		);
		assert_eq!(
			serde_json::from_slice::<WindowState>(&serde_json::to_vec(&saved).unwrap()).unwrap(),
			saved
		);
		assert!(serde_json::from_str::<Bounds>(r#"{"x":0,"y":0,"width":-1,"height":2}"#).is_err());
		assert_eq!(
			Bounds::new(f64::NAN, 0.0, 1.0, 1.0),
			Err(Error::InvalidGeometry)
		);
		assert_eq!(
			Bounds::new(f64::MAX, 0.0, f64::MAX, 1.0),
			Err(Error::InvalidGeometry)
		);
	}
}
