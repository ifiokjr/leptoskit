//! App-supplied CSS custom properties for native color resolution.
//!
//! Web keeps `var(--name)` in the stylesheet and lets the browser resolve it.
//! Native hosts have no cascade, so [`Theme`] maps the same names to
//! [`Color`](crate::Color) values. Set them before resolving a tree; clear or
//! replace them between tests and screens.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::Color;
use crate::hash::FastHash;

thread_local! {
	static COLORS: RefCell<HashMap<Box<str>, Color, FastHash>> =
		RefCell::new(HashMap::with_hasher(FastHash::default()));
}

/// The process-wide map of CSS custom property names to solid colors.
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

	/// The color for `name`, if one was set. Accepts `--brand` or `brand`.
	#[must_use]
	pub fn color(name: &str) -> Option<Color> {
		let name = normalize_name(name);
		COLORS.with(|colors| colors.borrow().get(&name).copied())
	}

	/// Removes every color from the theme.
	pub fn clear() {
		COLORS.with(|colors| colors.borrow_mut().clear());
	}

	/// A `:root { --name:#rrggbbaa; … }` sheet for the current theme, or an
	/// empty string when nothing is set.
	#[must_use]
	pub fn css() -> String {
		COLORS.with(|colors| {
			let colors = colors.borrow();
			if colors.is_empty() {
				return String::new();
			}

			let mut names: Vec<_> = colors.keys().cloned().collect();
			names.sort_unstable();
			let mut css = String::from(":root{");
			for name in names {
				let color = colors[&name];
				use std::fmt::Write as _;
				let _ = write!(css, "{name}:#{:08x};", color.0);
			}
			css.push('}');
			css
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

#[cfg(test)]
mod tests {
	use super::Theme;
	use crate::Color;

	#[test]
	fn theme_round_trips_and_emits_root_css() {
		Theme::clear();
		Theme::set_color("brand", Color::rgba(0x11, 0x22, 0x33, 0xff));
		assert_eq!(
			Theme::color("--brand"),
			Some(Color::rgba(0x11, 0x22, 0x33, 0xff))
		);
		assert_eq!(Theme::css(), ":root{--brand:#112233ff;}");
		Theme::clear();
		assert_eq!(Theme::css(), "");
	}
}
