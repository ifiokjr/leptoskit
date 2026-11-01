//! UIKit text attributes matching the shared Core Text measurement contract.

use leptoskit_native::TextAlign;
use leptoskit_native::TextStyle;
use leptoskit_yoga::Direction;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::NSAttributedString;
use objc2_foundation::NSAttributedStringKey;
use objc2_foundation::NSDictionary;
use objc2_foundation::NSString;
use objc2_ui_kit::NSFontAttributeName;
use objc2_ui_kit::NSForegroundColorAttributeName;
use objc2_ui_kit::NSLineBreakMode;
use objc2_ui_kit::NSMutableParagraphStyle;
use objc2_ui_kit::NSParagraphStyleAttributeName;
use objc2_ui_kit::NSTextAlignment;
use objc2_ui_kit::NSWritingDirection;
use objc2_ui_kit::UIColor;
use objc2_ui_kit::UIFont;

use crate::measure::font_weight;

pub(crate) fn alignment(style: TextStyle) -> NSTextAlignment {
	match (style.text_align, style.direction) {
		(TextAlign::Center, _) => NSTextAlignment::Center,
		(TextAlign::Start, Direction::Ltr) | (TextAlign::End, Direction::Rtl) => {
			NSTextAlignment::Left
		}
		(TextAlign::Start, Direction::Rtl) | (TextAlign::End, Direction::Ltr) => {
			NSTextAlignment::Right
		}
	}
}

pub(crate) fn attributed(
	text: &str,
	style: TextStyle,
	color: &UIColor,
) -> Retained<NSAttributedString> {
	let font =
		UIFont::systemFontOfSize_weight(f64::from(style.font_size), font_weight(style.font_weight));
	let paragraph = NSMutableParagraphStyle::new();
	paragraph.setAlignment(alignment(style));
	paragraph.setBaseWritingDirection(match style.direction {
		Direction::Ltr => NSWritingDirection::LeftToRight,
		Direction::Rtl => NSWritingDirection::RightToLeft,
	});

	paragraph.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
	// SAFETY: UIKit exports immutable attribute keys with the documented font,
	// UIColor, and paragraph-style value types supplied in this dictionary.
	unsafe {
		let keys: [&NSAttributedStringKey; 3] = [
			NSFontAttributeName,
			NSForegroundColorAttributeName,
			NSParagraphStyleAttributeName,
		];
		let values: [&AnyObject; 3] = [&font, color, &paragraph];
		let attributes = NSDictionary::from_slices(&keys, &values);
		NSAttributedString::initWithString_attributes(
			NSAttributedString::alloc(),
			&NSString::from_str(text),
			Some(&attributes),
		)
	}
}
