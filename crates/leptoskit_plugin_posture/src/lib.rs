//! Fold state, hinge geometry, and hinge angles for foldable phones.
//!
//! A [`Posture`] describes how the device the app runs on is folded right now:
//! closed on its cover screen, half open like a laptop or a book, or open
//! flat. Each [`Hinge`] reports its own fold, its axis, where it crosses the
//! app's window, and its angle when the platform measures one. Book folds and
//! flips have one hinge; trifolds report one per hinge.
//!
//! Hinge regions are fractions of the app's window (`0.0..=1.0` on both axes),
//! so [`Posture::panes`] lays out content on either side of a fold with
//! percentage sizes and never needs the window's size in points.
//!
//! The wire format is little-endian u32 words: foldable flag, fold, hinge
//! count, then nine words per hinge: fold, axis, occluding flag, region flag,
//! the region's x, y, width, and height as `f32` bits, and the angle in whole
//! degrees. Flags use unknown/false/true = 0/1/2; an unknown angle is
//! `u32::MAX`; an absent region is encoded as zeros.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Registered plugin name.
pub const NAME: &str = "posture";
/// Empty payload -> current [`Posture`].
pub const SNAPSHOT: MethodId = MethodId(0);
/// Previous encoded posture -> next differing posture. One outstanding wait is
/// supported per plugin; a second receives `Busy`. Dropping the guest call
/// removes the native observers.
pub const WAIT_FOR_CHANGE: MethodId = MethodId(1);
/// The most hinges a posture can report. Shipping trifolds have two.
pub const MAX_HINGES: usize = 4;

const HEADER_WORDS: usize = 3;
const HINGE_WORDS: usize = 9;
/// Tolerance for regions that end at the window edge after float rounding.
const EDGE_EPSILON: f32 = 1e-3;

/// How far a device or a single hinge is folded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum Fold {
	/// The platform cannot tell, or the device does not fold.
	#[default]
	Unknown = 0,
	/// Folded shut: the app is on the cover screen, or this hinge is closed.
	Closed = 1,
	/// Partly open, like a laptop on a table or a book held in two hands.
	HalfOpen = 2,
	/// Open flat.
	Open = 3,
}

impl Fold {
	fn decode(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::Closed),
			2 => Ok(Self::HalfOpen),
			3 => Ok(Self::Open),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// The direction a hinge runs across the app's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum Axis {
	/// The platform does not report the hinge's direction.
	#[default]
	Unknown = 0,
	/// Top to bottom, splitting the window into left and right halves.
	Vertical = 1,
	/// Side to side, splitting the window into top and bottom halves.
	Horizontal = 2,
}

impl Axis {
	fn decode(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::Vertical),
			2 => Ok(Self::Horizontal),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// A rectangle in fractions of the app's window: `x` and `width` of its width,
/// `y` and `height` of its height.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Region {
	/// Left edge, from 0.0 at the window's left to 1.0 at its right.
	pub x: f32,
	/// Top edge, from 0.0 at the window's top to 1.0 at its bottom.
	pub y: f32,
	/// Width as a fraction of the window's width.
	pub width: f32,
	/// Height as a fraction of the window's height.
	pub height: f32,
}

impl Region {
	/// The whole window.
	pub const FULL: Self = Self {
		x: 0.0,
		y: 0.0,
		width: 1.0,
		height: 1.0,
	};

	/// Converts a rectangle measured in any unit into fractions of a window
	/// measured in the same unit. Returns `None` for an empty window or a
	/// rectangle that does not fit inside it.
	#[must_use]
	pub fn within(
		x: f32,
		y: f32,
		width: f32,
		height: f32,
		window_width: f32,
		window_height: f32,
	) -> Option<Self> {
		if !(window_width > 0.0 && window_height > 0.0) {
			return None;
		}

		let region = Self {
			x: x / window_width,
			y: y / window_height,
			width: width / window_width,
			height: height / window_height,
		};
		region.is_valid().then_some(region)
	}

	fn is_valid(self) -> bool {
		let fields = [self.x, self.y, self.width, self.height];
		fields
			.iter()
			.all(|value| value.is_finite() && *value >= 0.0)
			&& self.x + self.width <= 1.0 + EDGE_EPSILON
			&& self.y + self.height <= 1.0 + EDGE_EPSILON
	}
}

