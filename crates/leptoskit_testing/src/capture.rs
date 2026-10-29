//! Pixel captures and deliberately distinct colored layout diagnostics.

use std::io::Cursor;
use std::path::Path;

use leptoskit_native::Frame;

const MAX_PIXELS: usize = 32_000_000;
/// Widget fills of the flat-colour layout diagnostic, assigned in tree order.
const PALETTE: [[u8; 3]; 5] = [
	[42, 111, 151],
	[233, 160, 59],
	[94, 159, 116],
	[180, 94, 138],
	[131, 112, 181],
];

/// An invalid or unwritable capture.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
	/// Filesystem or stream failure.
	#[error(transparent)]
	Io(#[from] std::io::Error),
	/// The driver did not return a valid PNG.
	#[error(transparent)]
	Decode(#[from] png::DecodingError),
	/// Diagnostic encoding failed.
	#[error(transparent)]
	Encode(#[from] png::EncodingError),
	/// The image exceeds the bounded decoding/rendering budget.
	#[error("capture dimensions must be positive and contain at most {MAX_PIXELS} pixels")]
	Dimensions,
}

/// Unmodified PNG bytes captured by the operating-system automation driver.
///
/// Construction fully decodes the PNG to validate it. Saving preserves the
/// original bytes, color profile, scale, and alpha; there is no synthetic
/// fallback when capture fails.
#[derive(Debug)]
pub struct Screenshot {
	bytes: Vec<u8>,
	width: u32,
	height: u32,
}

impl Screenshot {
	/// Validates a driver capture without changing its encoded pixels.
	pub fn from_png(bytes: Vec<u8>) -> Result<Self, CaptureError> {
		let mut decoder = png::Decoder::new(Cursor::new(&bytes));
		decoder.set_limits(png::Limits {
			bytes: MAX_PIXELS * 8,
		});
		let mut reader = decoder.read_info()?;
		let (width, height) = (reader.info().width, reader.info().height);
		pixel_count(width, height)?;
		let mut pixels = vec![0; reader.output_buffer_size()];
		reader.next_frame(&mut pixels)?;
		reader.finish()?;
		Ok(Self {
			bytes,
			width,
			height,
		})
	}

	/// Physical pixel dimensions, which may differ from logical UI coordinates.
	pub fn dimensions(&self) -> (u32, u32) {
		(self.width, self.height)
	}

	/// The original PNG, suitable for rendering in reports.
	pub fn png(&self) -> &[u8] {
		&self.bytes
	}

	/// Saves the real PNG, without recompression or replacement.
	pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CaptureError> {
		std::fs::write(path, &self.bytes)?;
		Ok(())
	}

	/// Counts distinct pixel values, stopping once more than `limit` are seen.
	///
	/// A rendered screen with text, icons, and anti-aliasing has many; a blank
	/// or flat placeholder frame has very few. Assert a lower bound to reject
	/// drivers that return a substitute image.
	pub fn distinct_colors(&self, limit: usize) -> Result<usize, CaptureError> {
		distinct_colors(&self.bytes, limit)
	}
}

/// Synthetic, colored widget bounds. This is not an OS screenshot.
#[derive(Debug)]
pub struct LayoutImage {
	bytes: Vec<u8>,
}

impl LayoutImage {
	/// The most distinct colours a layout image contains: the background,
	/// widget outlines, and each palette fill.
	pub const MAX_COLORS: usize = PALETTE.len() + 2;

	pub(crate) fn render(width: f32, height: f32, frames: &[Frame]) -> Result<Self, CaptureError> {
		if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
			return Err(CaptureError::Dimensions);
		}
		let (width, height) = (width.ceil() as u32, height.ceil() as u32);
		let pixels = pixel_count(width, height)?;
		let mut rgba = vec![240; pixels * 4];
		for pixel in rgba.as_chunks_mut::<4>().0 {
			pixel[3] = 255;
		}
		for (index, frame) in frames.iter().enumerate() {
			fill_frame(
				&mut rgba,
				width,
				height,
				frame,
				PALETTE[index % PALETTE.len()],
			);
		}
		let mut bytes = Vec::new();
		{
			let mut encoder = png::Encoder::new(&mut bytes, width, height);
			encoder.set_color(png::ColorType::Rgba);
			encoder.set_depth(png::BitDepth::Eight);
			encoder.add_text_chunk(
				"Description".to_owned(),
				"Leptoskit synthetic layout diagnostic; not a native screenshot".to_owned(),
			)?;
			encoder.write_header()?.write_image_data(&rgba)?;
		}
		Ok(Self { bytes })
	}

	/// The diagnostic PNG. Its metadata identifies it as synthetic.
	pub fn png(&self) -> &[u8] {
		&self.bytes
	}

	/// Counts distinct pixel values, stopping once more than `limit` are seen.
	/// A layout image never exceeds [`Self::MAX_COLORS`].
	pub fn distinct_colors(&self, limit: usize) -> Result<usize, CaptureError> {
		distinct_colors(&self.bytes, limit)
	}

	/// Saves a diagnostic PNG. Use a name such as `layout-diagnostic.png`.
	pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CaptureError> {
		std::fs::write(path, &self.bytes)?;
		Ok(())
	}
}

/// Paints `frame`, clipped to the `width` by `height` image, into `rgba`:
/// `color` inside, with a dark one-pixel outline. Alpha is left as is.
fn fill_frame(rgba: &mut [u8], width: u32, height: u32, frame: &Frame, color: [u8; 3]) {
	let left = frame.x.max(0.0).min(width as f32) as u32;
	let top = frame.y.max(0.0).min(height as f32) as u32;
	let right = (frame.x + frame.width).max(0.0).min(width as f32) as u32;
	let bottom = (frame.y + frame.height).max(0.0).min(height as f32) as u32;
	for y in top..bottom {
		for x in left..right {
			let offset = (y as usize * width as usize + x as usize) * 4;
			let border = x == left || y == top || x + 1 == right || y + 1 == bottom;
			let fill = if border { [30; 3] } else { color };
			rgba[offset..offset + 3].copy_from_slice(&fill);
		}
	}
}

fn distinct_colors(bytes: &[u8], limit: usize) -> Result<usize, CaptureError> {
	let mut decoder = png::Decoder::new(Cursor::new(bytes));
	decoder.set_limits(png::Limits {
		bytes: MAX_PIXELS * 8,
	});
	// Normalise palettes, low bit depths, and 16-bit channels so equal colours
	// compare equal regardless of encoding.
	decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
	let mut reader = decoder.read_info()?;
	let mut pixels = vec![0; reader.output_buffer_size()];
	let frame = reader.next_frame(&mut pixels)?;
	let mut colors = std::collections::HashSet::new();
	for pixel in pixels[..frame.buffer_size()].chunks_exact(frame.color_type.samples()) {
		colors.insert(pixel);
		if colors.len() > limit {
			break;
		}
	}
	Ok(colors.len())
}

fn pixel_count(width: u32, height: u32) -> Result<usize, CaptureError> {
	let count = u64::from(width) * u64::from(height);
	if count == 0 || count > MAX_PIXELS as u64 {
		return Err(CaptureError::Dimensions);
	}
	Ok(count as usize)
}
