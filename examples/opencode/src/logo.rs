//! The two-tone block-letter logo, drawn from glyph masks the way opencode
//! draws it.

use leptoskit::prelude::*;
use leptoskit_native::TextSpan;

use crate::NBSP;
use crate::theme;

/// The dim "open" half. In the masks, `_` is a shaded blank, `^` an upper
/// half block on the shade, and `~` an upper half block in the shade color.
const OPEN: [&str; 4] = [
	"                   ",
	"█▀▀█ █▀▀█ █▀▀█ █▀▀▄",
	"█__█ █__█ █^^^ █__█",
	"▀▀▀▀ █▀▀▀ ▀▀▀▀ ▀~~▀",
];

/// The bright "code" half; see [`OPEN`] for the mask characters.
const CODE: [&str; 4] = [
	"             ▄     ",
	"█▀▀▀ █▀▀█ █▀▀█ █▀▀█",
	"█___ █__█ █__█ █^^^",
	"▀▀▀▀ ▀▀▀▀ ▀▀▀▀ ▀▀▀▀",
];

/// The logo: four rows, each a `Text` of styled runs.
pub fn logo() -> Element {
	let rows = (0..OPEN.len()).map(|row| {
		let mut spans = half(OPEN[row], theme::MUTED, theme::LOGO_OPEN_SHADE, false);
		spans.push(TextSpan::new(NBSP.to_string()));
		spans.extend(half(CODE[row], theme::TEXT, theme::LOGO_CODE_SHADE, true));
		view! { <Text spans=spans /> }
	});

	view! { <View>{rows.collect::<Fragment>()}</View> }
}

/// One half's row as runs, merging neighbours that share a style.
fn half(mask: &str, ink: Color, shade: Color, bold: bool) -> Vec<TextSpan> {
	let mut spans: Vec<TextSpan> = Vec::new();

	for symbol in mask.chars() {
		// Blanks are no-break spaces, so the rows keep their exact width.
		let mut span = match symbol {
			' ' => TextSpan::new(NBSP.to_string()).color(ink),
			'_' => TextSpan::new(NBSP.to_string()).color(ink).background(shade),
			'^' => TextSpan::new("▀").color(ink).background(shade),
			'~' => TextSpan::new("▀").color(shade),
			glyph => TextSpan::new(glyph.to_string()).color(ink),
		};

		if bold {
			span = span.bold();
		}

		match spans.last_mut() {
			Some(last)
				if last.color == span.color
					&& last.background == span.background
					&& last.font_weight == span.font_weight =>
			{
				last.text.push_str(&span.text);
			}
			_ => spans.push(span),
		}
	}

	spans
}
