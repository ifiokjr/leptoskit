//! App icons from one source image, `app.icon`: every size each platform
//! asks for, so a store never refuses the app for a missing one.
//!
//! - iOS: an `AppIcon` set with a single opaque 1024×1024 image, which Xcode
//!   turns into every size the devices use.
//! - Android: launcher icons for each density, and an adaptive icon whose
//!   foreground keeps the image inside the mask's safe zone.
//! - macOS: an `.icns` with the 16 to 1024 pixel images.
//! - The browser extension: 16, 32, 48, and 128 pixels.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use image::RgbaImage;
use image::imageops::FilterType;

/// The smallest source image every platform's sizes can be made from.
pub const MINIMUM: u32 = 1024;

/// The source image, square and at least [`MINIMUM`] pixels wide.
pub struct Icon {
	image: RgbaImage,
}

impl Icon {
	/// Reads a PNG, or draws an SVG at [`MINIMUM`] pixels.
	///
	/// # Errors
	///
	/// When the file can't be read, isn't a PNG or SVG, isn't square, or is
	/// too small.
	pub fn load(path: &Path) -> Result<Self> {
		let bytes =
			std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
		let svg = path
			.extension()
			.is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
		let image = if svg {
			draw_svg(&bytes).with_context(|| format!("could not draw {}", path.display()))?
		} else {
			image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
				.with_context(|| format!("{} isn't a PNG or an SVG", path.display()))?
				.to_rgba8()
		};

		anyhow::ensure!(
			image.width() == image.height(),
			"app.icon {} is {}×{}; it must be square",
			path.display(),
			image.width(),
			image.height()
		);

		anyhow::ensure!(
			image.width() >= MINIMUM,
			"app.icon {} is {}×{}; the App Store needs {MINIMUM}×{MINIMUM}, so make it at least \
			 that, or an SVG",
			path.display(),
			image.width(),
			image.height()
		);
		Ok(Self { image })
	}

	/// The icon at `size` pixels, as a PNG.
	///
	/// # Errors
	///
	/// When encoding fails.
	pub fn png(&self, size: u32) -> Result<Vec<u8>> {
		encode(&image::imageops::resize(
			&self.image,
			size,
			size,
			FilterType::Lanczos3,
		))
	}

	/// The icon at `size` pixels over white, as a PNG without transparency,
	/// which the App Store requires.
	///
	/// # Errors
	///
	/// When encoding fails.
	pub fn opaque_png(&self, size: u32) -> Result<Vec<u8>> {
		let mut resized = image::imageops::resize(&self.image, size, size, FilterType::Lanczos3);

		for pixel in resized.pixels_mut() {
			let alpha = u16::from(pixel[3]);

			for channel in 0..3 {
				let blended = (u16::from(pixel[channel]) * alpha + 255 * (255 - alpha)) / 255;
				pixel[channel] = u8::try_from(blended).unwrap_or(u8::MAX);
			}

			pixel[3] = u8::MAX;
		}

		encode(&image::DynamicImage::ImageRgba8(resized).to_rgb8())
	}

	/// The icon shrunk to `inner` pixels, centered on a transparent square
	/// of `size`: an adaptive icon's foreground, inside its safe zone.
	///
	/// # Errors
	///
	/// When encoding fails.
	pub fn padded_png(&self, size: u32, inner: u32) -> Result<Vec<u8>> {
		let resized = image::imageops::resize(&self.image, inner, inner, FilterType::Lanczos3);
		let mut canvas = RgbaImage::new(size, size);
		let offset = i64::from((size - inner) / 2);
		image::imageops::overlay(&mut canvas, &resized, offset, offset);
		encode(&canvas)
	}

	/// Every platform's files, by path relative to the generated directory.
	///
	/// # Errors
	///
	/// When encoding fails.
	pub fn files(&self, platform: Platform) -> Result<Vec<(String, Vec<u8>)>> {
		match platform {
			Platform::Ios => {
				Ok(vec![
					(
						"Assets.xcassets/Contents.json".to_owned(),
						CATALOG.as_bytes().to_vec(),
					),
					(
						"Assets.xcassets/AppIcon.appiconset/Contents.json".to_owned(),
						APP_ICON_SET.as_bytes().to_vec(),
					),
					(
						"Assets.xcassets/AppIcon.appiconset/icon-1024.png".to_owned(),
						self.opaque_png(1024)?,
					),
				])
			}
			Platform::Android => {
				let mut files = vec![
					(
						"mipmap-anydpi-v26/ic_launcher.xml".to_owned(),
						ADAPTIVE.as_bytes().to_vec(),
					),
					(
						"values/ic_launcher_background.xml".to_owned(),
						background(&self.background()).into_bytes(),
					),
				];
				for (density, scale) in ANDROID_DENSITIES {
					let launcher = 48 * scale / 4;
					files.push((
						format!("mipmap-{density}/ic_launcher.png"),
						self.png(launcher)?,
					));
					// 108dp, with the image in the 72dp safe zone's middle.
					files.push((
						format!("mipmap-{density}/ic_launcher_foreground.png"),
						self.padded_png(108 * scale / 4, 66 * scale / 4)?,
					));
				}

				Ok(files)
			}
			Platform::Macos => Ok(vec![("AppIcon.icns".to_owned(), self.icns()?)]),
			Platform::Extension => {
				crate::extension::ICON_SIZES
					.into_iter()
					.map(|size| Ok((format!("icon-{size}.png"), self.png(size)?)))
					.collect()
			}
		}
	}

