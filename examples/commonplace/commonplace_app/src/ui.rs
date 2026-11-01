//! The pieces every screen is built from, in Commonplace's colors: paper,
//! ink, and an amber accent, with a dark version of each.

use leptoskit::prelude::*;

/// The page's background and text.
pub const PAGE: &str = "flex-1 bg-stone-50 text-stone-900 dark:bg-stone-950 dark:text-stone-100";
/// Quieter text: dates, counts, hints.
pub const QUIET: &str = "text-sm text-stone-500 dark:text-stone-400";

/// A screen's heading.
#[component]
pub fn Title(text: impl Into<String>) -> Element {
	view! { <Heading aria_level=1 class="text-2xl font-semibold">{text.into()}</Heading> }
}

/// A section's heading.
#[component]
pub fn Section(text: impl Into<String>) -> Element {
	view! { <Heading aria_level=2 class="text-lg font-semibold mt-2">{text.into()}</Heading> }
}

/// A raised box that groups related things.
#[component]
pub fn PanelCard() -> Element {
	view! {
		<View class="gap-2 rounded-xl border border-stone-200 bg-white p-4 dark:border-stone-800 \
		 dark:bg-stone-900" />
	}
}

/// A labeled text field. `on_change` gets each new value.
#[component]
pub fn Field(
	label: &str,
	value: &str,
	placeholder: &str,
	on_change: impl Fn(&str) + Send + Sync + 'static,
) -> Element {
	view! {
		<View class="gap-1">
			<Text class="text-sm font-medium">{label}</Text>
			<TextInput
				accessibility_label=label
				value=value
				placeholder=placeholder
				class="rounded-lg border border-stone-300 bg-white px-3 py-2 dark:border-stone-700 \
					 dark:bg-stone-900"
				on:change=on_change
			/>
		</View>
	}
}

/// The main action on a screen.
#[component]
pub fn Primary(label: &str, enabled: bool, on_press: impl Fn() + Send + Sync + 'static) -> Element {
	view! {
		<Button
			enabled=enabled
			class="rounded-lg bg-amber-700 px-4 py-2 font-semibold text-white"
			class=if enabled { "" } else { "opacity-50" }
			on:press=on_press
		>
			{label}
		</Button>
	}
}

/// Any other action.
#[component]
pub fn Secondary(label: &str, on_press: impl Fn() + Send + Sync + 'static) -> Element {
	view! {
		<Button
			class="rounded-lg border border-stone-300 px-3 py-1 text-sm dark:border-stone-700"
			on:press=on_press
		>
			{label}
		</Button>
	}
}

/// A failure, read out by screen readers as soon as it appears.
#[component]
pub fn Problem(message: &str) -> Element {
	view! {
		<Alert class="rounded-lg bg-red-100 px-3 py-2 text-red-900 dark:bg-red-950 dark:text-red-100">
			{message}
		</Alert>
	}
}

/// Shown while something loads.
#[component]
pub fn Loading(what: &str) -> Element {
	view! { <Status class=QUIET>{format!("Loading {what}…")}</Status> }
}

/// Shown where a screen belongs to the browser extension.
#[component]
pub fn ExtensionOnly() -> Element {
	view! {
		<View class="gap-2">
			{view! { <Title text="Part of the browser extension" /> }}
			<Text>
				"This screen opens in the Commonplace browser extension: in its popup, its side \
			 panel, or on the pages you read."
			</Text>
		</View>
	}
}

/// The projects to clip into, with the chosen one marked.
#[component]
pub fn ProjectPicker(
	projects: &[commonplace_shared::ProjectSummary],
	chosen: Option<commonplace_shared::ProjectId>,
	on_pick: impl Fn(commonplace_shared::ProjectId) + Clone + Send + Sync + 'static,
) -> Element {
	if projects.is_empty() {
		return view! { <Text class=QUIET>"Start a project in Commonplace first."</Text> };
	}
	view! {
		<View class="flex-row flex-wrap gap-2">
			{projects
				.iter()
				.map(|summary| {
					let id = summary.project.id;
					let on_pick = on_pick.clone();
					let selected = chosen == Some(id);
					let label = if selected {
						format!("{} (chosen)", summary.project.title)
					} else {
						summary.project.title.clone()
					};
					view! {
						<Button
							class=if selected {
								"rounded-full bg-amber-700 px-3 py-1 text-sm text-white"
							} else {
								"rounded-full border border-stone-300 px-3 py-1 text-sm \
								 dark:border-stone-700"
							}
							on:press=move || on_pick(id)
						>
							{label}
						</Button>
					}
				})
				.collect::<Fragment>()}
		</View>
	}
}