impl Eq for Region {}

/// One hinge, as it affects the app's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hinge {
	/// How far this hinge is folded.
	pub fold: Fold,
	/// The direction it runs across the window.
	pub axis: Axis,
	/// Where it crosses the window, or `None` when the platform reports no
	/// geometry or the hinge lies outside the window.
	pub region: Option<Region>,
	/// The hinge angle in whole degrees: 0 shut, 180 flat. Platforms that
	/// fold backwards report up to 360.
	pub angle: Option<u16>,
	/// Whether the hinge physically hides content beneath it, as on
	/// dual-screen devices with a gap between panels.
	pub occluding: Option<bool>,
}

/// How the device is folded, with every hinge that crosses the app's window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Posture {
	/// Whether the device folds at all. `None` when the platform cannot say.
	pub foldable: Option<bool>,
	/// The device's overall fold: closed when every hinge is shut, half open
	/// when any hinge is partly open, and open otherwise.
	pub fold: Fold,
	/// Hinges in the order the platform reports them, at most [`MAX_HINGES`].
	pub hinges: Vec<Hinge>,
}

impl Posture {
	/// A device that does not fold.
	#[must_use]
	pub fn rigid() -> Self {
		Self {
			foldable: Some(false),
			fold: Fold::Unknown,
			hinges: Vec::new(),
		}
	}

	/// The overall fold implied by a set of hinges: closed when all are
	/// closed, half open when any is half open, open when at least one is
	/// open and none is half open, and unknown otherwise.
	#[must_use]
	pub fn fold_of(hinges: &[Hinge]) -> Fold {
		if hinges.iter().any(|hinge| hinge.fold == Fold::HalfOpen) {
			Fold::HalfOpen
		} else if hinges.iter().any(|hinge| hinge.fold == Fold::Open) {
			Fold::Open
		} else if !hinges.is_empty() && hinges.iter().all(|hinge| hinge.fold == Fold::Closed) {
			Fold::Closed
		} else {
			Fold::Unknown
		}
	}

	/// Half open with a hinge running side to side: the top half is a screen
	/// and the bottom half rests on a table, like a laptop.
	#[must_use]
	pub fn is_tabletop(&self) -> bool {
		self.fold == Fold::HalfOpen
			&& self
				.hinges
				.iter()
				.any(|hinge| hinge.fold == Fold::HalfOpen && hinge.axis == Axis::Horizontal)
	}

	/// Half open with a hinge running top to bottom, held like a book.
	#[must_use]
	pub fn is_book(&self) -> bool {
		self.fold == Fold::HalfOpen
			&& self
				.hinges
				.iter()
				.any(|hinge| hinge.fold == Fold::HalfOpen && hinge.axis == Axis::Vertical)
	}

	/// The window's regions on either side of every hinge that crosses it,
	/// left to right and then top to bottom. A window no hinge crosses is one
	/// pane covering all of it. Each hinge's own region is left out, so content
	/// laid out in the panes never sits under a fold.
	#[must_use]
	pub fn panes(&self) -> Vec<Region> {
		let mut columns = vec![(0.0, 1.0)];
		let mut rows = vec![(0.0, 1.0)];

		for hinge in &self.hinges {
			let Some(region) = hinge.region else {
				continue;
			};

			if hinge.fold == Fold::Closed {
				continue;
			}

			match hinge.axis {
				Axis::Vertical => cut(&mut columns, region.x, region.x + region.width),
				Axis::Horizontal => cut(&mut rows, region.y, region.y + region.height),
				Axis::Unknown => {}
			}
		}

		rows.iter()
			.flat_map(|&(top, bottom)| {
				columns.iter().map(move |&(left, right)| {
					Region {
						x: left,
						y: top,
						width: right - left,
						height: bottom - top,
					}
				})
			})
			.collect()
	}

