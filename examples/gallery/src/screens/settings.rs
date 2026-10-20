//! A settings list with grouped rows and controlled platform switches.

use leptoskit::prelude::Image;
use leptoskit::prelude::Switch;
use leptoskit::prelude::Text;
use leptoskit::prelude::TextInput;
use leptoskit::prelude::View;
use leptoskit_native::Color;
use leptoskit_native::Element;

use crate::ScreenState;
use crate::ui::circle;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::spacer;
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
fn tile(color: Color, glyph: Glyph) -> Element {
	let white = hex("#ffffff");
	let mark = match glyph {
		Glyph::Dot => circle(12.0, white),
		Glyph::Ring => View().class("w-3.5 h-3.5 rounded-[7px]").border(2.5, white),
		Glyph::Bar => View().class("w-3.5 h-1 rounded-[2px]").background(white),
		Glyph::Square => View().class("w-3 h-3 rounded-[3px]").background(white),
	};
	View()
		.class("w-[30px] h-[30px]")
		.background(color)
		.class("rounded-[7px] items-center justify-center")
		.child(mark)
}

/// The white shapes standing in for SF Symbols.
#[derive(Clone, Copy)]
enum Glyph {
	Dot,
	Ring,
	Bar,
	Square,
}

/// A controlled switch whose value remains owned by the shared screen state.
fn switch(state: &ScreenState, name: &'static str) -> Element {
	Switch(state.is_on(name))
		.accessibility_label(name)
		.on_toggle(state.set_switch(name))
}

fn chevron(colors: &Palette) -> Element {
	Text("›")
		.class("text-[22px] font-normal")
		.color(colors.chevron)
}

/// One settings row.
fn row(
	state: &ScreenState,
	icon: Element,
	title: &'static str,
	accessory: Accessory,
	colors: &Palette,
) -> Element {
	let trailing = match accessory {
		Accessory::Value(value) => {
			View()
				.class("flex-row items-center gap-2")
				.child(label(value, type_scale::BODY, colors.muted))
				.child(chevron(colors))
		}
		Accessory::Disclosure => chevron(colors),
		Accessory::Switch(name) => switch(state, name),
	};
	View()
		.class("flex-row items-center gap-3 h-[46px] py-0 px-4")
		.child(icon)
		.child(label(title, type_scale::BODY, colors.ink).class("flex-1"))
		.child(trailing)
}

/// A white, rounded group of rows with inset separators between them.
fn group(rows: Vec<Element>, colors: &Palette) -> Element {
	let count = rows.len();
	let mut group = View()
		.class("flex-col")
		.background(colors.group)
		.class("rounded-[12px]");
	for (index, row) in rows.into_iter().enumerate() {
		group = group.child(row);
		if index + 1 < count {
			// Separators start where the titles do, past the icon tiles.
			group = group.child(
				View()
					.background(colors.separator)
					.class("h-[1px] ml-[58px]"),
			);
		}
	}
	group
}

/// A group with a small explanatory caption below it.
fn with_footnote(group: Element, text: &'static str, colors: &Palette) -> Element {
	View()
		.class("flex-col gap-2")
		.child(group)
		.child(
			View()
				.class("py-0 px-4")
				.child(label(text, type_scale::FOOTNOTE, colors.muted)),
		)
}

/// The settings screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();

	let search = View()
		.class("flex-row items-center gap-2 h-9 py-0 px-3")
		.class("bg-[#e3e3e8]")
		.class("rounded-[10px]")
		.child(
			View()
				.class("w-[13px] h-[13px] rounded-[6.5px]")
				.border(2.0, colors.muted),
		)
		.child(
			TextInput()
				.placeholder("Search")
				.accessibility_label("Search settings")
				.font(type_scale::BODY.size, type_scale::BODY.weight)
				.color(colors.ink)
				.class("bg-[#00000000]")
				.class("h-5 flex-1"),
		);

	let account = View()
		.class("flex-row items-center gap-3 py-3 px-4")
		.background(colors.group)
		.class("rounded-[12px]")
		.child(Image(state.assets.avatar(2)).class("rounded-[30px] w-[60px] h-[60px]"))
		.child(
			View()
				.class("flex-col gap-0.5 flex-1")
				.child(label("Tolu Bankole", type_scale::TITLE, colors.ink))
				.child(label(
					"Account, backups, and purchases",
					type_scale::FOOTNOTE,
					colors.muted,
				)),
		)
		.child(chevron(&colors));

	let connectivity = group(
		vec![
			row(
				state,
				tile(hex("#ff9500"), Glyph::Square),
				"Airplane Mode",
				Accessory::Switch("Airplane Mode"),
				&colors,
			),
			row(
				state,
				tile(hex("#007aff"), Glyph::Ring),
				"Wi-Fi",
				Accessory::Value("Home 5G"),
				&colors,
			),
			row(
				state,
				tile(hex("#007aff"), Glyph::Dot),
				"Bluetooth",
				Accessory::Value("On"),
				&colors,
			),
			row(
				state,
				tile(hex("#34c759"), Glyph::Bar),
				"Mobile Data",
				Accessory::Disclosure,
				&colors,
			),
		],
		&colors,
	);

	let preferences = group(
		vec![
			row(
				state,
				tile(hex("#ff3b30"), Glyph::Dot),
				"Notifications",
				Accessory::Disclosure,
				&colors,
			),
			row(
				state,
				tile(hex("#5856d6"), Glyph::Ring),
				"Focus",
				Accessory::Value("Off"),
				&colors,
			),
			row(
				state,
				tile(hex("#ff2d55"), Glyph::Bar),
				"Haptics",
				Accessory::Switch("Haptics"),
				&colors,
			),
			row(
				state,
				tile(hex("#ffcc00"), Glyph::Square),
				"Low Power Mode",
				Accessory::Switch("Low Power Mode"),
				&colors,
			),
		],
		&colors,
	);

	View()
		.class("flex-col w-full h-full")
		.background(colors.canvas)
		.class("pt-14 px-4 gap-6")
		.child(
			View()
				.class("flex-col gap-3")
				.child(label("Settings", type_scale::LARGE_TITLE, colors.ink))
				.child(search),
		)
		.child(account)
		.child(connectivity)
		.child(with_footnote(
			preferences,
			"Low Power Mode reduces background activity like downloads and mail fetch until you \
			 charge your phone.",
			&colors,
		))
		.child(spacer())
}
