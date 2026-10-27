//! Portable borrowed pixel views, without any runtime or operating-system code.

use crate::Error;

/// Maximum pixels in one native backing buffer (16 MiB of RGBA data).
pub const MAX_FRAME_PIXELS: u64 = 4_194_304;

/// Premultiplied RGBA8888 pixels borrowed for synchronous upload into a native
/// image or texture. Rows have no padding. The owner retains the allocation.
pub struct Frame<'a> {
	pub(crate) width: u32,
	pub(crate) height: u32,
	pub(crate) bytes: &'a [u8],
}

impl std::fmt::Debug for Frame<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Frame")
			.field("width", &self.width)
			.field("height", &self.height)
			.field("byte_len", &self.bytes.len())
			.finish()
	}
}

impl<'a> Frame<'a> {
	/// Validates a tightly packed, bounded pixel view without copying its
	/// bytes.
	pub fn new(width: u32, height: u32, bytes: &'a [u8]) -> Result<Self, Error> {
		let count = u64::from(width) * u64::from(height);

		if width == 0
			|| height == 0
			|| width > 4096
			|| height > 4096
			|| count > MAX_FRAME_PIXELS
			|| count * 4 != bytes.len() as u64
		{
			return Err(Error::InvalidFrame);
		}
		Ok(Self {
			width,
			height,
			bytes,
		})
	}

	/// Backing width in physical pixels.
	pub fn width(&self) -> u32 {
		self.width
	}

	/// Backing height in physical pixels.
	pub fn height(&self) -> u32 {
		self.height
	}

	/// Bytes per row, without padding.
	pub fn stride(&self) -> usize {
		self.width as usize * 4
	}

	/// Premultiplied RGBA8888 pixels, borrowed from the owner's allocation.
	pub fn bytes(&self) -> &[u8] {
		self.bytes
	}
}
