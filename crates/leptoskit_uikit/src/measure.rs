//! Independent Core Text measurement with the same weighted system fonts as
//! UIKit.

use leptoskit_native::FontWeight;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;
use objc2_core_foundation::CFAttributedString;
use objc2_core_foundation::CFDictionary;
use objc2_core_foundation::CFRange;
use objc2_core_foundation::CFString;
use objc2_core_foundation::CFType;
use objc2_core_foundation::CGSize;
use objc2_core_text::CTFont;
use objc2_core_text::CTFramesetter;
use objc2_core_text::CTParagraphStyle;
use objc2_core_text::CTParagraphStyleSetting;
use objc2_core_text::CTParagraphStyleSpecifier;
use objc2_core_text::CTWritingDirection;
use objc2_core_text::kCTFontAttributeName;
use objc2_core_text::kCTParagraphStyleAttributeName;
use objc2_ui_kit::UIFont;

/// Measures text using independent Core Text layout values per call.
///
/// No UIKit object or layout object is shared across threads. Core Text permits
/// concurrent operations when each operation owns its own framesetter.
#[derive(Debug, Clone, Copy, Default)]
pub struct UIKitTextMeasurer;

impl UIKitTextMeasurer {
	/// Creates a stateless measurer.
	pub const fn new() -> Self {
		Self
	}

	fn size(text: &str, style: TextStyle, width: f64) -> CGSize {
		// UIFont is immutable and supports creation on any thread. Bridge the
		// exact drawing font rather than reconstructing its private descriptor.
		let ui_font = UIFont::systemFontOfSize_weight(
			f64::from(style.font_size),
			font_weight(style.font_weight),
		);
		let font: &CTFont = ui_font.as_ref();
		// SAFETY: All objects are local to this measurement. Core Text accepts
		// its documented font and paragraph-style keys and values.
		unsafe {
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
			let attributes = CFDictionary::from_slices(
				&[kCTFontAttributeName, kCTParagraphStyleAttributeName],
				&values,
			);
			let text = CFString::from_str(if text.is_empty() { " " } else { text });
			let string = CFAttributedString::new(None, Some(&text), Some(attributes.as_opaque()))
				.expect("valid Core Text attributed string");
			let framesetter = CTFramesetter::with_attributed_string(&string);
			framesetter.suggest_frame_size_with_constraints(
				CFRange::new(0, 0),
				None,
				CGSize::new(width.max(0.01), 1.0e7),
				std::ptr::null_mut(),
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
