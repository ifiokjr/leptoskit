//! Built-in, platform-neutral Tailwind CSS utilities.
//!
//! This compiler targets the default Tailwind **4.3** spacing scale (one unit
//! is `0.25rem`), breakpoints, type sizes, radii, and color palette. It parses
//! once into typed declarations; native commits only resolve viewport units
//! and conditions. Native `rem` is 16 logical pixels. [`Tailwind::css`] emits
//! the same declarations for browser rendering without a Node build step.
//!
//! Supported families are flexbox, grid, sizing, spacing, positioning, gaps,
//! aspect ratio, box sizing, and the visual properties in [`NativeProps`].
//! Safe-area padding follows the `tailwindcss-safe-area` naming: `pt-safe`
//! (and `p`, `px`, `py`, `pr`, `pb`, `pl`) pads by the host's inset on each
//! edge, and `pt-safe-offset-4` adds a spacing value to it. The web emits
//! `env(safe-area-inset-*)`, which needs `viewport-fit=cover`; native hosts
//! supply [`StyleContext::safe_area`].
//! `sm`/`md`/`lg`/`xl`/`2xl`, their `max-*` counterparts, arbitrary pixel
//! widths (`min-[968px]`, `max-[968px]`), `dark`, `active`, and trailing `!`
//! (or the older leading `!`) are supported. Dark mode follows the system
//! preference on the web; native hosts supply [`StyleContext::dark`]. `active`
//! styles an enabled control while it is held down: CSS `:active` on the web,
//! and [`StyleContext::active`] on native, which the surface sets for the
//! control the user is pressing.
//!
//! Classes have deterministic stylesheet precedence, independent of their
//! input order: base rules precede conditional rules, larger minimum widths
//! override smaller ones, broad spacing precedes axes and individual edges,
//! pressed rules override all of those (as the extra specificity of `:active`
//! does in CSS), and important rules win. Conflicting utilities of equal rank
//! use lexical class order. As with Tailwind, avoid specifying competing
//! utilities for the same property and variant. The exact ordering of such
//! conflicts is not a compatibility promise with Tailwind's upstream compiler.
//!
//! This is an explicitly supported subset, not a browser CSS engine. Unknown
//! utilities and variants are errors even when their conditions are inactive.
//! Block/inline/contents flow, fixed/sticky positioning, baseline alignment,
//! overflow on individual axes, other pseudo states, custom themes, arbitrary
//! selectors/properties, CSS variables/functions, animations, filters, and
//! shadows are rejected. Text sizes set font size only: line height is not yet
//! a native text prop. Colors use the default palette converted to 8-bit sRGB,
//! clipping colors outside its gamut, so native and web use the same color
//! values. Overflow clipping is supported. The surface lowers `overflow-scroll`
//! containers to native scroll widgets.
//!
//! Numeric inputs and computed fractions have magnitude at most 1,000,000;
//! aspect ratios range from 0.000001 to 1,000,000. Native viewport lengths
//! resolve with wider arithmetic and saturate at one billion logical pixels.
//! Nonfinite viewport dimensions resolve to zero. These bounds keep extreme
//! inputs from introducing infinity into CSS or native layout declarations.
//! Automatic min/max sizing is rejected; `max-w-none` and `max-h-none` remove
//! maximum constraints. Width, height, and flex basis still support `auto`.
//!
//! Defaults and utility semantics are grounded in the official references:
//! <https://tailwindcss.com/docs/theme>,
//! <https://tailwindcss.com/docs/responsive-design>, and
//! <https://tailwindcss.com/docs/styling-with-utility-classes>.

mod declaration;
pub(crate) mod keyword;
mod palette;
mod utility;
mod value;

use std::cmp::Reverse;
use std::fmt;
use std::fmt::Write;
use std::sync::Arc;

use leptoskit_yoga::Rect;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;

use self::declaration::Declaration;
use crate::Color;
use crate::NativeProps;

/// Runtime inputs needed to resolve conditional utilities and viewport units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StyleContext {
	/// The viewport size in logical pixels, independent of the containing box.
	pub viewport: Size<f32>,
	/// Whether the platform currently uses a dark appearance.
	pub dark: bool,
	/// The viewport edges covered by system UI, such as a phone's status bar,
	/// display cutout, or home indicator, in logical pixels. `*-safe`
	/// padding resolves against these; windows without such UI leave them
	/// zero. Negative and nonfinite insets resolve to zero.
	pub safe_area: Rect<f32>,
	/// Whether the control being styled is held down, so `active:` utilities
	/// apply, as CSS `:active` does while a button is pressed. Resolving a
	/// whole tree with it set shows every `Button` and `Pressable` pressed;
	/// [`NativeSurface`](crate::NativeSurface) sets it only for the control
	/// the user holds.
	pub active: bool,
}

/// The most distinct class strings [`Tailwind::parse`] keeps parsed.
const PARSED_LIMIT: usize = 4096;

