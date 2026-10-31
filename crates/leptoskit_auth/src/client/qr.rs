//! QR codes, for showing an authenticator app's setup link to a phone's
//! camera. Made on the device, so the link never goes to another service,
//! and drawn as path data for a `Vector`, which every platform draws, or as
//! SVG.

use std::fmt::Write;

use qrcode::Color;
use qrcode::EcLevel;

/// The light border around a code that scanners need: four modules.
const QUIET_ZONE: usize = 4;

/// A QR code: a square of modules, each dark or light.
#[derive(Clone, PartialEq, Eq)]
pub struct QrCode {
	size: usize,
	dark: Vec<bool>,
}

impl std::fmt::Debug for QrCode {
	// The modules spell out what was encoded, such as a secret.
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("QrCode")
			.field("size", &self.size)
			.finish_non_exhaustive()
	}
}

/// `text` as a QR code with medium error correction, or `None` when it's
/// too long for one.
///
/// ```ignore
/// let code = auth::client::qr_code(&setup.uri)?;
/// let side = code.side() as f32;
/// Vector(ViewBox::new(0.0, 0.0, side, side)?)
/// 	.class("w-48 h-48 bg-white")
/// 	.path(VectorPath::new(PathData::parse(&code.path())?).fill(Color::rgba(0, 0, 0, 255)))
/// ```
pub fn qr_code(text: &str) -> Option<QrCode> {
	let code = qrcode::QrCode::with_error_correction_level(text, EcLevel::M).ok()?;
	let dark = code
		.to_colors()
		.into_iter()
		.map(|color| color == Color::Dark)
		.collect();
	Some(QrCode {
		size: code.width(),
		dark,
	})
}

impl QrCode {
	/// Modules per side, without the quiet zone.
	pub fn size(&self) -> usize {
		self.size
	}

	/// Whether the module at column `x`, row `y` is dark. Outside the code
	/// is light.
	pub fn is_dark(&self, x: usize, y: usize) -> bool {
		x < self.size && y < self.size && self.dark[y * self.size + x]
	}

	/// Modules per side with the quiet zone: the size of the view box
	/// [`QrCode::path`] draws in.
	pub fn side(&self) -> usize {
		self.size + 2 * QUIET_ZONE
	}

	/// The dark modules as SVG path data, one unit square each, offset by
	/// the quiet zone. Fill it black on white in a `side` × `side` view
	/// box, such as a `Vector`, which every platform draws.
	pub fn path(&self) -> String {
		let mut path = String::new();

		for y in 0..self.size {
			for x in 0..self.size {
				if self.is_dark(x, y) {
					let _ = write!(path, "M{} {}h1v1h-1z", x + QUIET_ZONE, y + QUIET_ZONE);
				}
			}
		}

		path
	}

	/// The code as an SVG image, `module` pixels per module, quiet zone
	/// included: black on white, one path.
	pub fn svg(&self, module: u32) -> String {
		let side = self.side();
		let pixels = u32::try_from(side)
			.unwrap_or(u32::MAX)
			.saturating_mul(module);
		format!(
			"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{pixels}\" height=\"{pixels}\" \
			 viewBox=\"0 0 {side} {side}\" shape-rendering=\"crispEdges\"><rect width=\"{side}\" \
			 height=\"{side}\" fill=\"#fff\"/><path fill=\"#000\" d=\"{}\"/></svg>",
			self.path()
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_setup_link_makes_a_code_with_finder_patterns() {
		let code =
			qr_code("otpauth://totp/Notes:ada%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=Notes")
				.unwrap();
		assert!(code.size() >= 21);
		assert_eq!((code.size() - 17) % 4, 0, "a QR version's side");
		// The top-left finder pattern: a dark 7×7 ring around a light ring.
		assert!((0..7).all(|x| code.is_dark(x, 0) && code.is_dark(x, 6)));
		assert!(!code.is_dark(1, 1));
		assert!(code.is_dark(3, 3));
		assert!(!code.is_dark(code.size(), 0));

		let side = code.side();
		assert_eq!(side, code.size() + 8);
		let path = code.path();
		assert!(
			path.starts_with("M4 4h1v1h-1z"),
			"the quiet zone offsets it"
		);
		let dark = (0..code.size())
			.flat_map(|y| (0..code.size()).map(move |x| (x, y)))
			.filter(|&(x, y)| code.is_dark(x, y))
			.count();
		assert_eq!(path.matches('M').count(), dark);

		let svg = code.svg(4);
		assert!(svg.starts_with("<svg"));
		assert!(svg.contains(&format!("width=\"{}\"", side * 4)));
		assert!(svg.contains("M4 4h1v1h-1z"));
		assert_eq!(
			format!("{code:?}"),
			format!("QrCode {{ size: {}, .. }}", code.size())
		);
	}

	#[test]
	fn text_too_long_for_a_code_has_none() {
		assert!(qr_code(&"x".repeat(4000)).is_none());
	}
}
