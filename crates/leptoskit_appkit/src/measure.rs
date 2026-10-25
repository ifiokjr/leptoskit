//! Text measurement with real AppKit font metrics.

use leptoskit_native::TextAlign;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;
use objc2_app_kit::NSAttributedStringNSExtendedStringDrawing;
use objc2_app_kit::NSStringDrawingOptions;
use objc2_core_foundation::CGSize;

use crate::text::attributed_string;
use crate::text::text_color;

/// The horizontal inset an `NSTextField` label draws its text with, on each
/// side: the line fragment padding of its text container.
const LABEL_HORIZONTAL_INSET: f64 = 2.0;

/// The width a label cell reserves around its text, by alignment.
///
/// A cell setting a centered paragraph reserves the padding twice on each
/// side; given only the start-aligned width it wraps the last word. Measured
/// against `cellSizeForBounds:` across sizes and strings.
fn label_insets(align: TextAlign) -> f64 {
	match align {
		TextAlign::Center => 4.0 * LABEL_HORIZONTAL_INSET,
		TextAlign::Start | TextAlign::End => 2.0 * LABEL_HORIZONTAL_INSET,
	}
}

/// A width no interface reaches, standing in for "unbounded" when asking
/// AppKit for a single-line size.
const UNBOUNDED: f64 = 1.0e7;

/// Regular rounded `NSButton` cells reserve this title inset and minimum size.
/// Assigned-frame native/raster controls cover 10–42 point system fonts.
const BUTTON_TITLE_INSET: f64 = 24.0;
const BUTTON_MINIMUM_WIDTH: f64 = 34.0;
const BUTTON_HEIGHT: f64 = 24.0;

/// Measures text with the system font AppKit draws it with.
///
/// Sizes come from `NSAttributedString`'s `boundingRectWithSize:options:`
/// with line-fragment origins, which sets lines exactly as an `NSTextField`
/// label cell does, and include the horizontal insets the cell reserves for
/// the style's alignment, so a label given the measured frame draws its text
/// without clipping or rewrapping.
/// Sizes are rounded up to whole points.
///
/// The measurer holds no AppKit objects, so it is `Send` and `Sync` and can
/// measure on any thread; string measurement is thread-safe in AppKit.
///
/// Rounded buttons reserve the native regular control's 24-point height.
/// Large glyphs can paint outside that box, as native buttons do;
/// a caller that clips its parent must reserve any desired extra space.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppKitTextMeasurer;

impl AppKitTextMeasurer {
	/// A measurer.
	pub const fn new() -> Self {
		Self
	}

	/// The size of `text` wrapped to `max_width` (excluding the label inset).
	fn bounding_size(text: &str, style: TextStyle, max_width: f64) -> (f64, f64) {
		let (width, height) = Self::bounding_extent(text, style, max_width);
		(width.ceil(), height.ceil())
	}

	/// Unrounded title extent also preserves native half-point button widths.
	fn bounding_extent(text: &str, style: TextStyle, max_width: f64) -> (f64, f64) {
		let string = attributed_string(text, style, &text_color(None));
		let rect = string.boundingRectWithSize_options_context(
			CGSize::new(max_width.max(0.0), UNBOUNDED),
			// Without font leading: this is how an `NSTextField` cell sets
			// its lines, and at some sizes (15 and 16 points) the two differ
			// by a point per line.
			NSStringDrawingOptions::UsesLineFragmentOrigin,
			None,
		);
		(rect.size.width, rect.size.height)
	}
}

impl TextMeasurer for AppKitTextMeasurer {
	fn measure_button(
		&self,
		text: &str,
		style: TextStyle,
		available: Size<AvailableSpace>,
		native_chrome: bool,
	) -> Size<f32> {
		let (title_width, title_height) = Self::bounding_extent(text, style, UNBOUNDED);
		let (width, height) = if native_chrome {
			(
				((title_width * 2.0).ceil() / 2.0 + BUTTON_TITLE_INSET).max(BUTTON_MINIMUM_WIDTH),
				BUTTON_HEIGHT,
			)
		} else {
			(title_width.ceil(), title_height.ceil().max(1.0))
		};

		let width = match available.width {
			AvailableSpace::Definite(limit) => width.min(f64::from(limit.max(0.0))),
			AvailableSpace::MinContent | AvailableSpace::MaxContent => width,
		};

		Size::new(width as f32, height as f32)
	}

	fn measure(&self, text: &str, style: TextStyle, available: Size<AvailableSpace>) -> Size<f32> {
		let inset = label_insets(style.text_align);

		let (width, height) = match available.width {
			AvailableSpace::Definite(width) => {
				Self::bounding_size(text, style, f64::from(width) - inset)
			}
			AvailableSpace::MaxContent => Self::bounding_size(text, style, UNBOUNDED),
			AvailableSpace::MinContent => {
				// The min-content width is the widest word: wrapping can
				// never make a line narrower than that.
				let widest_word = text
					.split_whitespace()
					.map(|word| Self::bounding_size(word, style, UNBOUNDED).0)
					.fold(0.0, f64::max);
				Self::bounding_size(text, style, widest_word)
			}
		};

		// An empty string still occupies one line, like a label does.
		let height = if text.is_empty() {
			Self::bounding_size(" ", style, UNBOUNDED).1
		} else {
			height
		};

		Size::new((width + inset) as f32, height as f32)
	}
}