	/// Encodes a posture, rejecting more than [`MAX_HINGES`] hinges, regions
	/// outside the window, and angles above 360 degrees.
	pub fn encode(&self) -> Result<Bytes, Error> {
		if self.hinges.len() > MAX_HINGES {
			return Err(Error::InvalidArgument);
		}

		let mut output =
			Encoder::with_capacity(4 * (HEADER_WORDS + HINGE_WORDS * self.hinges.len()));
		output.u32(flag(self.foldable));
		output.u32(self.fold as u32);
		output.u32(self.hinges.len() as u32);

		for hinge in &self.hinges {
			if hinge.region.is_some_and(|region| !region.is_valid())
				|| hinge.angle.is_some_and(|angle| angle > 360)
			{
				return Err(Error::InvalidArgument);
			}
			let region = hinge.region.unwrap_or_default();
			output.u32(hinge.fold as u32);
			output.u32(hinge.axis as u32);
			output.u32(flag(hinge.occluding));
			output.u32(u32::from(hinge.region.is_some()));

			for value in [region.x, region.y, region.width, region.height] {
				output.u32(value.to_bits());
			}

			output.u32(hinge.angle.map_or(u32::MAX, u32::from));
		}

		Ok(output.finish())
	}

	/// Decodes a posture, rejecting truncated or trailing data, unknown enum
	/// values, regions outside the window, and out-of-range angles.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let foldable = read_flag(input.u32()?)?;
		let fold = Fold::decode(input.u32()?)?;
		let count = input.u32()? as usize;

		if count > MAX_HINGES {
			return Err(Error::InvalidArgument);
		}

		let mut hinges = Vec::with_capacity(count);

		for _ in 0..count {
			let fold = Fold::decode(input.u32()?)?;
			let axis = Axis::decode(input.u32()?)?;
			let occluding = read_flag(input.u32()?)?;

			let has_region = match input.u32()? {
				0 => false,
				1 => true,
				_ => return Err(Error::InvalidArgument),
			};

			let region = Region {
				x: f32::from_bits(input.u32()?),
				y: f32::from_bits(input.u32()?),
				width: f32::from_bits(input.u32()?),
				height: f32::from_bits(input.u32()?),
			};

			if !region.is_valid() {
				return Err(Error::InvalidArgument);
			}

			let angle = match input.u32()? {
				u32::MAX => None,
				value @ 0..=360 => Some(value as u16),
				_ => return Err(Error::InvalidArgument),
			};

			hinges.push(Hinge {
				fold,
				axis,
				region: has_region.then_some(region),
				angle,
				occluding,
			});
		}

		input.finish()?;
		Ok(Self {
			foldable,
			fold,
			hinges,
		})
	}
}

/// Splits whichever span contains `[start, end)` around it, dropping slivers
/// left by a hinge that touches the window's edge.
fn cut(spans: &mut Vec<(f32, f32)>, start: f32, end: f32) {
	let Some(index) = spans
		.iter()
		.position(|&(low, high)| start >= low && end <= high)
	else {
		return;
	};
	let (low, high) = spans.remove(index);
	let pieces = [(low, start), (end, high)];

	for (offset, piece) in pieces
		.into_iter()
		.filter(|(a, b)| b - a > EDGE_EPSILON)
		.enumerate()
	{
		spans.insert(index + offset, piece);
	}
}

fn flag(value: Option<bool>) -> u32 {
	match value {
		None => 0,
		Some(false) => 1,
		Some(true) => 2,
	}
}

