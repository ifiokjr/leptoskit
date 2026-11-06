//! App-supplied CSS custom properties for native resolution.
//!
//! Web keeps `var(--name)` in the stylesheet and lets the browser resolve it.
//! Native hosts have no cascade, so [`Theme`] maps the same names to
//! [`Color`](crate::Color) and length values. Set them before resolving a
//! tree; clear or replace them between tests and screens. Values from
//! `leptoskit.toml`'s `[theme]` table install through [`Theme::install_maps`].

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;

use crate::Color;
use crate::hash::FastHash;

thread_local! {
	static COLORS: RefCell<HashMap<Box<str>, Color, FastHash>> =
		RefCell::new(HashMap::with_hasher(FastHash::default()));
	static LENGTHS: RefCell<HashMap<Box<str>, ThemeLength, FastHash>> =
		RefCell::new(HashMap::with_hasher(FastHash::default()));
}

/// A length token stored in [`Theme`], matching the absolute units the
/// Tailwind compiler accepts in arbitrary values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThemeLength {
	/// Logical pixels.
	Px(f32),
	/// Root ems (`1rem` is 16 logical pixels on native).
	Rem(f32),
	/// A fraction of the containing block (`1.0` is 100%).
	Percent(f32),
	/// A fraction of the viewport width.
	Vw(f32),
	/// A fraction of the viewport height.
	Vh(f32),
}

impl ThemeLength {
	/// Parses `16px`, `1rem`, `50%`, `10vw`, or `10vh`.
	pub fn parse(value: &str) -> Result<Self, String> {
		let value = value.trim();
		let (number, unit) = ["rem", "px", "%", "vw", "vh"]
			.into_iter()
			.find_map(|unit| value.strip_suffix(unit).map(|number| (number, unit)))
			.ok_or_else(|| {
				format!("unsupported theme length `{value}`; use px, rem, %, vw, or vh")
			})?;
		let number: f32 = number
			.trim()
			.parse()
			.map_err(|_| format!("invalid number in theme length `{value}`"))?;
		if !number.is_finite() || number.abs() > 1_000_000.0 {
			return Err(format!(
				"theme length `{value}` must be finite with magnitude at most 1000000"
			));
		}
		Ok(match unit {
			"px" => Self::Px(number),
			"rem" => Self::Rem(number),
			"%" => Self::Percent(number / 100.0),
			"vw" => Self::Vw(number),
			"vh" => Self::Vh(number),
			_ => unreachable!("unit matched above"),
		})
	}

	/// CSS length syntax for [`Theme::css`].
	#[must_use]
	pub fn css(self) -> String {
		match self {
			Self::Px(value) => format!("{value}px"),
			Self::Rem(value) => format!("{value}rem"),
			Self::Percent(value) => format!("{}%", f64::from(value) * 100.0),
			Self::Vw(value) => format!("{value}vw"),
			Self::Vh(value) => format!("{value}vh"),
		}
	}
}

/// The process-wide map of CSS custom property names to colors and lengths.
///
/// Names are stored with the leading `--`, matching CSS and `var(--brand)`.
/// [`Theme::css`] emits a `:root` rule the web adapter (or a probe page) can
/// install beside utility sheets.
pub struct Theme;

impl Theme {
	/// Records `name` → `color`. `name` may be `--brand` or `brand`; both
	/// store as `--brand`.
	pub fn set_color(name: impl AsRef<str>, color: Color) {
		let name = normalize_name(name.as_ref());
		COLORS.with(|colors| {
			colors.borrow_mut().insert(name, color);
		});
	}

	/// Records `name` → `length`. `name` may be `--gap` or `gap`.
	pub fn set_length(name: impl AsRef<str>, length: ThemeLength) {
		let name = normalize_name(name.as_ref());
		LENGTHS.with(|lengths| {
			lengths.borrow_mut().insert(name, length);
		});
	}

	/// The color for `name`, if one was set. Accepts `--brand` or `brand`.
	#[must_use]
	pub fn color(name: &str) -> Option<Color> {
		let name = normalize_name(name);
		COLORS.with(|colors| colors.borrow().get(&name).copied())
	}

	/// The length for `name`, if one was set. Accepts `--gap` or `gap`.
	#[must_use]
	pub fn length(name: &str) -> Option<ThemeLength> {
		let name = normalize_name(name);
		LENGTHS.with(|lengths| lengths.borrow().get(&name).copied())
	}

	/// Replaces every color with `entries`, leaving lengths alone.
	pub fn replace_colors(
		entries: impl IntoIterator<Item = (impl AsRef<str>, Color)>,
	) {
		COLORS.with(|colors| {
			let mut colors = colors.borrow_mut();
			colors.clear();
			for (name, color) in entries {
				colors.insert(normalize_name(name.as_ref()), color);
			}
		});
	}

	/// Replaces every length with `entries`, leaving colors alone.
	pub fn replace_lengths(
		entries: impl IntoIterator<Item = (impl AsRef<str>, ThemeLength)>,
	) {
		LENGTHS.with(|lengths| {
			let mut lengths = lengths.borrow_mut();
			lengths.clear();
			for (name, length) in entries {
				lengths.insert(normalize_name(name.as_ref()), length);
			}
		});
	}

