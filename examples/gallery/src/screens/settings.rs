//! A settings list: grouped rows, icon tiles, values, and switches built from
//! buttons.

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_yoga::Align;
use leptoskit_yoga::JustifyContent;

use crate::ScreenState;
use crate::ui::Layout;
use crate::ui::circle;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::space;
use crate::ui::spacer;
use crate::ui::type_scale;

struct Palette {
	canvas: Color,
	group: Color,
	ink: Color,
	muted: Color,
	separator: Color,
	chevron: Color,
	switch_on: Color,
	switch_off: Color,
}

fn palette() -> Palette {
	Palette {
		canvas: hex("#f2f2f7"),
		group: hex("#ffffff"),
		ink: hex("#000000"),
		muted: hex("#8a8a8e"),
		separator: hex("#e3e3e6"),
		chevron: hex("#c4c4c7"),
		switch_on: hex("#34c759"),
		switch_off: hex("#e9e9eb"),
	}
}

const ROW_HEIGHT: f32 = 46.0;
const TILE: f32 = 30.0;
const SWITCH_WIDTH: f32 = 51.0;
const SWITCH_HEIGHT: f32 = 31.0;
const KNOB: f32 = 27.0;

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
		Glyph::Ring => {
			Element::view()
				.size(14.0, 14.0)
				.corner_radius(7.0)
				.border(2.5, white)
		}
		Glyph::Bar => {
			Element::view()
				.size(14.0, 4.0)
				.corner_radius(2.0)
				.background(white)
		}
		Glyph::Square => {
			Element::view()
				.size(12.0, 12.0)
				.corner_radius(3.0)
				.background(white)
		}
	};
	Element::view()
		.size(TILE, TILE)
		.background(color)
		.corner_radius(7.0)
		.align(Align::Center)
		.justify(JustifyContent::Center)
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

/// An iOS-style switch, built from a button: a pill track with a knob.
fn switch(state: &ScreenState, name: &'static str, colors: &Palette) -> Element {
	let on = state.is_on(name);
	let inset = (SWITCH_HEIGHT - KNOB) / 2.0;
	Element::button("")
		.background(if on {
			colors.switch_on
		} else {
			colors.switch_off
		})
		.corner_radius(SWITCH_HEIGHT / 2.0)
		.size(SWITCH_WIDTH, SWITCH_HEIGHT)
		.with_props(|props| props.accessibility_label = Some(name.to_owned()))
		.on_press(state.toggle(name))
		.child(
			circle(KNOB, colors.group)
				.border(0.5, hex("#00000014"))
				.pinned(
					inset,
					if on {
						SWITCH_WIDTH - KNOB - inset
					} else {
						inset
					},
				),
		)
}

fn chevron(colors: &Palette) -> Element {
	Element::text("›")
		.font(22.0, FontWeight::Regular)
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
			Element::view()
				.row()
				.gap(space::S)
				.child(label(value, type_scale::BODY, colors.muted))
				.child(chevron(colors))
		}
		Accessory::Disclosure => chevron(colors),
		Accessory::Switch(name) => switch(state, name, colors),
	};
	Element::view()
		.row()
		.gap(space::M)
		.height(ROW_HEIGHT)
		.padding(0.0, space::L)
		.child(icon)
		.child(label(title, type_scale::BODY, colors.ink).grow())
		.child(trailing)
}

/// A white, rounded group of rows with inset separators between them.
fn group(rows: Vec<Element>, colors: &Palette) -> Element {
	let count = rows.len();
	let mut group = Element::view()
		.column()
		.background(colors.group)
		.corner_radius(12.0);
	for (index, row) in rows.into_iter().enumerate() {
		group = group.child(row);
		if index + 1 < count {
			// Separators start where the titles do, past the icon tiles.
			group = group.child(
				Element::view()
					.background(colors.separator)
					.height(1.0)
					.with_style(|style| {
						style.margin.left =
							leptoskit_yoga::Dimension::Points(space::L + TILE + space::M);
					}),
			);
		}
	}
	group
}

/// A group with a small explanatory caption below it.
fn with_footnote(group: Element, text: &'static str, colors: &Palette) -> Element {
	Element::view().column().gap(space::S).child(group).child(
		Element::view().padding(0.0, space::L).child(label(
			text,
			type_scale::FOOTNOTE,
			colors.muted,
		)),
	)
}

/// The settings screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();

	let search = Element::view()
		.row()
		.gap(space::S)
		.height(36.0)
		.padding(0.0, space::S + space::XS)
		.background(hex("#e3e3e8"))
		.corner_radius(10.0)
		.child(
			Element::view()
				.size(13.0, 13.0)
				.corner_radius(6.5)
				.border(2.0, colors.muted),
		)
		.child(
			Element::text_input()
				.with_props(|props| props.placeholder = Some("Search".to_owned()))
				.font(type_scale::BODY.size, type_scale::BODY.weight)
				.color(colors.ink)
				.background(hex("#00000000"))
				.height(20.0)
				.grow(),
		);

	let account = Element::view()
		.row()
		.gap(space::M)
		.padding(space::M, space::L)
		.background(colors.group)
		.corner_radius(12.0)
		.child(
			Element::image(state.assets.avatar(2))
				.corner_radius(30.0)
				.size(60.0, 60.0),
		)
		.child(
			Element::view()
				.column()
				.gap(space::XXS)
				.grow()
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

	Element::view()
		.column()
		.fill()
		.background(colors.canvas)
		.padding_edges(56.0, space::L, 0.0, space::L)
		.gap(space::XL)
		.child(
			Element::view()
				.column()
				.gap(space::M)
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