	/// The adaptive icon's background: the image's corner color when it is
	/// opaque there, so a full-bleed icon has no seam, and white otherwise.
	fn background(&self) -> String {
		let corner = self.image.get_pixel(0, 0);

		if corner[3] == u8::MAX {
			format!("#{:02X}{:02X}{:02X}", corner[0], corner[1], corner[2])
		} else {
			"#FFFFFF".to_owned()
		}
	}

	/// The macOS `.icns`: the PNG images it holds, by their four-letter
	/// types.
	fn icns(&self) -> Result<Vec<u8>> {
		let mut body = Vec::new();

		for (kind, size) in ICNS {
			let png = self.png(size)?;
			body.extend_from_slice(kind.as_bytes());
			body.extend_from_slice(&u32::try_from(png.len() + 8)?.to_be_bytes());
			body.extend_from_slice(&png);
		}

		let mut icns = b"icns".to_vec();
		icns.extend_from_slice(&u32::try_from(body.len() + 8)?.to_be_bytes());

		icns.extend_from_slice(&body);
		Ok(icns)
	}
}

/// The platforms icons are made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
	Ios,
	Android,
	Macos,
	Extension,
}

/// Android's launcher densities, and their scale in quarters of `mdpi`.
const ANDROID_DENSITIES: [(&str, u32); 5] = [
	("mdpi", 4),
	("hdpi", 6),
	("xhdpi", 8),
	("xxhdpi", 12),
	("xxxhdpi", 16),
];

/// The `.icns` image types: 16 to 512 points, at one and two pixels each.
const ICNS: [(&str, u32); 10] = [
	("icp4", 16),
	("ic11", 32),
	("icp5", 32),
	("ic12", 64),
	("ic07", 128),
	("ic13", 256),
	("ic08", 256),
	("ic14", 512),
	("ic09", 512),
	("ic10", 1024),
];

const CATALOG: &str =
	"{\n  \"info\" : {\n    \"author\" : \"leptoskit\",\n    \"version\" : 1\n  }\n}\n";

const APP_ICON_SET: &str = r#"{
  "images" : [
    {
      "filename" : "icon-1024.png",
      "idiom" : "universal",
      "platform" : "ios",
      "size" : "1024x1024"
    }
  ],
  "info" : {
    "author" : "leptoskit",
    "version" : 1
  }
}
"#;

const ADAPTIVE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generated by leptoskit from app.icon. -->
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@color/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
</adaptive-icon>
"#;

/// The adaptive icon's background color resource.
fn background(color: &str) -> String {
	format!(
		"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- Generated by leptoskit: the adaptive \
		 icon's background. -->\n<resources>\n    <color \
		 name=\"ic_launcher_background\">{color}</color>\n</resources>\n"
	)
}

fn encode<P>(image: &image::ImageBuffer<P, Vec<u8>>) -> Result<Vec<u8>>
where
	P: image::Pixel<Subpixel = u8> + image::PixelWithColorType,
{
	let mut png = Vec::new();
	image
		.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
		.context("could not encode an icon")?;
	Ok(png)
}

/// Draws an SVG at [`MINIMUM`] pixels square.
fn draw_svg(bytes: &[u8]) -> Result<RgbaImage> {
	let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default())?;
	let mut pixmap = resvg::tiny_skia::Pixmap::new(MINIMUM, MINIMUM)
		.context("could not make a canvas for the icon")?;
	let size = tree.size();
	#[allow(
		clippy::cast_precision_loss,
		reason = "the canvas is 1024 pixels, which f32 holds exactly"
	)]
	let scale = MINIMUM as f32 / size.width().max(size.height());
	resvg::render(
		&tree,
		resvg::tiny_skia::Transform::from_scale(scale, scale),
		&mut pixmap.as_mut(),
	);
	// tiny-skia keeps premultiplied pixels; images hold straight ones.
	let mut image = RgbaImage::new(MINIMUM, MINIMUM);

	for (pixel, source) in image.pixels_mut().zip(pixmap.pixels()) {
		let color = source.demultiply();
		*pixel = image::Rgba([color.red(), color.green(), color.blue(), color.alpha()]);
	}

	Ok(image)
}

