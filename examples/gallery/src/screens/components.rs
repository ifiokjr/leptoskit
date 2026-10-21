//! A shared primitive studio with controlled inputs and Tailwind compositions.

use leptoskit::prelude::*;

use crate::ScreenState;

fn title(number: &str, heading: &str, detail: &str) -> Element {
	View().class("gap-2").children([
		Text(number).class("text-xs font-semibold text-indigo-600"),
		Text(heading).class("text-[22px] font-bold text-slate-900"),
		Text(detail).class("text-sm text-slate-500"),
	])
}

fn badge(label: &str) -> Element {
	View()
		.class(
			"self-start items-center justify-center min-w-5 min-h-5 px-1.5 py-0.5 rounded-[10px] \
			 text-[11px] font-semibold",
		)
		.child(Text(label))
}

fn separator() -> Element {
	View().class("h-px self-stretch bg-slate-300")
}

fn controls(state: &ScreenState) -> Element {
	let group = RadioGroup::new("density").expect("a literal group name is valid");
	View().class("rounded-xl p-4 gap-4 bg-white").children([
		title(
			"01 / INPUT",
			"Make it yours",
			"Real controls. One shared state.",
		),
		separator(),
		View()
			.class("flex-row items-center gap-3 min-h-8")
			.children([
				Text("Haptic feedback").class("flex-1 min-w-0 text-sm"),
				Switch(state.is_on("Haptics"))
					.accessibility_label("Haptic feedback")
					.on_toggle(state.set_switch("Haptics")),
			]),
		View()
			.class("flex-row items-center gap-3 min-h-8")
			.children([
				Checkbox(state.is_on("Preview enabled"))
					.accessibility_label("Enable previews")
					.on_toggle(state.set_switch("Preview enabled")),
				Text("Enable previews").class("flex-1 min-w-0 text-sm"),
			]),
		Text("Density").class("text-xs font-semibold text-slate-500"),
		View().class("flex-row items-center gap-3").children([
			Radio(group.clone(), state.density() == 0)
				.accessibility_label("Comfortable")
				.on_toggle(state.select_density(0)),
			Text("Comfortable").class("text-sm"),
			Radio(group, state.density() == 1)
				.accessibility_label("Compact")
				.on_toggle(state.select_density(1)),
			Text("Compact").class("text-sm"),
		]),
		View().class("gap-2").children([
			View().class("flex-row justify-between").children([
				Text("Volume").class("text-sm"),
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
		View().class("gap-2").children([
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
		View().class("flex-1 min-h-0"),
		View().class("flex-row gap-3 items-center").children([
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
	View().class("rounded-xl p-4 gap-5 bg-white").children([
		title(
			"02 / COMPOSE",
			"Small pieces, more room",
			"Native primitives plus Tailwind.",
		),
		View()
			.class(
				"relative h-[148px] items-center justify-center bg-indigo-100 rounded-xl \
				 overflow-hidden",
			)
			.children([
				Text("Built together").class("text-[24px] font-bold text-indigo-900"),
				badge("LIVE").class("absolute top-3 right-3 bg-[#1e40af] text-white"),
			]),
		View()
			.class("flex-row flex-wrap content-start gap-2")
			.children(["Native", "Web", "Tailwind"].map(|label| {
				View()
					.class(
						"self-start items-center min-h-8 px-3 py-1.5 rounded-2xl bg-slate-100 \
						 text-slate-900 text-[13px] font-medium",
					)
					.child(Text(label))
			})),
		separator(),
		View().class("flex-row gap-3 h-16").children([
			View()
				.class("flex-1 min-w-0 min-h-0 items-center justify-center bg-slate-100 rounded-lg")
				.child(Text("1 share").class("text-xs text-slate-500")),
			View()
				.class(
					"grow-2 shrink basis-0 min-w-0 min-h-0 items-center justify-center \
					 bg-indigo-100 rounded-lg",
				)
				.child(Text("2 shares").class("text-xs text-indigo-900")),
		]),
		View().class("flex-1 min-h-0"),
		Pressable("Add a preview")
			.class("flex-row gap-3 p-4 rounded-xl bg-indigo-600 text-white")
			.on_press(state.adjust_count(1))
			.children([
				Text("+").class("text-[26px] font-semibold"),
				View().class("gap-1").children([
					Text("Add a preview").class("text-sm font-semibold"),
					Text(format!("{} previews created", state.count())).class("text-xs"),
				]),
			]),
	])
}

fn collection() -> Element {
	View().class("rounded-xl p-4 gap-5 bg-white").children([
		title(
			"03 / COLLECT",
			"A place for every idea",
			"An eager list with room to scroll.",
		),
		ScrollView()
			.class("grow shrink min-h-0 min-w-0 gap-3")
			.children((1..=12).map(|index| {
				View()
					.class("flex-row items-center gap-3 min-h-12 p-2 bg-slate-50 rounded-xl")
					.children([
						View()
							.class("size-10 items-center justify-center rounded-lg bg-rose-100")
							.child(
								Text(format!("{index:02}"))
									.class("text-xs font-semibold text-rose-700"),
							),
						View().class("flex-1 min-w-0 min-h-0 gap-1").children([
							Text(format!("Preview {index:02}"))
								.class("text-sm font-semibold text-slate-900"),
							Text("Ready to explore").class("text-xs text-slate-500"),
						]),
						badge("NEW").class("bg-slate-100 text-slate-500"),
					])
			})),
	])
}

/// A desktop-sized studio rendered from native primitives on every adapter.
pub fn screen(state: &ScreenState) -> Element {
	View()
		.class("w-full h-full bg-slate-100 text-slate-900")
		.children([
			View()
				.class("flex-row items-center gap-3 bg-white px-6 py-3 h-20")
				.children([
					View()
						.class("flex-1 min-w-0 flex-row gap-3 items-center")
						.children([
							Text("leptoskit").class("text-[22px] font-bold text-slate-900"),
							badge("COMPONENT STUDIO").class("bg-indigo-100 text-indigo-700"),
						]),
					Text("Shared by design").class("text-sm text-slate-500"),
				]),
			View()
				.class("flex-1 min-w-0 min-h-0 grid grid-cols-3 grid-rows-1 p-6 gap-5")
				.children([controls(state), composition(state), collection()]),
			View()
				.class("flex-row justify-between items-center px-6 py-3")
				.children([
					Text("Primitives first. Helpers optional.").class("text-xs text-slate-500"),
					Text("Native + Web").class("text-xs font-semibold text-indigo-600"),
				]),
		])
}
