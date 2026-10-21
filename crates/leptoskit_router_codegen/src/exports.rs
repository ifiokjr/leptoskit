//! What a route file exports, read from its top-level items. The generator
//! never runs route code; it only reads names.

use std::collections::BTreeMap;

use syn::Item;
use syn::Type;
use syn::Visibility;
use syn::spanned::Spanned;

/// The things a route file can render. Each may be written as a lowercase
/// function (`page`) or as a capitalised component (`#[component] Page`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Component {
	/// The screen: `page` or `Page`.
	Page,
	/// Chrome around a directory's routes: `layout` or `Layout`.
	Layout,
	/// Shown while data loads: `loading` or `Loading`.
	Loading,
	/// The error boundary: `error` or `Error`.
	Error,
	/// Web title and meta tags: `head` or `Head`.
	Head,
	/// The SSR document shell in `+html.rs`: `html` or `Html`.
	Html,
}

impl Component {
	const ALL: [Self; 6] = [
		Self::Page,
		Self::Layout,
		Self::Loading,
		Self::Error,
		Self::Head,
		Self::Html,
	];

	/// The lowercase function name.
	pub fn function_name(self) -> &'static str {
		match self {
			Self::Page => "page",
			Self::Layout => "layout",
			Self::Loading => "loading",
			Self::Error => "error",
			Self::Head => "head",
			Self::Html => "html",
		}
	}

	/// The capitalised component name.
	pub fn component_name(self) -> &'static str {
		match self {
			Self::Page => "Page",
			Self::Layout => "Layout",
			Self::Loading => "Loading",
			Self::Error => "Error",
			Self::Head => "Head",
			Self::Html => "Html",
		}
	}
}

/// How a rendering export is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportStyle {
	/// A lowercase function, such as `pub fn page()`.
	Function,
	/// A capitalised function annotated with `#[component]`, such as `pub fn
	/// Page()`.
	Component,
}

/// A data or configuration function a route file can export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Hook {
	Load,
	Navigator,
	Redirect,
	Prerender,
}

impl Hook {
	const ALL: [Self; 4] = [Self::Load, Self::Navigator, Self::Redirect, Self::Prerender];

	pub(crate) fn name(self) -> &'static str {
		match self {
			Self::Load => "load",
			Self::Navigator => "navigator",
			Self::Redirect => "redirect",
			Self::Prerender => "prerender",
		}
	}
}

/// How a route struct field is read from the URL, by its written type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldKind {
	One,
	OneOrDefault,
	Optional,
	Many,
}

#[derive(Clone, Debug)]
pub(crate) struct RouteField {
	pub(crate) name: String,
	pub(crate) kind: FieldKind,
}

/// The `#[derive(Route)]` struct in a route file.
#[derive(Clone, Debug)]
pub(crate) struct RouteStruct {
	pub(crate) name: String,
	pub(crate) unit: bool,
	pub(crate) fields: Vec<RouteField>,
	pub(crate) line: usize,
}

/// What one route file exports.
#[derive(Clone, Debug, Default)]
pub(crate) struct Exports {
	pub(crate) components: BTreeMap<Component, (ExportStyle, usize)>,
	pub(crate) hooks: BTreeMap<Hook, usize>,
	pub(crate) route: Option<RouteStruct>,
}

/// A problem in a route file, at a line when one applies.
pub(crate) type Problem = (Option<usize>, String);

/// Reads the exports of a route file's source.
pub(crate) fn read(source: &str) -> Result<Exports, Vec<Problem>> {
	let file = syn::parse_file(source).map_err(|error| {
		vec![(
			Some(error.span().start().line),
			format!("could not parse: {error}"),
		)]
	})?;
	let mut exports = Exports::default();
	let mut problems = Vec::new();
	for item in &file.items {
		match item {
			Item::Fn(function) => {
				read_function(
					&function.sig.ident.to_string(),
					&function.vis,
					&function.attrs,
					item.span().start().line,
					&mut exports,
					&mut problems,
				);
			}
			Item::Struct(item_struct) if derives_route(&item_struct.attrs) => {
				let line = item_struct.ident.span().start().line;
				if !matches!(item_struct.vis, Visibility::Public(_)) {
					problems.push((
						Some(line),
						format!(
							"`{}` derives `Route`, so it must be `pub`: the generated `Route` \
							 enum holds it",
							item_struct.ident
						),
					));
				}
				if let Some(existing) = &exports.route {
					problems.push((
						Some(line),
						format!(
							"a route file declares one `#[derive(Route)]` struct, but this one \
							 also declares `{}`",
							existing.name
						),
					));
					continue;
				}
				let fields = item_struct
					.fields
					.iter()
					.filter_map(|field| {
						Some(RouteField {
							name: field
								.ident
								.as_ref()?
								.to_string()
								.trim_start_matches("r#")
								.to_owned(),
							kind: field_kind(field),
						})
					})
					.collect();
				exports.route = Some(RouteStruct {
					name: item_struct.ident.to_string(),
					unit: matches!(item_struct.fields, syn::Fields::Unit),
					fields,
					line,
				});
			}
			_ => {}
		}
	}
	if problems.is_empty() {
		Ok(exports)
	} else {
		Err(problems)
	}
}

