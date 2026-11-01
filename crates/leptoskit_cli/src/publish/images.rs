//! The stores' images: the screenshots `leptoskit publish screenshots`
//! saves in `store/screenshots/<folder>/`, checked against what each store
//! takes before anything is built or uploaded.
//!
//! A folder's name says which store size it is, such as `iphone-6.9` or
//! `android-phone`; [`TARGETS`] lists them. A folder leptoskit doesn't know
//! is refused, so a typo doesn't leave a size silently out of the listing.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::screenshots::DIR;

/// What sizes a store takes for one folder.
#[derive(Debug, Clone, Copy)]
pub enum Rule {
	/// Exactly one of these sizes, `(width, height)`, and, when `rotated`,
	/// each turned on its side too.
	Sizes {
		sizes: &'static [(u32, u32)],
		rotated: bool,
	},
	/// Google Play's screenshots: each side 320 to 3840 pixels, the long one
	/// at most twice the short one.
	PlayScreenshot,
}

/// One store's use of one folder.
#[derive(Debug)]
pub struct Target {
	pub store: &'static str,
	pub folder: &'static str,
	/// What the store's API calls it: App Store Connect's
	/// `screenshotDisplayType`, Google Play's image type, or `previews` on
	/// addons.mozilla.org. The browser stores' are uploaded by hand.
	pub kind: &'static str,
	pub rule: Rule,
	/// The most images the store takes for it.
	pub max: Option<usize>,
	/// The largest file the store takes, in bytes.
	pub max_bytes: Option<usize>,
	/// The store refuses transparency: PNGs with an alpha channel are sent
	/// without it, and refused when a pixel isn't opaque.
	pub opaque: bool,
}

const fn apple(folder: &'static str, kind: &'static str, sizes: &'static [(u32, u32)]) -> Target {
	Target {
		store: "app_store",
		folder,
		kind,
		rule: Rule::Sizes {
			sizes,
			rotated: true,
		},
		max: Some(10),
		max_bytes: None,
		opaque: true,
	}
}

const fn play(folder: &'static str, kind: &'static str) -> Target {
	Target {
		store: "play",
		folder,
		kind,
		rule: Rule::PlayScreenshot,
		max: Some(8),
		max_bytes: Some(8 * 1024 * 1024),
		opaque: true,
	}
}

const fn browser(
	store: &'static str,
	folder: &'static str,
	size: &'static [(u32, u32)],
	max: Option<usize>,
) -> Target {
	Target {
		store,
		folder,
		kind: "previews",
		rule: Rule::Sizes {
			sizes: size,
			rotated: false,
		},
		max,
		max_bytes: None,
		opaque: false,
	}
}

const CHROME_LARGE: &[(u32, u32)] = &[(1280, 800)];
const CHROME_SMALL: &[(u32, u32)] = &[(640, 400)];

/// Every folder leptoskit knows, by store. App Store Connect's display types
/// and sizes are from its screenshot specifications: the API names the
/// 6.9-inch iPhone `APP_IPHONE_67`, the 6.3-inch `APP_IPHONE_61`, and the
/// 6.1-inch `APP_IPHONE_58`. The App Store scales the largest size down for
/// the older ones, so those aren't here.
pub const TARGETS: &[Target] = &[
	apple(
		"iphone-6.9",
		"APP_IPHONE_67",
		&[(1320, 2868), (1290, 2796), (1260, 2736)],
	),
	apple("iphone-6.5", "APP_IPHONE_65", &[(1284, 2778), (1242, 2688)]),
	apple("iphone-6.3", "APP_IPHONE_61", &[(1206, 2622), (1179, 2556)]),
	apple(
		"iphone-6.1",
		"APP_IPHONE_58",
		&[(1170, 2532), (1125, 2436), (1080, 2340)],
	),
	apple("iphone-5.5", "APP_IPHONE_55", &[(1242, 2208)]),
	apple("iphone-4.7", "APP_IPHONE_47", &[(750, 1334)]),
	apple(
		"ipad-13",
		"APP_IPAD_PRO_3GEN_129",
		&[(2064, 2752), (2048, 2732)],
	),
	apple("ipad-12.9", "APP_IPAD_PRO_129", &[(2048, 2732)]),
	apple(
		"ipad-11",
		"APP_IPAD_PRO_3GEN_11",
		&[(1488, 2266), (1668, 2420), (1668, 2388), (1640, 2360)],
	),
	apple("ipad-10.5", "APP_IPAD_105", &[(1668, 2224)]),
	play("android-phone", "phoneScreenshots"),
	play("android-tablet-7", "sevenInchScreenshots"),
	play("android-tablet-10", "tenInchScreenshots"),
	Target {
		store: "play",
		folder: "play-feature-graphic",
		kind: "featureGraphic",
		rule: Rule::Sizes {
			sizes: &[(1024, 500)],
			rotated: false,
		},
		max: Some(1),
		max_bytes: None,
		opaque: true,
	},
	browser("chrome", "chrome-1280x800", CHROME_LARGE, Some(5)),
	browser("chrome", "chrome-640x400", CHROME_SMALL, Some(5)),
	browser("edge", "chrome-1280x800", CHROME_LARGE, Some(10)),
	// addons.mozilla.org takes any size; it uses the first of these that has
	// screenshots.
	browser("firefox", "chrome-1280x800", CHROME_LARGE, None),
	browser("firefox", "chrome-640x400", CHROME_SMALL, None),
];

