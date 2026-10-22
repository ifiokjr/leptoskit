//! Bounded JPEG/PNG transforms in Rust, with a native worker bridge.
//!
//! Browser calls use the same synchronous engine. Schedule large jobs in a Web
//! Worker; an async wrapper alone would not move CPU work off the browser UI.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
#[cfg(feature = "engine")]
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

#[cfg(feature = "engine")]
mod engine;
mod header;
#[cfg(feature = "engine")]
pub use engine::Bitmap;
#[cfg(feature = "engine")]
pub use engine::decode;
#[cfg(feature = "engine")]
pub use engine::encode;
#[cfg(feature = "engine")]
pub use engine::manipulate;
pub use header::inspect;

/// Inspect header dimensions. Raw encoded bytes -> format, width, height u32
/// LE.
pub const INSPECT: MethodId = MethodId(0);
/// Decode, transform and encode using the typed request/response protocol.
pub const MANIPULATE: MethodId = MethodId(1);
/// Maximum compressed source bytes.
pub const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
/// Maximum encoded output bytes.
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
/// Maximum width or height, independently of the total pixel bound.
pub const MAX_DIMENSION: u32 = 8192;
/// Maximum pixels in the source and every intermediate image.
pub const MAX_PIXELS: u64 = 4 * 1024 * 1024;
/// Budget used by preflight for owned image buffers and codec allowances.
/// Codec-private allocator overhead is not a process-wide hard heap limit.
pub const MAX_SCRATCH_BYTES: u64 = 64 * 1024 * 1024;
/// PNG decoder allocation limit, also reserved during scratch preflight.
/// The JPEG decoder enforces dimensions but does not implement an allocation
/// cap.
pub const MAX_CODEC_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum transforms per request.
pub const MAX_OPERATIONS: usize = 16;
/// Maximum sum of source/output pixels visited by the transformation plan.
pub const MAX_WORK_PIXELS: u64 = 64 * 1024 * 1024;

/// Supported encoded image formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
	/// PNG, preserving transparency.
	Png,
	/// JPEG with no alpha channel.
	Jpeg,
}
impl Format {
	fn tag(self) -> u32 {
		match self {
			Self::Png => 0,
			Self::Jpeg => 1,
		}
	}

	fn from_tag(tag: u32) -> Result<Self, Error> {
		match tag {
			0 => Ok(Self::Png),
			1 => Ok(Self::Jpeg),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Header dimensions; inspecting a header does not validate all compressed
/// data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
	/// Source or output format.
	pub format: Format,
	/// Pixel width.
	pub width: u32,
	/// Pixel height.
	pub height: u32,
}
impl ImageInfo {
	/// Fixed-width response for INSPECT.
	pub fn encode(self) -> Result<Bytes, Error> {
		pixels(self.width, self.height)?;
		let mut output = Encoder::with_capacity(12);
		output.u32(self.format.tag());
		output.u32(self.width);
		output.u32(self.height);
		Ok(output.finish())
	}

	/// Decodes and bounds header dimensions.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut decoder = Decoder::new(bytes);
		let info = Self {
			format: Format::from_tag(decoder.u32()?)?,
			width: decoder.u32()?,
			height: decoder.u32()?,
		};
		decoder.finish()?;
		pixels(info.width, info.height)?;
		Ok(info)
	}
}

/// Resize sampling. Bilinear interpolation uses premultiplied alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
	/// Fast nearest-neighbour sampling.
	Nearest,
	/// Four-sample interpolation without an intermediate float image.
	Bilinear,
}
/// Lossless right-angle rotation, clockwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
	/// 90 degrees clockwise.
	Clockwise90,
	/// 180 degrees.
	HalfTurn,
	/// 90 degrees counterclockwise.
	Counterclockwise90,
}
/// Flip direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
	/// Reverse pixel columns.
	Horizontal,
	/// Reverse pixel rows.
	Vertical,
}
/// Transformations are applied in order. Crop coordinates refer to the image
/// produced by previous operations. One omitted resize dimension preserves
/// ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
	/// Resize to specified dimensions; at least one must be present and
	/// nonzero.
	Resize {
		/// Requested width, or derive it from height.
		width: Option<u32>,
		/// Requested height, or derive it from width.
		height: Option<u32>,
		/// Sampling method.
		filter: Filter,
	},
	/// Crop a rectangle strictly inside the current image.
	Crop {
		/// Left pixel.
		x: u32,
		/// Top pixel.
		y: u32,
		/// Rectangle width.
		width: u32,
		/// Rectangle height.
		height: u32,
	},
	/// Rotate by a right angle.
	Rotate(Rotation),
	/// Flip without allocating a second image.
	Flip(Axis),
}
/// Output options. JPEG requires explicit alpha compositing against a
/// background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
	/// Lossless PNG, including alpha.
	Png,
	/// JPEG with quality in 1..=100 and an RGB background for transparent
	/// pixels.
	Jpeg {
		/// Compression quality.
		quality: u8,
		/// Background RGB bytes.
		background: [u8; 3],
	},
}
impl Output {
	fn validate(self) -> Result<(), Error> {
		if matches!(
			self,
			Self::Jpeg {
				quality: 0 | 101..=255,
				..
			}
		) {
			Err(Error::InvalidArgument)
		} else {
			Ok(())
		}
	}

