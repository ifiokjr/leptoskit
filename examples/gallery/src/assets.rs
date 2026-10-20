//! Procedurally generated images, so the gallery needs no network and ships
//! no binary files.
//!
//! Every image is painted pixel by pixel and written as an uncompressed
//! 24-bit BMP, a format every platform image loader reads.

use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

/// How many landscape "photos" the gallery generates.
pub const PHOTO_COUNT: usize = 15;

/// How many avatars the gallery generates.
pub const AVATAR_COUNT: usize = 4;

/// The gallery's image files: where they live and what they are called.
#[derive(Debug, Clone)]
pub struct Assets {
	dir: PathBuf,
}

impl Assets {
	/// The assets in `dir`, without generating them.
	///
	/// Element trees only carry the paths, so this is enough to build and
	/// lay out every screen.
	pub fn at(dir: impl Into<PathBuf>) -> Self {
		Self { dir: dir.into() }
	}

	/// Generates every image into `dir`, creating it when needed.
	pub fn generate(dir: impl Into<PathBuf>) -> io::Result<Self> {
		let assets = Self::at(dir);
		fs::create_dir_all(&assets.dir)?;
		for index in 0..PHOTO_COUNT {
			let image = landscape(index, 480, 480);
			image.write_bmp(&assets.path(&format!("photo-{index}.bmp")))?;
		}
		for index in 0..AVATAR_COUNT {
			let image = avatar(index, 240);
			image.write_bmp(&assets.path(&format!("avatar-{index}.bmp")))?;
		}
		landscape(7, 1170, 420).write_bmp(&assets.path("cover.bmp"))?;
		Ok(assets)
	}

	/// The default location: a directory under the system temp dir.
	pub fn default_dir() -> PathBuf {
		std::env::temp_dir().join("leptoskit-gallery-assets")
	}

	/// The path of a square landscape photo; indices wrap around.
	pub fn photo(&self, index: usize) -> String {
		self.source(&format!("photo-{}.bmp", index % PHOTO_COUNT))
	}

	/// The path of a square avatar portrait; indices wrap around.
	pub fn avatar(&self, index: usize) -> String {
		self.source(&format!("avatar-{}.bmp", index % AVATAR_COUNT))
	}

	/// The path of the wide cover image.
	pub fn cover(&self) -> String {
		self.source("cover.bmp")
	}

	fn path(&self, name: &str) -> PathBuf {
		self.dir.join(name)
	}

	fn source(&self, name: &str) -> String {
		self.path(name).to_string_lossy().into_owned()
	}
}

/// An RGB color with channels from `0.0` to `1.0`.
#[derive(Debug, Clone, Copy)]
struct Rgb(f32, f32, f32);

impl Rgb {
	const fn hex(packed: u32) -> Self {
		Self(
			((packed >> 16) & 0xff) as f32 / 255.0,
			((packed >> 8) & 0xff) as f32 / 255.0,
			(packed & 0xff) as f32 / 255.0,
		)
	}

	fn mix(self, other: Self, amount: f32) -> Self {
		let t = amount.clamp(0.0, 1.0);
		Self(
			self.0 + (other.0 - self.0) * t,
			self.1 + (other.1 - self.1) * t,
			self.2 + (other.2 - self.2) * t,
		)
	}
}

/// A painted image, row-major from the top-left pixel.
struct Image {
	width: usize,
	height: usize,
	pixels: Vec<Rgb>,
}

impl Image {
	fn paint(width: usize, height: usize, shade: impl Fn(f32, f32) -> Rgb) -> Self {
		let mut pixels = Vec::with_capacity(width * height);
		for y in 0..height {
			for x in 0..width {
				pixels.push(shade(
					(x as f32 + 0.5) / width as f32,
					(y as f32 + 0.5) / height as f32,
				));
			}
		}
		Self {
			width,
			height,
			pixels,
		}
	}

	/// Writes a bottom-up, 24-bit, uncompressed BMP.
	fn write_bmp(&self, path: &Path) -> io::Result<()> {
		const HEADER_BYTES: usize = 14 + 40;
		let row_bytes = (self.width * 3).div_ceil(4) * 4;
		let image_bytes = row_bytes * self.height;
		let too_large = || io::Error::new(io::ErrorKind::InvalidInput, "image is too large");
		let file_size = u32::try_from(HEADER_BYTES + image_bytes).map_err(|_| too_large())?;
		let width = i32::try_from(self.width).map_err(|_| too_large())?;
		let height = i32::try_from(self.height).map_err(|_| too_large())?;

		let mut bytes = Vec::with_capacity(HEADER_BYTES + image_bytes);
		bytes.extend_from_slice(b"BM");
		bytes.extend_from_slice(&file_size.to_le_bytes());
		bytes.extend_from_slice(&0u32.to_le_bytes());
		bytes.extend_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
		bytes.extend_from_slice(&40u32.to_le_bytes());
		bytes.extend_from_slice(&width.to_le_bytes());
		bytes.extend_from_slice(&height.to_le_bytes());
		bytes.extend_from_slice(&1u16.to_le_bytes());
		bytes.extend_from_slice(&24u16.to_le_bytes());
		bytes.extend_from_slice(&0u32.to_le_bytes());
		bytes.extend_from_slice(&(image_bytes as u32).to_le_bytes());
		// 72 DPI, in pixels per metre.
		bytes.extend_from_slice(&2835i32.to_le_bytes());
		bytes.extend_from_slice(&2835i32.to_le_bytes());
		bytes.extend_from_slice(&0u32.to_le_bytes());
		bytes.extend_from_slice(&0u32.to_le_bytes());
		let to_byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
		for row in self.pixels.chunks(self.width).rev() {
			for pixel in row {
				bytes.extend_from_slice(&[to_byte(pixel.2), to_byte(pixel.1), to_byte(pixel.0)]);
			}
			bytes.resize(bytes.len() + row_bytes - self.width * 3, 0);
		}
		fs::write(path, bytes)
	}
}