/// One image, ready to send.
#[derive(Debug, Clone)]
pub struct Image {
	/// The file's name, such as `01-projects.png`.
	pub name: String,
	pub bytes: Vec<u8>,
	pub width: u32,
	pub height: u32,
	/// `image/png` or `image/jpeg`.
	pub content_type: &'static str,
}

/// One folder's images, for one store.
#[derive(Debug)]
pub struct Set {
	pub target: &'static Target,
	pub images: Vec<Image>,
}

/// Every image `store` uploads, checked, by folder in [`TARGETS`]'s order.
/// A folder that is missing or has no images is left out.
///
/// # Errors
///
/// When `store/screenshots/` has a folder leptoskit doesn't know, or an
/// image the store would refuse: the wrong size, too many, too large, or
/// transparent. Every problem is named at once.
pub fn load(root: &Path, store: &str) -> Result<Vec<Set>> {
	let dir = root.join(DIR);
	let mut problems = Vec::new();

	for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
		let name = entry.file_name().to_string_lossy().into_owned();

		if entry.path().is_dir() && !TARGETS.iter().any(|target| target.folder == name) {
			problems.push(format!(
				"{DIR}/{name} isn't a store size leptoskit knows; name it one of {}",
				known()
			));
		}
	}

	let mut sets = Vec::new();

	for target in TARGETS.iter().filter(|target| target.store == store) {
		let folder = dir.join(target.folder);
		let mut files: Vec<_> = std::fs::read_dir(&folder)
			.into_iter()
			.flatten()
			.flatten()
			.map(|entry| entry.path())
			.filter(|path| content_type(path).is_some())
			.collect();
		files.sort();

		if files.is_empty() {
			continue;
		}

		if let Some(max) = target.max
			&& files.len() > max
		{
			problems.push(format!(
				"{DIR}/{} has {} images; {} takes at most {max}",
				target.folder,
				files.len(),
				store_name(store)
			));
		}
		let mut images = Vec::new();

		for path in files {
			match image(&path, target) {
				Ok(image) => images.push(image),
				Err(problem) => {
					problems.push(format!(
						"{DIR}/{}/{}: {problem:#}",
						target.folder,
						path.file_name().unwrap_or_default().to_string_lossy()
					));
				}
			}
		}

		sets.push(Set { target, images });
	}

	anyhow::ensure!(
		problems.is_empty(),
		"{} can't take these screenshots; nothing was uploaded:\n- {}",
		store_name(store),
		problems.join("\n- ")
	);
	Ok(sets)
}

/// Every folder name leptoskit knows.
fn known() -> String {
	let mut names: Vec<&str> = TARGETS.iter().map(|target| target.folder).collect();
	names.dedup();
	names.join(", ")
}

fn content_type(path: &Path) -> Option<&'static str> {
	let extension = path.extension()?.to_str()?.to_ascii_lowercase();

	match extension.as_str() {
		"png" => Some("image/png"),
		"jpg" | "jpeg" => Some("image/jpeg"),
		_ => None,
	}
}

