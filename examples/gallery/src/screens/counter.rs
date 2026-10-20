//! The counter, polished: a daily tap goal in the leptoskit brand palette.

use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
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

/// A handler that adds `delta` to the count, never going below zero.
fn adjust(count: &Arc<AtomicI32>, delta: i32) -> impl Fn() + Send + Sync + 'static {
	let count = Arc::clone(count);
	move || {
		let _ = count.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
			Some((value + delta).max(0))
		});
	}
}

/// A handler that resets the count.
fn reset(count: &Arc<AtomicI32>) -> impl Fn() + Send + Sync + 'static {
	let count = Arc::clone(count);
	move || count.store(0, Ordering::SeqCst)
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

	let header = Element::view()
		.column()
		.gap(space::XS)
		.child(label("LEPTOSKIT", type_scale::EYEBROW, colors.ember))
		.child(label("Daily taps", type_scale::LARGE_TITLE, colors.ink));

	let progress_bar = Element::view()
		.height(8.0)
		.background(alpha(colors.ink, 0.07))
		.corner_radius(4.0)
		.child(
			Element::view()
				.background(colors.mint)
				.corner_radius(4.0)
				.with_style(|style| {
					style.width = Dimension::Percent(progress);
					style.height = Dimension::Percent(1.0);
				}),
		);

	let hero = Element::view()
		.column()
		.align(Align::Center)
		.padding(space::XXL, space::XL)
		.gap(space::S)
		.background(colors.card)
		.corner_radius(28.0)
		.border(1.0, alpha(colors.ink, 0.08))
		.child(label("TODAY", type_scale::EYEBROW, colors.muted))
		.child(
			Element::text(count.to_string())
				.font(112.0, FontWeight::Bold)
				.color(colors.ink),
		)
		.child(label(status, type_scale::BODY, colors.muted))
		.child(
			Element::view()
				.column()
				.gap(space::S)
				.align_self(Align::Stretch)
				.padding_edges(space::L, 0.0, 0.0, 0.0)
				.child(progress_bar)
				.child(
					Element::view()
						.row()
						.justify(JustifyContent::SpaceBetween)
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

	let controls = Element::view()
		.row()
		.gap(space::M)
		.child(
			Element::button("−")
				.font(28.0, FontWeight::Medium)
				.color(colors.ink)
				.background(colors.card)
				.border(1.0, alpha(colors.ink, 0.12))
				.corner_radius(32.0)
				.size(64.0, 64.0)
				.on_press(adjust(&state.count, -1)),
		)
		.child(
			Element::button("Add one")
				.font(17.0, FontWeight::Semibold)
				.color(hex("#ffffff"))
				.background(colors.ember)
				.corner_radius(32.0)
				.height(64.0)
				.grow()
				.on_press(adjust(&state.count, 1)),
		);

	Element::view()
		.column()
		.fill()
		.background(colors.paper)
		.padding_edges(72.0, space::XL, 40.0, space::XL)
		.gap(space::XL)
		.child(header)
		.child(spacer())
		.child(hero)
		.child(spacer())
		.child(controls)
		.child(
			Element::button("Reset")
				.font(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
				.color(colors.muted)
				.background(Color::rgba(0, 0, 0, 0))
				.height(44.0)
				.on_press(reset(&state.count)),
		)
		.child(
			label(
				"Rendered by leptoskit with native AppKit views",
				type_scale::CAPTION,
				alpha(colors.ink, 0.4),
			)
			.text_align(TextAlign::Center),
		)
}
