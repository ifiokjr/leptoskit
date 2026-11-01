use super::*;

/// The PNG signature and the fixed header that follows it, through the width
/// and height fields.
const PNG_HEADER_BYTES: usize = 33;

/// The first byte of every JPEG marker.
const MARKER_PREFIX: u8 = 0xff;

/// The marker that introduces entropy-coded scan data.
const START_OF_SCAN: u8 = 0xda;

/// The marker that ends a JPEG stream.
const END_OF_IMAGE: u8 = 0xd9;

/// A frame header carries its precision, height, and width in six bytes.
const MIN_FRAME_BYTES: usize = 6;

/// Only eight-bit samples are supported.
const SAMPLE_PRECISION: u8 = 8;

/// Reads and bounds dimensions before constructing any pixel decoder. Does not
/// imply that compressed pixels, checksums or the rest of the file are valid.
pub fn inspect(input: &[u8]) -> Result<ImageInfo, Error> {
	if input.len() > MAX_INPUT_BYTES {
		return Err(Error::TooLarge);
	}
	let info = if input.starts_with(b"\x89PNG\r\n\x1a\n") {
		if input.len() < PNG_HEADER_BYTES
			|| input[8..12] != 13_u32.to_be_bytes()
			|| &input[12..16] != b"IHDR"
		{
			return Err(Error::InvalidArgument);
		}

		ImageInfo {
			format: Format::Png,
			width: u32::from_be_bytes(
				input[16..20]
					.try_into()
					.map_err(|_| Error::InvalidArgument)?,
			),
			height: u32::from_be_bytes(
				input[20..24]
					.try_into()
					.map_err(|_| Error::InvalidArgument)?,
			),
		}
	} else if input.starts_with(&[0xff, 0xd8]) {
		jpeg_info(input)?
	} else {
		return Err(Error::Unavailable);
	};

	pixels(info.width, info.height)?;
	Ok(info)
}

/// Walks the marker segments up to the first frame header, without decoding
/// pixels or trusting anything past it.
fn jpeg_info(input: &[u8]) -> Result<ImageInfo, Error> {
	let mut offset = 2_usize;

	while offset < input.len() {
		let marker = read_marker(input, &mut offset)?;

		if marker == 0 || matches!(marker, START_OF_SCAN | END_OF_IMAGE) {
			return Err(Error::InvalidArgument);
		}

		if matches!(marker, 0x01 | 0xd0..=0xd8) {
			continue;
		}

		let length = segment_length(input, offset)?;
		let segment = input
			.get(offset + 2..offset + length)
			.ok_or(Error::InvalidArgument)?;

		if is_frame(marker) {
			return frame_info(marker, segment);
		}

		offset += length;
	}

	Err(Error::InvalidArgument)
}

/// Skips the fill bytes before a marker, reads it, and advances past it.
fn read_marker(input: &[u8], offset: &mut usize) -> Result<u8, Error> {
	if input[*offset] != MARKER_PREFIX {
		return Err(Error::InvalidArgument);
	}

	while input.get(*offset) == Some(&MARKER_PREFIX) {
		*offset += 1;
	}

	let marker = *input.get(*offset).ok_or(Error::InvalidArgument)?;
	*offset += 1;

	Ok(marker)
}

/// The declared length of the segment whose length field sits at `offset`.
fn segment_length(input: &[u8], offset: usize) -> Result<usize, Error> {
	let length = u16::from_be_bytes(
		input
			.get(offset..offset + 2)
			.ok_or(Error::InvalidArgument)?
			.try_into()
			.map_err(|_| Error::InvalidArgument)?,
	) as usize;

	if length < 2 {
		return Err(Error::InvalidArgument);
	}

	Ok(length)
}

/// Frames the decoder may read: the baseline and progressive coding processes.
fn is_frame(marker: u8) -> bool {
	matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
}

/// Reads a supported frame's dimensions, refusing another coding process.
fn frame_info(marker: u8, segment: &[u8]) -> Result<ImageInfo, Error> {
	if !matches!(marker, 0xc0..=0xc2) {
		return Err(Error::Unavailable);
	}

	if segment.len() < MIN_FRAME_BYTES || segment[0] != SAMPLE_PRECISION {
		return Err(Error::Unavailable);
	}

	Ok(ImageInfo {
		format: Format::Jpeg,
		height: u32::from(u16::from_be_bytes([segment[1], segment[2]])),
		width: u32::from(u16::from_be_bytes([segment[3], segment[4]])),
	})
}
