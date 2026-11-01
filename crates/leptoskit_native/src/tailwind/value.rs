use std::fmt::Write;

use leptoskit_yoga::Dimension;

use super::StyleContext;
use crate::Color;

const MAX_MAGNITUDE: f32 = 1_000_000.0;
const MAX_RESOLVED_POINTS: f64 = 1_000_000_000.0;

/// Typed units retain their CSS meaning until a native viewport is known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Length {
	Auto,
	Px(f32),
	Rem(f32),
	Percent(f32),
	Vw(f32),
	Vh(f32),
}

impl Length {
	pub(super) fn is_zero(self) -> bool {
		match self {
			Self::Auto => false,
			Self::Px(value)
			| Self::Rem(value)
			| Self::Percent(value)
			| Self::Vw(value)
			| Self::Vh(value) => value == 0.0,
		}
	}

	pub(super) fn parse(value: &str, negative: bool, auto: bool) -> Result<Self, String> {
		let value = arbitrary(value).unwrap_or(value);

		if value == "auto" && auto {
			return Ok(Self::Auto);
		}

		let (number, unit) = ["rem", "px", "%", "vw", "vh"]
			.into_iter()
			.find_map(|unit| value.strip_suffix(unit).map(|number| (number, unit)))
			.unwrap_or((value, ""));
		let number = finite(number)?;

		if number < 0.0 && !negative {
			return Err("negative values are not allowed for this property".to_owned());
		}

		match unit {
			"px" => Ok(Self::Px(number)),
			"rem" if (number * 16.0).is_finite() => Ok(Self::Rem(number)),
			"%" => Ok(Self::Percent(number / 100.0)),
			"vw" => Ok(Self::Vw(number)),
			"vh" => Ok(Self::Vh(number)),
			"" if number == 0.0 => Ok(Self::Px(0.0)),
			_ => {
				Err(format!(
					"unsupported length `{value}`; use px, rem, %, vw, or vh"
				))
			}
		}
	}

	pub(super) fn spacing(
		value: &str,
		negative: bool,
		auto: bool,
		fraction: bool,
	) -> Result<Self, String> {
		if value == "auto" && auto {
			return Ok(Self::Auto);
		}

		if value == "px" {
			return Ok(Self::Px(1.0));
		}

		if value == "full" && fraction {
			return Ok(Self::Percent(1.0));
		}

		if arbitrary(value).is_some() {
			return Self::parse(value, negative, auto);
		}

		if fraction && let Some((numerator, denominator)) = value.split_once('/') {
			let numerator = finite(numerator)?;
			let denominator = finite(denominator)?;

			if numerator < 0.0 || denominator <= 0.0 || !(numerator / denominator).is_finite() {
				return Err("a fraction requires a non-negative numerator and positive \
				            denominator"
					.to_owned());
			}

			return bounded(numerator / denominator).map(Self::Percent);
		}

		let number = finite(value)?;

		if number < 0.0 || !(number * 4.0).is_finite() {
			return Err("spacing scale values must be finite and non-negative".to_owned());
		}

		Ok(Self::Rem(number / 4.0))
	}

	pub(super) fn negated(self) -> Self {
		match self {
			Self::Auto => Self::Auto,
			Self::Px(value) => Self::Px(-value),
			Self::Rem(value) => Self::Rem(-value),
			Self::Percent(value) => Self::Percent(-value),
			Self::Vw(value) => Self::Vw(-value),
			Self::Vh(value) => Self::Vh(-value),
		}
	}

	pub(super) fn dimension(self, context: &StyleContext) -> Dimension {
		match self {
			Self::Auto => Dimension::Auto,
			Self::Percent(value) => Dimension::Percent(value),
			_ => Dimension::Points(self.points(context)),
		}
	}

	pub(super) fn points(self, context: &StyleContext) -> f32 {
		let points = match self {
			Self::Px(value) => f64::from(value),
			Self::Rem(value) => f64::from(value) * 16.0,
			Self::Vw(value) => viewport_points(context.viewport.width, value),
			Self::Vh(value) => viewport_points(context.viewport.height, value),
			Self::Auto | Self::Percent(_) => {
				unreachable!("only absolute lengths have point values")
			}
		};

		points.clamp(-MAX_RESOLVED_POINTS, MAX_RESOLVED_POINTS) as f32
	}

