//! GDI resources shared by measurement, native control fonts, and painting.

use leptoskit_native::Color;
use leptoskit_native::FontWeight;
use leptoskit_native::TextAlign;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Direction;
use leptoskit_yoga::Size;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::*;
use windows::core::w;

/// Measures text with independent GDI resources on the calling thread.
///
/// No HWND, device context, or font crosses threads. Rendering uses the same
/// Segoe UI family, logical pixel sizes, weights, and word-wrapping flags.
#[derive(Debug, Clone, Copy)]
pub struct WindowsTextMeasurer {
	dpi: u32,
}

impl WindowsTextMeasurer {
	/// Captures immutable monitor metrics for one layout cache generation.
	pub fn new(dpi: u32) -> Self {
		assert!(dpi > 0, "Windows DPI must be positive");
		Self { dpi }
	}
}

impl Default for WindowsTextMeasurer {
	fn default() -> Self {
		Self::new(96)
	}
}

pub(crate) struct Font(pub HFONT);

impl Font {
	pub fn new(style: TextStyle, scale: f32) -> Self {
		let weight = match style.font_weight {
			FontWeight::Regular => 400,
			FontWeight::Medium => 500,
			FontWeight::Semibold => 600,
			FontWeight::Bold => 700,
		};
		// SAFETY: All font parameters are values; the static family is
		// terminated.
		let font = unsafe {
			CreateFontW(
				-((style.font_size * scale).round().max(1.0) as i32),
				0,
				0,
				0,
				weight,
				0,
				0,
				0,
				DEFAULT_CHARSET,
				OUT_DEFAULT_PRECIS,
				CLIP_DEFAULT_PRECIS,
				CLEARTYPE_QUALITY,
				u32::from(DEFAULT_PITCH.0),
				w!("Segoe UI"),
			)
		};
		assert!(!font.is_invalid(), "Windows could not allocate a font");
		Self(font)
	}
}

impl Drop for Font {
	fn drop(&mut self) {
		// SAFETY: The font is uniquely owned and has been deselected by
		// callers.
		unsafe {
			let _ = DeleteObject(self.0.into());
		}
	}
}

pub(crate) struct Brush(pub HBRUSH);

impl Brush {
	pub fn new(color: Color) -> Self {
		// SAFETY: CreateSolidBrush accepts a value and returns an owned GDI
		// object.
		let brush = unsafe { CreateSolidBrush(rgb(color)) };
		assert!(!brush.is_invalid(), "Windows could not allocate a brush");
		Self(brush)
	}
}

impl Drop for Brush {
	fn drop(&mut self) {
		// SAFETY: This uniquely owned brush is no longer selected by any DC.
		unsafe {
			let _ = DeleteObject(self.0.into());
		}
	}
}

pub(crate) fn rgb(color: Color) -> COLORREF {
	COLORREF(
		u32::from(color.red()) | (u32::from(color.green()) << 8) | (u32::from(color.blue()) << 16),
	)
}

/// Draws `text` into `rect` on `dc`, doing nothing for empty text.
///
/// An empty `Vec<u16>` has a dangling, non-null pointer, and `DrawTextW`
/// dereferences its string even when the length is zero.
pub(crate) fn draw_text(dc: HDC, text: &str, rect: &mut RECT, format: DRAW_TEXT_FORMAT) {
	if text.is_empty() {
		return;
	}
	let mut utf16: Vec<u16> = text.encode_utf16().collect();
	// SAFETY: The buffer is non-empty and outlives this synchronous call; the
	// caller owns `dc` on this thread.
	unsafe {
		DrawTextW(dc, &mut utf16, rect, format);
	}
}

pub(crate) fn text_flags(style: TextStyle) -> DRAW_TEXT_FORMAT {
	let rtl = style.direction == Direction::Rtl;
	DT_WORDBREAK
		| DT_NOPREFIX
		| if rtl {
			DT_RTLREADING
		} else {
			DRAW_TEXT_FORMAT::default()
		}
		| match (style.text_align, rtl) {
			(TextAlign::Start, false) | (TextAlign::End, true) => DT_LEFT,
			(TextAlign::Center, _) => DT_CENTER,
			(TextAlign::End, false) | (TextAlign::Start, true) => DT_RIGHT,
		}
}

impl TextMeasurer for WindowsTextMeasurer {
	fn measure(&self, text: &str, style: TextStyle, available: Size<AvailableSpace>) -> Size<f32> {
		if text.is_empty() {
			return Size {
				width: 0.0,
				height: 0.0,
			};
		}

		let scale = self.dpi as f32 / 96.0;
		let font = Font::new(style, scale);
		let width = match available.width {
			AvailableSpace::Definite(width) => (width * scale).ceil().max(1.0) as i32,
			AvailableSpace::MinContent => 1,
			AvailableSpace::MaxContent => 1_000_000,
		};
		let mut rect = RECT {
			left: 0,
			top: 0,
			right: width,
			bottom: 0,
		};
		let mut utf16: Vec<u16> = text.encode_utf16().collect();
		// SAFETY: The local memory DC and font are used only on this thread.
		unsafe {
			let dc = CreateCompatibleDC(None);
			assert!(
				!dc.is_invalid(),
				"Windows could not allocate a measurement DC"
			);
			let previous = SelectObject(dc, font.0.into());
			DrawTextW(
				dc,
				&mut utf16,
				&raw mut rect,
				text_flags(style) | DT_CALCRECT,
			);
			SelectObject(dc, previous);
			let _ = DeleteDC(dc);
		}

		Size {
			width: (rect.right - rect.left) as f32 / scale,
			height: (rect.bottom - rect.top) as f32 / scale,
		}
	}
}
