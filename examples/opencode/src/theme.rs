//! opencode's default dark theme, as its terminal interface paints it.
//!
//! Containers take these colors as Tailwind classes (`bg-[#141414]`); styled
//! text runs take them as values.

use leptoskit_native::Color;

/// An opaque color from `0xrrggbb`.
const fn rgb(value: u32) -> Color {
	Color((value << 8) | 0xff)
}

/// Body text.
pub const TEXT: Color = rgb(0x00ee_eeee);
/// Secondary text: hints, paths, tool titles.
pub const MUTED: Color = rgb(0x0080_8080);
/// The accent orange: list bullets, selections, function names.
pub const PRIMARY: Color = rgb(0x00fa_b283);
/// The agent blue: the prompt bar and the Build agent.
pub const SECONDARY: Color = rgb(0x005c_9cf5);
/// The purple of section headers and keywords.
pub const ACCENT: Color = rgb(0x009d_7cd8);
/// Inline code and strings.
pub const SUCCESS: Color = rgb(0x007f_d88f);
/// Bold markdown, numbers, and bindings.
pub const WARNING: Color = rgb(0x00f5_a742);
/// `self` and other special variables.
pub const ERROR: Color = rgb(0x00e0_6c75);
/// Operators and punctuation.
pub const INFO: Color = rgb(0x0056_b6c2);
/// Type names.
pub const TYPE: Color = rgb(0x00e5_c07b);
/// The page background, which the selected palette row's text uses.
pub const BACKGROUND: Color = rgb(0x000a_0a0a);

/// The shading inside the logo's dim "open" letters.
pub const LOGO_OPEN_SHADE: Color = rgb(0x0028_2828);
/// The shading inside the logo's bright "code" letters.
pub const LOGO_CODE_SHADE: Color = rgb(0x0043_4343);

/// Diff line numbers.
pub const DIFF_LINE_NUMBER: Color = rgb(0x008f_8f8f);
/// The `+` of an added line.
pub const DIFF_ADDED_SIGN: Color = rgb(0x00b8_db87);
/// The `-` of a removed line.
pub const DIFF_REMOVED_SIGN: Color = rgb(0x00e2_6a75);
