//! The screen: the postcard, the controls that change the trip, a stamp per
//! language, and a gauge of how each language counts.

use leptoskit::native::Role;
use leptoskit::native::SliderState;
use leptoskit::prelude::*;
use leptoskit::yoga::Direction;
use leptoskit_l10n::Locale;
use leptoskit_l10n::PluralCategory;
use leptoskit_l10n::PluralRules;
use reactive_graph::traits::Get;
use reactive_graph::traits::Set;

use crate::l10n::use_l10n;
use crate::trip::PLACE;
use crate::trip::Trip;

/// The languages written right to left among those the app has.
const RIGHT_TO_LEFT: [&str; 1] = ["ar"];

/// Whether `locale` is written right to left.
pub fn is_right_to_left(locale: &Locale) -> bool {
	RIGHT_TO_LEFT.contains(&locale.language())
}

/// Lays `element` and everything inside it out right to left: rows run from
/// the right, and text starts at the right edge.
///
/// Direction does not pass from a parent to its children in leptoskit yet,
/// so a right-to-left language sets it on every element of the subtree.
pub fn right_to_left(mut element: Element) -> Element {
	element.style.direction = Direction::Rtl;
	element.children = element.children.into_iter().map(right_to_left).collect();
	element
}

/// The whole screen.
#[component]
pub fn Screen(trip: Trip) -> impl IntoView {
	let l10n = use_l10n();
	let screen = view! {
		<View class="flex-row gap-8 p-8 w-full h-full bg-amber-50">
			<View class="gap-5 w-[460px]">
				<Heading class="text-[34px] font-bold text-slate-900" aria_level=1>
					{move || l10n.app_title()}
				</Heading>
				<Card trip=trip />
				<Controls trip=trip />
			</View>
			<View class="gap-5 flex-1">
				<Stamps />
				<PluralGauge trip=trip />
			</View>
		</View>
	};

	// The whole screen mirrors for a right-to-left language.
	if is_right_to_left(&l10n.locale()) {
		right_to_left(screen)
	} else {
		screen
	}
}

/// The postcard itself. Tapping the weather or the signature changes them,
/// so every case of their selects shows.
#[component]
pub fn Card(trip: Trip) -> impl IntoView {
	let l10n = use_l10n();
	let weather = l10n.weather(trip.sky.get().key());
	let signature = l10n.sign_off(trip.traveler.get().key());

	view! {
		<View class="gap-3 rounded-[18px] bg-white p-6">
			<Text class="text-[22px] font-semibold text-slate-900">
				{move || l10n.greeting(PLACE)}
			</Text>
			<Text class="text-slate-700">{move || l10n.days_away(trip.days.get())}</Text>
			<Text class="text-slate-700">{move || l10n.photos_taken(trip.photos.get())}</Text>
			<Pressable accessibility_label=weather.clone() on:press=move || trip.change_sky()>
				<Text class="text-slate-700">{weather}</Text>
			</Pressable>
			<Text class="text-slate-500">
				{move || l10n.letters_waiting("you", trip.letters.get())}
			</Text>
			<Pressable
				accessibility_label=signature.clone()
				on:press=move || trip.change_traveler()
			>
				<Text class="text-indigo-700 font-semibold">{signature}</Text>
			</Pressable>
		</View>
	}
}

/// Sliders for the numbers the plurals count.
#[component]
pub fn Controls(trip: Trip) -> impl IntoView {
	let l10n = use_l10n();
	view! {
		<View class="gap-3">
			<Counter
				label=l10n.days_label()
				value=trip.days.get()
				max=60.0
				on_change=move |days| {
					trip.days.set(days);
				}
			/>
			<Counter
				label=l10n.photos_label()
				value=trip.photos.get()
				max=120.0
				on_change=move |photos| {
					trip.photos.set(photos);
				}
			/>
		</View>
	}
}

/// One labelled slider over whole numbers from zero to `max`.
#[component]
fn Counter(
	label: &str,
	value: i64,
	max: f64,
	on_change: impl Fn(i64) + Send + Sync + 'static,
) -> Element {
	let state = SliderState::new(0.0, max, value as f64, Some(1.0))
		.expect("the counts stay on their whole-number grid");
	view! {
		<View class="gap-1">
			<Text class="text-sm text-slate-600">{format!("{label}: {value}")}</Text>
			<Slider
				slider=state
				aria_label=label
				class="w-full"
				on:value_change=move |value| on_change(value.round() as i64)
			/>
		</View>
	}
}

/// A stamp per language the app has, labelled with its tag, the one in use
/// pressed. Pressing one switches every message on screen.
#[component]
pub fn Stamps() -> impl IntoView {
	let l10n = use_l10n();
	let current = l10n.locale();
	let stamps = l10n.supported_locales().iter().map(|locale| {
		let chosen = *locale == current;
		let choice = locale.clone();
		view! {
			<Button
				aria_pressed=chosen
				on:press=move || l10n.set_locale(&choice)
				class=if chosen {
					"rounded-lg px-3 py-2 bg-indigo-600 text-white"
				} else {
					"rounded-lg px-3 py-2 bg-white text-slate-800"
				}
			>
				{locale.to_string()}
			</Button>
		}
	});

	view! {
		<View class="gap-2">
			<Text class="text-sm text-slate-600">{move || l10n.language_label()}</Text>
			<View
				class="flex-row flex-wrap gap-2"
				role=Role::Group
				aria_label=l10n.language_label()
			>
				{stamps.collect::<Vec<_>>()}
			</View>
		</View>
	}
}

/// How every language counts the days: the CLDR plural category each picks
/// for the same number, side by side. Drag the days and watch Polish move
/// from one to few to many while Japanese never leaves other.
#[component]
pub fn PluralGauge(trip: Trip) -> impl IntoView {
	let l10n = use_l10n();
	let days = trip.days.get();
	let rows = l10n.supported_locales().iter().map(move |locale| {
		let category = PluralRules::for_locale(locale).category(days);
		// The chip is a view around its text: native labels draw their text
		// from their own edge, ignoring padding.
		view! {
			<View role=Role::Row class="flex-row items-center gap-3">
				<Text role=Role::Cell class="w-[64px] font-semibold">{locale.to_string()}</Text>
				<View role=Role::Cell class=category_class(category)>
					<Text>{category.as_str()}</Text>
				</View>
			</View>
		}
	});

	view! {
		<View class="gap-2 rounded-[18px] bg-white p-5">
			<Text class="font-semibold text-slate-900">
				{move || l10n.plural_title(&trip.days.get().to_string())}
			</Text>
			<View class="gap-2" role=Role::Table aria_label="Plural forms">
				{rows.collect::<Vec<_>>()}
			</View>
		</View>
	}
}

/// A chip colour per plural category, so the gauge reads at a glance.
fn category_class(category: PluralCategory) -> &'static str {
	match category {
		PluralCategory::Zero => "rounded-md px-3 py-1 bg-slate-200 text-slate-800",
		PluralCategory::One => "rounded-md px-3 py-1 bg-emerald-200 text-emerald-900",
		PluralCategory::Two => "rounded-md px-3 py-1 bg-sky-200 text-sky-900",
		PluralCategory::Few => "rounded-md px-3 py-1 bg-amber-200 text-amber-900",
		PluralCategory::Many => "rounded-md px-3 py-1 bg-rose-200 text-rose-900",
		PluralCategory::Other => "rounded-md px-3 py-1 bg-indigo-100 text-indigo-900",
	}
}
