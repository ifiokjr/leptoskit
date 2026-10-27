//! Native immutable image uploads; frame storage remains owned by the player.

use leptoskit_native::RiveFrame;
use objc2::rc::Retained;
use objc2_core_foundation::CFData;
use objc2_core_graphics::CGBitmapInfo;
use objc2_core_graphics::CGColorRenderingIntent;
use objc2_core_graphics::CGColorSpace;
use objc2_core_graphics::CGDataProvider;
use objc2_core_graphics::CGImage;
use objc2_core_graphics::CGImageAlphaInfo;
use objc2_core_graphics::CGImageByteOrderInfo;
use objc2_core_graphics::kCGColorSpaceSRGB;
use objc2_ui_kit::UIImage;

pub(crate) fn image(frame: &RiveFrame<'_>) -> Option<Retained<UIImage>> {
	// CFData owns the one upload copy, so Core Graphics cannot retain the
	// borrowed frame buffer after the player advances its next frame.
	let data = CFData::from_bytes(frame.bytes());
	let provider = CGDataProvider::with_cf_data(Some(&data))?;
	// SAFETY: The framework's immutable sRGB name is available on all iOS
	// targets.
	let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
	let format = CGBitmapInfo::from_bits_retain(
		CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0,
	);
	// SAFETY: Frame validates dimensions, stride, and exact data length. The
	// provider retains CFData, decode=null requests the default color mapping,
	// and the explicit byte order matches the premultiplied RGBA channel order.
	let image = unsafe {
		CGImage::new(
			frame.width() as usize,
			frame.height() as usize,
			8,
			32,
			frame.stride(),
			Some(&space),
			format,
			Some(&provider),
			std::ptr::null(),
			true,
			CGColorRenderingIntent::RenderingIntentDefault,
		)
	}?;
	Some(UIImage::imageWithCGImage(&image))
}
