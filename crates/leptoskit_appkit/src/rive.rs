//! Reusable native bitmap storage for host-owned animation frames.

use leptoskit_native::RiveFrame;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_app_kit::NSBitmapImageRep;
use objc2_app_kit::NSCalibratedRGBColorSpace;
use objc2_app_kit::NSColorSpace;
use objc2_app_kit::NSImage;
use objc2_foundation::NSSize;

pub(crate) struct Bitmap {
	representation: Retained<NSBitmapImageRep>,
	pub image: Retained<NSImage>,
	width: u32,
	height: u32,
}

impl Bitmap {
	pub fn new(frame: &RiveFrame<'_>) -> Option<Self> {
		// SAFETY: AppKit owns the allocated planes. The validated frame uses
		// bounded premultiplied RGBA8888 with exactly this explicit row stride.
		let representation = unsafe {
			NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
				NSBitmapImageRep::alloc(), std::ptr::null_mut(), frame.width() as isize,
				frame.height() as isize, 8, 4, true, false, NSCalibratedRGBColorSpace,
				frame.stride() as isize, 32)
		}?.bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace())?;
		let image = NSImage::initWithSize(
			NSImage::alloc(),
			NSSize::new(f64::from(frame.width()), f64::from(frame.height())),
		);
		image.addRepresentation(&representation);
		Some(Self {
			representation,
			image,
			width: frame.width(),
			height: frame.height(),
		})
	}

	pub fn matches(&self, frame: &RiveFrame<'_>) -> bool {
		self.width == frame.width() && self.height == frame.height()
	}

	pub fn update(&self, frame: &RiveFrame<'_>) -> bool {
		let destination = self.representation.bitmapData();

		if !self.matches(frame)
			|| destination.is_null()
			|| self.representation.bytesPerRow() != frame.stride() as isize
		{
			return false;
		}
		// SAFETY: The representation owns height*stride writable bytes, and
		// the frame borrows a separate raster allocation for this synchronous
		// copy.
		unsafe {
			std::ptr::copy_nonoverlapping(frame.bytes().as_ptr(), destination, frame.bytes().len());
		}
		self.image.recache();
		true
	}
}
