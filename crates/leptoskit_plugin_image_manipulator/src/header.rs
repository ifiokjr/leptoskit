use super::*;

/// Reads and bounds dimensions before constructing any pixel decoder. Does not
/// imply that compressed pixels, checksums or the rest of the file are valid.
pub fn inspect(input: &[u8]) -> Result<ImageInfo, Error> {
	if input.len() > MAX_INPUT_BYTES {
		return Err(Error::TooLarge);
	}
	let info = if input.starts_with(b"\x89PNG\r\n\x1a\n") {
		if input.len() < 33 || input[8..12] != 13_u32.to_be_bytes() || &input[12..16] != b"IHDR" {
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
fn jpeg_info(input: &[u8]) -> Result<ImageInfo, Error> {
	let mut offset = 2_usize;
	while offset < input.len() {
		if input[offset] != 0xff {
			return Err(Error::InvalidArgument);
		}
		while input.get(offset) == Some(&0xff) {
			offset += 1;
		}
		let marker = *input.get(offset).ok_or(Error::InvalidArgument)?;
		offset += 1;
		if marker == 0 || matches!(marker, 0xda | 0xd9) {
			return Err(Error::InvalidArgument);
		}
		if matches!(marker, 0x01 | 0xd0..=0xd8) {
			continue;
		}
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
		let segment = input
			.get(offset + 2..offset + length)
			.ok_or(Error::InvalidArgument)?;
		if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
			if !matches!(marker, 0xc0..=0xc2) {
				return Err(Error::Unavailable);
			}
			if segment.len() < 6 || segment[0] != 8 {
				return Err(Error::Unavailable);
			}
			return Ok(ImageInfo {
				format: Format::Jpeg,
				height: u32::from(u16::from_be_bytes([segment[1], segment[2]])),
				width: u32::from(u16::from_be_bytes([segment[3], segment[4]])),
			});
		}
		offset += length;
	}
	Err(Error::InvalidArgument)
}
