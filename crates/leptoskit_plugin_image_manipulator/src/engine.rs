use std::io::Cursor;
use std::io::Write;

use image::DynamicImage;
use image::ExtendedColorType;
use image::ImageDecoder;
use image::ImageEncoder;
use image::ImageFormat;
use image::ImageReader;

use super::*;

/// The PNG signature length that precedes the chunk list.
const PNG_SIGNATURE_BYTES: usize = 8;

/// A chunk's length and type fields.
const PNG_CHUNK_HEADER_BYTES: usize = 8;

/// A chunk's length and type fields, plus its trailing CRC.
const PNG_CHUNK_OVERHEAD: usize = 12;

/// The first byte of every JPEG marker.
const MARKER_PREFIX: u8 = 0xff;

/// The marker that ends a JPEG stream.
const END_OF_IMAGE: u8 = 0xd9;

/// The marker that introduces entropy-coded scan data.
const START_OF_SCAN: u8 = 0xda;

/// A segment length includes its own two bytes.
const MIN_SEGMENT_BYTES: usize = 2;

/// Worst supported decoded PNG has 8 bytes per pixel.
const DECODED_PIXEL_BYTES: u64 = 8;

/// Decoding may retain the codec buffer and the RGBA8 copy: 4 + 8 bytes per
/// pixel.
const DECODE_SCRATCH_PER_PIXEL: u64 = 12;

/// Encoding may retain RGBA8 and the codec's worst-case scratch: 4 + 8 bytes
/// per pixel.
const ENCODE_SCRATCH_PER_PIXEL: u64 = 12;

/// One RGB channel's full value, and the bias that rounds a division by it.
const MAX_CHANNEL: u32 = 255;
const ROUNDING_BIAS: u32 = 127;

