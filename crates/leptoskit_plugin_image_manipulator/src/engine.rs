use std::io::Cursor;
use std::io::Write;

use image::DynamicImage;
use image::ExtendedColorType;
use image::ImageDecoder;
use image::ImageEncoder;
use image::ImageFormat;
use image::ImageReader;

use super::*;

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
		let plan = plan(self.width, self.height, operations)?;
		for (operation, width, height) in plan {
			if (width, height) == (self.width, self.height)
				&& matches!(
					operation,
					Operation::Resize { .. } | Operation::Crop { x: 0, y: 0, .. }
				) {
				continue;
			}
			match operation {
				Operation::Flip(axis) => self.flip(axis),
				Operation::Resize { filter, .. } => self = self.resize(width, height, filter),
				Operation::Crop { x, y, .. } => {
					let mut destination =
						vec![0; (u64::from(width) * u64::from(height) * 4) as usize];
					let count = width as usize * 4;
					for row in 0..height {
						let start = self.offset(x, y + row);
						let target = row as usize * count;
						destination[target..target + count]
							.copy_from_slice(&self.rgba[start..start + count]);
					}
					self = Self {
						width,
						height,
						rgba: destination,
					};
				}
				Operation::Rotate(Rotation::HalfTurn) => {
					let count = self.rgba.len() / 4;
					for index in 0..count / 2 {
						let opposite = count - 1 - index;
						for channel in 0..4 {
							self.rgba.swap(index * 4 + channel, opposite * 4 + channel);
						}
					}
				}
				Operation::Rotate(rotation) => {
					let mut destination = vec![0; self.rgba.len()];
					for y in 0..self.height {
						for x in 0..self.width {
							let (dx, dy) = match rotation {
								Rotation::Clockwise90 => (self.height - 1 - y, x),
								Rotation::HalfTurn => (self.width - 1 - x, self.height - 1 - y),
								Rotation::Counterclockwise90 => (y, self.width - 1 - x),
							};
							let source = self.offset(x, y);
							let target = ((dy * width + dx) * 4) as usize;
							destination[target..target + 4]
								.copy_from_slice(&self.rgba[source..source + 4]);
						}
					}
					self = Self {
						width,
						height,
						rgba: destination,
					};
				}
			}
		}
		Ok(self)
	}

	fn offset(&self, x: u32, y: u32) -> usize {
		((y * self.width + x) * 4) as usize
	}

	fn flip(&mut self, axis: Axis) {
		let (width, height) = match axis {
			Axis::Horizontal => (self.width / 2, self.height),
			Axis::Vertical => (self.width, self.height / 2),
		};
		for y in 0..height {
			for x in 0..width {
				let (dx, dy) = match axis {
					Axis::Horizontal => (self.width - 1 - x, y),
					Axis::Vertical => (x, self.height - 1 - y),
				};
				let source = self.offset(x, y);
				let target = self.offset(dx, dy);
				for channel in 0..4 {
					self.rgba.swap(source + channel, target + channel);
				}
			}
		}
	}

	fn resize(&self, width: u32, height: u32, filter: Filter) -> Self {
		let mut rgba = vec![0; (u64::from(width) * u64::from(height) * 4) as usize];
		for y in 0..height {
			for x in 0..width {
				let target = ((y * width + x) * 4) as usize;
				match filter {
					Filter::Nearest => {
						let sx = (((u64::from(x) * 2 + 1) * u64::from(self.width))
							/ (u64::from(width) * 2)) as u32;
						let sy = (((u64::from(y) * 2 + 1) * u64::from(self.height))
							/ (u64::from(height) * 2)) as u32;
						let source = self.offset(sx.min(self.width - 1), sy.min(self.height - 1));
						rgba[target..target + 4].copy_from_slice(&self.rgba[source..source + 4]);
					}
					Filter::Bilinear => {
						let sx = ((f64::from(x) + 0.5) * f64::from(self.width) / f64::from(width)
							- 0.5)
							.clamp(0.0, f64::from(self.width - 1));
						let sy = ((f64::from(y) + 0.5) * f64::from(self.height)
							/ f64::from(height)
							- 0.5)
							.clamp(0.0, f64::from(self.height - 1));
						let left = sx.floor() as u32;
						let top = sy.floor() as u32;
						let right = (left + 1).min(self.width - 1);
						let bottom = (top + 1).min(self.height - 1);
						let wx = sx - f64::from(left);
						let wy = sy - f64::from(top);
						let mut sum = [0.0_f64; 4];
						for (px, py, weight) in [
							(left, top, (1.0 - wx) * (1.0 - wy)),
							(right, top, wx * (1.0 - wy)),
							(left, bottom, (1.0 - wx) * wy),
							(right, bottom, wx * wy),
						] {
							let source = self.offset(px, py);
							let alpha = f64::from(self.rgba[source + 3]);
							sum[3] += alpha * weight;
							for (channel, value) in sum[..3].iter_mut().enumerate() {
								*value += f64::from(self.rgba[source + channel]) * alpha * weight;
							}
						}
						if sum[3] > 0.0 {
							for channel in 0..3 {
								rgba[target + channel] =
									(sum[channel] / sum[3]).round().clamp(0.0, 255.0) as u8;
							}
						}
						rgba[target + 3] = sum[3].round().clamp(0.0, 255.0) as u8;
					}
				}
			}
		}
		Self {
			width,
			height,
			rgba,
		}
	}
}