/// Reads and checks one image against `target`'s rules.
fn image(path: &Path, target: &Target) -> Result<Image> {
	let bytes =
		std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
	let content_type = content_type(path).expect("filtered by extension");
	let format = if content_type == "image/png" {
		image::ImageFormat::Png
	} else {
		image::ImageFormat::Jpeg
	};

	let decoded = image::load_from_memory_with_format(&bytes, format)
		.with_context(|| format!("it isn't a readable {}", &content_type[6..]))?;

	let (width, height) = (decoded.width(), decoded.height());

	match target.rule {
		Rule::Sizes { sizes, rotated } => {
			let fits = sizes
				.iter()
				.any(|&(w, h)| (width, height) == (w, h) || (rotated && (width, height) == (h, w)));
			let mut accepted: Vec<String> = sizes.iter().map(|(w, h)| format!("{w}×{h}")).collect();
			if accepted.len() > 1 {
				let last = accepted.pop().unwrap_or_default();
				accepted = vec![format!("{} or {last}", accepted.join(", "))];
			}

			anyhow::ensure!(
				fits,
				"it is {width}×{height}; {} takes {} for {}{}",
				store_name(target.store),
				accepted.concat(),
				target.folder,
				if rotated {
					", upright or on its side"
				} else {
					""
				}
			);
		}

		Rule::PlayScreenshot => {
			let (short, long) = (width.min(height), width.max(height));
			anyhow::ensure!(
				short >= 320 && long <= 3840 && long <= 2 * short,
				"it is {width}×{height}; Google Play takes screenshots 320 to 3840 pixels on each \
				 side, the long side at most twice the short one"
			);
		}
	}

	let flatten = target.opaque && decoded.color().has_alpha();
	let bytes = if flatten {
		let rgba = decoded.to_rgba8();
		anyhow::ensure!(
			rgba.pixels().all(|pixel| pixel[3] == u8::MAX),
			"it has transparent pixels; {} takes only opaque images",
			store_name(target.store)
		);
		// Opaque, but saved with an alpha channel, which the store refuses:
		// the same pixels without it.
		let mut png = Vec::new();
		image::DynamicImage::ImageRgba8(rgba)
			.to_rgb8()
			.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
			.context("could not write it without its alpha channel")?;
		png
	} else {
		bytes
	};

	if let Some(max) = target.max_bytes {
		anyhow::ensure!(
			bytes.len() <= max,
			"it is {} MB; {} takes at most {} MB",
			bytes.len().div_ceil(1024 * 1024),
			store_name(target.store),
			max / 1024 / 1024
		);
	}

	Ok(Image {
		name: path
			.file_name()
			.unwrap_or_default()
			.to_string_lossy()
			.into_owned(),
		bytes,
		width,
		height,
		content_type: if flatten { "image/png" } else { content_type },
	})
}

fn store_name(store: &str) -> &'static str {
	match store {
		"app_store" => "the App Store",
		"play" => "Google Play",
		"firefox" => "addons.mozilla.org",
		"chrome" => "the Chrome Web Store",
		"edge" => "Edge Add-ons",
		_ => "the store",
	}
}

/// How many images a folder holds, for the steps of stores whose images are
/// uploaded by hand.
#[must_use]
pub fn count(root: &Path, folder: &str) -> usize {
	std::fs::read_dir(root.join(DIR).join(folder))
		.into_iter()
		.flatten()
		.flatten()
		.filter(|entry| content_type(&entry.path()).is_some())
		.count()
}

/// A summary of what will go up, such as `3 screenshots for iphone-6.9`.
#[must_use]
pub fn describe(sets: &[Set]) -> String {
	sets.iter()
		.map(|set| {
			format!(
				"{} {} for {}",
				set.images.len(),
				if set.images.len() == 1 {
					"image"
				} else {
					"images"
				},
				set.target.folder
			)
		})
		.collect::<Vec<_>>()
		.join(", ")
}

#[cfg(test)]
pub mod tests {
	use super::*;

	/// A PNG of `width`×`height`, opaque unless `alpha` is below 255.
	pub fn png(width: u32, height: u32, alpha: u8) -> Vec<u8> {
		let mut bytes = Vec::new();
		image::RgbaImage::from_pixel(width, height, image::Rgba([20, 120, 220, alpha]))
			.write_to(
				&mut std::io::Cursor::new(&mut bytes),
				image::ImageFormat::Png,
			)
			.unwrap();
		bytes
	}

	/// Writes `files` under `store/screenshots/` in a new project.
	pub fn project(files: &[(impl AsRef<str>, Vec<u8>)]) -> tempfile::TempDir {
		let root = tempfile::tempdir().unwrap();

		for (path, bytes) in files {
			let path = root.path().join(DIR).join(path.as_ref());
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, bytes).unwrap();
		}

