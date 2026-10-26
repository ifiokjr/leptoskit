//! Color depth detection and quantization to what a terminal can show.

use crate::cell::TuiColor;

/// How many colors the terminal can distinguish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorDepth {
	/// 24-bit color.
	TrueColor,
	/// The xterm 256-color palette.
	#[default]
	Indexed256,
	/// The sixteen base colors.
	Indexed16,
	/// No color at all; attributes still render.
	Mono,
}

impl ColorDepth {
	/// The depth of the terminal this process is attached to.
	///
	/// `NO_COLOR` wins over everything; `COLORTERM` advertising truecolor wins
	/// over `TERM`; a `256color` `TERM` selects the palette; anything else
	/// falls back to the sixteen base colors. A terminal without any of these
	/// still shows attributes, so it degrades to [`ColorDepth::Mono`].
	#[must_use]
	pub fn detect() -> Self {
		Self::detect_from(
			std::env::var_os("TERM")
				.as_deref()
				.map(std::ffi::OsStr::to_string_lossy),
			std::env::var_os("COLORTERM")
				.as_deref()
				.map(std::ffi::OsStr::to_string_lossy),
			std::env::var_os("NO_COLOR").is_some(),
		)
	}

	/// The depth implied by the terminal environment pieces, for tests.
	#[must_use]
	pub fn detect_from(
		term: Option<std::borrow::Cow<'_, str>>,
		colorterm: Option<std::borrow::Cow<'_, str>>,
		no_color: bool,
	) -> Self {
		if no_color {
			return Self::Mono;
		}

		if colorterm.is_some_and(|value| {
			let value = value.to_ascii_lowercase();
			value == "truecolor" || value == "24bit"
		}) {
			return Self::TrueColor;
		}

		let term = term.unwrap_or_default().to_ascii_lowercase();
		if term.contains("256color") {
			Self::Indexed256
		} else if term.contains("color") || term.contains("ansi") || term.contains("xterm") {
			Self::Indexed16
		} else {
			Self::Mono
		}
	}
}

/// A color the differ can write as one escape sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalColor {
	/// The terminal's default.
	Default,
	/// A 24-bit color.
	TrueColor(u8, u8, u8),
	/// A palette index: 0-255.
	Indexed(u8),
}

/// Encodes a cell color for a terminal depth, quantizing when needed.
#[must_use]
pub fn encode(color: TuiColor, depth: ColorDepth) -> TerminalColor {
	match color {
		TuiColor::Default => TerminalColor::Default,
		TuiColor::Rgb(red, green, blue) => {
			match depth {
				ColorDepth::TrueColor | ColorDepth::Mono => {
					TerminalColor::TrueColor(red, green, blue)
				}
				ColorDepth::Indexed256 => TerminalColor::Indexed(nearest_256(red, green, blue)),
				ColorDepth::Indexed16 => TerminalColor::Indexed(nearest_16(red, green, blue)),
			}
		}
	}
}

/// The six cube levels of the xterm 256-color palette.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The nearest xterm palette entry from 16 to 255, searching the color cube
/// and the gray ramp; the base sixteen are theme-defined, so exact search
/// never targets them.
#[must_use]
pub fn nearest_256(red: u8, green: u8, blue: u8) -> u8 {
	let mut best = 16;
	let mut best_distance = u32::MAX;

	for index in 16..=255u8 {
		let (candidate_red, candidate_green, candidate_blue) = if let Some(gray) = gray_ramp(index)
		{
			(gray, gray, gray)
		} else {
			cube(index)
		};

		let distance = squared_distance(
			(u32::from(red), u32::from(green), u32::from(blue)),
			(
				u32::from(candidate_red),
				u32::from(candidate_green),
				u32::from(candidate_blue),
			),
		);

		if distance < best_distance {
			best_distance = distance;
			best = index;
		}
	}

	best
}

/// The gray a ramp index 232-255 shows, or `None` for any other index.
fn gray_ramp(index: u8) -> Option<u8> {
	(232..=255).contains(&index).then(|| (index - 232) * 10 + 8)
}

/// The cube color an index 16-231 shows.
fn cube(index: u8) -> (u8, u8, u8) {
	let offset = index - 16;
	(
		CUBE_LEVELS[usize::from(offset / 36)],
		CUBE_LEVELS[usize::from((offset / 6) % 6)],
		CUBE_LEVELS[usize::from(offset % 6)],
	)
}

/// The nearest of the sixteen base colors: gray levels by brightness, hues by
/// sector, with the bright variant for light values.
#[must_use]
pub fn nearest_16(red: u8, green: u8, blue: u8) -> u8 {
	let red = f32::from(red) / 255.0;
	let green = f32::from(green) / 255.0;
	let blue = f32::from(blue) / 255.0;

	let max = red.max(green).max(blue);
	let min = red.min(green).min(blue);
	let saturation = if max <= f32::EPSILON {
		0.0
	} else {
		(max - min) / max
	};

	if saturation < 0.2 {
		return match max {
			below if below < 0.25 => 0,
			below if below < 0.62 => 8,
			below if below < 0.9 => 7,
			_ => 15,
		};
	}

	// Hue sector: red, yellow, green, cyan, blue, magenta, then red again.
	let (sector, _) = hue_sector(red, green, blue);
	let bright = u8::from(max > 0.6);

	match sector {
		0 => 1 + bright * 8, // red
		1 => 3 + bright * 8, // yellow
		2 => 2 + bright * 8, // green
		3 => 6 + bright * 8, // cyan
		4 => 4 + bright * 8, // blue
		_ => 5 + bright * 8, // magenta
	}
}

/// The hue's sector, each sixty degrees wide, counting from red.
fn hue_sector(red: f32, green: f32, blue: f32) -> (u8, f32) {
	let max = red.max(green).max(blue);
	let delta = max - red.min(green).min(blue);

	if delta <= f32::EPSILON {
		return (0, 0.0);
	}

	let hue = 60.0
		* if (max - red) <= f32::EPSILON {
			(green - blue) / delta % 6.0
		} else if (max - green) <= f32::EPSILON {
			(blue - red) / delta + 2.0
		} else {
			(red - green) / delta + 4.0
		};

	let hue = if hue < 0.0 { hue + 360.0 } else { hue };
	((hue / 60.0) as u8 % 6, hue)
}

fn squared_distance(a: (u32, u32, u32), b: (u32, u32, u32)) -> u32 {
	(a.0.abs_diff(b.0)).pow(2) + (a.1.abs_diff(b.1)).pow(2) + (a.2.abs_diff(b.2)).pow(2)
}