/// Owned RGBA8 pixels. Dimensions and byte length always agree and satisfy the
/// public pixel limits. Color values are not color-profile converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
	width: u32,
	height: u32,
	rgba: Vec<u8>,
}
impl Bitmap {
	/// Creates a bounded bitmap from exactly width × height × 4 bytes.
	pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, Error> {
		let count = pixels(width, height)?;

		if rgba.len() as u64 != count * 4 {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			width,
			height,
			rgba,
		})
	}

	/// Pixel width.
	pub fn width(&self) -> u32 {
		self.width
	}

	/// Pixel height.
	pub fn height(&self) -> u32 {
		self.height
	}

	/// Borrows row-major RGBA8 bytes.
	pub fn rgba(&self) -> &[u8] {
		&self.rgba
	}

	/// Applies a preflighted operation sequence, consuming the input
	/// allocation.
	pub fn transform(mut self, operations: &[Operation]) -> Result<Self, Error> {
		for (operation, width, height) in plan(self.width, self.height, operations)? {
			if is_noop(&operation, (width, height), (self.width, self.height)) {
				continue;
			}

			self = self.apply(operation, width, height);
		}

		Ok(self)
	}

	/// Applies one preflighted operation at the size the plan computed.
	fn apply(self, operation: Operation, width: u32, height: u32) -> Self {
		match operation {
			Operation::Flip(axis) => self.flipped(axis),
			Operation::Resize { filter, .. } => self.resize(width, height, filter),
			Operation::Crop { x, y, .. } => self.cropped(x, y, width, height),
			Operation::Rotate(Rotation::HalfTurn) => self.half_turned(),
			Operation::Rotate(rotation) => self.turned(rotation, width, height),
		}
	}

	/// Copies the crop window into a fresh buffer, one row at a time.
	fn cropped(self, x: u32, y: u32, width: u32, height: u32) -> Self {
		let mut destination = vec![0; (u64::from(width) * u64::from(height) * 4) as usize];
		let count = width as usize * 4;

		for row in 0..height {
			let start = self.offset(x, y + row);
			let target = row as usize * count;
			destination[target..target + count].copy_from_slice(&self.rgba[start..start + count]);
		}

		Self {
			width,
			height,
			rgba: destination,
		}
	}

	/// Swaps the two halves of the buffer in place.
	fn half_turned(mut self) -> Self {
		let count = self.rgba.len() / 4;

		for index in 0..count / 2 {
			let opposite = count - 1 - index;

			for channel in 0..4 {
				self.rgba.swap(index * 4 + channel, opposite * 4 + channel);
			}
		}

		self
	}

	/// Copies every pixel to its rotated destination in a fresh buffer.
	fn turned(self, rotation: Rotation, width: u32, height: u32) -> Self {
		let mut destination = vec![0; self.rgba.len()];

		for y in 0..self.height {
			for x in 0..self.width {
				let (dx, dy) = rotation.destination(x, y, self.width, self.height);
				let source = self.offset(x, y);
				let target = ((dy * width + dx) * 4) as usize;
				destination[target..target + 4].copy_from_slice(&self.rgba[source..source + 4]);
			}
		}

		Self {
			width,
			height,
			rgba: destination,
		}
	}

	fn offset(&self, x: u32, y: u32) -> usize {
		((y * self.width + x) * 4) as usize
	}

	/// Swaps mirrored pixel pairs in place, one channel at a time.
	fn flipped(mut self, axis: Axis) -> Self {
		let (span_x, span_y) = match axis {
			Axis::Horizontal => (self.width / 2, self.height),
			Axis::Vertical => (self.width, self.height / 2),
		};

		for y in 0..span_y {
			for x in 0..span_x {
				let source = self.offset(x, y);
				let (dx, dy) = self.mirror(x, y, axis);
				let target = self.offset(dx, dy);

				self.swap_pixel(source, target);
			}
		}

		self
	}

	/// Swaps one pixel's four channels with another's.
	fn swap_pixel(&mut self, source: usize, target: usize) {
		for channel in 0..4 {
			self.rgba.swap(source + channel, target + channel);
		}
	}

	/// The pixel that this one trades places with around the axis.
	fn mirror(&self, x: u32, y: u32, axis: Axis) -> (u32, u32) {
		match axis {
			Axis::Horizontal => (self.width - 1 - x, y),
			Axis::Vertical => (x, self.height - 1 - y),
		}
	}

	/// Samples every output pixel into a fresh buffer.
	fn resize(&self, width: u32, height: u32, filter: Filter) -> Self {
		let mut rgba = vec![0; (u64::from(width) * u64::from(height) * 4) as usize];

		for y in 0..height {
			for x in 0..width {
				let target = ((y * width + x) * 4) as usize;
				let pixel = self.sample(x, y, width, height, filter);

				rgba[target..target + 4].copy_from_slice(&pixel);
			}
		}

		Self {
			width,
			height,
			rgba,
		}
	}

	/// Composites the pixels over the caller's background, dropping alpha.
	fn composite(&self, background: [u8; 3]) -> Vec<u8> {
		let mut rgb = Vec::with_capacity(self.rgba.len() / 4 * 3);

		for pixel in self.rgba.as_chunks::<4>().0 {
			let alpha = u32::from(pixel[3]);

			for channel in 0..3 {
				let composite = u32::from(pixel[channel]) * alpha
					+ u32::from(background[channel]) * (MAX_CHANNEL - alpha)
					+ ROUNDING_BIAS;
				rgb.push((composite / MAX_CHANNEL) as u8);
			}
		}

		rgb
	}

	/// The sampled source pixel under one output pixel.
	fn sample(&self, x: u32, y: u32, width: u32, height: u32, filter: Filter) -> [u8; 4] {
		match filter {
			Filter::Nearest => self.nearest(x, y, width, height),
			Filter::Bilinear => self.bilinear(x, y, width, height),
		}
	}

	/// Output pixel (`x`, `y`) of a `width` by `height` resize: the source
	/// pixel under its centre.
	fn nearest(&self, x: u32, y: u32, width: u32, height: u32) -> [u8; 4] {
		let sx = (((u64::from(x) * 2 + 1) * u64::from(self.width)) / (u64::from(width) * 2)) as u32;
		let sy =
			(((u64::from(y) * 2 + 1) * u64::from(self.height)) / (u64::from(height) * 2)) as u32;
		let source = self.offset(sx.min(self.width - 1), sy.min(self.height - 1));
		let mut pixel = [0; 4];
		pixel.copy_from_slice(&self.rgba[source..source + 4]);
		pixel
	}

	/// Output pixel (`x`, `y`) of a `width` by `height` resize: the four
	/// source pixels around its centre, blended with alpha-weighted colour so
	/// transparent neighbours lend none. A fully transparent result is black.
	fn bilinear(&self, x: u32, y: u32, width: u32, height: u32) -> [u8; 4] {
		let sx = ((f64::from(x) + 0.5) * f64::from(self.width) / f64::from(width) - 0.5)
			.clamp(0.0, f64::from(self.width - 1));
		let sy = ((f64::from(y) + 0.5) * f64::from(self.height) / f64::from(height) - 0.5)
			.clamp(0.0, f64::from(self.height - 1));
		let left = sx.floor() as u32;
		let top = sy.floor() as u32;
		let right = (left + 1).min(self.width - 1);
		let bottom = (top + 1).min(self.height - 1);
		let left_weight = sx - f64::from(left);
		let top_weight = sy - f64::from(top);

		let mut sum = [0.0_f64; 4];
		for (px, py, weight) in [
			(left, top, (1.0 - left_weight) * (1.0 - top_weight)),
			(right, top, left_weight * (1.0 - top_weight)),
			(left, bottom, (1.0 - left_weight) * top_weight),
			(right, bottom, left_weight * top_weight),
		] {
			let source = self.offset(px, py);
			let alpha = f64::from(self.rgba[source + 3]);
			sum[3] += alpha * weight;

			for (channel, value) in sum[..3].iter_mut().enumerate() {
				*value += f64::from(self.rgba[source + channel]) * alpha * weight;
			}
		}
		let mut pixel = [0; 4];

		if sum[3] > 0.0 {
			for (value, total) in pixel[..3].iter_mut().zip(&sum[..3]) {
				*value = (total / sum[3]).round().clamp(0.0, 255.0) as u8;
			}
		}

		pixel[3] = sum[3].round().clamp(0.0, 255.0) as u8;
		pixel
	}
}

