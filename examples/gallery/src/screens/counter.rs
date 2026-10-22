//! The counter, polished: a daily tap goal in the leptoskit brand palette.

use leptoskit::prelude::*;
use leptoskit_native::Color;
use leptoskit_yoga::Dimension;

use crate::ScreenState;
use crate::ui::Label;
use crate::ui::alpha;
use crate::ui::hex;
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

	view! {
		// The spacers center the hero on tall phones; short ones scroll instead.
		<ScrollView class="w-full h-full" background=colors.paper>
			<View class="flex-col gap-6 px-6 min-h-full pt-safe-offset-4 pb-safe-offset-4">
				<View class="flex-col gap-1">
					<Label style=type_scale::EYEBROW color=colors.ember>"LEPTOSKIT"</Label>
					<Label style=type_scale::LARGE_TITLE color=colors.ink>"Daily taps"</Label>
				</View>
				<View class="flex-1" />
				<View
					class="flex-col gap-2 items-center py-8 px-6"
					background=colors.card
					class="rounded-[28px]"
					border=(1.0, alpha(colors.ink, 0.08))
				>
					<Label style=type_scale::EYEBROW color=colors.muted>"TODAY"</Label>
					<Text class="font-bold text-[112px]" color=colors.ink>{count}</Text>
					<Label style=type_scale::BODY color=colors.muted>{status}</Label>
					<View class="flex-col gap-2 self-stretch pt-4">
						<View class="h-2" background=alpha(colors.ink, 0.07) class="rounded-[4px]">
							<View
								background=colors.mint
								class="h-full rounded-[4px]"
								with_style=move |style| style.width = Dimension::Percent(progress)
							/>
						</View>
						<View class="flex-row justify-between items-center">
							<Label style=type_scale::FOOTNOTE color=colors.muted>
								{count} " of " {GOAL}
							</Label>
							<Label style=type_scale::FOOTNOTE color=colors.mint>
								{(progress * 100.0).round()} "%"
							</Label>
						</View>
					</View>
				</View>
				<View class="flex-1" />
				<View class="flex-row gap-3 items-center">
					<Button
						class="font-medium text-[28px]"
						color=colors.ink
						background=colors.card
						border=(1.0, alpha(colors.ink, 0.12))
						class="w-16 h-16 rounded-[32px]"
						on:press=state.adjust_count(-1)
					>
						"−"
					</Button>
					<Button
						class="font-semibold text-[17px]"
						class="text-[#ffffff]"
						background=colors.ember
						class="flex-1 h-16 rounded-[32px]"
						on:press=state.adjust_count(1)
					>
						"Add one"
					</Button>
				</View>
				<Button
					font=(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
					color=colors.muted
					background=Color::rgba(0, 0, 0, 0)
					class="h-11"
					on:press=state.reset_count()
				>
					"Reset"
				</Button>
				<Label style=type_scale::CAPTION color=alpha(colors.ink, 0.4) class="text-center">
					"One leptoskit screen, on every platform"
				</Label>
			</View>
		</ScrollView>
	}
}
