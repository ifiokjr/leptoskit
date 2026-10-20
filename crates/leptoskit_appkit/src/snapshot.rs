//! Offscreen PNG snapshots of a view tree.

use leptoskit_yoga::NodeId;
use objc2::AnyThread;
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSBitmapImageFileType;
use objc2_app_kit::NSBitmapImageRep;
use objc2_app_kit::NSCalibratedRGBColorSpace;
use objc2_app_kit::NSColorSpace;
use objc2_foundation::NSDictionary;

use crate::backend::AppKitBackend;

/// Why a snapshot could not be taken.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SnapshotError {
	/// The node is not a live widget of this backend.
	#[error("node {0:?} is not a live widget of this backend")]
	UnknownNode(NodeId),
	/// The view has no area to draw, usually because layout has not run.
	#[error("the view is {width}×{height} points; lay it out before snapshotting")]
	EmptyView {
		/// The view's width, in points.
		width: f64,
		/// The view's height, in points.
		height: f64,
	},
	/// The scale is not a positive, finite number.
	#[error("the snapshot scale {0} is not a positive, finite number")]
	InvalidScale(f64),
	/// AppKit could not allocate the bitmap.
	#[error("AppKit could not allocate a {width}×{height} pixel bitmap")]
	Allocation {
		/// The requested width, in pixels.
		width: isize,
		/// The requested height, in pixels.
		height: isize,
	},
	/// AppKit could not encode the bitmap as PNG.
	#[error("AppKit could not encode the bitmap as PNG")]
	Encoding,
}

impl AppKitBackend {
	/// Renders a node's view and everything inside it into PNG bytes.
	///
	/// The view is drawn offscreen with `cacheDisplayInRect:toBitmapImageRep:`
	/// into a bitmap of `scale` pixels per point — `2.0` for retina quality —
	/// tagged as sRGB. Nothing is captured from the screen, so no window is
	/// needed and no Screen Recording permission is requested.
	///
	/// The view must have been laid out: commit a tree and set a viewport on
	/// the surface first.
	pub fn snapshot_png(&self, root: NodeId, scale: f64) -> Result<Vec<u8>, SnapshotError> {
		autoreleasepool(|_| self.render_png(root, scale))
	}

	fn render_png(&self, root: NodeId, scale: f64) -> Result<Vec<u8>, SnapshotError> {
		let view = self.view(root).ok_or(SnapshotError::UnknownNode(root))?;
		if !(scale.is_finite() && scale > 0.0) {
			return Err(SnapshotError::InvalidScale(scale));
		}
		let bounds = view.bounds();
		let (width, height) = (bounds.size.width, bounds.size.height);
		if !(width >= 1.0 && height >= 1.0) {
			return Err(SnapshotError::EmptyView { width, height });
		}
		let pixels_wide = (width * scale).round() as isize;
		let pixels_high = (height * scale).round() as isize;

		// SAFETY: null planes ask AppKit to allocate and own the pixel buffer;
		// 8 bits × 4 samples (RGBA, alpha included, meshed) is a layout AppKit
		// supports, and zero row bytes and pixel bits let it compute both.
		let rep = unsafe {
			NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
				NSBitmapImageRep::alloc(),
				std::ptr::null_mut(),
				pixels_wide,
				pixels_high,
				8,
				4,
				true,
				false,
				NSCalibratedRGBColorSpace,
				0,
				0,
			)
		}
		.ok_or(SnapshotError::Allocation {
			width: pixels_wide,
			height: pixels_high,
		})?;
		// Tag the empty bitmap as sRGB before drawing, so the colors views
		// draw in (leptoskit colors are sRGB) land in the file unconverted.
		let rep = rep
			.bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace())
			.ok_or(SnapshotError::Allocation {
				width: pixels_wide,
				height: pixels_high,
			})?;
		// A point size smaller than the pixel size is what makes the bitmap
		// draw at `scale` pixels per point.
		rep.setSize(bounds.size);
		view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);

		// SAFETY: an empty property dictionary is valid for every file type.
		let data = unsafe {
			rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
		}
		.ok_or(SnapshotError::Encoding)?;
		Ok(data.to_vec())
	}
}
