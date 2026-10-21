//! A photo library: a segmented header over a scrolling CSS grid of
//! generated photos, with a feature tile spanning two columns and two rows.

use leptoskit::prelude::Button;
use leptoskit::prelude::Image;
use leptoskit::prelude::ScrollView;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::TrackSizing;

use crate::PHONE;
use crate::ScreenState;
use crate::ui::alpha;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::type_scale;

struct Palette {
	surface: Color,
	ink: Color,
	muted: Color,
	accent: Color,
	fill: Color,
	line: Color,
}

fn palette() -> Palette {
	Palette {
		surface: hex("#ffffff"),
		ink: hex("#000000"),
		muted: hex("#8a8a8e"),
		accent: hex("#007aff"),
		fill: hex("#eeeef0"),
		line: hex("#e3e3e6"),
	}
}

const COLUMNS: usize = 3;
const GUTTER: f32 = 2.0;

/// The side of one grid cell, so every row is square.
fn cell_size() -> f32 {
	(PHONE.0 - GUTTER * (COLUMNS - 1) as f32) / COLUMNS as f32
}

/// One photo, optionally marked as a video with its duration.
fn tile(state: &ScreenState, photo: usize, duration: Option<&str>) -> Element {
	let mut tile = View().child(Image(state.assets.photo(photo)).class("w-full h-full"));
	if let Some(duration) = duration {
		tile = tile.child(
			View()
				.class("absolute right-1.5 bottom-1.5 py-0.5 px-1.5")
				.background(alpha(hex("#000000"), 0.45))
				.class("rounded-[6px]")
				.child(
					Text(duration)
						.class("text-[11px] font-semibold")
						.class("text-[#ffffff]"),
				),
		);
	}
	tile
}

/// A month of photos: a header and a square grid, led by a feature tile.
fn month(
	state: &ScreenState,
	title: &str,
	places: &str,
	photos: std::ops::Range<usize>,
	colors: &Palette,
) -> Element {
	let first = photos.start;
	let grid = View()
		.class("grid grid-cols-3 gap-0.5")
		.with_style(|style| {
			style.grid = Some(GridStyle {
				auto_rows: TrackSizing::Points(cell_size()),
				..GridStyle::default()
			});
		})
		.child(tile(state, first, None).class("col-span-2 row-span-2"))
		.children(photos.skip(1).map(|photo| {
			let duration = match photo % 5 {
				2 => Some("0:42"),
				4 => Some("1:08"),
				_ => None,
			};
			tile(state, photo, duration)
		}));
	View()
		.class("flex-col")
		.child(
			View()
				.class("flex-row items-center gap-2 pt-4 px-4 pb-3")
				.child(label(title, type_scale::HEADLINE, colors.ink))
				.child(label(places, type_scale::FOOTNOTE, colors.muted)),
		)
		.child(grid)
}

/// The view switcher under the title.
fn segmented(selected: &str, colors: &Palette) -> Element {
	View()
		.class("flex-row items-center p-0.5")
		.background(colors.fill)
		.class("rounded-[9px]")
		.children(["Years", "Months", "Days", "All Photos"].map(|segment| {
			let button = Button(segment)
				.font(13.0, FontWeight::Medium)
				.color(colors.ink)
				.class("h-7 rounded-[7px] flex-1");
			if segment == selected {
				button
					.background(colors.surface)
					.border(0.5, alpha(colors.ink, 0.08))
					.font(13.0, FontWeight::Semibold)
			} else {
				button.background(alpha(colors.fill, 0.0))
			}
		}))
}

/// The bottom navigation, with the library selected.
fn tab_bar(colors: &Palette) -> Element {
	let tab = |title: &str, icon: Element, selected: bool| {
		let tint = if selected {
			colors.accent
		} else {
			colors.muted
		};
		View()
			.class("flex-col items-center gap-1 flex-1")
			.child(icon)
			.child(
				Text(title)
					.class("text-[10px] font-medium")
					.color(tint)
					.class("text-center"),
			)
	};
	let icon = |radius: f32, selected: bool| {
		let tint = if selected {
			colors.accent
		} else {
			colors.muted
		};
		let icon = View().class("w-6 h-[22px]").corner_radius(radius);
		if selected {
			icon.background(tint)
		} else {
			icon.border(2.0, tint)
		}
	};
	View()
		.class("flex-col")
		.background(alpha(colors.surface, 0.96))
		.child(View().background(colors.line).class("h-[1px]"))
		.child(
			View()
				.class("flex-row items-center pt-2 px-2 pb-[30px]")
				.child(tab("Library", icon(5.0, true), true))
				.child(tab("For You", icon(11.0, false), false))
				.child(tab("Albums", icon(3.0, false), false))
				.child(tab("Search", icon(11.0, false), false)),
		)
}

/// The photos screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();

	let header = View()
		.class("flex-col gap-3 pt-14 px-4 pb-2")
		.child(
			View()
				.class("flex-row items-end")
				.child(
					View()
						.class("flex-col gap-0.5 flex-1")
						.child(label("Library", type_scale::LARGE_TITLE, colors.ink))
						.child(label(
							"1,204 photos · 86 videos",
							type_scale::FOOTNOTE,
							colors.muted,
						)),
				)
				.child(
					Button("Select")
						.font(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
						.color(colors.accent)
						.background(colors.fill)
						.class("rounded-[15px] py-0 px-3.5 h-[30px]"),
				),
		)
		.child(segmented("All Photos", &colors));

	let library = ScrollView().class("flex-1").child(
		View()
			.class("flex-col pb-6")
			.child(month(state, "October 2026", "Lagos & Accra", 0..9, &colors))
			.child(month(state, "September 2026", "Cape Coast", 9..15, &colors)),
	);

	View()
		.class("flex-col w-full h-full")
		.background(colors.surface)
		.child(header)
		.child(library)
		.child(tab_bar(&colors))
}
