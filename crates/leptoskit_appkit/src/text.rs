//! Fonts, colors, and attributed strings shared by measurement and drawing.
//!
//! Measurement and the widgets build their attributed strings here, from the
//! same [`TextStyle`], so the size layout reserves is the size AppKit draws.

use leptoskit_native::Color;
use leptoskit_native::FontWeight;
use leptoskit_native::TextAlign;
use leptoskit_native::TextStyle;
use leptoskit_yoga::Direction;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSColor;
use objc2_app_kit::NSFont;
use objc2_app_kit::NSFontAttributeName;
use objc2_app_kit::NSFontWeightBold;
use objc2_app_kit::NSFontWeightMedium;
use objc2_app_kit::NSFontWeightRegular;
use objc2_app_kit::NSFontWeightSemibold;
use objc2_app_kit::NSForegroundColorAttributeName;
use objc2_app_kit::NSLineBreakMode;
use objc2_app_kit::NSMutableParagraphStyle;
use objc2_app_kit::NSParagraphStyleAttributeName;
use objc2_app_kit::NSTextAlignment;
use objc2_app_kit::NSWritingDirection;
use objc2_foundation::NSAttributedString;
use objc2_foundation::NSAttributedStringKey;
use objc2_foundation::NSDictionary;
use objc2_foundation::NSString;

/// The system font at the style's size and weight.
pub(crate) fn system_font(style: TextStyle) -> Retained<NSFont> {
	// SAFETY: the weight constants are immutable `CGFloat` statics exported by
	// AppKit, initialized before any code in this process runs.
	let weight = unsafe {
		match style.font_weight {
			FontWeight::Regular => NSFontWeightRegular,
			FontWeight::Medium => NSFontWeightMedium,
			FontWeight::Semibold => NSFontWeightSemibold,
			FontWeight::Bold => NSFontWeightBold,
		}
	};
	NSFont::systemFontOfSize_weight(f64::from(style.font_size), weight)
}

/// An sRGB color from a packed leptoskit color.
pub(crate) fn ns_color(color: Color) -> Retained<NSColor> {
	NSColor::colorWithSRGBRed_green_blue_alpha(
		f64::from(color.red()) / 255.0,
		f64::from(color.green()) / 255.0,
		f64::from(color.blue()) / 255.0,
		f64::from(color.alpha()) / 255.0,
	)
}

/// Resolves logical alignment against the node's layout direction.
pub(crate) fn ns_alignment(style: TextStyle) -> NSTextAlignment {
	match (style.text_align, style.direction) {
		(TextAlign::Start, Direction::Ltr) | (TextAlign::End, Direction::Rtl) => {
			NSTextAlignment::Left
		}
		(TextAlign::Center, _) => NSTextAlignment::Center,
		(TextAlign::End, Direction::Ltr) | (TextAlign::Start, Direction::Rtl) => {
			NSTextAlignment::Right
		}
	}
}

/// The color text is drawn in: the given color, else the system label
/// color, which adapts to light and dark appearances.
pub(crate) fn text_color(color: Option<Color>) -> Retained<NSColor> {
	color.map_or_else(NSColor::labelColor, ns_color)
}

/// Builds the attributed string AppKit measures and draws: the style's font
/// and alignment, wrapping at word boundaries.
pub(crate) fn attributed_string(
	text: &str,
	style: TextStyle,
	color: &NSColor,
) -> Retained<NSAttributedString> {
	let font = system_font(style);
	let paragraph = NSMutableParagraphStyle::new();
	paragraph.setAlignment(ns_alignment(style));
	paragraph.setBaseWritingDirection(match style.direction {
		Direction::Ltr => NSWritingDirection::LeftToRight,
		Direction::Rtl => NSWritingDirection::RightToLeft,
	});

	paragraph.setLineBreakMode(NSLineBreakMode::ByWordWrapping);

	// SAFETY: the attribute name keys are immutable `NSString` statics
	// exported by AppKit.
	let keys: [&NSAttributedStringKey; 3] = unsafe {
		[
			NSFontAttributeName,
			NSForegroundColorAttributeName,
			NSParagraphStyleAttributeName,
		]
	};
	let values: [&AnyObject; 3] = [&font, color, &paragraph];
	let dictionary = NSDictionary::from_slices(&keys, &values);
	// SAFETY: every value has the type its key documents: an `NSFont` for the
	// font, an `NSColor` for the foreground color, and an `NSParagraphStyle`
	// for the paragraph style.
	unsafe {
		NSAttributedString::initWithString_attributes(
			NSAttributedString::alloc(),
			&NSString::from_str(text),
			Some(&dictionary),
		)
	}
}