	pub(super) fn absolute(self) -> Result<Self, String> {
		if matches!(self, Self::Auto | Self::Percent(_)) {
			return Err("this property requires px, rem, vw, or vh".to_owned());
		}

		Ok(self)
	}

	pub(super) fn css(self) -> String {
		match self {
			Self::Auto => "auto".to_owned(),
			Self::Px(value) => format!("{value}px"),
			Self::Rem(value) => format!("{value}rem"),
			Self::Percent(value) => format!("{}%", f64::from(value) * 100.0),
			Self::Vw(value) => format!("{value}vw"),
			Self::Vh(value) => format!("{value}vh"),
		}
	}
}

/// A safe-area inset plus an absolute offset. Negative and nonfinite insets
/// count as zero, and the sum saturates like every resolved length.
pub(super) fn safe_inset(inset: f32, offset: Length, context: &StyleContext) -> f32 {
	let inset = if inset.is_finite() {
		f64::from(inset.max(0.0))
	} else {
		0.0
	};

	(inset + f64::from(offset.points(context))).min(MAX_RESOLVED_POINTS) as f32
}

pub(super) fn arbitrary(value: &str) -> Option<&str> {
	value.strip_prefix('[')?.strip_suffix(']')
}

pub(super) fn finite(value: &str) -> Result<f32, String> {
	let number = value
		.parse::<f32>()
		.map_err(|_| format!("invalid number `{value}`"))?;

	bounded(number).map_err(|reason| format!("number `{value}` {reason}"))
}

pub(super) fn bounded(number: f32) -> Result<f32, String> {
	if !number.is_finite() || number.abs() > MAX_MAGNITUDE {
		return Err("must be finite with magnitude at most 1000000".to_owned());
	}

	Ok(number)
}

fn viewport_points(viewport: f32, value: f32) -> f64 {
	let viewport = if viewport.is_finite() {
		f64::from(viewport).max(0.0)
	} else {
		0.0
	};

	viewport * (f64::from(value) / 100.0)
}

pub(super) fn color(value: &str) -> Result<Color, String> {
	let (value, alpha) = value
		.rsplit_once('/')
		.map_or((value, None), |(color, alpha)| (color, Some(alpha)));
	let value = arbitrary(value).unwrap_or(value);
	let value = value.strip_prefix("color:").unwrap_or(value);

	let mut color = match value {
		"transparent" => Color::rgba(0, 0, 0, 0),
		"black" => Color::rgba(0, 0, 0, 255),
		"white" => Color::rgba(255, 255, 255, 255),

		value if value.starts_with('#') => hex_color(value)?,
		value => {
			super::palette::lookup(value).ok_or_else(|| format!("unsupported color `{value}`"))?
		}
	};

	if let Some(alpha) = alpha {
		let alpha = if let Some(alpha) = arbitrary(alpha) {
			if let Some(percent) = alpha.strip_suffix('%') {
				finite(percent)? / 100.0
			} else {
				finite(alpha)?
			}
		} else {
			finite(alpha)? / 100.0
		};

		if !(0.0..=1.0).contains(&alpha) {
			return Err("color opacity must be between 0 and 100%".to_owned());
		}

		color = Color::rgba(
			color.red(),
			color.green(),
			color.blue(),
			(alpha * f32::from(color.alpha())).round() as u8,
		);
	}

	Ok(color)
}

fn hex_color(value: &str) -> Result<Color, String> {
	let digits = &value[1..];

	if matches!(digits.len(), 3 | 4) && digits.is_ascii() {
		let mut expanded = String::from("#");

		for digit in digits.chars() {
			let _ = write!(expanded, "{digit}{digit}");
		}

		return Color::hex(&expanded).ok_or_else(|| format!("invalid hex color `{value}`"));
	}

	Color::hex(value).ok_or_else(|| format!("invalid hex color `{value}`"))
}
