//! Cached vector paths and bounded software rasterization, shared by every OS.

use std::collections::HashMap;
use std::rc::Rc;

use tiny_skia::BlendMode;
use tiny_skia::Color;
use tiny_skia::FillRule;
use tiny_skia::GradientStop;
use tiny_skia::LineCap;
use tiny_skia::LineJoin;
use tiny_skia::LinearGradient;
use tiny_skia::Mask;
use tiny_skia::Paint;
use tiny_skia::Path;
use tiny_skia::PathBuilder;
use tiny_skia::Pixmap;
use tiny_skia::Point;
use tiny_skia::RadialGradient;
use tiny_skia::Shader;
use tiny_skia::SpreadMode;
use tiny_skia::Stroke;
use tiny_skia::Transform;

use super::ffi::PaintView;
use super::ffi::PathView;
use crate::Error;

#[derive(Clone, Default)]
struct State {
	transform: Transform,
	opacity: f32,
	mask: Option<Rc<Mask>>,
	clip_depth: u8,
}

struct CachedPath {
	revision: u64,
	path: Option<Path>,
}

pub(super) struct Raster {
	pixels: Pixmap,
	state: State,
	stack: Vec<State>,
	paths: HashMap<u64, CachedPath>,
	masks: Vec<Rc<Mask>>,
	error: Option<Error>,
}

fn invalid() -> Error {
	Error::Runtime("invalid native raster data".into())
}

fn transform(values: [f32; 6]) -> Result<Transform, Error> {
	if values.iter().any(|v| !v.is_finite()) {
		return Err(invalid());
	}
	let [sx, ky, kx, sy, tx, ty] = values;
	Ok(Transform::from_row(sx, ky, kx, sy, tx, ty))
}

fn rule(value: u32) -> Result<FillRule, Error> {
	match value {
		0 => Ok(FillRule::Winding),
		1 => Ok(FillRule::EvenOdd),
		_ => Err(Error::UnsupportedFeature),
	}
}

fn color(value: u32) -> Color {
	let [a, r, g, b] = value.to_be_bytes();
	Color::from_rgba8(r, g, b, a)
}

fn blend(value: u32) -> Result<BlendMode, Error> {
	Ok(match value {
		3 => BlendMode::SourceOver,
		12 => BlendMode::Plus,
		14 => BlendMode::Screen,
		15 => BlendMode::Overlay,
		16 => BlendMode::Darken,
		17 => BlendMode::Lighten,
		18 => BlendMode::ColorDodge,
		19 => BlendMode::ColorBurn,
		20 => BlendMode::HardLight,
		21 => BlendMode::SoftLight,
		22 => BlendMode::Difference,
		23 => BlendMode::Exclusion,
		24 => BlendMode::Multiply,
		25 => BlendMode::Hue,
		26 => BlendMode::Saturation,
		27 => BlendMode::Color,
		28 => BlendMode::Luminosity,
		_ => return Err(Error::UnsupportedFeature),
	})
}

fn cached<'a>(
	cache: &'a mut HashMap<u64, CachedPath>,
	view: &PathView,
	verbs: &[u8],
	points: &[f32],
) -> Result<Option<&'a Path>, Error> {
	let stale = cache
		.get(&view.id)
		.is_none_or(|entry| entry.revision != view.revision);

	if stale {
		if points.iter().any(|p| !p.is_finite()) {
			return Err(invalid());
		}
		let mut builder = PathBuilder::new();
		let mut points = points;

		for verb in verbs {
			let count = match verb {
				0 | 1 => 2,
				2 => 4,
				4 => 6,
				5 => 0,
				_ => return Err(invalid()),
			};
			let Some((p, remaining)) = points.split_at_checked(count) else {
				return Err(invalid());
			};
			points = remaining;

			match verb {
				0 => builder.move_to(p[0], p[1]),
				1 => builder.line_to(p[0], p[1]),
				2 => builder.quad_to(p[0], p[1], p[2], p[3]),
				4 => builder.cubic_to(p[0], p[1], p[2], p[3], p[4], p[5]),
				5 => builder.close(),
				_ => return Err(invalid()),
			}
		}

		if !points.is_empty() {
			return Err(invalid());
		}
		cache.insert(
			view.id,
			CachedPath {
				revision: view.revision,
				path: builder.finish(),
			},
		);
	}

	Ok(cache.get(&view.id).and_then(|entry| entry.path.as_ref()))
}

