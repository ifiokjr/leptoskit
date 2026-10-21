//! Host-side checks for the gallery's screen captures.
//!
//! This crate deliberately depends on no widget backend, so the mobile
//! verifier builds on any host (an Android CI runner has no GTK).

use std::collections::HashSet;
use std::io::Cursor;

/// Every gallery screen, in `gallery::SCREENS` order. The gallery's tests
/// fail if the two lists drift apart.
pub const SCREENS: [&str; 7] = [
	"counter",
	"profile",
	"settings",
	"photos",
	"chat",
	"dashboard",
	"components",
];

/// The fewest distinct colors a rendered gallery screen shows. Anti-aliased
/// text and the generated artwork produce thousands; a blank window, a
/// single fill, or a black capture produces a handful.
pub const MIN_COLORS: usize = 64;

/// Checks that `png` decodes and shows at least [`MIN_COLORS`] colors.
///
/// # Errors
/// Returns a message when `png` is not a PNG or looks blank.
pub fn check_rendered(png: &[u8]) -> Result<(), String> {
	let mut decoder = png::Decoder::new(Cursor::new(png));
	decoder.set_transformations(png::Transformations::normalize_to_color8());
	let mut reader = decoder
		.read_info()
		.map_err(|error| format!("not a PNG: {error}"))?;
	let mut pixels = vec![
		0;
		reader
			.output_buffer_size()
			.ok_or("the PNG is too large to decode")?
	];
	let frame = reader
		.next_frame(&mut pixels)
		.map_err(|error| format!("the PNG does not decode: {error}"))?;
	let colors: HashSet<&[u8]> = pixels[..frame.buffer_size()]
		.chunks_exact(frame.color_type.samples())
		.collect();
	if colors.len() < MIN_COLORS {
		return Err(format!(
			"the {}x{} capture shows only {} colors, so it looks blank",
			frame.width,
			frame.height,
			colors.len()
		));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
		let mut bytes = Vec::new();
		let mut encoder = png::Encoder::new(&mut bytes, width, height);
		encoder.set_color(png::ColorType::Rgba);
		encoder.set_depth(png::BitDepth::Eight);
		let mut writer = encoder.write_header().unwrap();
		writer.write_image_data(pixels).unwrap();
		writer.finish().unwrap();
		bytes
	}

	#[test]
	fn blank_captures_are_not_rendered_screens() {
		let blank = png(390, 844, &vec![255; 390 * 844 * 4]);
		let error = check_rendered(&blank).unwrap_err();
		assert!(error.contains("shows only 1 colors"), "{error}");
	}

	#[test]
	fn varied_captures_are_rendered_screens() {
		let pixels: Vec<u8> = (0..32 * 32_u32)
			.flat_map(|index| [(index % 256) as u8, (index / 4) as u8, 0, 255])
			.collect();
		assert_eq!(check_rendered(&png(32, 32, &pixels)), Ok(()));
	}

	#[test]
	fn other_bytes_are_not_pngs() {
		assert!(
			check_rendered(b"not a png")
				.unwrap_err()
				.starts_with("not a PNG")
		);
	}
}