	/// Parses color and length maps (as in `leptoskit.toml`'s `[theme]` table)
	/// and replaces the current theme. Color values accept hex, `black`,
	/// `white`, and `transparent`. Length values accept `px`, `rem`, `%`,
	/// `vw`, and `vh`.
	///
	/// # Errors
	///
	/// Returns a message naming the first entry that cannot be parsed.
	pub fn install_maps(
		colors: &BTreeMap<String, String>,
		lengths: &BTreeMap<String, String>,
	) -> Result<(), String> {
		let mut parsed_colors = Vec::with_capacity(colors.len());
		for (name, value) in colors {
			let color = parse_theme_color(value)
				.map_err(|reason| format!("theme.colors.{name}: {reason}"))?;
			parsed_colors.push((name.as_str(), color));
		}
		let mut parsed_lengths = Vec::with_capacity(lengths.len());
		for (name, value) in lengths {
			let length = ThemeLength::parse(value)
				.map_err(|reason| format!("theme.lengths.{name}: {reason}"))?;
			parsed_lengths.push((name.as_str(), length));
		}
		Self::replace_colors(parsed_colors);
		Self::replace_lengths(parsed_lengths);
		Ok(())
	}

	/// Removes every color and length from the theme.
	pub fn clear() {
		COLORS.with(|colors| colors.borrow_mut().clear());
		LENGTHS.with(|lengths| lengths.borrow_mut().clear());
	}

	/// A `:root { --name:…; … }` sheet for the current theme, or an empty
	/// string when nothing is set.
	#[must_use]
	pub fn css() -> String {
		COLORS.with(|colors| {
			LENGTHS.with(|lengths| {
				let colors = colors.borrow();
				let lengths = lengths.borrow();
				if colors.is_empty() && lengths.is_empty() {
					return String::new();
				}

				let mut names: Vec<_> = colors
					.keys()
					.cloned()
					.chain(lengths.keys().cloned())
					.collect();
				names.sort_unstable();
				names.dedup();
				let mut css = String::from(":root{");
				for name in names {
					use std::fmt::Write as _;
					if let Some(color) = colors.get(&name) {
						let _ = write!(css, "{name}:#{:08x};", color.0);
					} else if let Some(length) = lengths.get(&name) {
						let _ = write!(css, "{name}:{};", length.css());
					}
				}
				css.push('}');
				css
			})
		})
	}
}

fn normalize_name(name: &str) -> Box<str> {
	let name = name.trim();
	if name.starts_with("--") {
		name.into()
	} else {
		format!("--{name}").into()
	}
}

fn parse_theme_color(value: &str) -> Result<Color, String> {
	let value = value.trim();
	match value {
		"transparent" => Ok(Color::rgba(0, 0, 0, 0)),
		"black" => Ok(Color::rgba(0, 0, 0, 255)),
		"white" => Ok(Color::rgba(255, 255, 255, 255)),
		value if value.starts_with('#') => {
			let hex = expand_hex(value);
			Color::hex(&hex).ok_or_else(|| format!("invalid hex color `{value}`"))
		}
		value => Err(format!(
			"unsupported color `{value}`; use hex, black, white, or transparent"
		)),
	}
}

fn expand_hex(value: &str) -> String {
	let digits = &value[1..];
	if matches!(digits.len(), 3 | 4) && digits.is_ascii() {
		let mut expanded = String::from("#");
		for digit in digits.chars() {
			use std::fmt::Write as _;
			let _ = write!(expanded, "{digit}{digit}");
		}
		return expanded;
	}
	value.to_owned()
}

#[cfg(test)]
mod tests {
	use super::Theme;
	use super::ThemeLength;
	use crate::Color;
	use std::collections::BTreeMap;

	#[test]
	fn theme_round_trips_and_emits_root_css() {
		Theme::clear();
		Theme::set_color("brand", Color::rgba(0x11, 0x22, 0x33, 0xff));
		Theme::set_length("gap", ThemeLength::Rem(1.0));
		assert_eq!(
			Theme::color("--brand"),
			Some(Color::rgba(0x11, 0x22, 0x33, 0xff))
		);
		assert_eq!(Theme::length("gap"), Some(ThemeLength::Rem(1.0)));
		assert_eq!(Theme::css(), ":root{--brand:#112233ff;--gap:1rem;}");
		Theme::clear();
		assert_eq!(Theme::css(), "");
	}

	#[test]
	fn install_maps_loads_toml_shaped_tokens() {
		Theme::clear();
		let colors = BTreeMap::from([
			("brand".to_owned(), "#aabbcc".to_owned()),
			("ink".to_owned(), "black".to_owned()),
		]);
		let lengths = BTreeMap::from([("gap".to_owned(), "16px".to_owned())]);
		Theme::install_maps(&colors, &lengths).expect("install");
		assert_eq!(
			Theme::color("--brand"),
			Some(Color::rgba(0xaa, 0xbb, 0xcc, 0xff))
		);
		assert_eq!(Theme::color("ink"), Some(Color::rgba(0, 0, 0, 255)));
		assert_eq!(Theme::length("--gap"), Some(ThemeLength::Px(16.0)));
		Theme::clear();
	}
}