impl Raster {
	pub fn new() -> Result<Self, Error> {
		Ok(Self {
			pixels: Pixmap::new(1, 1).ok_or(Error::InvalidFrame)?,
			state: State::default(),
			stack: Vec::with_capacity(64),
			paths: HashMap::new(),
			masks: Vec::with_capacity(8),
			error: None,
		})
	}

	pub fn begin(&mut self, width: u32, height: u32) -> Result<(), Error> {
		self.state = State {
			opacity: 1.0,
			..State::default()
		};
		self.stack.clear();
		if self.pixels.width() != width || self.pixels.height() != height {
			self.masks.clear();
			self.pixels = Pixmap::new(width, height).ok_or(Error::InvalidFrame)?;
		}
		self.pixels.fill(Color::TRANSPARENT);
		self.error = None;
		Ok(())
	}

	pub fn finish(&mut self) -> Result<(), Error> {
		if let Some(error) = self.error.take() {
			return Err(error);
		}
		if !self.stack.is_empty() {
			return Err(invalid());
		}
		Ok(())
	}

	pub fn pixels(&self) -> &Pixmap {
		&self.pixels
	}

	pub fn failed(&self) -> bool {
		self.error.is_some()
	}

	pub fn fail(&mut self, error: Error) {
		if self.error.is_none() {
			self.error = Some(error);
		}
	}

	pub fn release(&mut self, id: u64) {
		self.paths.remove(&id);
	}

	pub fn save(&mut self) -> Result<(), Error> {
		if self.stack.len() >= 64 {
			return Err(Error::UnsupportedFeature);
		}
		self.stack.push(self.state.clone());
		Ok(())
	}

	pub fn restore(&mut self) -> Result<(), Error> {
		self.state = self.stack.pop().ok_or_else(invalid)?;
		Ok(())
	}

	pub fn transform(&mut self, matrix: [f32; 6]) -> Result<(), Error> {
		self.state.transform = self.state.transform.pre_concat(transform(matrix)?);
		if !self.state.transform.is_finite() {
			return Err(invalid());
		}
		Ok(())
	}

