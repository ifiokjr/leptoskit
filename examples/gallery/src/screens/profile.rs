//! A profile card: cover, overlapping avatar, bio, stats, and recent work.

use leptoskit::prelude::*;
use leptoskit_native::Color;
use leptoskit_yoga::Dimension;

use crate::ScreenState;
use crate::ui::Label;
use crate::ui::alpha;
use crate::ui::hex;
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
#[component]
fn Stat(value: &'static str, caption: &'static str, colors: &Palette) -> Element {
	view! {
		<View class="flex-col flex-1 gap-0.5 items-center">
			<Label style=type_scale::HEADLINE color=colors.ink>{value}</Label>
			<Label style=type_scale::CAPTION color=colors.muted>{caption}</Label>
		</View>
	}
}

/// A vertical rule between stats.
#[component]
fn StatDivider(colors: &Palette) -> Element {
	view! { <View background=colors.line class="h-7 w-[1px]" /> }
}

/// The pill buttons beside the avatar.
#[component]
fn Action(colors: &Palette, #[prop(optional)] filled: bool, children: Children) -> Element {
	let button = view! {
		<Button
			font=(type_scale::BODY_STRONG.size, type_scale::BODY_STRONG.weight)
			class="py-0 h-9 rounded-[18px] px-[18px]"
		>
			{children}
		</Button>
	};
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
	let colors = &colors;

	view! {
		<View class="flex-col w-full h-full" background=colors.surface>
			// The cover runs under the status bar; everything above the tab bar
			// scrolls on short phones.
			<ScrollView class="flex-1">
				<View class="flex-col pb-5">
					<Image source=assets.cover() aria_hidden=true class="w-full h-[140px]" />
					<View class="flex-col gap-4 px-5">
						// The actions share a band with the lower half of the avatar.
						<View class="flex-row gap-2 justify-end items-center pt-3 h-12">
							<Action colors=colors>"Message"</Action>
							<Action colors=colors filled>"Follow"</Action>
						</View>
						<View class="flex-col gap-1">
							<View class="flex-row gap-2 items-center">
								<Label style=type_scale::TITLE color=colors.ink>
									"Amara Okafor"
								</Label>
								<View
									class="py-0.5 px-2"
									background=alpha(colors.accent, 0.1)
									class="rounded-[6px]"
								>
									<Label style=type_scale::EYEBROW color=colors.accent>
										"PRO"
									</Label>
								</View>
							</View>
							<Label style=type_scale::FOOTNOTE color=colors.muted>
								"@amara · Product designer"
							</Label>
						</View>
						<Label style=type_scale::BODY color=colors.ink>
							"Designing calm software for loud cities. Currently building a field guide to West \
							 African type foundries, one specimen at a time."
						</Label>
						<Label style=type_scale::FOOTNOTE color=colors.muted>
							"Lagos, Nigeria  ·  Joined March 2021  ·  amara.design"
						</Label>
						<View
							class="flex-row items-center py-3 px-0"
							border=(1.0, colors.line)
							class="rounded-[14px]"
						>
							<Stat value="1,284" caption="Followers" colors=colors />
							<StatDivider colors=colors />
							<Stat value="312" caption="Following" colors=colors />
							<StatDivider colors=colors />
							<Stat value="86" caption="Projects" colors=colors />
						</View>
						<View class="flex-row justify-between items-center mt-1">
							<Label style=type_scale::HEADLINE color=colors.ink>"Recent work"</Label>
							<Label style=type_scale::BODY color=colors.accent>"See all"</Label>
						</View>
						<View class="flex-row gap-2 items-center">
							{(1..=3)
								.map(|index| {
									view! {
										<Image
											source=assets.photo(index * 4)
											class="flex-1 rounded-[14px] aspect-square"
										/>
									}
								})
								.collect::<Fragment>()}
						</View>
						<Mutuals state=state colors=colors />
					</View>
					// The avatar straddles the cover's bottom edge, so it sits outside the
					// flow, over both.
					<View
						class="absolute left-5 p-1 w-24 h-24 top-[92px]"
						background=colors.surface
						class="rounded-[48px]"
					>
						<Image
							source=assets.avatar(0)
							accessibility_label="Profile photo"
							class="w-full h-full rounded-[44px]"
						/>
					</View>
				</View>
			</ScrollView>
			<TabBar colors=colors />
		</View>
	}
}

/// Overlapping faces of people you both follow, with a summary.
#[component]
fn Mutuals(state: &ScreenState, colors: &Palette) -> Element {
	const FACE: f32 = 28.0;
	const OVERLAP: f32 = 10.0;
	let faces = (1..=3)
		.map(|index| {
			view! {
				<View
					class="absolute top-0"
					with_style=|style| {
						style.inset.left = Dimension::Points((index - 1) as f32 * (FACE - OVERLAP));
					}
					class="p-0.5 w-7 h-7"
					background=colors.surface
					class="rounded-[14px]"
				>
					<Image
						source=state.assets.avatar(index)
						aria_hidden=true
						class="w-full h-full rounded-[12px]"
					/>
				</View>
			}
		})
		.collect::<Fragment>();

	view! {
		<View class="flex-row gap-2 items-center">
			<View class="w-16 h-7">{faces}</View>
			<Label style=type_scale::FOOTNOTE color=colors.muted class="flex-1">
				"Followed by Kemi, Tolu, and 12 others"
			</Label>
		</View>
	}
}

/// One destination in the tab bar.
#[component]
fn Tab(
	title: &'static str,
	icon: Element,
	#[prop(optional)] selected: bool,
	colors: &Palette,
) -> Element {
	let tint = if selected { colors.ink } else { colors.muted };
	view! {
		<View class="flex-col flex-1 gap-1 items-center">
			{icon.opacity(if selected { 1.0 } else { 0.7 })}
			<Label style=type_scale::CAPTION color=tint class="text-center">{title}</Label>
		</View>
	}
}

/// The bottom navigation, with the profile tab selected.
#[component]
fn TabBar(colors: &Palette) -> Element {
	let outline = |radius: f32| {
		view! { <View class="w-[22px] h-[22px]" corner_radius=radius border=(2.0, colors.muted) /> }
	};
	view! {
		<View class="flex-col">
			<View background=colors.line class="h-[1px]" />
			<View class="flex-row items-center px-2 pt-2 pb-safe-offset-2">
				<Tab title="Home" icon=outline(6.0) colors=colors />
				<Tab title="Search" icon=outline(11.0) colors=colors />
				<Tab title="Inbox" icon=outline(4.0) colors=colors />
				<Tab
					title="Profile"
					icon=view! {
						<View class="w-[22px] h-[22px] rounded-[11px]" background=colors.ink />
					}
					selected
					colors=colors
				/>
			</View>
		</View>
	}
}
