use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use leptos::prelude::*;

use crate::Props;
use crate::props::PropError;

/// A component a document can use: it reads its [`Props`] and renders its
/// children, the Markdown between its opening and closing tags.
pub type Component = dyn Fn(&Props<'_>, Children) -> Result<AnyView, PropError> + Send + Sync;

/// The components documents can use, by tag name.
///
/// [`Components::new`] starts with the built-in set:
///
/// - `<Callout kind="note|tip|important|warning|caution" title="…">`: an aside,
///   the same one a GitHub alert (`> [!TIP]`) renders.
/// - `<Steps>`: numbers the `###` headings inside it as a sequence of steps.
/// - `<Cards>` holding `<Card title="…" href="…">`: a grid of link cards.
/// - `<Tabs>` holding `<Tab label="…">`: alternatives such as one command per
///   platform. Tabs with the same label switch together across the page.
///
/// Register your own leptos components with [`Components::with`]:
///
/// ```
/// use leptos::prelude::*;
/// use leptoskit_docs::Components;
///
/// #[component]
/// fn Kbd(children: Children) -> impl IntoView {
/// 	view! { <kbd class="kbd">{children()}</kbd> }
/// }
///
/// let components = Components::new().with("Kbd", |_props, children| {
/// 	Ok(view! { <Kbd>{children()}</Kbd> }.into_any())
/// });
///
/// let html =
/// 	leptoskit_docs::render_to_html("Press <Kbd>⌘K</Kbd> to search.", &components).unwrap();
/// assert_eq!(html, "<p>Press <kbd class=\"kbd\">⌘K</kbd> to search.</p>");
/// ```
#[derive(Clone)]
pub struct Components {
	entries: BTreeMap<String, Arc<Component>>,
}

impl Components {
	/// The built-in components.
	pub fn new() -> Self {
		Self::empty()
			.with("Callout", |props, children| {
				let kind = props.parse("kind")?.unwrap_or(CalloutKind::Note);
				let title = props.get("title").map(str::to_owned);
				Ok(callout(kind, title, children()))
			})
			.with("Steps", |_props, children| {
				Ok(view! { <div class="lk-steps">{children()}</div> }.into_any())
			})
			.with("Cards", |_props, children| {
				Ok(view! { <div class="lk-cards">{children()}</div> }.into_any())
			})
			.with("Card", |props, children| {
				let title = props.require("title")?.to_owned();
				let body = view! {
					<span class="lk-card-title">{title}</span>
					<span class="lk-card-body">{children()}</span>
				};

				Ok(match props.url("href") {
					Some(href) => view! { <a class="lk-card" href=href>{body}</a> }.into_any(),
					None => view! { <div class="lk-card">{body}</div> }.into_any(),
				})
			})
			.with("Tabs", |_props, children| {
				Ok(view! { <div class="lk-tabs" data-lk-tabs="">{children()}</div> }.into_any())
			})
			.with("Tab", |props, children| {
				let label = props.require("label")?.to_owned();
				let data_label = label.clone();
				Ok(view! {
					<section class="lk-tab" data-label=data_label>
						<p class="lk-tab-label">{label}</p>
						{children()}
					</section>
				}
				.into_any())
			})
	}

	/// No components at all, not even the built-in ones.
	pub fn empty() -> Self {
		Self {
			entries: BTreeMap::new(),
		}
	}

	/// Adds `component` under `name`, replacing any component already
	/// registered with that name.
	///
	/// # Panics
	///
	/// When `name` does not start with an ASCII capital letter. Lowercase tags
	/// are HTML elements, so a lowercase component could never be used.
	#[must_use]
	pub fn with<F>(mut self, name: impl Into<String>, component: F) -> Self
	where
		F: Fn(&Props<'_>, Children) -> Result<AnyView, PropError> + Send + Sync + 'static,
	{
		let name = name.into();
		assert!(
			name.starts_with(|first: char| first.is_ascii_uppercase()),
			"component names start with a capital letter, as in MDX; `{name}` would be an HTML \
			 element"
		);
		self.entries.insert(name, Arc::new(component));
		self
	}

	/// The registered names, in alphabetical order.
	pub fn names(&self) -> impl Iterator<Item = &str> {
		self.entries.keys().map(String::as_str)
	}

	pub(crate) fn get(&self, name: &str) -> Option<&Component> {
		self.entries.get(name).map(Arc::as_ref)
	}
}

impl Default for Components {
	fn default() -> Self {
		Self::new()
	}
}

impl fmt::Debug for Components {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_set().entries(self.names()).finish()
	}
}

/// The flavours of [`Components`]' `<Callout>`, named after GitHub's alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CalloutKind {
	Note,
	Tip,
	Important,
	Warning,
	Caution,
}

impl CalloutKind {
	pub(crate) fn as_str(self) -> &'static str {
		match self {
			Self::Note => "note",
			Self::Tip => "tip",
			Self::Important => "important",
			Self::Warning => "warning",
			Self::Caution => "caution",
		}
	}

	fn title(self) -> &'static str {
		match self {
			Self::Note => "Note",
			Self::Tip => "Tip",
			Self::Important => "Important",
			Self::Warning => "Warning",
			Self::Caution => "Caution",
		}
	}
}

impl FromStr for CalloutKind {
	type Err = String;

	fn from_str(kind: &str) -> Result<Self, Self::Err> {
		match kind.to_ascii_lowercase().as_str() {
			"note" => Ok(Self::Note),
			"tip" => Ok(Self::Tip),
			"important" => Ok(Self::Important),
			"warning" => Ok(Self::Warning),
			"caution" => Ok(Self::Caution),
			_ => Err("expected one of note, tip, important, warning, or caution".to_owned()),
		}
	}
}

pub(crate) fn callout(kind: CalloutKind, title: Option<String>, body: AnyView) -> AnyView {
	let title = title.unwrap_or_else(|| kind.title().to_owned());
	view! {
		<aside class="lk-callout" data-kind=kind.as_str()>
			<p class="lk-callout-title">{title}</p>
			<div class="lk-callout-body">{body}</div>
		</aside>
	}
	.into_any()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn starts_with_the_built_in_components() {
		let components = Components::new();
		let names: Vec<_> = components.names().collect();

		assert_eq!(names, ["Callout", "Card", "Cards", "Steps", "Tab", "Tabs"]);
		assert_eq!(Components::empty().names().count(), 0);
		assert_eq!(
			format!("{:?}", Components::default()),
			format!("{names:?}").replace('[', "{").replace(']', "}")
		);
	}

	#[test]
	#[should_panic(expected = "`button` would be an HTML element")]
	fn refuses_lowercase_names() {
		let _ = Components::empty().with("button", |_, children| Ok(children()));
	}

	#[test]
	fn parses_callout_kinds_case_insensitively() {
		assert_eq!("TIP".parse(), Ok(CalloutKind::Tip));
		assert_eq!("Caution".parse(), Ok(CalloutKind::Caution));
		assert!("danger".parse::<CalloutKind>().is_err());
	}
}
