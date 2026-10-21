//! Stateless `PangoCairo` measurement with the renderer's system font.

use gtk::pango;
use leptoskit_native::FontWeight;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;

/// Uses a new Pango font map and context per call; no GTK objects cross
/// threads.
///
/// GTK decides per widget whether to hint font metrics: GTK 4.14 always does
/// on displays at scale 1, rounding every line to whole pixels, while older
/// releases and `HiDPI` displays leave them unhinted. The measurer copies that
/// decision so Yoga frames match the drawn labels.
#[derive(Debug, Clone, Copy, Default)]
pub struct GtkTextMeasurer {
	hinted: bool,
}

impl GtkTextMeasurer {
	/// Creates a measurer with unhinted metrics, without retaining
	/// thread-affine GTK objects.
	pub const fn new() -> Self {
		Self { hinted: false }
	}

	/// Creates a measurer that matches text drawn by `widget` on its current
	/// display. Create a new one after the widget moves to a display with a
	/// different scale.
	pub fn for_widget(widget: &impl gtk::prelude::IsA<gtk::Widget>) -> Self {
		use gtk::prelude::WidgetExt;
		// GTK rounds glyph positions exactly when it hints font metrics.
		Self {
			hinted: widget.pango_context().is_round_glyph_positions(),
		}
	}
}

pub(crate) fn font(style: TextStyle) -> pango::FontDescription {
	let mut font = pango::FontDescription::new();
	font.set_family("Sans");
	font.set_absolute_size(f64::from(style.font_size) * f64::from(pango::SCALE));
	font.set_weight(match style.font_weight {
		FontWeight::Regular => pango::Weight::Normal,
		FontWeight::Medium => pango::Weight::Medium,
		FontWeight::Semibold => pango::Weight::Semibold,
		FontWeight::Bold => pango::Weight::Bold,
	});
	font
}

impl TextMeasurer for GtkTextMeasurer {
	fn measure(&self, text: &str, style: TextStyle, available: Size<AvailableSpace>) -> Size<f32> {
		use pango::prelude::*;
		let map = pangocairo::FontMap::new();
		let context = map.create_context();
		context.set_base_dir(if style.direction == leptoskit_yoga::Direction::Rtl {
			pango::Direction::Rtl
		} else {
			pango::Direction::Ltr
		});
		let mut options =
			pangocairo::cairo::FontOptions::new().expect("Cairo font options allocation");
		context.set_round_glyph_positions(self.hinted);
		options.set_hint_metrics(if self.hinted {
			pangocairo::cairo::HintMetrics::On
		} else {
			pangocairo::cairo::HintMetrics::Off
		});
		pangocairo::functions::context_set_font_options(&context, Some(&options));
		let layout = pango::Layout::new(&context);
		layout.set_auto_dir(false);
		layout.set_font_description(Some(&font(style)));
		layout.set_text(if text.is_empty() { " " } else { text });
		layout.set_wrap(pango::WrapMode::WordChar);

		match available.width {
			AvailableSpace::Definite(width) => {
				layout.set_width((width.max(0.0) * pango::SCALE as f32) as i32);
			}
			AvailableSpace::MinContent => {
				let width = text
					.split_whitespace()
					.map(|word| {
						layout.set_text(word);
						layout.pixel_size().0
					})
					.max()
					.unwrap_or(0);
				layout.set_text(if text.is_empty() { " " } else { text });
				layout.set_width(width * pango::SCALE);
			}
			AvailableSpace::MaxContent => {}
		}
		let (width, height) = layout.pixel_size();
		Size::new(
			if text.is_empty() { 0.0 } else { width as f32 },
			height as f32,
		)
	}
}
