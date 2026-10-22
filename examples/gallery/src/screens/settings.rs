//! A settings list with grouped rows and controlled platform switches.

use leptoskit::prelude::*;
use leptoskit_native::Color;

use crate::ScreenState;
use crate::ui::Circle;
use crate::ui::Label;
use crate::ui::hex;
use crate::ui::type_scale;

struct Palette {
	canvas: Color,
	group: Color,
	ink: Color,
	muted: Color,
	separator: Color,
	chevron: Color,
}

fn palette() -> Palette {
	Palette {
		canvas: hex("#f2f2f7"),
		group: hex("#ffffff"),
		ink: hex("#000000"),
		muted: hex("#8a8a8e"),
		separator: hex("#e3e3e6"),
		chevron: hex("#c4c4c7"),
	}
}

/// What sits at the trailing edge of a row.
#[derive(Clone, Copy)]
enum Accessory {
	/// A value and a disclosure chevron.
	Value(&'static str),
	/// Only a disclosure chevron.
	Disclosure,
	/// An on/off switch bound to a named setting.
	Switch(&'static str),
}

/// A rounded icon tile: a white glyph shape on a colored square.
#[component]
fn Tile(color: Color, glyph: Glyph) -> Element {
	let white = hex("#ffffff");
	let mark = match glyph {
		Glyph::Dot => Circle(12.0, white, None),
		Glyph::Ring => view! { <View class="w-3.5 h-3.5 rounded-[7px]" border=(2.5, white) /> },
		Glyph::Bar => view! { <View class="w-3.5 h-1 rounded-[2px]" background=white /> },
		Glyph::Square => view! { <View class="w-3 h-3 rounded-[3px]" background=white /> },
	};
	view! {
		<View
			class="w-[30px] h-[30px]"
			background=color
			class="justify-center items-center rounded-[7px]"
		>
			{mark}
		</View>
	}
}

/// The white shapes standing in for SF Symbols.
#[derive(Clone, Copy)]
enum Glyph {
	Dot,
	Ring,
	Bar,
	Square,
}

#[component]
fn Chevron(colors: &Palette) -> Element {
	view! { <Text class="font-normal text-[22px]" color=colors.chevron>"›"</Text> }
}

/// One settings row.
#[component]
fn Row(
	state: &ScreenState,
	icon: Element,
	title: &'static str,
	accessory: Accessory,
	colors: &Palette,
) -> Element {
	let trailing = match accessory {
		Accessory::Value(value) => {
			view! {
				<View class="flex-row gap-2 items-center">
					<Label style=type_scale::BODY color=colors.muted>{value}</Label>
					<Chevron colors=colors />
				</View>
			}
		}
		Accessory::Disclosure => Chevron(colors),
		// A controlled switch whose value remains owned by the shared screen
		// state.
		Accessory::Switch(name) => {
			view! {
				<Switch
					checked=state.is_on(name)
					accessibility_label=name
					on:toggle=state.set_switch(name)
				/>
			}
		}
	};
	view! {
		<View class="flex-row gap-3 items-center py-0 px-4 h-[46px]">
			{icon} <Label style=type_scale::BODY color=colors.ink class="flex-1">{title}</Label>
			{trailing}
		</View>
	}
}

/// A white, rounded group of rows with inset separators between them.
#[component]
fn Group(colors: &Palette, children: Children) -> Element {
	let count = children.len();
	let rows = children
		.into_iter()
		.enumerate()
		.map(|(index, row)| {
			// Separators start where the titles do, past the icon tiles.
			let separator = (index + 1 < count)
				.then(|| view! { <View background=colors.separator class="h-[1px] ml-[58px]" /> });
			view! { <>{row}{separator}</> }
		})
		.collect::<Fragment>();

	view! { <View class="flex-col" background=colors.group class="rounded-[12px]">{rows}</View> }
}

/// A group with a small explanatory caption below it.
#[component]
fn WithFootnote(text: &'static str, colors: &Palette, children: Children) -> Element {
	view! {
		<View class="flex-col gap-2">
			{children}
			<View class="py-0 px-4">
				<Label style=type_scale::FOOTNOTE color=colors.muted>{text}</Label>
			</View>
		</View>
	}
}

/// The settings screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = &palette();

	view! {
		<ScrollView class="w-full h-full" background=colors.canvas>
			<View class="flex-col gap-6 px-4 pt-safe-offset-4 pb-safe-offset-6">
				<View class="flex-col gap-3">
					<Label style=type_scale::LARGE_TITLE color=colors.ink>"Settings"</Label>
					<View
						class="flex-row gap-2 items-center py-0 px-3 h-9"
						class="bg-[#e3e3e8]"
						class="rounded-[10px]"
					>
						<View
							class="w-[13px] h-[13px] rounded-[6.5px]"
							border=(2.0, colors.muted)
						/>
						<TextInput
							placeholder="Search"
							accessibility_label="Search settings"
							font=(type_scale::BODY.size, type_scale::BODY.weight)
							color=colors.ink
							class="bg-[#00000000]"
							class="flex-1 h-5"
						/>
					</View>
				</View>
				<View
					class="flex-row gap-3 items-center py-3 px-4"
					background=colors.group
					class="rounded-[12px]"
				>
					<Image source=state.assets.avatar(2) class="rounded-[30px] w-[60px] h-[60px]" />
					<View class="flex-col flex-1 gap-0.5">
						<Label style=type_scale::TITLE color=colors.ink>"Tolu Bankole"</Label>
						<Label style=type_scale::FOOTNOTE color=colors.muted>
							"Account, backups, and purchases"
						</Label>
					</View>
					<Chevron colors=colors />
				</View>
				<Group colors=colors>
					<Row
						state=state
						icon=Tile(hex("#ff9500"), Glyph::Square)
						title="Airplane Mode"
						accessory=Accessory::Switch("Airplane Mode")
						colors=colors
					/>
					<Row
						state=state
						icon=Tile(hex("#007aff"), Glyph::Ring)
						title="Wi-Fi"
						accessory=Accessory::Value("Home 5G")
						colors=colors
					/>
					<Row
						state=state
						icon=Tile(hex("#007aff"), Glyph::Dot)
						title="Bluetooth"
						accessory=Accessory::Value("On")
						colors=colors
					/>
					<Row
						state=state
						icon=Tile(hex("#34c759"), Glyph::Bar)
						title="Mobile Data"
						accessory=Accessory::Disclosure
						colors=colors
					/>
				</Group>
				<WithFootnote
					text="Low Power Mode reduces background activity like downloads and mail fetch until \
					you charge your phone."
					colors=colors
				>
					<Group colors=colors>
						<Row
							state=state
							icon=Tile(hex("#ff3b30"), Glyph::Dot)
							title="Notifications"
							accessory=Accessory::Disclosure
							colors=colors
						/>
						<Row
							state=state
							icon=Tile(hex("#5856d6"), Glyph::Ring)
							title="Focus"
							accessory=Accessory::Value("Off")
							colors=colors
						/>
						<Row
							state=state
							icon=Tile(hex("#ff2d55"), Glyph::Bar)
							title="Haptics"
							accessory=Accessory::Switch("Haptics")
							colors=colors
						/>
						<Row
							state=state
							icon=Tile(hex("#ffcc00"), Glyph::Square)
							title="Low Power Mode"
							accessory=Accessory::Switch("Low Power Mode")
							colors=colors
						/>
					</Group>
				</WithFootnote>
			</View>
		</ScrollView>
	}
}
