//! Tells a screenshot of a rendered screen from a blank or missing capture.
//!
//! Shared by `tests/cli.rs` and `examples/verify_mobile.rs`.

use std::collections::HashSet;
use std::io::Cursor;

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