		root
	}

	#[test]
	fn every_apple_folder_has_a_display_type_the_api_names() {
		// From App Store Connect's ScreenshotDisplayType.
		let types = [
			"APP_IPHONE_67",
			"APP_IPHONE_65",
			"APP_IPHONE_61",
			"APP_IPHONE_58",
			"APP_IPHONE_55",
			"APP_IPHONE_47",
			"APP_IPAD_PRO_3GEN_129",
			"APP_IPAD_PRO_129",
			"APP_IPAD_PRO_3GEN_11",
			"APP_IPAD_105",
		];

		for target in TARGETS.iter().filter(|target| target.store == "app_store") {
			assert!(types.contains(&target.kind), "{}", target.kind);
		}

		// Each folder is one store size for each store.
		for target in TARGETS {
			assert_eq!(
				TARGETS
					.iter()
					.filter(|other| other.store == target.store && other.folder == target.folder)
					.count(),
				1
			);
		}
	}

	#[test]
	fn images_are_checked_in_name_order_and_sent_without_alpha() {
		let root = project(&[
			("iphone-6.9/02-reading.png", png(2868, 1320, 255)),
			("iphone-6.9/01-projects.png", png(1320, 2868, 255)),
			("iphone-6.9/notes.txt", b"not an image".to_vec()),
			("android-phone/01-projects.png", png(1080, 1920, 255)),
		]);
		let sets = load(root.path(), "app_store").unwrap();
		assert_eq!(sets.len(), 1);
		assert_eq!(sets[0].target.kind, "APP_IPHONE_67");
		let names: Vec<&str> = sets[0]
			.images
			.iter()
			.map(|image| image.name.as_str())
			.collect();
		assert_eq!(names, ["01-projects.png", "02-reading.png"]);
		let sent = image::load_from_memory(&sets[0].images[0].bytes).unwrap();
		assert!(!sent.color().has_alpha());

		assert_eq!(describe(&sets), "2 images for iphone-6.9");
		// Google Play takes the phone screenshots; nothing else is its.
		let sets = load(root.path(), "play").unwrap();
		assert_eq!(sets.len(), 1);
		assert_eq!(sets[0].target.kind, "phoneScreenshots");
		assert!(load(root.path(), "firefox").unwrap().is_empty());
		assert_eq!(count(root.path(), "iphone-6.9"), 2);
	}

	#[test]
	fn wrong_sizes_too_many_transparency_and_unknown_folders_are_refused() {
		let mut files = vec![
			("iphone-6.9/01-wrong.png".to_owned(), png(1179, 2556, 255)),
			("ipad-13/01-clear.png".to_owned(), png(2064, 2752, 0)),
			("iphone-7/01-typo.png".to_owned(), png(1320, 2868, 255)),
		];
		let opaque = png(1206, 2622, 255);

		for index in 1..=11 {
			files.push((format!("iphone-6.3/{index:02}.png"), opaque.clone()));
		}

		let root = project(&files);
		let error = load(root.path(), "app_store").unwrap_err().to_string();
		assert!(
			error.contains("store/screenshots/iphone-7 isn't a store size leptoskit knows"),
			"{error}"
		);
		assert!(
			error.contains(
				"iphone-6.9/01-wrong.png: it is 1179×2556; the App Store takes 1320×2868, \
				 1290×2796 or 1260×2736 for iphone-6.9, upright or on its side"
			),
			"{error}"
		);

		assert!(
			error.contains("ipad-13/01-clear.png: it has transparent pixels"),
			"{error}"
		);
		assert!(
			error.contains(
				"store/screenshots/iphone-6.3 has 11 images; the App Store takes at most 10"
			),
			"{error}"
		);
		assert!(error.contains("nothing was uploaded"), "{error}");

		let root = project(&[
			("android-phone/01.png", png(300, 1000, 255)),
			("play-feature-graphic/01.png", png(1024, 512, 255)),
		]);
		let error = load(root.path(), "play").unwrap_err().to_string();
		assert!(
			error.contains("android-phone/01.png: it is 300×1000; Google Play takes"),
			"{error}"
		);
		assert!(
			error.contains("play-feature-graphic/01.png: it is 1024×512"),
			"{error}"
		);
	}
}