thread_local! {
	/// Parsed class strings, by their exact text.
	static PARSED: std::cell::RefCell<std::collections::HashMap<Box<str>, Tailwind>> =
		std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A validated, reusable set of platform-neutral utility declarations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tailwind {
	classes: Arc<str>,
	rules: Arc<[Rule]>,
}

/// Why a utility class cannot be compiled on every supported renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassError {
	/// The complete class token containing the error.
	pub class: String,
	/// The unsupported syntax or invalid value.
	pub reason: String,
}

impl fmt::Display for ClassError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"invalid Tailwind class `{}`: {}",
			self.class, self.reason
		)
	}
}

impl std::error::Error for ClassError {}

#[derive(Debug, Clone, PartialEq)]
struct Rule {
	class: String,
	conditions: Conditions,
	important: bool,
	order: u16,
	declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Conditions {
	min_width: Option<u16>,
	max_width: Option<u16>,
	dark: bool,
	active: bool,
}

impl Tailwind {
	/// Compiles and validates every utility, including inactive variants.
	///
	/// Whitespace and duplicate tokens are normalized. Unsupported utilities,
	/// variants, malformed values, and contradictory breakpoint ranges return
	/// an error rather than silently producing a different interface.
	///
	/// # Errors
	///
	/// Returns [`ClassError`] describing the offending class and reason.
	pub fn parse(classes: &str) -> Result<Self, ClassError> {
		// Apps repeat a small set of class strings across many elements and
		// every render; parsing is the costly part, especially inside the
		// native guest's interpreter, so each distinct string parses once.
		if let Some(parsed) = PARSED.with_borrow(|parsed| parsed.get(classes).cloned()) {
			return Ok(parsed);
		}

		let parsed = Self::parse_uncached(classes)?;
		PARSED.with_borrow_mut(|cache| {
			// Generated class strings could grow without bound; starting over
			// keeps the cache small, and the next render refills it.
			if cache.len() >= PARSED_LIMIT {
				cache.clear();
			}
			cache.insert(classes.into(), parsed.clone());
		});
		Ok(parsed)
	}

	fn parse_uncached(classes: &str) -> Result<Self, ClassError> {
		let mut tokens = classes.split_whitespace().collect::<Vec<_>>();
		tokens.sort_unstable();
		tokens.dedup();
		let classes = tokens.join(" ");
		let mut rules = Vec::with_capacity(tokens.len());

		for token in tokens {
			let rule = Rule::parse(token).map_err(|reason| {
				ClassError {
					class: token.to_owned(),
					reason,
				}
			})?;
			rules.push(rule);
		}

		rules.sort_by(|a, b| a.precedence().cmp(&b.precedence()));
		Ok(Self {
			classes: classes.into(),
			rules: rules.into(),
		})
	}

	/// The normalized class attribute, with duplicate tokens removed.
	pub fn classes(&self) -> &str {
		&self.classes
	}

	/// The first class styling the pressed state, which only pressable
	/// controls have.
	pub(crate) fn active_class(&self) -> Option<&str> {
		self.rules
			.iter()
			.find(|rule| rule.conditions.active)
			.map(|rule| rule.class.as_str())
	}

	/// Structural contracts prohibit scrollable descendants of buttons at
	/// every breakpoint, including rules inactive in the current environment.
	pub(crate) fn may_scroll(&self) -> bool {
		self.rules.iter().any(|rule| {
			rule.declarations.iter().any(|declaration| {
				matches!(
					declaration,
					Declaration::Overflow(leptoskit_yoga::Overflow::Scroll)
				)
			})
		})
	}

	/// Applies active declarations over the supplied base style and props.
	///
	/// Start from the same unmodified base values on each call: rules which
	/// become inactive do not reset declarations from an earlier application.
	pub fn apply(&self, style: &mut Style, props: &mut NativeProps, context: &StyleContext) {
		for rule in self.rules.iter() {
			if rule.conditions.active(context) {
				for declaration in &rule.declarations {
					declaration.apply(style, props, context);
				}
			}
		}

		if props.border_width.is_some_and(|width| width > 0.0) && props.border_color.is_none() {
			props.border_color = Some(props.color.unwrap_or(Color::rgba(0, 0, 0, 255)));
		}
	}

	/// Generates escaped utility selectors and conditional CSS rules.
	///
	/// Relative units stay relative. The caller installs the returned sheet
	/// once per document and applies [`classes`](Self::classes) to its node.
	/// Primitive base styles, including their native flex defaults, belong to
	/// the web adapter rather than this utility stylesheet.
	pub fn css(&self) -> String {
		self.stylesheet(None)
	}

	/// Generates utilities restricted to a trusted CSS selector, such as
	/// `[data-lk-style="hash"]`, so independent component styles cannot
	/// conflict.
	///
	/// `scope` is a selector supplied by the web adapter, not a utility value.
	/// It is wrapped in `:where()` to preserve utility specificity.
	pub fn css_scoped(&self, scope: &str) -> String {
		self.stylesheet(Some(scope))
	}