fn read_function(
	name: &str,
	visibility: &Visibility,
	attributes: &[syn::Attribute],
	line: usize,
	exports: &mut Exports,
	problems: &mut Vec<Problem>,
) {
	let public = matches!(visibility, Visibility::Public(_));
	for hook in Hook::ALL {
		if name == hook.name() {
			if public {
				exports.hooks.insert(hook, line);
			} else {
				problems.push((
					Some(line),
					format!("`{name}` must be `pub` for the router to use it"),
				));
			}
			return;
		}
	}
	for component in Component::ALL {
		let style = if name == component.function_name() {
			ExportStyle::Function
		} else if name == component.component_name() {
			if !is_annotated_component(attributes) {
				problems.push((
					Some(line),
					format!(
						"`{name}` is capitalised, so annotate it with `#[component]`, or rename \
						 it to `{}`",
						component.function_name()
					),
				));
				return;
			}
			ExportStyle::Component
		} else {
			continue;
		};
		if !public {
			problems.push((
				Some(line),
				format!("`{name}` must be `pub` for the router to use it"),
			));
			return;
		}
		if exports
			.components
			.insert(component, (style, line))
			.is_some()
		{
			problems.push((
				Some(line),
				format!(
					"export either `{}` or `{}`, not both",
					component.function_name(),
					component.component_name()
				),
			));
		}
		return;
	}
}

/// Any attribute whose path ends in `component`: `#[component]`,
/// `#[leptoskit::component]`.
fn is_annotated_component(attributes: &[syn::Attribute]) -> bool {
	attributes.iter().any(|attribute| {
		attribute
			.path()
			.segments
			.last()
			.is_some_and(|segment| segment.ident == "component")
	})
}

/// `#[derive(..., Route, ...)]`, with any path ending in `Route`.
fn derives_route(attributes: &[syn::Attribute]) -> bool {
	attributes.iter().any(|attribute| {
		if !attribute.path().is_ident("derive") {
			return false;
		}
		let mut found = false;
		let _ = attribute.parse_nested_meta(|meta| {
			found |= meta
				.path
				.segments
				.last()
				.is_some_and(|segment| segment.ident == "Route");
			Ok(())
		});
		found
	})
}

/// Mirrors how `#[derive(Route)]` reads a field: by its written type and
/// `#[route(default)]`.
fn field_kind(field: &syn::Field) -> FieldKind {
	let default = field.attrs.iter().any(|attribute| {
		attribute.path().is_ident("route")
			&& attribute
				.parse_args::<syn::Ident>()
				.is_ok_and(|ident| ident == "default")
	});
	let wrapper = match &field.ty {
		Type::Path(path) => {
			path.path
				.segments
				.last()
				.map(|segment| segment.ident.to_string())
		}
		_ => None,
	};
	match (wrapper.as_deref(), default) {
		(Some("Option"), _) => FieldKind::Optional,
		(Some("Vec"), _) => FieldKind::Many,
		(_, true) => FieldKind::OneOrDefault,
		(_, false) => FieldKind::One,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn accepts_a_lowercase_page_or_an_annotated_capitalised_one() {
		let lowercase = read("pub fn page() {}").unwrap();
		assert_eq!(
			lowercase
				.components
				.get(&Component::Page)
				.map(|(style, _)| *style),
			Some(ExportStyle::Function)
		);
		let component = read("#[component]\npub fn Page() {}").unwrap();
		assert_eq!(
			component
				.components
				.get(&Component::Page)
				.map(|(style, _)| *style),
			Some(ExportStyle::Component)
		);
		let qualified = read("#[leptoskit::component]\npub fn Loading() {}").unwrap();
		assert!(qualified.components.contains_key(&Component::Loading));
	}

	#[test]
	fn rejects_an_unannotated_component_both_spellings_and_private_exports() {
		let problems = read("pub fn Page() {}").unwrap_err();
		assert!(
			problems[0].1.contains("annotate it with `#[component]`"),
			"{problems:?}"
		);
		let problems = read("pub fn page() {}\n#[component]\npub fn Page() {}").unwrap_err();
		assert_eq!(problems[0].0, Some(2));
		assert!(problems[0].1.contains("not both"), "{problems:?}");
		let problems = read("fn load() {}").unwrap_err();
		assert!(problems[0].1.contains("must be `pub`"), "{problems:?}");
	}

	#[test]
	fn reads_the_route_struct_and_its_field_kinds() {
		let exports = read(
			"#[derive(Clone, leptoskit_router::Route)]\npub struct Note {\n pub note: u64,\n pub \
			 highlight: Option<u32>,\n pub tag: Vec<String>,\n #[route(default)]\n pub page: \
			 u32,\n pub r#type: String,\n}",
		)
		.unwrap();
		let route = exports.route.unwrap();
		assert_eq!(route.name, "Note");
		assert_eq!(route.line, 2);
		let kinds: Vec<_> = route
			.fields
			.iter()
			.map(|field| (field.name.as_str(), field.kind))
			.collect();
		assert_eq!(
			kinds,
			[
				("note", FieldKind::One),
				("highlight", FieldKind::Optional),
				("tag", FieldKind::Many),
				("page", FieldKind::OneOrDefault),
				("type", FieldKind::One),
			]
		);
	}

	#[test]
	fn reports_parse_errors_with_a_line() {
		let problems = read("pub fn page( {").unwrap_err();
		assert!(problems[0].0.is_some());
	}
}
