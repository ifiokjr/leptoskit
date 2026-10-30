//! Independent Core Text measurement with the same weighted system fonts as
//! UIKit.

use std::cell::RefCell;

use leptoskit_native::FontWeight;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;
use objc2::rc::Retained;
use objc2_core_foundation::CFAttributedString;
use objc2_core_foundation::CFDictionary;
use objc2_core_foundation::CFRange;
use objc2_core_foundation::CFRetained;
use objc2_core_foundation::CFString;
use objc2_core_foundation::CFType;
use objc2_core_foundation::CGPoint;
use objc2_core_foundation::CGRect;
use objc2_core_foundation::CGSize;
use objc2_core_graphics::CGPath;
use objc2_core_text::CTFont;
use objc2_core_text::CTFramesetter;
use objc2_core_text::CTParagraphStyle;
use objc2_core_text::CTParagraphStyleSetting;
use objc2_core_text::CTParagraphStyleSpecifier;
use objc2_core_text::CTWritingDirection;
use objc2_core_text::kCTFontAttributeName;
use objc2_core_text::kCTParagraphStyleAttributeName;
use objc2_ui_kit::UIFont;

/// Core Text string attributes: a font and a paragraph style.
type Attributes = CFRetained<CFDictionary<CFString, CFType>>;

/// Measures text with Core Text, typesetting each call independently.
///
/// Fonts and attributes are immutable and cached per thread; no UIKit or
/// layout object is shared across threads. Core Text permits concurrent
/// operations when each operation owns its own framesetter.
#[derive(Debug, Clone, Copy, Default)]
pub struct UIKitTextMeasurer;

impl UIKitTextMeasurer {
	const CACHED_STYLES: usize = 64;

	/// Creates a stateless measurer.
	pub const fn new() -> Self {
		Self
	}

	/// The font and Core Text attributes for `style`, made once per thread:
	/// a screen measures many labels in a few styles, and building the font,
	/// paragraph style and dictionary for each cost as much as typesetting.
	fn attributes(style: TextStyle) -> (Retained<UIFont>, Attributes) {
		thread_local! {
			static CACHE: RefCell<Vec<(TextStyle, Retained<UIFont>, Attributes)>> =
				const { RefCell::new(Vec::new()) };
		}

		if let Some(found) = CACHE.with_borrow(|cache| {
			cache
				.iter()
				.find(|(cached, ..)| *cached == style)
				.map(|(_, font, attributes)| (font.clone(), attributes.clone()))
		}) {
			return found;
		}

		// UIFont is immutable and supports creation on any thread. Bridge the
		// exact drawing font rather than reconstructing its private descriptor.
		let ui_font = UIFont::systemFontOfSize_weight(
			f64::from(style.font_size),
			font_weight(style.font_weight),
		);
		let font: &CTFont = ui_font.as_ref();
		// SAFETY: Core Text accepts its documented font and paragraph-style
		// keys and values; the objects are immutable once made.
		let attributes = unsafe {
			let direction = match style.direction {
				leptoskit_yoga::Direction::Ltr => CTWritingDirection::LeftToRight,
				leptoskit_yoga::Direction::Rtl => CTWritingDirection::RightToLeft,
			};
			let setting = CTParagraphStyleSetting {
				spec: CTParagraphStyleSpecifier::BaseWritingDirection,
				valueSize: size_of_val(&direction),
				value: std::ptr::NonNull::from(&direction).cast(),
			};
			let paragraph = CTParagraphStyle::new(&raw const setting, 1);
			let values: [&CFType; 2] = [font, &paragraph];
			CFDictionary::from_slices(
				&[kCTFontAttributeName, kCTParagraphStyleAttributeName],
				&values,
			)
		};
		CACHE.with_borrow_mut(|cache| {
			// A handful of styles per app; a bound keeps a pathological one
			// small.
			if cache.len() >= Self::CACHED_STYLES {
				cache.clear();
			}
			cache.push((style, ui_font.clone(), attributes.clone()));
		});
		(ui_font, attributes)
	}

	fn size(text: &str, style: TextStyle, width: f64) -> CGSize {
		let (ui_font, attributes) = Self::attributes(style);
		// SAFETY: The attributes hold only Core Text's documented keys and
		// values, and every other object is local to this measurement.
		unsafe {
			// UILabel also lays out an empty line after a trailing line break.
			let trailing_break = isize::from(text.ends_with(['\n', '\r', '\u{2028}', '\u{2029}']));
			let text = CFString::from_str(if text.is_empty() { " " } else { text });
			let string = CFAttributedString::new(None, Some(&text), Some(attributes.as_opaque()))
				.expect("valid Core Text attributed string");
			let framesetter = CTFramesetter::with_attributed_string(&string);
			let bounds = CGSize::new(width.max(0.01), 1.0e7);
			let size = framesetter.suggest_frame_size_with_constraints(
				CFRange::new(0, 0),
				None,
				bounds,
				std::ptr::null_mut(),
			);
			// UILabel advances each line by the font's lineHeight, which is
			// taller than Core Text's natural line pitch for the system font.
			// Measuring only Core Text clips the last line of wrapped labels.
			let path = CGPath::with_rect(
				CGRect::new(CGPoint::new(0.0, 0.0), bounds),
				std::ptr::null(),
			);
			let lines = framesetter
				.frame(CFRange::new(0, 0), &path, None)
				.lines()
				.count()
				+ trailing_break;
			CGSize::new(
				size.width,
				size.height.max(lines as f64 * ui_font.lineHeight()),
			)
		}
	}
}

pub(crate) const fn font_weight(weight: FontWeight) -> f64 {
	match weight {
		FontWeight::Regular => 0.0,
		FontWeight::Medium => 0.23,
		FontWeight::Semibold => 0.3,
		FontWeight::Bold => 0.4,
	}
}

impl TextMeasurer for UIKitTextMeasurer {
	fn measure(&self, text: &str, style: TextStyle, available: Size<AvailableSpace>) -> Size<f32> {
		let width = match available.width {
			AvailableSpace::Definite(width) => f64::from(width),
			AvailableSpace::MaxContent => 1.0e7,
			AvailableSpace::MinContent => {
				text.split_whitespace()
					.map(|word| Self::size(word, style, 1.0e7).width)
					.fold(0.0, f64::max)
			}
		};
		let size = Self::size(text, style, width);
		Size::new(
			if text.is_empty() {
				0.0
			} else {
				size.width.ceil() as f32
			},
			size.height.ceil() as f32,
		)
	}
}
