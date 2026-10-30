//! The pieces every screen is built from, in Commonplace's colors: paper,
//! ink, and an amber accent, with a dark version of each.

use leptoskit::prelude::*;

/// The page's background and text.
pub const PAGE: &str = "flex-1 bg-stone-50 text-stone-900 dark:bg-stone-950 dark:text-stone-100";
/// Quieter text: dates, counts, hints.
pub const QUIET: &str = "text-sm text-stone-500 dark:text-stone-400";

/// A screen's heading.
pub fn title(text: impl Into<String>) -> Element {
	Heading(1, text).class("text-2xl font-semibold")
}

/// A section's heading.
pub fn section(text: impl Into<String>) -> Element {
	Heading(2, text).class("text-lg font-semibold mt-2")
}

/// A raised box that groups related things.
pub fn card() -> Element {
	View().class(
		"gap-2 rounded-xl border border-stone-200 bg-white p-4 dark:border-stone-800 \
		 dark:bg-stone-900",
	)
}

/// A labeled text field. `on_change` gets each new value.
pub fn field(
	label: &str,
	value: &str,
	placeholder: &str,
	on_change: impl Fn(&str) + Send + Sync + 'static,
) -> Element {
	View()
		.class("gap-1")
		.child(Text(label).class("text-sm font-medium"))
		.child(
			TextInput()
				.accessibility_label(label)
				.value(value)
				.placeholder(placeholder)
				.class(
					"rounded-lg border border-stone-300 bg-white px-3 py-2 dark:border-stone-700 \
					 dark:bg-stone-900",
				)
				.on_change(on_change),
		)
}

/// The main action on a screen.
pub fn primary(label: &str, enabled: bool, on_press: impl Fn() + Send + Sync + 'static) -> Element {
	Button(label)
		.enabled(enabled)
		.class("rounded-lg bg-amber-700 px-4 py-2 font-semibold text-white")
		.class(if enabled { "" } else { "opacity-50" })
		.on_press(on_press)
}

/// Any other action.
pub fn secondary(label: &str, on_press: impl Fn() + Send + Sync + 'static) -> Element {
	Button(label)
		.class("rounded-lg border border-stone-300 px-3 py-1 text-sm dark:border-stone-700")
		.on_press(on_press)
}

/// A failure, read out by screen readers as soon as it appears.
pub fn problem(message: &str) -> Element {
	Alert(message)
		.class("rounded-lg bg-red-100 px-3 py-2 text-red-900 dark:bg-red-950 dark:text-red-100")
}

/// Shown while something loads.
pub fn loading(what: &str) -> Element {
	Status(format!("Loading {what}…")).class(QUIET)
}

/// Shown where a screen belongs to the browser extension.
pub fn extension_only() -> Element {
	View()
		.class("gap-2")
		.child(title("Part of the browser extension"))
		.child(Text(
			"This screen opens in the Commonplace browser extension: in its popup, its side \
			 panel, or on the pages you read.",
		))
}

/// The projects to clip into, with the chosen one marked.
pub fn project_picker(
	projects: &[commonplace_shared::ProjectSummary],
	chosen: Option<commonplace_shared::ProjectId>,
	on_pick: impl Fn(commonplace_shared::ProjectId) + Clone + Send + Sync + 'static,
) -> Element {
	if projects.is_empty() {
		return Text("Start a project in Commonplace first.").class(QUIET);
	}
	View()
		.class("flex-row flex-wrap gap-2")
		.children(projects.iter().map(|summary| {
			let id = summary.project.id;
			let on_pick = on_pick.clone();
			let selected = chosen == Some(id);
			let label = if selected {
				format!("{} (chosen)", summary.project.title)
			} else {
				summary.project.title.clone()
			};
			Button(label)
				.class(if selected {
					"rounded-full bg-amber-700 px-3 py-1 text-sm text-white"
				} else {
					"rounded-full border border-stone-300 px-3 py-1 text-sm dark:border-stone-700"
				})
				.on_press(move || on_pick(id))
		}))
}
