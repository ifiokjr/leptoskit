//! Small binary control-message codec shared by plugin implementations.
//!
//! Integers are little-endian; strings/bytes use a u32 byte-length prefix.
//! Decoding borrows the input, performs no allocation, rejects truncation and
//! invalid UTF-8, and [`Decoder::finish`] rejects trailing bytes. Large binary
//! payloads should travel directly as `Bytes`, without this framing.

use crate::Error;

/// Appends fields into a reusable output buffer.
#[derive(Debug, Default)]
pub struct Encoder {
	bytes: Vec<u8>,
}

impl Encoder {
	/// Reserves the known encoded size.
	pub fn with_capacity(capacity: usize) -> Self {
		Self {
			bytes: Vec::with_capacity(capacity),
		}
	}

	/// Appends a little-endian integer.
	pub fn u32(&mut self, value: u32) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	/// Appends a length-prefixed byte slice.
	pub fn bytes(&mut self, value: &[u8]) -> Result<(), Error> {
		self.u32(u32::try_from(value.len()).map_err(|_| Error::TooLarge)?);
		self.bytes.extend_from_slice(value);
		Ok(())
	}

	/// Transfers ownership of the output allocation.
	pub fn finish(self) -> crate::Bytes {
		self.bytes.into()
	}
}

/// Allocation-free decoder over a borrowed message.
#[derive(Debug)]
pub struct Decoder<'a> {
	remaining: &'a [u8],
}

impl<'a> Decoder<'a> {
	/// Reads fields from `input`.
	pub fn new(input: &'a [u8]) -> Self {
		Self { remaining: input }
	}

	/// Reads a little-endian integer.
	pub fn u32(&mut self) -> Result<u32, Error> {
		let bytes = self.take(4)?;
		Ok(u32::from_le_bytes(
			bytes.try_into().map_err(|_| Error::InvalidArgument)?,
		))
	}

	/// Borrows a length-prefixed byte slice.
	pub fn bytes(&mut self) -> Result<&'a [u8], Error> {
		let len = self.u32()? as usize;
		self.take(len)
	}

	/// Borrows a UTF-8 string.
	pub fn string(&mut self) -> Result<&'a str, Error> {
		std::str::from_utf8(self.bytes()?).map_err(|_| Error::InvalidArgument)
	}

	/// Requires that every byte was consumed.
	pub fn finish(self) -> Result<(), Error> {
		if self.remaining.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}

	fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
		let bytes = self.remaining.get(..len).ok_or(Error::InvalidArgument)?;
		self.remaining = &self.remaining[len..];
		Ok(bytes)
	}
}
