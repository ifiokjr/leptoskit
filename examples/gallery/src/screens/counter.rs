//! The counter, polished: a daily tap goal in the leptoskit brand palette.

use leptoskit::prelude::Button;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_yoga::Dimension;

use crate::ScreenState;
use crate::ui::alpha;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::spacer;
use crate::ui::type_scale;

/// The daily goal the progress bar fills toward.
pub const GOAL: i32 = 20;

struct Palette {
	paper: Color,
	ink: Color,
	muted: Color,
	ember: Color,
	mint: Color,
	card: Color,
}

fn palette() -> Palette {
	let ink = hex("#16111f");
	Palette {
		paper: hex("#faf6f0"),
		ink,
		muted: alpha(ink, 0.55),
		ember: hex("#e5304f"),
		mint: hex("#14bf9b"),
		card: hex("#ffffff"),
	}
}

/// The counter screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();
	let count = state.count();
	let progress = (count as f32 / GOAL as f32).clamp(0.0, 1.0);
	let remaining = (GOAL - count).max(0);
	let status = if remaining == 0 {
		"Goal reached. Nicely done.".to_owned()
	} else if remaining == 1 {
		"1 tap to go".to_owned()
	} else {
		format!("{remaining} taps to go")
	};

	let header = View()
		.class("flex-col gap-1")
		.child(label("LEPTOSKIT", type_scale::EYEBROW, colors.ember))
		.child(label("Daily taps", type_scale::LARGE_TITLE, colors.ink));

	let progress_bar = View()
		.class("h-2")
		.background(alpha(colors.ink, 0.07))
		.class("rounded-[4px]")
		.child(
			View()
				.background(colors.mint)
				.class("rounded-[4px] h-full")
				.with_style(|style| style.width = Dimension::Percent(progress)),
		);

	let hero = View()
		.class("flex-col items-center py-8 px-6 gap-2")
		.background(colors.card)
		.class("rounded-[28px]")
		.border(1.0, alpha(colors.ink, 0.08))
		.child(label("TODAY", type_scale::EYEBROW, colors.muted))
		.child(
			Text(count.to_string())
				.class("text-[112px] font-bold")
				.color(colors.ink),
		)
		.child(label(status, type_scale::BODY, colors.muted))
		.child(
			View()
				.class("flex-col gap-2 self-stretch pt-4")
				.child(progress_bar)
				.child(
					View()
						.class("flex-row items-center justify-between")
						.child(label(
							format!("{count} of {GOAL}"),
							type_scale::FOOTNOTE,
							colors.muted,
						))
						.child(label(
							format!("{}%", (progress * 100.0).round()),
							type_scale::FOOTNOTE,
							colors.mint,
						)),
				),
		);

	let controls = View()
		.class("flex-row items-center gap-3")
		.child(
			Button("−")
				.class("text-[28px] font-medium")
				.color(colors.ink)
				.background(colors.card)
				.border(1.0, alpha(colors.ink, 0.12))
				.class("rounded-[32px] w-16 h-16")
				.on_press(state.adjust_count(-1)),
		)
		.child(
			Button("Add one")
				.class("text-[17px] font-semibold")
				.class("text-[#ffffff]")
				.background(colors.ember)
				.class("rounded-[32px] h-16 flex-1")
				.on_press(state.adjust_count(1)),
		);

	View()
		.class("flex-col w-full h-full")
		.background(colors.paper)
		.class("pt-[72px] pr-6 pb-10 pl-6 gap-6")
		.child(header)
		.child(spacer())
		.child(hero)
		.child(spacer())
		.child(controls)
		.child(
			Button("Reset")
				.font(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
				.color(colors.muted)
				.background(Color::rgba(0, 0, 0, 0))
				.class("h-11")
				.on_press(state.reset_count()),
		)
		.child(
			label(
				"One leptoskit screen, on every platform",
				type_scale::CAPTION,
				alpha(colors.ink, 0.4),
			)
			.class("text-center"),
		)
}
