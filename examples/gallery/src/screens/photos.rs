//! A photo library: a segmented header over a scrolling CSS grid of
//! generated photos, with a feature tile spanning two columns and two rows.

use leptoskit::prelude::*;
use leptoskit_native::Color;
use leptoskit_native::FontWeight;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::TrackSizing;

use crate::PHONE;
use crate::ScreenState;
use crate::ui::Label;
use crate::ui::alpha;
use crate::ui::hex;
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
#[component]
fn Tile(
	state: &ScreenState,
	photo: usize,
	#[prop(optional)] duration: Option<&'static str>,
) -> Element {
	view! {
		<View>
			<Image source=state.assets.photo(photo) class="w-full h-full" />
			{duration
				.map(|duration| {
					view! {
						<View
							class="absolute right-1.5 bottom-1.5 py-0.5 px-1.5"
							background=alpha(hex("#000000"), 0.45)
							class="rounded-[6px]"
						>
							<Text class="font-semibold text-[11px]" class="text-[#ffffff]">
								{duration}
							</Text>
						</View>
					}
				})}
		</View>
	}
}

/// A month of photos: a header and a square grid, led by a feature tile.
#[component]
fn Month(
	state: &ScreenState,
	title: &'static str,
	places: &'static str,
	photos: std::ops::Range<usize>,
	colors: &Palette,
) -> Element {
	let first = photos.start;
	let rest = photos
		.skip(1)
		.map(|photo| {
			let duration = match photo % 5 {
				2 => Some("0:42"),
				4 => Some("1:08"),
				_ => None,
			};
			Tile(state, photo, duration)
		})
		.collect::<Fragment>();

	view! {
		<View class="flex-col">
			<View class="flex-row gap-2 items-center px-4 pt-4 pb-3">
				<Label style=type_scale::HEADLINE color=colors.ink>
					{title}
				</Label>
				<Label style=type_scale::FOOTNOTE color=colors.muted>
					{places}
				</Label>
			</View>
			<View
				class="grid grid-cols-3 gap-0.5"
				with_style=|style| {
					style.grid = Some(GridStyle {
						auto_rows: TrackSizing::Points(cell_size()),
						..GridStyle::default()
					});
				}
			>
				{Tile(state, first, None).class("col-span-2 row-span-2")}
				{rest}
			</View>
		</View>
	}
}

/// The view switcher under the title.
#[component]
fn Segmented(selected: &'static str, colors: &Palette) -> Element {
	let segments = ["Years", "Months", "Days", "All Photos"].map(|segment| {
		let button = view! {
			<Button
				font=(13.0, FontWeight::Medium)
				color=colors.ink
				class="flex-1 h-7 rounded-[7px]"
			>
				{segment}
			</Button>
		};
		if segment == selected {
			button
				.background(colors.surface)
				.border(0.5, alpha(colors.ink, 0.08))
				.font(13.0, FontWeight::Semibold)
		} else {
			button.background(alpha(colors.fill, 0.0))
		}
	});

	view! {
		<View class="flex-row items-center p-0.5" background=colors.fill class="rounded-[9px]">
			{segments}
		</View>
	}
}

/// One destination in the tab bar, with a shape for its icon.
#[component]
fn Tab(
	title: &'static str,
	radius: f32,
	#[prop(optional)] selected: bool,
	colors: &Palette,
) -> Element {
	let tint = if selected {
		colors.accent
	} else {
		colors.muted
	};
	let icon = view! { <View class="w-6 h-[22px]" corner_radius=radius /> };
	let icon = if selected {
		icon.background(tint)
	} else {
		icon.border(2.0, tint)
	};

	view! {
		<View class="flex-col flex-1 gap-1 items-center">
			{icon} <Text class="font-medium text-[10px]" color=tint class="text-center">
				{title}
			</Text>
		</View>
	}
}

/// The bottom navigation, with the library selected.
#[component]
fn TabBar(colors: &Palette) -> Element {
	view! {
		<View class="flex-col" background=alpha(colors.surface, 0.96)>
			<View background=colors.line class="h-[1px]" />
			<View class="flex-row items-center px-2 pt-2 pb-safe-offset-2">
				<Tab title="Library" radius=5.0 selected colors=colors />
				<Tab title="For You" radius=11.0 colors=colors />
				<Tab title="Albums" radius=3.0 colors=colors />
				<Tab title="Search" radius=11.0 colors=colors />
			</View>
		</View>
	}
}

/// The photos screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = &palette();

	view! {
		<View class="flex-col w-full h-full" background=colors.surface>
			<View class="flex-col gap-3 px-4 pb-2 pt-safe-offset-4">
				<View class="flex-row items-end">
					<View class="flex-col flex-1 gap-0.5">
						<Label style=type_scale::LARGE_TITLE color=colors.ink>
							"Library"
						</Label>
						<Label style=type_scale::FOOTNOTE color=colors.muted>
							"1,204 photos · 86 videos"
						</Label>
					</View>
					<Button
						font=(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
						color=colors.accent
						background=colors.fill
						class="py-0 px-3.5 rounded-[15px] h-[30px]"
					>
						"Select"
					</Button>
				</View>
				<Segmented selected="All Photos" colors=colors />
			</View>
			<ScrollView class="flex-1">
				<View class="flex-col pb-6">
					<Month
						state=state
						title="October 2026"
						places="Lagos & Accra"
						photos=0..9
						colors=colors
					/>
					<Month
						state=state
						title="September 2026"
						places="Cape Coast"
						photos=9..15
						colors=colors
					/>
				</View>
			</ScrollView>
			<TabBar colors=colors />
		</View>
	}
}