/// Walks the PNG chunk list, rejecting animation and a truncated stream.
/// Every chunk length is checked before the bytes it claims are read.
fn validate_png_chunks(input: &[u8]) -> Result<(), Error> {
	let mut offset = PNG_SIGNATURE_BYTES;

	while offset < input.len() {
		let header = input
			.get(offset..offset + PNG_CHUNK_HEADER_BYTES)
			.ok_or(Error::InvalidArgument)?;
		let length = u32::from_be_bytes(header[..4].try_into().map_err(|_| Error::InvalidArgument)?)
			as usize;
		let end = offset
			.checked_add(PNG_CHUNK_OVERHEAD)
			.and_then(|start| start.checked_add(length))
			.ok_or(Error::TooLarge)?;

		if end > input.len() {
			return Err(Error::InvalidArgument);
		}

		let kind = &header[4..8];

		if matches!(kind, b"acTL" | b"fcTL" | b"fdAT") {
			return Err(Error::Unavailable);
		}

		if kind == b"IEND" {
			return finished(input, length, end);
		}

		offset = end;
	}

	Err(Error::InvalidArgument)
}

/// A complete PNG ends with an empty IEND chunk and no trailing bytes.
fn finished(input: &[u8], length: usize, end: usize) -> Result<(), Error> {
	if length == 0 && end == input.len() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// A resize or a top-left crop to the current size leaves every pixel in place.
fn is_noop(operation: &Operation, planned: (u32, u32), current: (u32, u32)) -> bool {
	planned == current
		&& matches!(
			operation,
			Operation::Resize { .. } | Operation::Crop { x: 0, y: 0, .. }
		)
}

/// Length of a JPEG stream through its end-of-image marker. Camera apps may
/// append data after it, for example a motion photo's video or a vendor
/// trailer. That data is not part of the image and is never decoded.
fn jpeg_length(input: &[u8]) -> Result<usize, Error> {
	let mut offset = 2_usize;
	// True after a start-of-scan header, while reading entropy-coded data.
	let mut scan = false;

	while offset < input.len() {
		if input[offset] != MARKER_PREFIX {
			if !scan {
				return Err(Error::InvalidArgument);
			}

			offset += 1;
			continue;
		}

		let marker = *input.get(offset + 1).ok_or(Error::InvalidArgument)?;

		match marker {
			END_OF_IMAGE => return Ok(offset + 2),
			// A fill byte; the next 0xff introduces the marker.
			MARKER_PREFIX => offset += 1,
			// Stuffed zero bytes and restart markers belong to the scan.
			0x00 | 0xd0..=0xd7 if scan => offset += 2,
			0x00 => return Err(Error::InvalidArgument),
			0x01 | 0xd0..=0xd8 => offset += 2,
			// Segment payloads, such as an EXIF thumbnail, may contain their
			// own end-of-image bytes and are skipped by length.
			_ => {
				offset += 2 + segment_length(input, offset)?;
				scan = marker == START_OF_SCAN;
			}
		}
	}

	Err(Error::InvalidArgument)
}

/// The declared length of the segment whose length field sits at `offset`.
fn segment_length(input: &[u8], offset: usize) -> Result<usize, Error> {
	let length = input
		.get(offset + 2..offset + 4)
		.map(|bytes| usize::from(u16::from_be_bytes([bytes[0], bytes[1]])))
		.ok_or(Error::InvalidArgument)?;

	if length < MIN_SEGMENT_BYTES {
		return Err(Error::InvalidArgument);
	}

	Ok(length)
}

/// Decodes a bounded source into RGBA8. PNG animation is rejected; metadata and
/// EXIF orientation are not applied or propagated. 16-bit PNG is converted to
/// 8-bit. Bytes after a JPEG's end-of-image marker are ignored.
pub fn decode(input: &[u8]) -> Result<Bitmap, Error> {
	let info = inspect(input)?;

	let input = match info.format {
		Format::Png => {
			validate_png_chunks(input)?;
			input
		}
		Format::Jpeg => &input[..jpeg_length(input)?],
	};

	let count = pixels(info.width, info.height)?;
	// Worst supported decoded PNG has 8 bytes/pixel; RGBA8 conversion may
	// retain both buffers. JPEG also retains its own copy of the compressed
	// source.
	scratch(count * DECODE_SCRATCH_PER_PIXEL + input.len() as u64 * 2 + MAX_CODEC_BYTES)?;

	let format = match info.format {
		Format::Png => ImageFormat::Png,
		Format::Jpeg => ImageFormat::Jpeg,
	};

	let mut reader = ImageReader::with_format(Cursor::new(input), format);
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(MAX_DIMENSION);

	limits.max_image_height = Some(MAX_DIMENSION);
	limits.max_alloc = Some(MAX_CODEC_BYTES);
	reader.limits(limits);
	let decoder = reader.into_decoder().map_err(|error| image_error(&error))?;

	if decoder.dimensions() != (info.width, info.height)
		|| decoder.total_bytes() > count * DECODED_PIXEL_BYTES
	{
		return Err(Error::InvalidArgument);
	}

	let bitmap = DynamicImage::from_decoder(decoder)
		.map_err(|error| image_error(&error))?
		.into_rgba8();
	Bitmap::from_rgba(info.width, info.height, bitmap.into_raw())
}

/// Computes every intermediate size before any pixel buffer is allocated, and
/// refuses a sequence whose work exceeds the budget.
fn plan(
	mut width: u32,
	mut height: u32,
	operations: &[Operation],
) -> Result<Vec<(Operation, u32, u32)>, Error> {
	if operations.len() > MAX_OPERATIONS {
		return Err(Error::TooLarge);
	}

	let mut work = pixels(width, height)?;
	let mut result = Vec::with_capacity(operations.len());

	for &operation in operations {
		let previous = pixels(width, height)?;
		let (next_width, next_height) = next_size(operation, width, height)?;
		let next = pixels(next_width, next_height)?;
		scratch((previous + next) * 4 + MAX_INPUT_BYTES as u64)?;
		work = work.checked_add(previous + next).ok_or(Error::TooLarge)?;

		if work > MAX_WORK_PIXELS {
			return Err(Error::TooLarge);
		}

		(width, height) = (next_width, next_height);
		result.push((operation, width, height));
	}

	Ok(result)
}

/// The size one operation produces from the current size.
fn next_size(operation: Operation, width: u32, height: u32) -> Result<(u32, u32), Error> {
	match operation {
		Operation::Resize {
			width: target_width,
			height: target_height,
			..
		} => resized_size(target_width, target_height, width, height),
		Operation::Crop {
			x,
			y,
			width: w,
			height: h,
		} => {
			if x.checked_add(w).is_none_or(|end| end > width)
				|| y.checked_add(h).is_none_or(|end| end > height)
			{
				return Err(Error::InvalidArgument);
			}

			Ok((w, h))
		}
		Operation::Rotate(Rotation::Clockwise90 | Rotation::Counterclockwise90) => {
			Ok((height, width))
		}
		Operation::Rotate(Rotation::HalfTurn) | Operation::Flip(_) => Ok((width, height)),
	}
}

/// A resize keeps the aspect ratio when one side is omitted; zero is not a
/// size.
fn resized_size(
	target_width: Option<u32>,
	target_height: Option<u32>,
	width: u32,
	height: u32,
) -> Result<(u32, u32), Error> {
	match (target_width, target_height) {
		(None, None) | (Some(0), _) | (_, Some(0)) => Err(Error::InvalidArgument),
		(Some(w), Some(h)) => Ok((w, h)),
		(Some(w), None) => Ok((w, ratio(height, w, width)?)),
		(None, Some(h)) => Ok((ratio(width, h, height)?, h)),
	}
}

fn ratio(value: u32, numerator: u32, denominator: u32) -> Result<u32, Error> {
	let scaled = (u64::from(value) * u64::from(numerator) + u64::from(denominator) / 2)
		/ u64::from(denominator);
	u32::try_from(scaled.max(1)).map_err(|_| Error::TooLarge)
}

/// Encodes pixels with a capped output writer. JPEG composites alpha against
/// the caller's background. Output strips EXIF, ICC, text and other source
/// metadata.
pub fn encode(bitmap: &Bitmap, output: Output) -> Result<EncodedImage, Error> {
	encode_bounded(bitmap, output, MAX_OUTPUT_BYTES)
}
/// Encodes with a caller-supplied output cap, so tests can prove the bound.
fn encode_bounded(bitmap: &Bitmap, output: Output, limit: usize) -> Result<EncodedImage, Error> {
	output.validate()?;
	let count = pixels(bitmap.width, bitmap.height)?;
	// Includes RGBA, worst-case codec scratch allowance and the output buffer.
	scratch(
		count * ENCODE_SCRATCH_PER_PIXEL + limit as u64 + MAX_INPUT_BYTES as u64 + MAX_CODEC_BYTES,
	)?;
	let mut writer = BoundedWriter {
		bytes: Vec::new(),
		limit,
		exceeded: false,
	};

	let result = match output {
		Output::Png => {
			image::codecs::png::PngEncoder::new_with_quality(
				&mut writer,
				image::codecs::png::CompressionType::Fast,
				image::codecs::png::FilterType::Adaptive,
			)
			.write_image(
				&bitmap.rgba,
				bitmap.width,
				bitmap.height,
				ExtendedColorType::Rgba8,
			)
		}
		Output::Jpeg {
			quality,
			background,
		} => {
			let rgb = bitmap.composite(background);

			image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality).encode(
				&rgb,
				bitmap.width,
				bitmap.height,
				ExtendedColorType::Rgb8,
			)
		}
	};

	if writer.exceeded {
		return Err(Error::TooLarge);
	}

	result.map_err(|error| image_error(&error))?;

	Ok(EncodedImage {
		info: ImageInfo {
			format: output.format(),
			width: bitmap.width,
			height: bitmap.height,
		},
		bytes: writer.bytes.into(),
	})
}
struct BoundedWriter {
	bytes: Vec<u8>,
	limit: usize,
	exceeded: bool,
}
impl Write for BoundedWriter {
	fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
		let required = self
			.bytes
			.len()
			.checked_add(input.len())
			.filter(|required| *required <= self.limit);
		let Some(required) = required else {
			self.exceeded = true;

			return Err(std::io::Error::other("encoded image exceeds output budget"));
		};