	#[cfg(feature = "engine")]
	fn format(self) -> Format {
		match self {
			Self::Png => Format::Png,
			Self::Jpeg { .. } => Format::Jpeg,
		}
	}
}

/// Encoded result without metadata, filesystem paths or base64 conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedImage {
	/// Output dimensions and format.
	pub info: ImageInfo,
	/// Encoded JPEG or PNG bytes.
	pub bytes: Bytes,
}
impl EncodedImage {
	/// Encodes dimensions followed by length-prefixed encoded bytes.
	pub fn encode_reply(&self) -> Result<Bytes, Error> {
		if self.bytes.len() > MAX_OUTPUT_BYTES {
			return Err(Error::TooLarge);
		}
		let info = self.info.encode()?;
		let mut encoder = Encoder::with_capacity(20 + self.bytes.len());
		encoder.bytes(&info)?;
		encoder.bytes(&self.bytes)?;
		Ok(encoder.finish())
	}

	/// Verifies dimensions and format against the returned encoded header.
	/// Retains a slice of the owned reply without copying the image bytes.
	pub fn decode_reply(mut bytes: Bytes) -> Result<Self, Error> {
		if bytes.len() > MAX_OUTPUT_BYTES + 20 {
			return Err(Error::TooLarge);
		}
		let mut decoder = Decoder::new(&bytes);
		let info = ImageInfo::decode(decoder.bytes()?)?;
		let image = decoder.bytes()?;
		decoder.finish()?;
		if image.len() > MAX_OUTPUT_BYTES {
			return Err(Error::TooLarge);
		}
		if inspect(image)? != info {
			return Err(Error::InvalidArgument);
		}
		bytes = bytes.slice_ref(image);
		Ok(Self { info, bytes })
	}
}

/// Encodes a bounded transformation request; image data remains binary.
pub fn encode_request(
	input: &[u8],
	operations: &[Operation],
	output: Output,
) -> Result<Bytes, Error> {
	if input.len() > MAX_INPUT_BYTES || operations.len() > MAX_OPERATIONS {
		return Err(Error::TooLarge);
	}
	output.validate()?;
	let mut encoder = Encoder::with_capacity(input.len() + operations.len() * 20 + 28);
	match output {
		Output::Png => encoder.u32(0),
		Output::Jpeg {
			quality,
			background,
		} => {
			encoder.u32(1);
			encoder.u32(u32::from(quality));
			for byte in background {
				encoder.u32(u32::from(byte));
			}
		}
	}
	encoder.u32(operations.len() as u32);
	for operation in operations {
		match *operation {
			Operation::Resize {
				width,
				height,
				filter,
			} => {
				if width == Some(0) || height == Some(0) || (width.is_none() && height.is_none()) {
					return Err(Error::InvalidArgument);
				}
				encoder.u32(0);
				encoder.u32(width.unwrap_or(0));
				encoder.u32(height.unwrap_or(0));
				encoder.u32(match filter {
					Filter::Nearest => 0,
					Filter::Bilinear => 1,
				});
			}
			Operation::Crop {
				x,
				y,
				width,
				height,
			} => {
				encoder.u32(1);
				for value in [x, y, width, height] {
					encoder.u32(value);
				}
			}
			Operation::Rotate(rotation) => {
				encoder.u32(2);
				encoder.u32(match rotation {
					Rotation::Clockwise90 => 0,
					Rotation::HalfTurn => 1,
					Rotation::Counterclockwise90 => 2,
				});
			}
			Operation::Flip(axis) => {
				encoder.u32(3);
				encoder.u32(match axis {
					Axis::Horizontal => 0,
					Axis::Vertical => 1,
				});
			}
		}
	}
	encoder.bytes(input)?;
	Ok(encoder.finish())
}