/// The colors of one landscape: sky top and horizon, sun, far and near hills.
struct Palette {
	sky_top: Rgb,
	horizon: Rgb,
	sun: Rgb,
	far: Rgb,
	near: Rgb,
}

const PALETTES: [Palette; 5] = [
	// Dawn over violet hills.
	Palette {
		sky_top: Rgb::hex(0x2b_2d5c),
		horizon: Rgb::hex(0xf7_a072),
		sun: Rgb::hex(0xff_e6b8),
		far: Rgb::hex(0x7a_5f8f),
		near: Rgb::hex(0x2f_2a4a),
	},
	// Clear midday over green fields.
	Palette {
		sky_top: Rgb::hex(0x3d_8fd1),
		horizon: Rgb::hex(0xd4_ecf7),
		sun: Rgb::hex(0xff_fbe8),
		far: Rgb::hex(0x7f_b493),
		near: Rgb::hex(0x2f_6b50),
	},
	// Magenta dusk.
	Palette {
		sky_top: Rgb::hex(0x1d_1b3a),
		horizon: Rgb::hex(0xe2_557b),
		sun: Rgb::hex(0xff_d08a),
		far: Rgb::hex(0x5b_2a55),
		near: Rgb::hex(0x22_132f),
	},
	// Desert afternoon.
	Palette {
		sky_top: Rgb::hex(0xf0_a46c),
		horizon: Rgb::hex(0xfd_e8c8),
		sun: Rgb::hex(0xff_f6e6),
		far: Rgb::hex(0xd9_8a59),
		near: Rgb::hex(0x9c_4a2e),
	},
	// Arctic morning.
	Palette {
		sky_top: Rgb::hex(0x7f_a9d6),
		horizon: Rgb::hex(0xf0_f5fb),
		sun: Rgb::hex(0xff_ffff),
		far: Rgb::hex(0xae_c6df),
		near: Rgb::hex(0x5e_83aa),
	},
];

/// A smooth 0→1 ramp over `[edge0, edge1]`.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
	let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

/// A landscape: a graded sky, a glowing sun, and two ridges of hills.
fn landscape(seed: usize, width: usize, height: usize) -> Image {
	let palette = &PALETTES[seed % PALETTES.len()];
	let phase = seed as f32 * 1.7;
	let sun_x = 0.25 + 0.5 * ((seed as f32 * 0.37).sin() * 0.5 + 0.5);
	let sun_y = 0.36 + 0.12 * (seed as f32 * 0.91).cos();
	let aspect = width as f32 / height as f32;
	// One pixel, in normalized height units, for anti-aliased edges.
	let pixel = 1.0 / height as f32;
	Image::paint(width, height, |x, y| {
		let mut color = palette
			.sky_top
			.mix(palette.horizon, smoothstep(0.0, 0.75, y));
		let dx = (x - sun_x) * aspect;
		let dy = y - sun_y;
		let distance = (dx * dx + dy * dy).sqrt();
		color = color.mix(palette.sun, 0.35 * (1.0 - smoothstep(0.0, 0.35, distance)));
		color = color.mix(
			palette.sun,
			1.0 - smoothstep(0.085, 0.085 + pixel, distance),
		);
		let wx = x * aspect;
		let far = 0.6 + 0.05 * (wx * 5.0 + phase).sin() + 0.03 * (wx * 11.0 + phase * 2.3).sin();
		color = color.mix(palette.far, smoothstep(far, far + pixel, y));
		let near = 0.76 + 0.04 * (wx * 3.0 + phase * 0.7).sin() + 0.02 * (wx * 13.0 + phase).cos();
		let shade = palette
			.near
			.mix(palette.far, 0.25 * (1.0 - (y - near) * 4.0));
		color.mix(shade, smoothstep(near, near + pixel, y))
	})
}

/// An abstract portrait: a head and shoulders on a soft gradient.
fn avatar(seed: usize, size: usize) -> Image {
	const BACKGROUNDS: [(u32, u32); AVATAR_COUNT] = [
		(0xff_b199, 0xff_7a45),
		(0x9b_e7d2, 0x14_bf9b),
		(0xc7_b8ff, 0x7c_5cff),
		(0xff_d58a, 0xf2_9e1f),
	];
	const SKIN: [u32; AVATAR_COUNT] = [0x8d_5524, 0xe0_ac69, 0xc6_8642, 0x5c_3a21];
	const CLOTHES: [u32; AVATAR_COUNT] = [0x16_111f, 0xfa_f6f0, 0x2e_3a59, 0x1f_6f5c];
	let (top, bottom) = BACKGROUNDS[seed % AVATAR_COUNT];
	let skin = Rgb::hex(SKIN[seed % AVATAR_COUNT]);
	let clothes = Rgb::hex(CLOTHES[seed % AVATAR_COUNT]);
	let pixel = 1.0 / size as f32;
	Image::paint(size, size, |x, y| {
		let mut color = Rgb::hex(top).mix(Rgb::hex(bottom), y);
		let shoulders = ((x - 0.5) / 0.36).powi(2) + ((y - 1.02) / 0.34).powi(2);
		color = color.mix(clothes, 1.0 - smoothstep(1.0 - 4.0 * pixel, 1.0, shoulders));
		let head = ((x - 0.5) / 0.19).powi(2) + ((y - 0.43) / 0.23).powi(2);
		color.mix(skin, 1.0 - smoothstep(1.0 - 6.0 * pixel, 1.0, head))
	})
}
