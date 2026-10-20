//! A photo library: a segmented header over a scrolling CSS grid of
//! generated photos, with a feature tile spanning two columns and two rows.

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::TextAlign;
use leptoskit_yoga::Align;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::GridPlacement;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::TrackSizing;

use crate::PHONE;
use crate::ScreenState;
use crate::ui::Layout;
use crate::ui::alpha;
use crate::ui::edges;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::space;
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
	let mut tile = Element::view().child(Element::image(state.assets.photo(photo)).fill());
	if let Some(duration) = duration {
		tile = tile.child(
			Element::view()
				.with_style(|style| {
					style.position = PositionType::Absolute;
					style.inset.right = Dimension::Points(space::XS + 2.0);
					style.inset.bottom = Dimension::Points(space::XS + 2.0);
				})
				.padding(2.0, 6.0)
				.background(alpha(hex("#000000"), 0.45))
				.corner_radius(6.0)
				.child(
					Element::text(duration)
						.font(11.0, FontWeight::Semibold)
						.color(hex("#ffffff")),
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
	let grid = Element::view()
		.with_style(|style| {
			style.display = Display::Grid;
			style.gap = edges(GUTTER);
			style.grid = Some(GridStyle {
				template_columns: vec![TrackSizing::Fraction(1.0); COLUMNS],
				auto_rows: TrackSizing::Points(cell_size()),
				..GridStyle::default()
			});
		})
		.child(tile(state, first, None).with_style(|style| {
			style.grid = Some(GridStyle {
				column: GridPlacement::span(2),
				row: GridPlacement::span(2),
				..GridStyle::default()
			});
		}))
		.children(photos.skip(1).map(|photo| {
			let duration = match photo % 5 {
				2 => Some("0:42"),
				4 => Some("1:08"),
				_ => None,
			};
			tile(state, photo, duration)
		}));
	Element::view()
		.column()
		.child(
			Element::view()
				.row()
				.gap(space::S)
				.padding_edges(space::L, space::L, space::M, space::L)
				.child(label(title, type_scale::HEADLINE, colors.ink))
				.child(label(places, type_scale::FOOTNOTE, colors.muted)),
		)
		.child(grid)
}

/// The view switcher under the title.
fn segmented(selected: &str, colors: &Palette) -> Element {
	Element::view()
		.row()
		.padding(2.0, 2.0)
		.background(colors.fill)
		.corner_radius(9.0)
		.children(["Years", "Months", "Days", "All Photos"].map(|segment| {
			let button = Element::button(segment)
				.font(13.0, FontWeight::Medium)
				.color(colors.ink)
				.height(28.0)
				.corner_radius(7.0)
				.grow();
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
		Element::view()
			.column()
			.align(Align::Center)
			.gap(space::XS)
			.grow()
			.child(icon)
			.child(
				Element::text(title)
					.font(10.0, FontWeight::Medium)
					.color(tint)
					.text_align(TextAlign::Center),
			)
	};
	let icon = |radius: f32, selected: bool| {
		let tint = if selected {
			colors.accent
		} else {
			colors.muted
		};
		let icon = Element::view().size(24.0, 22.0).corner_radius(radius);
		if selected {
			icon.background(tint)
		} else {
			icon.border(2.0, tint)
		}
	};
	Element::view()
		.column()
		.background(alpha(colors.surface, 0.96))
		.child(Element::view().background(colors.line).height(1.0))
		.child(
			Element::view()
				.row()
				.padding_edges(space::S, space::S, 30.0, space::S)
				.child(tab("Library", icon(5.0, true), true))
				.child(tab("For You", icon(11.0, false), false))
				.child(tab("Albums", icon(3.0, false), false))
				.child(tab("Search", icon(11.0, false), false)),
		)
}

/// The photos screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();

	let header = Element::view()
		.column()
		.gap(space::M)
		.padding_edges(56.0, space::L, space::S, space::L)
		.child(
			Element::view()
				.row()
				.align(Align::FlexEnd)
				.child(
					Element::view()
						.column()
						.gap(space::XXS)
						.grow()
						.child(label("Library", type_scale::LARGE_TITLE, colors.ink))
						.child(label(
							"1,204 photos · 86 videos",
							type_scale::FOOTNOTE,
							colors.muted,
						)),
				)
				.child(
					Element::button("Select")
						.font(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
						.color(colors.accent)
						.background(colors.fill)
						.corner_radius(15.0)
						.padding(0.0, space::M + 2.0)
						.height(30.0),
				),
		)
		.child(segmented("All Photos", &colors));

	let library = Element::scroll_view().grow().child(
		Element::view()
			.column()
			.padding_edges(0.0, 0.0, space::XL, 0.0)
			.child(month(state, "October 2026", "Lagos & Accra", 0..9, &colors))
			.child(month(state, "September 2026", "Cape Coast", 9..15, &colors)),
	);

	Element::view()
		.column()
		.fill()
		.background(colors.surface)
		.child(header)
		.child(library)
		.child(tab_bar(&colors))
}