#[cfg(feature = "engine")]
fn decode_request(bytes: &[u8]) -> Result<(&[u8], Vec<Operation>, Output), Error> {
	if bytes.len() > MAX_INPUT_BYTES + MAX_OPERATIONS * 20 + 28 {
		return Err(Error::TooLarge);
	}
	let mut decoder = Decoder::new(bytes);
	let output = match decoder.u32()? {
		0 => Output::Png,
		1 => {
			Output::Jpeg {
				quality: u8::try_from(decoder.u32()?).map_err(|_| Error::InvalidArgument)?,
				background: [
					read_byte(&mut decoder)?,
					read_byte(&mut decoder)?,
					read_byte(&mut decoder)?,
				],
			}
		}
		_ => return Err(Error::InvalidArgument),
	};
	output.validate()?;
	let count = decoder.u32()? as usize;
	if count > MAX_OPERATIONS {
		return Err(Error::TooLarge);
	}
	let mut operations = Vec::with_capacity(count);
	for _ in 0..count {
		operations.push(match decoder.u32()? {
			0 => {
				let width = decoder.u32()?;
				let height = decoder.u32()?;
				let filter = match decoder.u32()? {
					0 => Filter::Nearest,
					1 => Filter::Bilinear,
					_ => return Err(Error::InvalidArgument),
				};
				Operation::Resize {
					width: (width != 0).then_some(width),
					height: (height != 0).then_some(height),
					filter,
				}
			}
			1 => {
				Operation::Crop {
					x: decoder.u32()?,
					y: decoder.u32()?,
					width: decoder.u32()?,
					height: decoder.u32()?,
				}
			}
			2 => {
				Operation::Rotate(match decoder.u32()? {
					0 => Rotation::Clockwise90,
					1 => Rotation::HalfTurn,
					2 => Rotation::Counterclockwise90,
					_ => return Err(Error::InvalidArgument),
				})
			}
			3 => {
				Operation::Flip(match decoder.u32()? {
					0 => Axis::Horizontal,
					1 => Axis::Vertical,
					_ => return Err(Error::InvalidArgument),
				})
			}
			_ => return Err(Error::InvalidArgument),
		});
	}
	let input = decoder.bytes()?;
	decoder.finish()?;
	if input.len() > MAX_INPUT_BYTES {
		return Err(Error::TooLarge);
	}
	Ok((input, operations, output))
}
#[cfg(feature = "engine")]
fn read_byte(decoder: &mut Decoder<'_>) -> Result<u8, Error> {
	u8::try_from(decoder.u32()?).map_err(|_| Error::InvalidArgument)
}

fn pixels(width: u32, height: u32) -> Result<u64, Error> {
	if width == 0 || height == 0 {
		return Err(Error::InvalidArgument);
	}
	let count = u64::from(width) * u64::from(height);
	if width > MAX_DIMENSION || height > MAX_DIMENSION || count > MAX_PIXELS {
		return Err(Error::TooLarge);
	}
	Ok(count)
}
#[cfg(feature = "engine")]
fn scratch(bytes: u64) -> Result<(), Error> {
	if bytes > MAX_SCRATCH_BYTES {
		Err(Error::TooLarge)
	} else {
		Ok(())
	}
}

/// Automatically registered native worker implementation.
#[derive(Debug, Default)]
#[cfg(feature = "engine")]
pub struct ImageManipulator;
#[cfg(feature = "engine")]
impl Plugin for ImageManipulator {
	fn name(&self) -> &'static str {
		"image_manipulator"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[INSPECT, MANIPULATE]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		match method {
			INSPECT => inspect(&payload)?.encode(),
			MANIPULATE => {
				let (input, operations, output) = decode_request(&payload)?;
				manipulate(input, &operations, output)?.encode_reply()
			}
			_ => Err(Error::Unavailable),
		}
	}
}

/// Typed interpreter client; native operations execute on the Rust worker.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::guest::Client;

	use super::*;
	/// Resolves the installed manipulator once and reuses its plugin ID.
	#[derive(Debug, Clone, Copy)]
	pub struct ImageManipulator(Client);
	impl ImageManipulator {
		/// Resolves the permitted plugin.
		pub fn new() -> Result<Self, Error> {
			Client::new("image_manipulator").map(Self)
		}

		/// Inspects bounded image dimensions without decoding pixels.
		pub async fn inspect(self, input: &[u8]) -> Result<ImageInfo, Error> {
			if input.len() > MAX_INPUT_BYTES {
				return Err(Error::TooLarge);
			}
			ImageInfo::decode(&self.0.call(INSPECT, input)?.await?)
		}

		/// Transforms binary image data, validating the returned encoded
		/// header.
		pub async fn manipulate(
			self,
			input: &[u8],
			operations: &[Operation],
			output: Output,
		) -> Result<EncodedImage, Error> {
			EncodedImage::decode_reply(
				self.0
					.call(MANIPULATE, &encode_request(input, operations, output)?)?
					.await?,
			)
		}
	}
}
