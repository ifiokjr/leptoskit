//! A retained DIB and memory DC; animation paints allocate neither.

use std::ptr::NonNull;

use leptoskit_native::RiveFrame;
use windows::Win32::Graphics::Gdi::*;

pub(crate) struct Bitmap {
	dc: HDC,
	handle: HBITMAP,
	previous: HGDIOBJ,
	bits: NonNull<u8>,
	width: u32,
	height: u32,
}

impl Bitmap {
	pub fn new(frame: &RiveFrame<'_>) -> Option<Self> {
		let info = BITMAPINFO {
			bmiHeader: BITMAPINFOHEADER {
				biSize: size_of::<BITMAPINFOHEADER>() as u32,
				biWidth: frame.width() as i32,
				biHeight: -(frame.height() as i32),
				biPlanes: 1,
				biBitCount: 32,
				biCompression: BI_RGB.0,
				..Default::default()
			},
			..Default::default()
		};
		// SAFETY: The DC and top-down 32-bit DIB are owned by this UI-thread
		// value. Failure branches release already-created handles.
		unsafe {
			let dc = CreateCompatibleDC(None);
			if dc.is_invalid() {
				return None;
			}
			let mut bits = std::ptr::null_mut();
			let Ok(bitmap) = CreateDIBSection(
				Some(dc),
				&raw const info,
				DIB_RGB_COLORS,
				&raw mut bits,
				None,
				0,
			) else {
				let _ = DeleteDC(dc);
				return None;
			};
			let Some(bits) = NonNull::new(bits.cast()) else {
				let _ = DeleteObject(bitmap.into());
				let _ = DeleteDC(dc);
				return None;
			};
			let previous = SelectObject(dc, bitmap.into());
			if previous.is_invalid() {
				let _ = DeleteObject(bitmap.into());
				let _ = DeleteDC(dc);
				return None;
			}
			Some(Self {
				dc,
				handle: bitmap,
				previous,
				bits,
				width: frame.width(),
				height: frame.height(),
			})
		}
	}

	pub fn matches(&self, frame: &RiveFrame<'_>) -> bool {
		self.width == frame.width() && self.height == frame.height()
	}

	pub fn update(&mut self, frame: &RiveFrame<'_>) -> bool {
		if !self.matches(frame) {
			return false;
		}
		// SAFETY: GDI batches may still read this UI-thread DIB. Complete
		// those operations before writing through its raw pixel pointer.
		if !unsafe { GdiFlush() }.as_bool() {
			return false;
		}
		// SAFETY: The retained DIB has exactly the validated frame's byte
		// count.
		let destination =
			unsafe { std::slice::from_raw_parts_mut(self.bits.as_ptr(), frame.bytes().len()) };

		for (source, destination) in frame
			.bytes()
			.chunks_exact(4)
			.zip(destination.chunks_exact_mut(4))
		{
			// Pixels are already premultiplied: only swap RGBA to GDI's BGRA.
			destination.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
		}
		true
	}

	pub fn paint(&self, dc: HDC, width: i32, height: i32) -> bool {
		// SAFETY: The caller lends its active paint DC, and our bitmap remains
		// selected in a distinct owned memory DC until Drop.
		unsafe {
			AlphaBlend(
				dc,
				0,
				0,
				width,
				height,
				self.dc,
				0,
				0,
				self.width as i32,
				self.height as i32,
				BLENDFUNCTION {
					BlendOp: AC_SRC_OVER as u8,
					BlendFlags: 0,
					SourceConstantAlpha: 255,
					AlphaFormat: AC_SRC_ALPHA as u8,
				},
			)
			.as_bool()
		}
	}
}

impl Drop for Bitmap {
	fn drop(&mut self) {
		// SAFETY: Deselect before deleting; each handle has exactly one owner.
		unsafe {
			let _ = GdiFlush();
			SelectObject(self.dc, self.previous);
			let _ = DeleteObject(self.handle.into());
			let _ = DeleteDC(self.dc);
		}
	}
}
