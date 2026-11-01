//! A shared primitive studio with controlled inputs and Tailwind compositions.

use leptoskit::prelude::*;

use crate::ScreenState;

/// A numbered card heading, with the card title as a level-two heading.
#[component]
fn Title(number: &'static str, heading: &'static str, detail: &'static str) -> Element {
	view! {
		<View class="gap-2">
			<Text class="text-xs font-semibold text-indigo-600">{number}</Text>
			<Heading aria_level=2 class="font-bold text-[22px] text-slate-900">{heading}</Heading>
			<Text class="text-sm text-slate-500">{detail}</Text>
		</View>
	}
}

/// A small rounded label; `class` sets its colors.
#[component]
fn Pill(#[prop(into)] class: String, children: Children) -> Element {
	view! {
		<View
			class="justify-center items-center self-start py-0.5 px-1.5 font-semibold min-w-5 min-h-5 rounded-[10px] text-[11px]"
			class=class
		>
			{children}
		</View>
	}
}

#[component]
fn Separator() -> Element {
	view! { <View class="self-stretch h-px bg-slate-300" /> }
}

#[component]
fn Controls(state: &ScreenState) -> Element {
	let group = RadioGroup::new("density").expect("a literal group name is valid");
	let volume = SliderState::new(0.0, 100.0, state.volume(), Some(1.0))
		.expect("gallery volume stays on its validated grid");
	let completion =
		ProgressValue::new(state.volume() / 100.0).expect("volume is a completion fraction");

	view! {
		<View class="gap-4 p-4 bg-white rounded-xl">
			<Title
				number="01 / INPUT"
				heading="Make it yours"
				detail="Real controls. One shared state."
			/>
			<Separator />
			<View class="flex-row gap-3 items-center min-h-8">
				<Text class="flex-1 min-w-0 text-sm">"Haptic feedback"</Text>
				<Switch
					checked=state.is_on("Haptics")
					accessibility_label="Haptic feedback"
					on:toggle=state.set_switch("Haptics")
				/>
			</View>
			<View class="flex-row gap-3 items-center min-h-8">
				<Checkbox
					checked=state.is_on("Preview enabled")
					accessibility_label="Enable previews"
					on:toggle=state.set_switch("Preview enabled")
				/>
				<Text class="flex-1 min-w-0 text-sm">"Enable previews"</Text>
			</View>
			<Text class="text-xs font-semibold text-slate-500">"Density"</Text>
			<View class="flex-row flex-wrap gap-3 items-center">
				<Radio
					radio_group=group.clone()
					checked=state.density() == 0
					accessibility_label="Comfortable"
					on:toggle=state.select_density(0)
				/>
				<Text class="text-sm">"Comfortable"</Text>
				<Radio
					radio_group=group
					checked=state.density() == 1
					accessibility_label="Compact"
					on:toggle=state.select_density(1)
				/>
				<Text class="text-sm">"Compact"</Text>
			</View>
			<View class="gap-2">
				<View class="flex-row justify-between">
					<Text class="text-sm">"Volume"</Text>
					<Text class="text-sm font-semibold">{format!("{:.0}%", state.volume())}</Text>
				</View>
				<Slider
					slider=volume
					class="w-full"
					accessibility_label="Volume"
					on:value_change=state.set_volume()
				/>
			</View>
			<View class="gap-2">
				<Text class="text-xs font-semibold text-slate-500">"Password"</Text>
				<View class="p-2 w-full h-10 rounded-lg border border-slate-300">
					<PasswordInput
						class="w-full h-full border-0 text-slate-900"
						value=state.password()
						placeholder="Try a masked value"
						accessibility_label="Demo password"
						on:change=state.set_password()
					/>
				</View>
			</View>
			<View class="flex-1 min-h-0" />
			<View class="flex-row gap-3 items-center">
				<ActivityIndicator running accessibility_label="Syncing component preview" />
				<Text class="text-xs text-slate-500">"Syncing in the background"</Text>
			</View>
			<LinearProgressIndicator
				progress=Progress::Determinate(completion)
				class="w-full"
				accessibility_label="Preview completion"
			/>
		</View>
	}
}

/// A filled sparkle that keeps its shape wherever it is placed.
#[component]
fn Sparkle() -> Element {
	let ink = Color::rgba(0x1e, 0x40, 0xaf, 0xff);
	let sparkle = VectorPath::new(
		PathData::parse(
			"M12 0 C13 8 16 11 24 12 C16 13 13 16 12 24 C11 16 8 13 0 12 C8 11 11 8 12 0 Z",
		)
		.expect("literal path data is valid"),
	)
	.fill(ink);

	view! {
		<Vector
			view_box=ViewBox::new(0.0, 0.0, 24.0, 24.0).expect("a literal view box is valid")
			path=sparkle
			class="absolute top-3 left-3 size-6"
		/>
	}
}

/// A dashed stamp with a tick, imported once from an SVG document. Its
/// strokes use `currentColor`, so the `Vector`'s text color paints them.
static STAMP: std::sync::LazyLock<Drawing> = std::sync::LazyLock::new(|| {
	Drawing::parse_svg(
		r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" fill="none"
			stroke="currentColor" stroke-linecap="round" stroke-linejoin="round">
			<circle cx="24" cy="24" r="20" stroke-width="2.5" stroke-dasharray="4 5"/>
			<path d="M15 25 l6 6 l12 -14" stroke-width="3.5"/>
		</svg>"#,
	)
	.expect("the stamp is a supported SVG document")
});

