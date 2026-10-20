//! A profile card: cover, overlapping avatar, bio, stats, and recent work.

use leptoskit::prelude::Button;
use leptoskit::prelude::Image;
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

struct Palette {
	surface: Color,
	ink: Color,
	muted: Color,
	line: Color,
	accent: Color,
}

fn palette() -> Palette {
	Palette {
		surface: hex("#ffffff"),
		ink: hex("#111827"),
		muted: hex("#6b7280"),
		line: hex("#e5e7eb"),
		accent: hex("#2563eb"),
	}
}

/// One figure in the stats strip.
fn stat(value: &str, caption: &str, colors: &Palette) -> Element {
	View()
		.class("flex-col items-center gap-0.5 flex-1")
		.child(label(value, type_scale::HEADLINE, colors.ink))
		.child(label(caption, type_scale::CAPTION, colors.muted))
}

/// A vertical rule between stats.
fn stat_divider(colors: &Palette) -> Element {
	View().background(colors.line).class("w-[1px] h-7")
}

/// The pill buttons beside the avatar.
fn action(title: &str, filled: bool, colors: &Palette) -> Element {
	let button = Button(title)
		.font(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
		.class("rounded-[18px] py-0 px-[18px] h-9");
	if filled {
		button.background(colors.ink).color(colors.surface)
	} else {
		button
			.background(colors.surface)
			.color(colors.ink)
			.border(1.0, colors.line)
	}
}

/// The profile screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();
	let assets = &state.assets;

	let cover = Image(assets.cover()).class("w-full h-[140px]");

	// The avatar straddles the cover's bottom edge, so it sits outside the
	// flow, over both.
	let avatar = View()
		.class("absolute top-[92px] left-5 w-24 h-24 p-1")
		.background(colors.surface)
		.class("rounded-[48px]")
		.child(Image(assets.avatar(0)).class("rounded-[44px] w-full h-full"));

	// The actions share a band with the lower half of the avatar.
	let identity = View()
		.class("flex-row items-center justify-end h-12 pt-3 gap-2")
		.child(action("Message", false, &colors))
		.child(action("Follow", true, &colors));

	let name = View()
		.class("flex-row items-center gap-2")
		.child(label("Amara Okafor", type_scale::TITLE, colors.ink))
		.child(
			View()
				.class("py-0.5 px-2")
				.background(alpha(colors.accent, 0.1))
				.class("rounded-[6px]")
				.child(label("PRO", type_scale::EYEBROW, colors.accent)),
		);

	let stat_strip = View()
		.class("flex-row items-center py-3 px-0")
		.border(1.0, colors.line)
		.class("rounded-[14px]")
		.child(stat("1,284", "Followers", &colors))
		.child(stat_divider(&colors))
		.child(stat("312", "Following", &colors))
		.child(stat_divider(&colors))
		.child(stat("86", "Projects", &colors));

	let recent =
		View()
			.class("flex-row items-center gap-2")
			.children((1..=3).map(|index| {
				Image(assets.photo(index * 4)).class("rounded-[14px] aspect-square flex-1")
			}));

	let body = View()
		.class("flex-col px-5 gap-4")
		.child(identity)
		.child(View().class("flex-col gap-1").child(name).child(label(
			"@amara · Product designer",
			type_scale::FOOTNOTE,
			colors.muted,
		)))
		.child(label(
			"Designing calm software for loud cities. Currently building a field guide to West \
			 African type foundries, one specimen at a time.",
			type_scale::BODY,
			colors.ink,
		))
		.child(label(
			"Lagos, Nigeria  ·  Joined March 2021  ·  amara.design",
			type_scale::FOOTNOTE,
			colors.muted,
		))
		.child(stat_strip)
		.child(
			View()
				.class("flex-row items-center justify-between mt-1")
				.child(label("Recent work", type_scale::HEADLINE, colors.ink))
				.child(label("See all", type_scale::BODY, colors.accent)),
		)
		.child(recent)
		.child(mutuals(state, &colors));

	View()
		.class("flex-col w-full h-full")
		.background(colors.surface)
		.child(cover)
		.child(body)
		.child(spacer())
		.child(tab_bar(&colors))
		.child(avatar)
}

/// Overlapping faces of people you both follow, with a summary.
fn mutuals(state: &ScreenState, colors: &Palette) -> Element {
	const FACE: f32 = 28.0;
	const OVERLAP: f32 = 10.0;
	let faces = View().class("w-16 h-7").children((1..=3).map(|index| {
		View()
			.class("absolute top-0")
			.with_style(|style| {
				style.inset.left = Dimension::Points((index - 1) as f32 * (FACE - OVERLAP));
			})
			.class("w-7 h-7 p-0.5")
			.background(colors.surface)
			.class("rounded-[14px]")
			.child(Image(state.assets.avatar(index)).class("rounded-[12px] w-full h-full"))
	}));
	View()
		.class("flex-row items-center gap-2")
		.child(faces)
		.child(
			label(
				"Followed by Kemi, Tolu, and 12 others",
				type_scale::FOOTNOTE,
				colors.muted,
			)
			.class("flex-1"),
		)
}

/// The bottom navigation, with the profile tab selected.
fn tab_bar(colors: &Palette) -> Element {
	let tab = |title: &str, icon: Element, selected: bool| {
		let tint = if selected { colors.ink } else { colors.muted };
		View()
			.class("flex-col items-center gap-1 flex-1")
			.child(icon.opacity(if selected { 1.0 } else { 0.7 }))
			.child(label(title, type_scale::CAPTION, tint).class("text-center"))
	};
	let outline = |radius: f32, color: Color| {
		View()
			.class("w-[22px] h-[22px]")
			.corner_radius(radius)
			.border(2.0, color)
	};
	View()
		.class("flex-col")
		.child(View().background(colors.line).class("h-[1px]"))
		.child(
			View()
				.class("flex-row items-center pt-2 px-2 pb-[30px]")
				.child(tab("Home", outline(6.0, colors.muted), false))
				.child(tab("Search", outline(11.0, colors.muted), false))
				.child(tab("Inbox", outline(4.0, colors.muted), false))
				.child(tab(
					"Profile",
					View()
						.class("w-[22px] h-[22px] rounded-[11px]")
						.background(colors.ink),
					true,
				)),
		)
}
