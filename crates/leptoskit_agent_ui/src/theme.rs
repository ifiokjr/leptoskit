//! The colours documents draw with, as named tokens that follow the
//! application's theme and the platform's dark appearance.
//!
//! A document never picks a hex value. It names a role, such as
//! `bg-surface` or `text-muted`, and the role becomes a Tailwind colour that
//! reads the application's theme variable, `--surface`, with a `dark:` twin
//! that reads `--surface-dark`. When the application sets neither, the
//! token's own light and dark colours show. So the same document matches
//! whichever application shows it, in light and in dark, without the agent
//! knowing either.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use leptoskit_native::Color;

/// The roles every theme has, in the order the reference lists them.
const ROLES: &[(&str, &str)] = &[
	(
		"accent",
		"Primary buttons, selected options, links, and meters.",
	),
	("on-accent", "Text and icons on an accent background."),
	("positive", "Good news, such as money left over."),
	(
		"warning",
		"Something to watch, such as a budget nearly spent.",
	),
	("negative", "Bad news, such as an overspent budget."),
	(
		"surface",
		"Behind cards, stats, fields, and secondary buttons.",
	),
	(
		"line",
		"Hairlines: dividers, chart baselines, meter tracks.",
	),
	(
		"muted",
		"Secondary text: labels, captions, and axis values.",
	),
	("series-1", "The first chart series."),
	("series-2", "The second chart series."),
	("series-3", "The third chart series."),
	("series-4", "The fourth chart series."),
	("series-5", "The fifth chart series."),
	("series-6", "The sixth chart series."),
	("series-7", "The seventh chart series."),
	("series-8", "The eighth chart series."),
];

/// The utilities that take a colour.
const COLOR_UTILITIES: &[&str] = &["bg", "text", "border"];

/// One named colour: the theme variable it reads, and the colours it falls
/// back to when the application does not set that variable.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
	/// The application theme variable, without `--`: the token reads
	/// `--{variable}` in light appearance and `--{variable}-dark`, then
	/// `--{variable}`, in dark.
	pub variable: String,
	/// Shown in light appearance when the variable is not set.
	pub light: Color,
	/// Shown in dark appearance when neither variable is set.
	pub dark: Color,
	/// What the token is for, for the language reference.
	pub summary: String,
}

/// The colours documents draw with, as named roles.
///
/// Documents style with roles, never colours: `bg-surface`,
/// `text-muted`, `border-accent/40`. Each role reads a variable from the
/// application's theme, so an application that already sets
/// `[theme.colors] accent = "#0f766e"` in `leptoskit.toml` sees its own
/// accent in every document. Set `accent-dark` for dark appearance.
///
/// ```
/// use leptoskit_agent_ui::AgentUi;
/// use leptoskit_agent_ui::Theme;
/// use leptoskit_native::Color;
///
/// let theme = Theme::default()
/// 	// Read the application's existing `--brand` and `--brand-dark`.
/// 	.variable("accent", "brand")
/// 	// A role of the application's own, usable as `bg-savings`.
/// 	.color(
/// 		"savings",
/// 		Color::rgba(0x0f, 0x76, 0x6e, 255),
/// 		Color::rgba(0x2d, 0xd4, 0xbf, 255),
/// 		"Money set aside.",
/// 	);
/// let ui = AgentUi::new().theme(theme);
/// # let _ = ui;
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
	tokens: BTreeMap<String, Token>,
	/// The roles in reference order: the built-in ones, then the
	/// application's.
	order: Vec<String>,
}

const fn hex(value: u32) -> Color {
	Color::rgba((value >> 16) as u8, (value >> 8) as u8, value as u8, 255)
}

