use std::fmt::Write;

use leptoskit_yoga::Dimension;

use super::StyleContext;
use super::theme::ThemeLength;
use crate::Color;

const MAX_MAGNITUDE: f32 = 1_000_000.0;
const MAX_RESOLVED_POINTS: f64 = 1_000_000_000.0;
/// Outer `var()` plus one nested fallback, matching common theme patterns.
const MAX_VAR_DEPTH: u8 = 2;

/// Typed units retain their CSS meaning until a native viewport is known.
#[derive(Debug, Clone, PartialEq)]
#[allow(
	variant_size_differences,
	reason = "Var carries the custom-property name; solid units stay unboxed"
)]
pub(super) enum Length {
	Auto,
	Px(f32),
	Rem(f32),
	Percent(f32),
	Vw(f32),
	Vh(f32),
	/// Kept as CSS for the web; native resolves through [`super::Theme`].
	Var {
		name: Box<str>,
		fallback: Option<Box<Length>>,
	},
}

impl Length {
	pub(super) fn is_zero(&self) -> bool {
		match self {
			Self::Auto | Self::Var { .. } => false,
			Self::Px(value)
			| Self::Rem(value)
			| Self::Percent(value)
			| Self::Vw(value)
			| Self::Vh(value) => *value == 0.0,
		}
	}