		if required > self.bytes.capacity() {
			let capacity = required
				.max(self.bytes.capacity().saturating_mul(2))
				.min(self.limit);
			self.bytes
				.try_reserve_exact(capacity - self.bytes.len())
				.map_err(std::io::Error::other)?;
		}

		self.bytes.extend_from_slice(input);

		Ok(input.len())
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}
fn image_error(error: &image::ImageError) -> Error {
	match error {
		image::ImageError::Limits(_) => Error::TooLarge,
		image::ImageError::Unsupported(_) => Error::Unavailable,
		_ => Error::InvalidArgument,
	}
}

/// Performs preflight, decoding, sequential transforms and encoding. Native
/// plugin calls run on a worker; direct Rust and browser calls are synchronous.
pub fn manipulate(
	input: &[u8],
	operations: &[Operation],
	output: Output,
) -> Result<EncodedImage, Error> {
	let info = inspect(input)?;
	output.validate()?;
	plan(info.width, info.height, operations)?;
	let bitmap = decode(input)?.transform(operations)?;
	encode(&bitmap, output)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A budget too small for any real encoding.
	const TINY_BUDGET: usize = 16;

	#[test]
	fn encoded_writer_stops_before_exceeding_budget() {
		let bitmap = Bitmap::from_rgba(1, 1, vec![1, 2, 3, 255]).unwrap();
		assert_eq!(
			encode_bounded(&bitmap, Output::Png, TINY_BUDGET),
			Err(Error::TooLarge)
		);
		assert_eq!(
			encode_bounded(
				&bitmap,
				Output::Jpeg {
					quality: 80,
					background: [255; 3]
				},
				TINY_BUDGET
			),
			Err(Error::TooLarge)
		);
		let mut writer = BoundedWriter {
			bytes: Vec::new(),
			limit: 5,
			exceeded: false,
		};
		writer.write_all(&[1, 2, 3]).unwrap();
		assert!(writer.write_all(&[4, 5, 6]).is_err());

		assert_eq!(writer.bytes, [1, 2, 3]);
		assert!(writer.bytes.capacity() <= 5);
	}
}