fn validate_png_chunks(input: &[u8]) -> Result<(), Error> {
	let mut offset = 8_usize;
	while offset < input.len() {
		let header = input
			.get(offset..offset + 8)
			.ok_or(Error::InvalidArgument)?;
		let length = u32::from_be_bytes(header[..4].try_into().map_err(|_| Error::InvalidArgument)?)
			as usize;
		let end = offset
			.checked_add(12)
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
			return if length == 0 && end == input.len() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			};
		}
		offset = end;
	}
	Err(Error::InvalidArgument)
}

/// Decodes a bounded source into RGBA8. PNG animation is rejected; metadata and
/// EXIF orientation are not applied or propagated. 16-bit PNG is converted to
/// 8-bit.
pub fn decode(input: &[u8]) -> Result<Bitmap, Error> {
	let info = inspect(input)?;
	let count = pixels(info.width, info.height)?;
	// Worst supported decoded PNG has 8 bytes/pixel; RGBA8 conversion may
	// retain both buffers. JPEG also retains its own copy of the compressed
	// source.
	scratch(count * 12 + input.len() as u64 * 2 + MAX_CODEC_BYTES)?;
	if info.format == Format::Png {
		validate_png_chunks(input)?;
	} else if !input.ends_with(&[0xff, 0xd9]) {
		return Err(Error::InvalidArgument);
	}
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
	if decoder.dimensions() != (info.width, info.height) || decoder.total_bytes() > count * 8 {
		return Err(Error::InvalidArgument);
	}
	let bitmap = DynamicImage::from_decoder(decoder)
		.map_err(|error| image_error(&error))?
		.into_rgba8();
	Bitmap::from_rgba(info.width, info.height, bitmap.into_raw())
}

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
		let (next_width, next_height) = match operation {
			Operation::Resize {
				width: target_width,
				height: target_height,
				..
			} => {
				match (target_width, target_height) {
					(None, None) | (Some(0), _) | (_, Some(0)) => {
						return Err(Error::InvalidArgument);
					}
					(Some(w), Some(h)) => (w, h),
					(Some(w), None) => (w, ratio(height, w, width)?),
					(None, Some(h)) => (ratio(width, h, height)?, h),
				}
			}
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
				(w, h)
			}
			Operation::Rotate(Rotation::Clockwise90 | Rotation::Counterclockwise90) => {
				(height, width)
			}
			Operation::Rotate(Rotation::HalfTurn) | Operation::Flip(_) => (width, height),
		};
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
fn encode_bounded(bitmap: &Bitmap, output: Output, limit: usize) -> Result<EncodedImage, Error> {
	output.validate()?;
	let count = pixels(bitmap.width, bitmap.height)?;
	// Includes RGBA, worst-case codec scratch allowance and the output buffer.
	scratch(count * 12 + limit as u64 + MAX_INPUT_BYTES as u64 + MAX_CODEC_BYTES)?;
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
			let mut rgb = Vec::with_capacity(count as usize * 3);
			for pixel in bitmap.rgba.chunks_exact(4) {
				let alpha = u32::from(pixel[3]);
				for channel in 0..3 {
					rgb.push(
						((u32::from(pixel[channel]) * alpha
							+ u32::from(background[channel]) * (255 - alpha)
							+ 127)
							/ 255) as u8,
					);
				}
			}
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
	#[test]
	fn encoded_writer_stops_before_exceeding_budget() {
		let bitmap = Bitmap::from_rgba(1, 1, vec![1, 2, 3, 255]).unwrap();
		assert_eq!(
			encode_bounded(&bitmap, Output::Png, 16),
			Err(Error::TooLarge)
		);
		assert_eq!(
			encode_bounded(
				&bitmap,
				Output::Jpeg {
					quality: 80,
					background: [255; 3]
				},
				16
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