/// Writes `platform`'s icons into `dir` from `app.icon`, and returns
/// whether there were any: `false` when the app has no icon.
///
/// # Errors
///
/// When the icon can't be read, or the files can't be written.
pub fn write(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	platform: Platform,
	dir: &Path,
) -> Result<bool> {
	let Some(source) = &config.app.icon else {
		return Ok(false);
	};
	let icon = Icon::load(&root.join(source))?;

	for (name, bytes) in icon.files(platform)? {
		let path = dir.join(name);

		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}

		std::fs::write(&path, bytes)
			.with_context(|| format!("could not write {}", path.display()))?;
	}

	Ok(true)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A 1024-pixel test icon: a filled circle, transparent outside.
	fn circle() -> Icon {
		let mut image = RgbaImage::new(MINIMUM, MINIMUM);

		for (x, y, pixel) in image.enumerate_pixels_mut() {
			let (dx, dy) = (f64::from(x) - 512.0, f64::from(y) - 512.0);

			if dx.hypot(dy) < 400.0 {
				*pixel = image::Rgba([200, 40, 40, 255]);
			}
		}

		Icon { image }
	}

	fn size(png: &[u8]) -> (u32, u32, image::ColorType) {
		let image = image::load_from_memory(png).unwrap();
		(image.width(), image.height(), image.color())
	}

	#[test]
	fn every_platform_gets_its_sizes() {
		let icon = circle();
		let ios = icon.files(Platform::Ios).unwrap();
		let (width, _, color) = size(&ios[2].1);
		// The App Store refuses an icon with transparency.
		assert_eq!((width, color), (1024, image::ColorType::Rgb8));
		let set: serde_json::Value = serde_json::from_slice(&ios[1].1).unwrap();
		assert_eq!(set["images"][0]["filename"], "icon-1024.png");

		let android = icon.files(Platform::Android).unwrap();
		let names: Vec<(&str, u32)> = android
			.iter()
			.filter(|(name, _)| {
				Path::new(name)
					.extension()
					.is_some_and(|extension| extension == "png")
			})
			.map(|(name, png)| (name.as_str(), size(png).0))
			.collect();
		assert_eq!(
			names,
			[
				("mipmap-mdpi/ic_launcher.png", 48),
				("mipmap-mdpi/ic_launcher_foreground.png", 108),
				("mipmap-hdpi/ic_launcher.png", 72),
				("mipmap-hdpi/ic_launcher_foreground.png", 162),
				("mipmap-xhdpi/ic_launcher.png", 96),
				("mipmap-xhdpi/ic_launcher_foreground.png", 216),
				("mipmap-xxhdpi/ic_launcher.png", 144),
				("mipmap-xxhdpi/ic_launcher_foreground.png", 324),
				("mipmap-xxxhdpi/ic_launcher.png", 192),
				("mipmap-xxxhdpi/ic_launcher_foreground.png", 432),
			]
		);
		// A transparent corner leaves the background white.
		assert!(String::from_utf8_lossy(&android[1].1).contains("#FFFFFF"));
		// The foreground's corners stay clear of the mask.
		let foreground = image::load_from_memory(&android[3].1).unwrap().to_rgba8();
		assert_eq!(foreground.get_pixel(0, 0)[3], 0);

		let extension = icon.files(Platform::Extension).unwrap();
		let sizes: Vec<u32> = extension.iter().map(|(_, png)| size(png).0).collect();
		assert_eq!(sizes, [16, 32, 48, 128]);
	}

	#[test]
	fn the_icns_holds_every_size() {
		let icns = circle().icns().unwrap();
		assert_eq!(&icns[..4], b"icns");
		assert_eq!(
			u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize,
			icns.len()
		);
		let mut offset = 8;
		let mut kinds = Vec::new();

		while offset < icns.len() {
			let kind = std::str::from_utf8(&icns[offset..offset + 4]).unwrap();
			let length = u32::from_be_bytes(icns[offset + 4..offset + 8].try_into().unwrap());

			let png = &icns[offset + 8..offset + length as usize];
			kinds.push((kind.to_owned(), size(png).0));
			offset += length as usize;
		}

		assert_eq!(kinds.len(), 10);
		assert_eq!(kinds.last().unwrap(), &("ic10".to_owned(), 1024));
	}

	#[test]
	fn svgs_are_drawn_and_small_or_oblong_images_refused() {
		let dir = tempfile::tempdir().unwrap();
		let svg = dir.path().join("icon.svg");
		std::fs::write(
			&svg,
			r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="#2a6"/></svg>"##,
		)
		.unwrap();
		let icon = Icon::load(&svg).unwrap();
		assert_eq!(icon.image.width(), MINIMUM);
		assert_eq!(icon.image.get_pixel(512, 512).0, [0x22, 0xaa, 0x66, 255]);

		let small = dir.path().join("small.png");
		RgbaImage::new(512, 512).save(&small).unwrap();
		let error = Icon::load(&small).err().unwrap().to_string();
		assert!(error.contains("at least that"), "{error}");
		let oblong = dir.path().join("oblong.png");
		RgbaImage::new(1024, 1000).save(&oblong).unwrap();
		let error = Icon::load(&oblong).err().unwrap().to_string();
		assert!(error.contains("must be square"), "{error}");
	}
}
