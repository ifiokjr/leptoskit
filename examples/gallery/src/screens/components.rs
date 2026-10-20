//! A shared component studio with live controlled values and layout examples.

use std::num::NonZeroU8;

use leptoskit::prelude::*;
use leptoskit::yoga::Rect;

use crate::ScreenState;

fn title(number: &str, heading: &str, detail: &str) -> Element {
	Column().class("gap-2").children([
		Text(number).class("text-xs font-semibold text-indigo-600"),
		Text(heading).class("text-[22px] font-bold text-slate-900"),
		Text(detail).class("text-sm text-slate-500"),
	])
}

fn controls(state: &ScreenState) -> Element {
	let group = RadioGroup::new("density").expect("a literal group name is valid");
	Card().class("gap-4 bg-white").children([
		title(
			"01 / INPUT",
			"Make it yours",
			"Real controls. One shared state.",
		),
		Divider(),
		ListTile(
			None,
			Text("Haptic feedback").class("text-sm"),
			Some(
				Switch(state.is_on("Haptics"))
					.accessibility_label("Haptic feedback")
					.on_toggle(state.set_switch("Haptics")),
			),
		)
		.class("p-0 min-h-8"),
		ListTile(
			Some(
				Checkbox(state.is_on("Preview enabled"))
					.accessibility_label("Enable previews")
					.on_toggle(state.set_switch("Preview enabled")),
			),
			Text("Enable previews").class("text-sm"),
			None,
		)
		.class("p-0 min-h-8"),
		Text("Density").class("text-xs font-semibold text-slate-500"),
		Row().class("items-center gap-3").children([
			Radio(group.clone(), state.density() == 0)
				.accessibility_label("Comfortable")
				.on_toggle(state.select_density(0)),
			Text("Comfortable").class("text-sm"),
			Radio(group, state.density() == 1)
				.accessibility_label("Compact")
				.on_toggle(state.select_density(1)),
			Text("Compact").class("text-sm"),
		]),
		Column().class("gap-2").children([
			Row().children([
				Text("Volume").class("text-sm"),
				Spacer(),
				Text(format!("{:.0}%", state.volume())).class("text-sm font-semibold"),
			]),
			Slider(
				SliderState::new(0.0, 100.0, state.volume(), Some(1.0))
					.expect("gallery volume stays on its validated grid"),
			)
			.class("w-full")
			.accessibility_label("Volume")
			.on_value_change(state.set_volume()),
		]),
		Column().class("gap-2").children([
			Text("Password").class("text-xs font-semibold text-slate-500"),
			View()
				.class("w-full h-10 rounded-lg border border-slate-300 p-2")
				.child(
					PasswordInput()
						.class("w-full h-full border-0 text-slate-900")
						.value(state.password())
						.placeholder("Try a masked value")
						.accessibility_label("Demo password")
						.on_change(state.set_password()),
				),
		]),
		Spacer(),
		Row().class("gap-3 items-center").children([
			ActivityIndicator(true).accessibility_label("Syncing component preview"),
			Text("Syncing in the background").class("text-xs text-slate-500"),
		]),
		LinearProgressIndicator(Progress::Determinate(
			ProgressValue::new(state.volume() / 100.0).expect("volume is a completion fraction"),
		))
		.class("w-full")
		.accessibility_label("Preview completion"),
	])
}

fn composition(state: &ScreenState) -> Element {
	let insets = Rect::all_edges(
		Dimension::Points(12.0),
		Dimension::Points(12.0),
		Dimension::Auto,
		Dimension::Auto,
	);
	let stack = Stack([
		Center()
			.class("w-full h-[148px] bg-indigo-100")
			.child(Text("Built together").class("text-[24px] font-bold text-indigo-900")),
		Positioned(insets, Badge("LIVE")),
	]);
	Card().class("gap-5 bg-white").children([
		title(
			"02 / COMPOSE",
			"Small pieces, more room",
			"Typed layout meets Tailwind.",
		),
		ClipRRect(12.0).expect("literal radius").child(stack),
		Wrap()
			.class("gap-2")
			.children([Chip("Native"), Chip("Web"), Chip("Tailwind")]),
		Divider(),
		Row().class("gap-3 h-16").children([
			Expanded(
				Center()
					.class("bg-slate-100 rounded-lg")
					.child(Text("1 share").class("text-xs text-slate-500")),
			),
			Expanded(
				Center()
					.class("bg-indigo-100 rounded-lg")
					.child(Text("2 shares").class("text-xs text-indigo-900")),
			)
			.with_style(|style| style.flex_grow = 2.0),
		]),
		Spacer(),
		Pressable("Add a preview")
			.class("flex-row gap-3 p-4 rounded-xl bg-indigo-600 text-white")
			.on_press(state.adjust_count(1))
			.children([
				Text("+").class("text-[26px] font-semibold"),
				Column().class("gap-1").children([
					Text("Add a preview").class("text-sm font-semibold"),
					Text(format!("{} previews created", state.count())).class("text-xs"),
				]),
			]),
	])
}

fn collection() -> Element {
	Card().class("gap-5 bg-white").children([
		title(
			"03 / COLLECT",
			"A place for every idea",
			"An eager list with room to scroll.",
		),
		ListView().class("gap-3").children((1..=12).map(|index| {
			ListTile(
				Some(Center().class("size-10 rounded-lg bg-rose-100").child(
					Text(format!("{index:02}")).class("text-xs font-semibold text-rose-700"),
				)),
				Column().class("gap-1").children([
					Text(format!("Preview {index:02}"))
						.class("text-sm font-semibold text-slate-900"),
					Text("Ready to explore").class("text-xs text-slate-500"),
				]),
				Some(Badge("NEW").class("bg-slate-100 text-slate-500")),
			)
			.class("p-2 bg-slate-50 rounded-xl")
		})),
	])
}

/// A desktop-sized studio, rendered by the same native and browser adapters.
pub fn screen(state: &ScreenState) -> Element {
	let header = AppBar(Row().class("gap-3 items-center").children([
		Text("leptoskit").class("text-[22px] font-bold text-slate-900"),
		Badge("COMPONENT STUDIO").class("bg-indigo-100 text-indigo-700"),
	]))
	.class("bg-white px-6 h-20")
	.child(Text("Shared by design").class("text-sm text-slate-500"));
	let body = Grid(NonZeroU8::new(3).expect("three is nonzero"))
		.class("p-6 gap-5 grid-rows-1")
		.children([controls(state), composition(state), collection()]);
	let footer = Row().class("px-6 py-3 items-center").children([
		Text("45 builders. One Element tree.").class("text-xs text-slate-500"),
		Spacer(),
		Text("AppKit + DOM").class("text-xs font-semibold text-indigo-600"),
	]);
	Scaffold(Some(header), body, Some(footer)).class("w-full h-full bg-slate-100 text-slate-900")
}