impl Default for Theme {
	fn default() -> Self {
		// A categorical order checked for colour-blind separation between
		// neighbours, stepped separately for light and dark surfaces: blue,
		// orange, aqua, yellow, magenta, green, violet, red.
		let series = [
			(0x002a_78d6, 0x0039_87e5),
			(0x00eb_6834, 0x00d9_5926),
			(0x001b_af7a, 0x0019_9e70),
			(0x00ed_a100, 0x00c9_8500),
			(0x00e8_7ba4, 0x00d5_5181),
			(0x0000_8300, 0x0000_8300),
			(0x0062_50d6, 0x0090_85e9),
			(0x00e3_4948, 0x00e6_6767),
		];
		let grey = |alpha| Color::rgba(128, 128, 128, alpha);

		let mut colors: Vec<(&str, Color, Color)> = vec![
			("accent", hex(0x002a_78d6), hex(0x0039_87e5)),
			("on-accent", hex(0x00ff_ffff), hex(0x00ff_ffff)),
			// Status colours stay apart from the series, so a state never
			// reads as a series.
			("positive", hex(0x0000_7a00), hex(0x000c_a30c)),
			("warning", hex(0x00a8_6b00), hex(0x00fa_b219)),
			("negative", hex(0x00d0_3b3b), hex(0x00e6_6767)),
			// Translucent, so they sit on whatever surface the application
			// gives the document.
			("surface", grey(24), grey(40)),
			("line", grey(72), grey(96)),
			("muted", hex(0x0052_514e), hex(0x00c3_c2b7)),
		];
		let names: Vec<String> = (1..=8).map(|index| format!("series-{index}")).collect();
		for (name, (light, dark)) in names.iter().zip(series) {
			colors.push((name, hex(light), hex(dark)));
		}

		let mut theme = Self {
			tokens: BTreeMap::new(),
			order: Vec::new(),
		};
		for (role, light, dark) in colors {
			let summary = ROLES
				.iter()
				.find(|(name, _)| *name == role)
				.map_or("", |(_, summary)| summary);
			theme = theme.color(role, light, dark, summary);
		}
		theme
	}
}

impl Theme {
	/// Sets a role's own colours, used when the application's theme does not
	/// set its variable. A role that does not exist yet is added, so
	/// documents can use it as `bg-{role}`, and the reference lists it with
	/// `summary`.
	///
	/// # Panics
	///
	/// When `role` is not lowercase letters, digits, and hyphens, starting
	/// with a letter.
	#[must_use]
	pub fn color(mut self, role: &str, light: Color, dark: Color, summary: &str) -> Self {
		assert!(
			is_role(role),
			"theme roles are lowercase letters, digits, and hyphens: {role:?}"
		);
		if let Some(token) = self.tokens.get_mut(role) {
			token.light = light;
			token.dark = dark;
			if !summary.is_empty() {
				summary.clone_into(&mut token.summary);
			}
		} else {
			self.order.push(role.to_owned());
			self.tokens.insert(
				role.to_owned(),
				Token {
					variable: role.to_owned(),
					light,
					dark,
					summary: summary.to_owned(),
				},
			);
		}
		self
	}

	/// Reads a role from the application theme variable `--{variable}`, and
	/// `--{variable}-dark` in dark appearance, instead of `--{role}`.
	///
	/// # Panics
	///
	/// When the role does not exist, or `variable` is not a CSS custom
	/// property name.
	#[must_use]
	pub fn variable(mut self, role: &str, variable: &str) -> Self {
		let variable = variable.strip_prefix("--").unwrap_or(variable);
		assert!(
			!variable.is_empty()
				&& variable
					.chars()
					.all(|character| character.is_ascii_alphanumeric() || "-_".contains(character)),
			"not a theme variable name: {variable:?}"
		);
		let token = self
			.tokens
			.get_mut(role)
			.unwrap_or_else(|| panic!("no theme role {role:?}; add it with `color` first"));
		variable.clone_into(&mut token.variable);
		self
	}

	/// A role's token.
	pub fn token(&self, role: &str) -> Option<&Token> {
		self.tokens.get(role)
	}

	/// Every role, in reference order.
	pub fn roles(&self) -> impl Iterator<Item = (&str, &Token)> {
		self.order
			.iter()
			.filter_map(|role| Some((role.as_str(), self.tokens.get(role)?)))
	}

	/// The classes that paint `utility` (`bg`, `text`, or `border`) with
	/// `role`. Built-in roles always exist, so the components here call it
	/// with fixed names.
	pub(crate) fn class(&self, utility: &str, role: &str) -> String {
		let mut out = String::new();
		let token = self
			.tokens
			.get(role)
			.unwrap_or_else(|| panic!("no theme role {role:?}"));
		push_token(&mut out, "", utility, token, None, false);
		out
	}

	/// The colour a role shows in light appearance right now: the
	/// application's theme variable, or the token's own. For the few places
	/// that take a colour rather than a class, such as a run of text.
	pub(crate) fn resolve(&self, role: &str) -> Color {
		let token = self
			.tokens
			.get(role)
			.unwrap_or_else(|| panic!("no theme role {role:?}"));
		leptoskit_native::Theme::color(&token.variable).unwrap_or(token.light)
	}

	/// `classes` with every role utility, such as `bg-surface` or
	/// `md:text-accent/80`, replaced by the colour classes that paint it.
	/// Everything else passes through unchanged.
	pub(crate) fn expand(&self, classes: &str) -> String {
		let mut out = String::with_capacity(classes.len() * 2);
		for class in classes.split_whitespace() {
			if !out.is_empty() {
				out.push(' ');
			}
			match self.parse(class) {
				Some((variants, utility, token, alpha)) => {
					let dark_only = variants.split(':').any(|variant| variant == "dark");
					push_token(&mut out, variants, utility, token, alpha, dark_only);
				}
				None => out.push_str(class),
			}
		}
		out
	}