	pub(super) fn parse(value: &str, negative: bool, auto: bool) -> Result<Self, String> {
		let value = arbitrary(value).unwrap_or(value);

		if value == "auto" && auto {
			return Ok(Self::Auto);
		}

		if let Some(variable) = parse_length_var(value, MAX_VAR_DEPTH)? {
			if negative {
				return Err("negative var() lengths are not supported".to_owned());
			}
			return Ok(variable);
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
					"unsupported length `{value}`; use px, rem, %, vw, vh, or var(--name)"
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

	pub(super) fn negated(self) -> Result<Self, String> {
		Ok(match self {
			Self::Auto => Self::Auto,
			Self::Px(value) => Self::Px(-value),
			Self::Rem(value) => Self::Rem(-value),
			Self::Percent(value) => Self::Percent(-value),
			Self::Vw(value) => Self::Vw(-value),
			Self::Vh(value) => Self::Vh(-value),
			Self::Var { .. } => {
				return Err("negative var() lengths are not supported".to_owned());
			}
		})
	}

	pub(super) fn dimension(&self, context: &StyleContext) -> Dimension {
		match self.resolve() {
			None | Some(Self::Auto) => Dimension::Auto,
			Some(Self::Percent(value)) => Dimension::Percent(value),
			Some(length) => Dimension::Points(length.points_solid(context)),
		}
	}

	/// Resolves theme `var()` references to a solid length, or `None` when
	/// neither the theme nor a fallback provides one.
	pub(super) fn resolve(&self) -> Option<Self> {
		match self {
			Self::Var { name, fallback } => {
				if let Some(length) = super::Theme::length(name) {
					return Some(length_from_theme(length));
				}
				fallback.as_ref().and_then(|length| length.resolve())
			}
			other => Some(other.clone()),
		}
	}

	pub(super) fn points(&self, context: &StyleContext) -> f32 {
		match self.resolve() {
			Some(length) => length.points_solid(context),
			None => 0.0,
		}
	}

	fn points_solid(&self, context: &StyleContext) -> f32 {
		let points = match self {
			Self::Px(value) => f64::from(*value),
			Self::Rem(value) => f64::from(*value) * 16.0,
			Self::Vw(value) => viewport_points(context.viewport.width, *value),
			Self::Vh(value) => viewport_points(context.viewport.height, *value),
			Self::Auto | Self::Percent(_) | Self::Var { .. } => {
				unreachable!("only absolute lengths have point values")
			}
		};

		points.clamp(-MAX_RESOLVED_POINTS, MAX_RESOLVED_POINTS) as f32
	}

	pub(super) fn absolute(self) -> Result<Self, String> {
		match self {
			Self::Auto | Self::Percent(_) => {
				Err("this property requires px, rem, vw, vh, or var(--name)".to_owned())
			}
			Self::Var { name, fallback } => {
				let fallback = fallback.map(|length| length.absolute()).transpose()?;
				Ok(Self::Var {
					name,
					fallback: fallback.map(Box::new),
				})
			}
			other => Ok(other),
		}
	}

	pub(super) fn css(&self) -> String {
		match self {
			Self::Auto => "auto".to_owned(),
			Self::Px(value) => format!("{value}px"),
			Self::Rem(value) => format!("{value}rem"),
			Self::Percent(value) => format!("{}%", f64::from(*value) * 100.0),
			Self::Vw(value) => format!("{value}vw"),
			Self::Vh(value) => format!("{value}vh"),
			Self::Var {
				name,
				fallback: None,
			} => format!("var({name})"),
			Self::Var {
				name,
				fallback: Some(fallback),
			} => format!("var({name},{})", fallback.css()),
		}
	}
}

fn length_from_theme(length: ThemeLength) -> Length {
	match length {
		ThemeLength::Px(value) => Length::Px(value),
		ThemeLength::Rem(value) => Length::Rem(value),
		ThemeLength::Percent(value) => Length::Percent(value),
		ThemeLength::Vw(value) => Length::Vw(value),
		ThemeLength::Vh(value) => Length::Vh(value),
	}
}

/// A safe-area inset plus an absolute offset. Negative and nonfinite insets
/// count as zero, and the sum saturates like every resolved length.
pub(super) fn safe_inset(inset: f32, offset: &Length, context: &StyleContext) -> f32 {
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

/// A solid color or a CSS `var(--name[, fallback])` reference.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum PaintColor {
	Solid(Color),
	/// Kept as CSS for the web; native resolves through [`super::Theme`].
	Var {
		name: Box<str>,
		fallback: Option<Box<PaintColor>>,
		/// Opacity fraction `0.0..=1.0` from a `/50` modifier, applied after
		/// theme or fallback resolution on native and as `color-mix` on web.
		alpha: Option<f32>,
	},
}

impl PaintColor {
	/// The color native paint uses: the theme entry, else the fallback.
	pub(super) fn resolve(&self) -> Option<Color> {
		match self {
			Self::Solid(color) => Some(*color),
			Self::Var {
				name,
				fallback,
				alpha,
			} => {
				let color = super::Theme::color(name)
					.or_else(|| fallback.as_ref().and_then(|color| color.resolve()))?;
				Some(apply_alpha(color, *alpha))
			}
		}
	}

	/// CSS color syntax for utility sheets.
	pub(super) fn css(&self) -> String {
		match self {
			Self::Solid(color) => format!("#{:08x}", color.0),
			Self::Var {
				name,
				fallback,
				alpha,
			} => {
				let base = match fallback.as_deref() {
					None => format!("var({name})"),
					Some(fallback) => format!("var({name},{})", fallback.css()),
				};
				match *alpha {
					None => base,
					Some(alpha) => {
						let percent = (alpha * 100.0).round();
						format!("color-mix(in srgb,{base} {percent}%,transparent)")
					}
				}
			}
		}
	}
}

fn apply_alpha(color: Color, alpha: Option<f32>) -> Color {
	let Some(alpha) = alpha else {
		return color;
	};
	Color::rgba(
		color.red(),
		color.green(),
		color.blue(),
		(alpha * f32::from(color.alpha())).round() as u8,
	)
}

pub(super) fn color(value: &str) -> Result<PaintColor, String> {
	let (value, alpha) = value
		.rsplit_once('/')
		.map_or((value, None), |(color, alpha)| (color, Some(alpha)));
	let value = arbitrary(value).unwrap_or(value);
	let value = value.strip_prefix("color:").unwrap_or(value);

	if let Some(mut variable) = parse_var(value, MAX_VAR_DEPTH)? {
		if let Some(alpha) = alpha {
			let alpha = parse_opacity(alpha)?;
			if let PaintColor::Var {
				alpha: slot,
				..
			} = &mut variable
			{
				*slot = Some(alpha);
			}
		}
		return Ok(variable);
	}

	let mut color = solid_color(value)?;

	if let Some(alpha) = alpha {
		let alpha = parse_opacity(alpha)?;
		color = apply_alpha(color, Some(alpha));
	}

	Ok(PaintColor::Solid(color))
}

fn parse_opacity(alpha: &str) -> Result<f32, String> {
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

	Ok(alpha)
}

fn solid_color(value: &str) -> Result<Color, String> {
	match value {
		"transparent" => Ok(Color::rgba(0, 0, 0, 0)),
		"black" => Ok(Color::rgba(0, 0, 0, 255)),
		"white" => Ok(Color::rgba(255, 255, 255, 255)),
		value if value.starts_with('#') => hex_color(value),
		value => {
			super::palette::lookup(value).ok_or_else(|| format!("unsupported color `{value}`"))
		}
	}
}

/// `var(--name)` or `var(--name, fallback)`. Nested `var()` fallbacks are
/// allowed up to [`MAX_VAR_DEPTH`] levels; the innermost fallback may be a
/// solid color.
fn parse_var(value: &str, depth: u8) -> Result<Option<PaintColor>, String> {
	if !value.starts_with("var(") {
		return Ok(None);
	}

	if depth == 0 {
		return Err("nested var() is not supported beyond two levels".to_owned());
	}

	let Some(inner) = value
		.strip_prefix("var(")
		.and_then(|rest| rest.strip_suffix(')'))
	else {
		return Err(
			"malformed var(); expected `var(--name)` or `var(--name, fallback)`".to_owned(),
		);
	};

	let inner = inner.trim();
	let (name, fallback) = match split_var_args(inner) {
		Some((name, fallback)) => (name.trim(), Some(fallback.trim())),
		None => (inner, None),
	};

	validate_custom_property(name)?;

	let fallback = match fallback {
		None => None,
		Some(fallback) if fallback.starts_with("var(") => {
			Some(Box::new(parse_var(fallback, depth - 1)?.ok_or_else(|| {
				"malformed nested var() fallback".to_owned()
			})?))
		}
		Some(fallback) => Some(Box::new(PaintColor::Solid(solid_color(fallback)?))),
	};

	Ok(Some(PaintColor::Var {
		name: name.into(),
		fallback,
		alpha: None,
	}))
}

fn parse_length_var(value: &str, depth: u8) -> Result<Option<Length>, String> {
	if !value.starts_with("var(") {
		return Ok(None);
	}

	if depth == 0 {
		return Err("nested var() is not supported beyond two levels".to_owned());
	}

	let Some(inner) = value
		.strip_prefix("var(")
		.and_then(|rest| rest.strip_suffix(')'))
	else {
		return Err(
			"malformed var(); expected `var(--name)` or `var(--name, fallback)`".to_owned(),
		);
	};

	let inner = inner.trim();
	let (name, fallback) = match split_var_args(inner) {
		Some((name, fallback)) => (name.trim(), Some(fallback.trim())),
		None => (inner, None),
	};

	validate_custom_property(name)?;

	let fallback = match fallback {
		None => None,
		Some(fallback) if fallback.starts_with("var(") => {
			Some(Box::new(parse_length_var(fallback, depth - 1)?.ok_or_else(
				|| "malformed nested var() fallback".to_owned(),
			)?))
		}
		Some(fallback) => {
			Some(Box::new(Length::parse(fallback, false, false)?))
		}
	};

	Ok(Some(Length::Var {
		name: name.into(),
		fallback,
	}))
}

/// Splits `name, fallback` on the first comma that is not inside nested
/// parentheses, so `var(--a, var(--b, #fff))` keeps the nested call intact.
fn split_var_args(inner: &str) -> Option<(&str, &str)> {
	let mut depth = 0_i32;
	for (index, character) in inner.char_indices() {
		match character {
			'(' => depth += 1,
			')' => depth -= 1,
			',' if depth == 0 => return Some((&inner[..index], &inner[index + 1..])),
			_ => {}
		}
	}
	None
}

fn validate_custom_property(name: &str) -> Result<(), String> {
	if !name.starts_with("--") || name.len() < 3 || name.contains(char::is_whitespace) {
		return Err(format!(
			"var() needs a custom property name like `--brand`; got `{name}`"
		));
	}

	if name
		.chars()
		.any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')))
	{
		return Err(format!("unsupported custom property name `{name}`"));
	}

	Ok(())
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