#[component]
fn Stamp() -> Element {
	view! {
		<Vector drawing=STAMP.clone() class="absolute bottom-3 left-3 size-12 text-[#1e40af]" />
	}
}

/// A hand-drawn underline stretched to whatever frame it is given, with a
/// stroke that stays three units wide.
#[component]
fn Underline() -> Element {
	let ink = Color::rgba(0x1e, 0x40, 0xaf, 0xff);
	let underline = VectorPath::new(
		PathData::parse("M2 6 C20 2 38 9 56 5 S86 2 98 6").expect("literal path data is valid"),
	)
	.stroke(Stroke::new(ink, 3.0).expect("a positive width").round());

	view! {
		<View class="mt-1 w-[180px] h-[12px]">
			<Vector
				view_box=ViewBox::new(0.0, 0.0, 100.0, 10.0).expect("a literal view box is valid")
				scaling=VectorScaling::Stretch
				path=underline
				class="absolute inset-0"
			/>
		</View>
	}
}

#[component]
fn Composition(state: &ScreenState) -> Element {
	let tags = ["Native", "Web", "Tailwind"].map(|label| {
		view! {
			<View class="items-center self-start py-1.5 px-3 font-medium rounded-2xl min-h-8 bg-slate-100 text-slate-900 text-[13px]">
				<Text>{label}</Text>
			</View>
		}
	});

	view! {
		<View class="gap-5 p-4 bg-white rounded-xl">
			<Title
				number="02 / COMPOSE"
				heading="Small pieces, more room"
				detail="Native primitives plus Tailwind."
			/>
			<View class="overflow-hidden relative justify-center items-center bg-indigo-100 rounded-xl h-[148px]">
				<Sparkle />
				<Stamp />
				<Text class="font-bold text-indigo-900 text-[24px]">"Built together"</Text>
				<Underline />
				<Pill class="absolute top-3 right-3 text-white bg-[#1e40af]">"LIVE"</Pill>
			</View>
			<View class="flex-row flex-wrap gap-2 content-start">{tags}</View>
			<Separator />
			<View class="flex-row gap-3 h-16">
				<View class="flex-1 justify-center items-center min-w-0 min-h-0 rounded-lg bg-slate-100">
					<Text class="text-xs text-slate-500">"1 share"</Text>
				</View>
				<View class="justify-center items-center min-w-0 min-h-0 bg-indigo-100 rounded-lg grow-2 shrink basis-0">
					<Text class="text-xs text-indigo-900">"2 shares"</Text>
				</View>
			</View>
			<View class="flex-1 min-h-0" />
			<Pressable
				accessibility_label="Add a preview"
				class="flex-row gap-3 p-4 text-white bg-indigo-600 rounded-xl"
				on:press=state.adjust_count(1)
			>
				<Text class="font-semibold text-[26px]">"+"</Text>
				<View class="gap-1">
					<Text class="text-sm font-semibold">"Add a preview"</Text>
					<Text class="text-xs">{state.count()}" previews created"</Text>
				</View>
			</Pressable>
		</View>
	}
}

#[component]
fn Collection() -> Element {
	let previews = (1..=12)
		.map(|index| {
			view! {
				<View class="flex-row gap-3 items-center p-2 rounded-xl min-h-12 bg-slate-50">
					<View class="justify-center items-center bg-rose-100 rounded-lg size-10">
						<Text class="text-xs font-semibold text-rose-700">
							{format!("{index:02}")}
						</Text>
					</View>
					<View class="flex-1 gap-1 min-w-0 min-h-0">
						<Text class="text-sm font-semibold text-slate-900">
							"Preview "{format!("{index:02}")}
						</Text>
						<Text class="text-xs text-slate-500">"Ready to explore"</Text>
					</View>
					<Pill class="bg-slate-100 text-slate-500">"NEW"</Pill>
				</View>
			}
		})
		.collect::<Fragment>();

	view! {
		<View class="gap-5 p-4 bg-white rounded-xl">
			<Title
				number="03 / COLLECT"
				heading="A place for every idea"
				detail="An eager list with room to scroll."
			/>
			<ScrollView class="gap-3 min-w-0 h-80 min-h-0 lg:h-auto lg:grow lg:shrink">
				{previews}
			</ScrollView>
		</View>
	}
}

/// A responsive studio rendered from native primitives on every adapter.
#[component]
pub fn Screen(state: &ScreenState) -> Element {
	view! {
		<View class="overflow-scroll w-full h-full lg:overflow-hidden bg-slate-100 text-slate-900">
			<View class="flex-row flex-wrap gap-3 items-center px-6 pb-3 bg-white pt-safe-offset-3 min-h-20">
				<View class="flex-row flex-wrap flex-1 gap-3 items-center min-w-0">
					<Text class="font-bold text-[22px] text-slate-900">"leptoskit"</Text>
					<Pill class="text-indigo-700 bg-indigo-100">"COMPONENT STUDIO"</Pill>
				</View>
				<Text class="text-sm text-slate-500">"Shared by design"</Text>
			</View>
			<View class="grid grid-cols-1 gap-5 p-6 min-w-0 min-h-0 lg:flex-1 lg:grid-cols-3 lg:grid-rows-1">
				<Controls state=state />
				<Composition state=state />
				<Collection />
			</View>
			<View class="flex-row flex-wrap gap-3 justify-between items-center px-6 pt-3 pb-safe-offset-3">
				<Text class="text-xs text-slate-500">"Primitives first. Helpers optional."</Text>
				<Text class="text-xs font-semibold text-indigo-600">"Native + Web"</Text>
			</View>
		</View>
	}
}