	/// Splits `md:bg-accent/40` into its variants, utility, token, and
	/// opacity, when it names a role.
	fn parse<'a>(
		&'a self,
		class: &'a str,
	) -> Option<(&'a str, &'a str, &'a Token, Option<&'a str>)> {
		let (variants, utility) = match class.rfind(':') {
			Some(index) => class.split_at(index + 1),
			None => ("", class),
		};
		let (name, rest) = utility.split_once('-')?;
		if !COLOR_UTILITIES.contains(&name) {
			return None;
		}
		let (role, alpha) = match rest.split_once('/') {
			Some((role, alpha)) => (role, Some(alpha)),
			None => (rest, None),
		};
		let token = self.tokens.get(role)?;
		Some((variants, name, token, alpha))
	}
}

fn is_role(role: &str) -> bool {
	role.starts_with(|first: char| first.is_ascii_lowercase())
		&& role.chars().all(|character| {
			character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
		})
}

fn hex_color(color: Color) -> String {
	format!(
		"#{:02x}{:02x}{:02x}{:02x}",
		color.red(),
		color.green(),
		color.blue(),
		color.alpha()
	)
}

/// Writes the light class and its `dark:` twin, or only the dark one when the
/// document asked for it under `dark:` itself.
fn push_token(
	out: &mut String,
	variants: &str,
	utility: &str,
	token: &Token,
	alpha: Option<&str>,
	dark_only: bool,
) {
	let variable = &token.variable;
	let alpha = alpha.map(|alpha| format!("/{alpha}")).unwrap_or_default();
	if dark_only {
		let _ = write!(
			out,
			"{variants}{utility}-[var(--{variable}-dark,var(--{variable},{}))]{alpha}",
			hex_color(token.dark)
		);
		return;
	}
	let _ = write!(
		out,
		"{variants}{utility}-[var(--{variable},{})]{alpha} \
		 dark:{variants}{utility}-[var(--{variable}-dark,var(--{variable},{}))]{alpha}",
		hex_color(token.light),
		hex_color(token.dark)
	);
}

#[cfg(test)]
mod tests {
	use leptoskit_native::StyleContext;
	use leptoskit_native::components::View;

	use super::*;

	fn background(classes: &str, dark: bool) -> Option<Color> {
		View()
			.class(classes)
			.resolved(&StyleContext {
				dark,
				..StyleContext::default()
			})
			.props
			.background
	}

	#[test]
	fn roles_become_classes_that_follow_dark_appearance() {
		let theme = Theme::default();
		let classes = theme.expand("p-4 bg-accent md:text-muted/80");
		assert!(
			classes.starts_with("p-4 bg-[var(--accent,#2a78d6ff)]"),
			"{classes}"
		);
		assert!(classes.contains("dark:md:text-[var(--muted-dark,var(--muted,#c3c2b7ff))]/80"));

		leptoskit_native::Theme::clear();
		assert_eq!(background(&classes, false), Some(hex(0x002a_78d6)));
		assert_eq!(background(&classes, true), Some(hex(0x0039_87e5)));
	}

	#[test]
	fn roles_read_the_application_theme() {
		let classes = Theme::default()
			.variable("accent", "brand")
			.expand("bg-accent");
		leptoskit_native::Theme::clear();
		leptoskit_native::Theme::set_color("brand", hex(0x000f_766e));
		// Without `--brand-dark`, dark appearance uses `--brand` too.
		assert_eq!(background(&classes, false), Some(hex(0x000f_766e)));
		assert_eq!(background(&classes, true), Some(hex(0x000f_766e)));
		leptoskit_native::Theme::set_color("brand-dark", hex(0x002d_d4bf));
		assert_eq!(background(&classes, true), Some(hex(0x002d_d4bf)));
		leptoskit_native::Theme::clear();
	}

	#[test]
	fn other_classes_pass_through() {
		let theme = Theme::default();
		assert_eq!(
			theme.expand("text-sm bg-red-500 border"),
			"text-sm bg-red-500 border"
		);
		assert_eq!(
			theme.expand("dark:bg-surface"),
			"dark:bg-[var(--surface-dark,var(--surface,#80808028))]"
		);
	}

	#[test]
	fn applications_add_roles() {
		let theme = Theme::default().color("savings", hex(0x000f_766e), hex(0x002d_d4bf), "Saved.");
		assert!(theme.expand("bg-savings").contains("--savings"));
		assert_eq!(theme.roles().last().map(|(role, _)| role), Some("savings"));
	}
}