	pub fn opacity(&mut self, opacity: f32) -> Result<(), Error> {
		if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
			return Err(invalid());
		}
		self.state.opacity *= opacity;
		Ok(())
	}

	pub fn clip(&mut self, view: &PathView, verbs: &[u8], points: &[f32]) -> Result<(), Error> {
		if self.state.clip_depth >= 8 {
			return Err(Error::UnsupportedFeature);
		}
		let path = cached(&mut self.paths, view, verbs, points)?;
		// Saved states retain only live masks. Reuse a free buffer between
		// siblings and frames rather than allocating an image-sized mask each
		// clip.
		let index = match self
			.masks
			.iter()
			.position(|mask| Rc::strong_count(mask) == 1)
		{
			Some(index) => index,
			None if self.masks.len() < 8 => {
				self.masks.push(Rc::new(
					Mask::new(self.pixels.width(), self.pixels.height())
						.ok_or(Error::InvalidFrame)?,
				));
				self.masks.len() - 1
			}
			None => return Err(Error::UnsupportedFeature),
		};
		let mask = Rc::get_mut(&mut self.masks[index]).ok_or_else(invalid)?;
		if let Some(previous) = &self.state.mask {
			mask.data_mut().copy_from_slice(previous.data());
		} else {
			mask.clear();
		}

		if let Some(path) = path {
			if self.state.mask.is_some() {
				mask.intersect_path(path, rule(view.fill_rule)?, true, self.state.transform);
			} else {
				mask.fill_path(path, rule(view.fill_rule)?, true, self.state.transform);
			}
		} else {
			mask.clear();
		}
		self.state.mask = Some(Rc::clone(&self.masks[index]));
		self.state.clip_depth += 1;
		Ok(())
	}

	pub fn draw(
		&mut self,
		view: &PathView,
		verbs: &[u8],
		points: &[f32],
		values: &PaintView,
		colors: &[u32],
		stops: &[f32],
	) -> Result<(), Error> {
		let Some(path) = cached(&mut self.paths, view, verbs, points)? else {
			return Ok(());
		};
		let mut shader = match values.gradient {
			0 => Shader::SolidColor(color(values.color)),
			1 | 2 => {
				if stops.is_empty()
					|| stops
						.iter()
						.any(|s| !s.is_finite() || !(0.0..=1.0).contains(s))
					|| stops.windows(2).any(|s| s[0] > s[1])
				{
					return Err(invalid());
				}
				let stops = colors
					.iter()
					.zip(stops)
					.map(|(&c, &s)| GradientStop::new(s, color(c)))
					.collect();
				let [a, b, c, d, radius] = values.coordinates;
				let matrix = transform(values.shader_matrix)?;
				if values.coordinates.iter().any(|v| !v.is_finite()) {
					return Err(invalid());
				}
				if values.gradient == 1 {
					LinearGradient::new(
						Point::from_xy(a, b),
						Point::from_xy(c, d),
						stops,
						SpreadMode::Pad,
						matrix,
					)
				} else {
					RadialGradient::new(
						Point::from_xy(a, b),
						Point::from_xy(a, b),
						radius,
						stops,
						SpreadMode::Pad,
						matrix,
					)
				}
				.ok_or_else(invalid)?
			}
			_ => return Err(Error::UnsupportedFeature),
		};
		shader.apply_opacity(self.state.opacity);
		let paint = Paint {
			shader,
			blend_mode: blend(values.blend)?,
			anti_alias: true,
			..Paint::default()
		};
		let mask = self.state.mask.as_deref();

		if values.style == 1 {
			self.pixels.fill_path(
				path,
				&paint,
				rule(view.fill_rule)?,
				self.state.transform,
				mask,
			);
		} else if values.style == 0 {
			if !values.thickness.is_finite() || values.thickness < 0.0 {
				return Err(invalid());
			}
			let stroke = Stroke {
				width: values.thickness,
				line_cap: match values.cap {
					0 => LineCap::Butt,
					1 => LineCap::Round,
					2 => LineCap::Square,
					_ => return Err(invalid()),
				},
				line_join: match values.join {
					0 => LineJoin::Miter,
					1 => LineJoin::Round,
					2 => LineJoin::Bevel,
					_ => return Err(invalid()),
				},
				..Stroke::default()
			};
			self.pixels
				.stroke_path(path, &paint, &stroke, self.state.transform, mask);
		} else {
			return Err(invalid());
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn rectangle() -> PathView {
		PathView {
			id: 1,
			revision: 0,
			verbs: std::ptr::null(),
			verb_count: 5,
			points: std::ptr::null(),
			point_count: 8,
			fill_rule: 0,
		}
	}

	const VERBS: &[u8] = &[0, 1, 1, 1, 5];
	const POINTS: &[f32] = &[2.0, 2.0, 14.0, 2.0, 14.0, 14.0, 2.0, 14.0];

	#[test]
	fn sibling_clips_and_later_frames_reuse_bounded_mask_storage() {
		let mut raster = Raster::new().unwrap();
		let mut allocation = None;
		for _ in 0..10 {
			raster.begin(16, 16).unwrap();
			for _ in 0..32 {
				raster.save().unwrap();
				raster.clip(&rectangle(), VERBS, POINTS).unwrap();
				let mask = raster.state.mask.as_ref().unwrap();
				assert_eq!(mask.data()[8 * 16 + 8], 255);
				assert_eq!(mask.data()[0], 0);
				if let Some(previous) = allocation {
					assert_eq!(mask.data().as_ptr(), previous);
				}
				allocation = Some(mask.data().as_ptr());
				raster.restore().unwrap();
			}
			raster.finish().unwrap();
			assert_eq!(raster.masks.len(), 1);
		}
	}

	#[test]
	fn clipping_depth_and_save_depth_fail_before_unbounded_allocation() {
		let mut raster = Raster::new().unwrap();
		raster.begin(16, 16).unwrap();
		for _ in 0..8 {
			raster.save().unwrap();
			raster.clip(&rectangle(), VERBS, POINTS).unwrap();
		}
		assert_eq!(
			raster.clip(&rectangle(), VERBS, POINTS),
			Err(Error::UnsupportedFeature)
		);
		assert_eq!(raster.masks.len(), 8);
		for _ in 0..8 {
			raster.restore().unwrap();
		}
		for _ in 0..64 {
			raster.save().unwrap();
		}
		assert_eq!(raster.save(), Err(Error::UnsupportedFeature));
		for _ in 0..64 {
			raster.restore().unwrap();
		}
		raster.finish().unwrap();
	}
}