fn read_flag(value: u32) -> Result<Option<bool>, Error> {
	match value {
		0 => Ok(None),
		1 => Ok(Some(false)),
		2 => Ok(Some(true)),
		_ => Err(Error::InvalidArgument),
	}
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached guest client using the binary bridge.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use std::convert::Infallible;

	use super::*;

	/// The resolved posture plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct DevicePosture(leptoskit_bridge::guest::Client);

	impl DevicePosture {
		/// Resolves the installed host plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads the current posture.
		pub async fn snapshot(self) -> Result<Posture, Error> {
			Posture::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Waits for a posture that differs from `previous`. Intermediate
		/// states can coalesce; this is the latest state, not a transition log.
		pub async fn wait_for_change(self, previous: &Posture) -> Result<Posture, Error> {
			Posture::decode(&self.0.call(WAIT_FOR_CHANGE, &previous.encode()?)?.await?)
		}

		/// Calls `on_change` with the current posture and again after every
		/// change, until the plugin fails. Drop the future to stop watching.
		pub async fn watch(self, mut on_change: impl FnMut(&Posture)) -> Result<Infallible, Error> {
			let mut current = self.snapshot().await?;
			on_change(&current);

			loop {
				current = self.wait_for_change(&current).await?;
				on_change(&current);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hinge(fold: Fold, axis: Axis, region: Option<Region>) -> Hinge {
		Hinge {
			fold,
			axis,
			region,
			angle: None,
			occluding: None,
		}
	}

	fn vertical_band(x: f32, width: f32) -> Region {
		Region {
			x,
			y: 0.0,
			width,
			height: 1.0,
		}
	}

	#[test]
	fn every_posture_round_trips_through_the_wire_format() {
		let trifold = Posture {
			foldable: Some(true),
			fold: Fold::HalfOpen,
			hinges: vec![
				Hinge {
					fold: Fold::Open,
					axis: Axis::Vertical,
					region: Some(vertical_band(0.333, 0.0)),
					angle: Some(180),
					occluding: Some(false),
				},
				Hinge {
					fold: Fold::HalfOpen,
					axis: Axis::Vertical,
					region: Some(vertical_band(0.666, 0.01)),
					angle: Some(97),
					occluding: None,
				},
			],
		};

		for posture in [Posture::default(), Posture::rigid(), trifold] {
			let bytes = posture.encode().unwrap();
			assert_eq!(
				bytes.len(),
				4 * (HEADER_WORDS + HINGE_WORDS * posture.hinges.len())
			);
			assert_eq!(Posture::decode(&bytes), Ok(posture));
		}
	}

	#[test]
	fn decode_rejects_malformed_messages() {
		let posture = Posture {
			foldable: Some(true),
			fold: Fold::Open,
			hinges: vec![hinge(
				Fold::Open,
				Axis::Vertical,
				Some(vertical_band(0.5, 0.0)),
			)],
		};
		let bytes = posture.encode().unwrap().to_vec();
		// foldable, fold, hinge fold, axis, occluding, region flag, angle.
		for word in [0, 1, 3, 4, 5, 6, 11] {
			let mut invalid = bytes.clone();
			invalid[word * 4..word * 4 + 4].copy_from_slice(&361_u32.to_le_bytes());
			assert_eq!(
				Posture::decode(&invalid),
				Err(Error::InvalidArgument),
				"word {word}"
			);
		}

		let mut too_many = bytes.clone();
		too_many[8..12].copy_from_slice(&5_u32.to_le_bytes());

		assert_eq!(Posture::decode(&too_many), Err(Error::InvalidArgument));
		let mut outside = bytes.clone();
		outside[28..32].copy_from_slice(&1.5_f32.to_bits().to_le_bytes());
		assert_eq!(Posture::decode(&outside), Err(Error::InvalidArgument));
		let mut not_finite = bytes.clone();
		not_finite[28..32].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
		assert_eq!(Posture::decode(&not_finite), Err(Error::InvalidArgument));
		assert_eq!(
			Posture::decode(&bytes[..bytes.len() - 1]),
			Err(Error::InvalidArgument)
		);
		let mut trailing = bytes;
		trailing.push(0);
		assert_eq!(Posture::decode(&trailing), Err(Error::InvalidArgument));
	}

	#[test]
	fn encode_rejects_values_the_wire_cannot_carry() {
		let mut posture = Posture {
			foldable: Some(true),
			fold: Fold::Open,
			hinges: vec![Hinge::default(); MAX_HINGES + 1],
		};
		assert_eq!(posture.encode(), Err(Error::InvalidArgument));
		posture.hinges = vec![Hinge {
			angle: Some(361),
			..Hinge::default()
		}];
		assert_eq!(posture.encode(), Err(Error::InvalidArgument));
		posture.hinges = vec![hinge(
			Fold::Open,
			Axis::Vertical,
			Some(vertical_band(0.9, 0.2)),
		)];
		assert_eq!(posture.encode(), Err(Error::InvalidArgument));
	}

	#[test]
	fn overall_fold_follows_the_most_folded_open_hinge() {
		let closed = hinge(Fold::Closed, Axis::Vertical, None);
		let half = hinge(Fold::HalfOpen, Axis::Vertical, None);
		let open = hinge(Fold::Open, Axis::Vertical, None);
		assert_eq!(Posture::fold_of(&[]), Fold::Unknown);
		assert_eq!(Posture::fold_of(&[closed, closed]), Fold::Closed);
		// A trifold with one panel tucked behind the other two is open.
		assert_eq!(Posture::fold_of(&[open, closed]), Fold::Open);
		assert_eq!(Posture::fold_of(&[open, half]), Fold::HalfOpen);
		assert_eq!(Posture::fold_of(&[Hinge::default()]), Fold::Unknown);
	}

	#[test]
	fn tabletop_and_book_need_a_half_open_hinge_on_the_right_axis() {
		let tabletop = Posture {
			foldable: Some(true),
			fold: Fold::HalfOpen,
			hinges: vec![hinge(Fold::HalfOpen, Axis::Horizontal, None)],
		};
		assert!(tabletop.is_tabletop());
		assert!(!tabletop.is_book());
		let book = Posture {
			hinges: vec![hinge(Fold::HalfOpen, Axis::Vertical, None)],
			..tabletop.clone()
		};
		assert!(book.is_book());
		assert!(!book.is_tabletop());
		let flat = Posture {
			fold: Fold::Open,
			hinges: vec![hinge(Fold::Open, Axis::Horizontal, None)],
			..tabletop
		};
		assert!(!flat.is_tabletop());

		assert!(!flat.is_book());
		assert!(!Posture::rigid().is_tabletop());
	}

	#[test]
	fn panes_split_the_window_around_each_crossing_hinge() {
		assert_eq!(Posture::rigid().panes(), vec![Region::FULL]);

		let book = Posture {
			foldable: Some(true),
			fold: Fold::HalfOpen,
			hinges: vec![hinge(
				Fold::HalfOpen,
				Axis::Vertical,
				Some(vertical_band(0.48, 0.04)),
			)],
		};
		let panes = book.panes();
		assert_eq!(panes.len(), 2);
		assert_eq!(panes[0], vertical_band(0.0, 0.48));
		assert!((panes[1].x - 0.52).abs() < 1e-6 && (panes[1].width - 0.48).abs() < 1e-6);

		let flip = Posture {
			hinges: vec![hinge(
				Fold::HalfOpen,
				Axis::Horizontal,
				Some(Region {
					x: 0.0,
					y: 0.5,
					width: 1.0,
					height: 0.0,
				}),
			)],
			..book.clone()
		};
		let panes = flip.panes();
		assert_eq!(panes.len(), 2);
		assert_eq!((panes[0].y, panes[0].height), (0.0, 0.5));
		assert_eq!((panes[1].y, panes[1].height), (0.5, 0.5));

		let trifold = Posture {
			fold: Fold::Open,
			hinges: vec![
				hinge(Fold::Open, Axis::Vertical, Some(vertical_band(0.25, 0.0))),
				hinge(Fold::Open, Axis::Vertical, Some(vertical_band(0.75, 0.0))),
			],
			..book.clone()
		};
		let widths: Vec<f32> = trifold.panes().iter().map(|pane| pane.width).collect();
		assert_eq!(widths, vec![0.25, 0.5, 0.25]);

		// Closed hinges, hinges without geometry, and slivers at the window
		// edge do not split the window.
		let ignored = Posture {
			hinges: vec![
				hinge(Fold::Closed, Axis::Vertical, Some(vertical_band(0.5, 0.0))),
				hinge(Fold::Open, Axis::Vertical, None),
				hinge(Fold::Open, Axis::Unknown, Some(vertical_band(0.3, 0.0))),
				hinge(Fold::Open, Axis::Vertical, Some(vertical_band(0.0, 0.0))),
			],
			..book
		};
		assert_eq!(ignored.panes(), vec![Region::FULL]);
	}

	#[test]
	fn regions_convert_from_window_units() {
		let region = Region::within(1038.0, 0.0, 0.0, 2152.0, 2076.0, 2152.0).unwrap();
		assert_eq!(region, vertical_band(0.5, 0.0));
		assert_eq!(Region::within(0.0, 0.0, 10.0, 10.0, 0.0, 10.0), None);
		assert_eq!(Region::within(0.0, 0.0, 20.0, 10.0, 10.0, 10.0), None);
		assert_eq!(Region::within(f32::NAN, 0.0, 1.0, 1.0, 10.0, 10.0), None);
	}
}