	fn stylesheet(&self, scope: Option<&str>) -> String {
		let mut css = String::new();

		for rule in self.rules.iter() {
			let conditions = rule.conditions;
			let mut wrappers = 0;

			if conditions.dark {
				css.push_str("@media (prefers-color-scheme:dark){");
				wrappers += 1;
			}

			if let Some(min) = conditions.min_width {
				let _ = write!(css, "@media (width>={}rem){{", f32::from(min) / 16.0);
				wrappers += 1;
			}

			if let Some(max) = conditions.max_width {
				let _ = write!(css, "@media (width<{}rem){{", f32::from(max) / 16.0);
				wrappers += 1;
			}

			if let Some(scope) = scope {
				let _ = write!(css, ":where({scope})");
			}

			css.push('.');
			escape_identifier(&rule.class, &mut css);

			// Disabled links render without an `href`, which browsers would
			// still mark `:active` under the pointer; native controls never
			// press while disabled.
			if conditions.active {
				css.push_str(":active:not([aria-disabled=true])");
			}

			css.push('{');

			for declaration in &rule.declarations {
				declaration.css(&mut css, rule.important);
			}

			css.push('}');

			for _ in 0..wrappers {
				css.push('}');
			}
		}

		css
	}
}

impl Rule {
	fn parse(class: &str) -> Result<Self, String> {
		let mut parts = split_variants(class)?;
		let utility = parts.pop().ok_or("empty utility")?;
		let mut conditions = Conditions::default();

		for variant in parts {
			if variant == "dark" {
				conditions.dark = true;
				continue;
			}

			if variant == "active" {
				conditions.active = true;
				continue;
			}

			if let Some(name) = variant.strip_prefix("max-") {
				let breakpoint =
					breakpoint(name).ok_or_else(|| format!("unsupported variant `{variant}`"))?;
				conditions.max_width = Some(
					conditions
						.max_width
						.map_or(breakpoint, |old| old.min(breakpoint)),
				);
				continue;
			}

			// `min-[968px]` is an arbitrary minimum, as `md` is a named one.
			let minimum = variant
				.strip_prefix("min-")
				.filter(|name| name.starts_with('['))
				.unwrap_or(variant);
			let breakpoint =
				breakpoint(minimum).ok_or_else(|| format!("unsupported variant `{variant}`"))?;
			conditions.min_width = Some(
				conditions
					.min_width
					.map_or(breakpoint, |old| old.max(breakpoint)),
			);
		}

		if matches!((conditions.min_width, conditions.max_width), (Some(min), Some(max)) if min >= max)
		{
			return Err("breakpoint range is empty".to_owned());
		}

		let important = utility.starts_with('!') || utility.ends_with('!');
		let utility = utility.strip_prefix('!').unwrap_or(utility);
		let utility = utility.strip_suffix('!').unwrap_or(utility);
		let (order, declarations) = utility::parse(utility)?;

		Ok(Self {
			class: class.to_owned(),
			conditions,
			important,
			order,
			declarations,
		})
	}

	fn precedence(&self) -> (bool, bool, bool, u16, Reverse<u16>, u16, &str) {
		(
			self.important,
			self.conditions.active,
			self.conditions.dark,
			self.conditions.min_width.unwrap_or(0),
			Reverse(self.conditions.max_width.unwrap_or(u16::MAX)),
			self.order,
			&self.class,
		)
	}
}

impl Conditions {
	fn active(self, context: &StyleContext) -> bool {
		(!self.dark || context.dark)
			&& (!self.active || context.active)
			&& self
				.min_width
				.is_none_or(|min| context.viewport.width >= f32::from(min))
			&& self
				.max_width
				.is_none_or(|max| context.viewport.width < f32::from(max))
	}
}

fn breakpoint(name: &str) -> Option<u16> {
	match name {
		"sm" => Some(640),
		"md" => Some(768),
		"lg" => Some(1024),
		"xl" => Some(1280),
		"2xl" => Some(1536),
		// An arbitrary width in pixels: `[968px]`.
		_ => {
			name.strip_prefix('[')?
				.strip_suffix("px]")?
				.parse()
				.ok()
				.filter(|width| *width > 0)
		}
	}
}

/// Splits only outside brackets, so `text-[color:#123]` stays one utility.
fn split_variants(class: &str) -> Result<Vec<&str>, String> {
	let mut parts = Vec::new();
	let mut bracket = false;
	let mut start = 0;

	for (index, character) in class.char_indices() {
		match character {
			'[' if !bracket => bracket = true,
			']' if bracket => bracket = false,
			'[' | ']' => return Err("unbalanced or nested arbitrary-value brackets".to_owned()),
			':' if !bracket => {
				parts.push(&class[start..index]);
				start = index + 1;
			}
			_ => {}
		}
	}

	if bracket {
		return Err("unclosed arbitrary-value bracket".to_owned());
	}

	parts.push(&class[start..]);
	Ok(parts)
}

/// Hex escapes also handle leading digits (`2xl:`), punctuation, and Unicode.
fn escape_identifier(class: &str, output: &mut String) {
	for (index, character) in class.chars().enumerate() {
		if character.is_ascii_alphabetic()
			|| character == '_'
			|| character == '-'
			|| (index > 0 && character.is_ascii_digit())
		{
			output.push(character);
		} else {
			let _ = write!(output, "\\{:x} ", u32::from(character));
		}
	}
}
