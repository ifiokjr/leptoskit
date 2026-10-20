//! A profile card: cover, overlapping avatar, bio, stats, and recent work.

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::TextAlign;
use leptoskit_yoga::Align;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::JustifyContent;

use crate::ScreenState;
use crate::ui::Layout;
use crate::ui::alpha;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::space;
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

const AVATAR: f32 = 96.0;
const AVATAR_RING: f32 = 4.0;
/// The cover's height at the phone width, from its 1170×420 source.
const COVER_HEIGHT: f32 = 140.0;

/// One figure in the stats strip.
fn stat(value: &str, caption: &str, colors: &Palette) -> Element {
	Element::view()
		.column()
		.align(Align::Center)
		.gap(space::XXS)
		.grow()
		.child(label(value, type_scale::HEADLINE, colors.ink))
		.child(label(caption, type_scale::CAPTION, colors.muted))
}

/// A vertical rule between stats.
fn stat_divider(colors: &Palette) -> Element {
	Element::view().background(colors.line).size(1.0, 28.0)
}

/// The pill buttons beside the avatar.
fn action(title: &str, filled: bool, colors: &Palette) -> Element {
	let button = Element::button(title)
		.font(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
		.corner_radius(18.0)
		.padding(0.0, 18.0)
		.height(36.0);
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

	let cover = Element::image(assets.cover())
		.with_style(|style| style.width = Dimension::Percent(1.0))
		.height(COVER_HEIGHT);

	// The avatar straddles the cover's bottom edge, so it sits outside the
	// flow, over both.
	let avatar = Element::view()
		.pinned(COVER_HEIGHT - AVATAR / 2.0, space::L + space::XS)
		.size(AVATAR, AVATAR)
		.padding(AVATAR_RING, AVATAR_RING)
		.background(colors.surface)
		.corner_radius(AVATAR / 2.0)
		.child(
			Element::image(assets.avatar(0))
				.corner_radius(AVATAR / 2.0 - AVATAR_RING)
				.fill(),
		);

	// The actions share a band with the lower half of the avatar.
	let identity = Element::view()
		.row()
		.justify(JustifyContent::FlexEnd)
		.height(AVATAR / 2.0)
		.padding_edges(space::M, 0.0, 0.0, 0.0)
		.gap(space::S)
		.child(action("Message", false, &colors))
		.child(action("Follow", true, &colors));

	let name = Element::view()
		.row()
		.gap(space::S)
		.child(label("Amara Okafor", type_scale::TITLE, colors.ink))
		.child(
			Element::view()
				.padding(2.0, space::S)
				.background(alpha(colors.accent, 0.1))
				.corner_radius(6.0)
				.child(label("PRO", type_scale::EYEBROW, colors.accent)),
		);

	let stat_strip = Element::view()
		.row()
		.padding(space::M, 0.0)
		.border(1.0, colors.line)
		.corner_radius(14.0)
		.child(stat("1,284", "Followers", &colors))
		.child(stat_divider(&colors))
		.child(stat("312", "Following", &colors))
		.child(stat_divider(&colors))
		.child(stat("86", "Projects", &colors));

	let recent = Element::view()
		.row()
		.gap(space::S)
		.children((1..=3).map(|index| {
			Element::image(assets.photo(index * 4))
				.corner_radius(14.0)
				.aspect_ratio(1.0)
				.grow()
		}));

	let body = Element::view()
		.column()
		.padding_edges(0.0, space::L + space::XS, 0.0, space::L + space::XS)
		.gap(space::L)
		.child(identity)
		.child(
			Element::view()
				.column()
				.gap(space::XS)
				.child(name)
				.child(label(
					"@amara · Product designer",
					type_scale::FOOTNOTE,
					colors.muted,
				)),
		)
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
			Element::view()
				.row()
				.justify(JustifyContent::SpaceBetween)
				.margin_top(space::XS)
				.child(label("Recent work", type_scale::HEADLINE, colors.ink))
				.child(label("See all", type_scale::BODY, colors.accent)),
		)
		.child(recent)
		.child(mutuals(state, &colors));

	Element::view()
		.column()
		.fill()
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
	let faces = Element::view()
		.size(FACE + 2.0 * (FACE - OVERLAP), FACE)
		.children((1..=3).map(|index| {
			Element::view()
				.pinned(0.0, (index - 1) as f32 * (FACE - OVERLAP))
				.size(FACE, FACE)
				.padding(2.0, 2.0)
				.background(colors.surface)
				.corner_radius(FACE / 2.0)
				.child(
					Element::image(state.assets.avatar(index))
						.corner_radius(FACE / 2.0 - 2.0)
						.fill(),
				)
		}));
	Element::view().row().gap(space::S).child(faces).child(
		label(
			"Followed by Kemi, Tolu, and 12 others",
			type_scale::FOOTNOTE,
			colors.muted,
		)
		.grow(),
	)
}

/// The bottom navigation, with the profile tab selected.
fn tab_bar(colors: &Palette) -> Element {
	let tab = |title: &str, icon: Element, selected: bool| {
		let tint = if selected { colors.ink } else { colors.muted };
		Element::view()
			.column()
			.align(Align::Center)
			.gap(space::XS)
			.grow()
			.child(icon.opacity(if selected { 1.0 } else { 0.7 }))
			.child(label(title, type_scale::CAPTION, tint).text_align(TextAlign::Center))
	};
	let outline = |radius: f32, color: Color| {
		Element::view()
			.size(22.0, 22.0)
			.corner_radius(radius)
			.border(2.0, color)
	};
	Element::view()
		.column()
		.child(Element::view().background(colors.line).height(1.0))
		.child(
			Element::view()
				.row()
				.padding_edges(space::S, space::S, 30.0, space::S)
				.child(tab("Home", outline(6.0, colors.muted), false))
				.child(tab("Search", outline(11.0, colors.muted), false))
				.child(tab("Inbox", outline(4.0, colors.muted), false))
				.child(tab(
					"Profile",
					Element::view()
						.size(22.0, 22.0)
						.corner_radius(11.0)
						.background(colors.ink),
					true,
				)),
		)
}
