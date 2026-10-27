//! The route table: every route the tree defines, checked and ordered from
//! most to least specific.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use crate::Diagnostic;
use crate::ExportStyle;
use crate::RouteKind;
use crate::exports::Component;
use crate::exports::ComponentExport;
use crate::exports::Exports;
use crate::exports::FieldKind;
use crate::exports::Hook;
use crate::exports::Problem;
use crate::exports::RouteStruct;
use crate::name;
use crate::name::Name;
use crate::name::Segment;
use crate::tree::Directory;
use crate::tree::File;

/// How a route's variant holds its parameters.
#[derive(Clone, Debug)]
pub(crate) enum Target {
	/// A static route with no struct: a unit variant.
	Unit,
	/// A route struct from the file. A unit struct still gives a unit
	/// variant.
	Struct { name: String, unit: bool },
	/// `+not-found.rs`: holds `leptoskit_router::NotFound`.
	NotFound,
}

#[derive(Clone, Debug)]
pub(crate) struct Route {
	pub(crate) variant: String,
	pub(crate) segments: Vec<Segment>,
	/// The shapes of URL the route matches, without repeats.
	pub(crate) shapes: Vec<Shape>,
	pub(crate) file: PathBuf,
	pub(crate) kind: RouteKind,
	pub(crate) target: Target,
	pub(crate) page: ExportStyle,
	/// The route file's page.
	pub(crate) page_call: Call,
	/// The layouts around the page, outermost first.
	pub(crate) layouts: Vec<Call>,
}

/// One shape of URL a route matches: its segments with each optional one
/// either present, written as a required one, or absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shape {
	pub(crate) segments: Vec<Segment>,
	/// The optional parameters this shape leaves out.
	pub(crate) absent: Vec<String>,
}

/// The checked route table.
#[derive(Clone, Debug)]
pub(crate) struct Table {
	/// Every route, in the order of its most specific shape.
	pub(crate) routes: Vec<Route>,
	/// Every route's shapes as `(route, shape)` indices, most specific first:
	/// the order the generated code tries them in.
	pub(crate) matches: Vec<(usize, usize)>,
}

/// A page or layout the generated code calls, with what to pass it.
#[derive(Clone, Debug)]
pub(crate) struct Call {
	/// The module path from the routes module to the file.
	pub(crate) modules: Vec<String>,
	/// The function's name as written: `page` or `Page`.
	pub(crate) function: String,
	pub(crate) props: Vec<Prop>,
}

/// A router prop, in the order the function declares its parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prop {
	/// A page's route struct, or for a layout the whole `Route`.
	Route { by_ref: bool },
	/// The router, to navigate with.
	Navigation { by_ref: bool },
	/// A layout's content: the page, or the layout nested inside it.
	Children,
}

/// The props a page receives.
const PAGE_PROPS: &str = "`route` and `navigation`";
/// The props a layout receives.
const LAYOUT_PROPS: &str = "`children`, `route` and `navigation`";

/// The most optional segments one route may have. Each doubles the shapes of
/// URL the route matches, and so its entries in the generated table.
const MAX_OPTIONAL_SEGMENTS: usize = 8;

impl Route {
	/// The path parameters the route's struct must declare.
	pub(crate) fn params(&self) -> impl Iterator<Item = &str> {
		self.segments.iter().filter_map(Segment::param)
	}
}

/// Builds the ordered table, adding a diagnostic for every problem.
///
/// Precedence is decided per shape, not per route: `[[lang]]/about.rs`
/// matches `/about` as a static segment, before `[section].rs` can read it as
/// a parameter, while `/fr/about` starts with a parameter, so `fr/[page].rs`
/// wins it.
pub(crate) fn build(root: &Directory, dir: &Path, diagnostics: &mut Vec<Diagnostic>) -> Table {
	let mut routes = Vec::new();
	collect(root, &[], &[], &[], true, &mut routes, diagnostics);
	check_names(&routes, dir, diagnostics);
	routes.sort_by_cached_key(|route| {
		route
			.shapes
			.iter()
			.map(|shape| order_key(route, shape))
			.min()
	});
	let mut matches: Vec<(usize, usize)> = routes
		.iter()
		.enumerate()
		.flat_map(|(index, route)| (0..route.shapes.len()).map(move |shape| (index, shape)))
		.collect();
	matches.sort_by_cached_key(|&(route, shape)| {
		order_key(&routes[route], &routes[route].shapes[shape])
	});
	check_duplicates(&routes, &matches, dir, diagnostics);
	Table { routes, matches }
}

/// A route file's path relative to the routes directory, for messages that
/// name a second file.
fn relative(file: &Path, dir: &Path) -> String {
	crate::slash_path(file.strip_prefix(dir).unwrap_or(file))
}

fn collect(
	directory: &Directory,
	prefix: &[Segment],
	modules: &[String],
	layouts: &[Call],
	is_root: bool,
	routes: &mut Vec<Route>,
	diagnostics: &mut Vec<Diagnostic>,
) {
	let mut layouts = layouts.to_vec();
	layouts.extend(directory_layout(directory, modules, diagnostics));
	let layouts = layouts.as_slice();
	for file in &directory.files {
		let mut file_modules = modules.to_vec();
		file_modules.push(file.module.clone());
		match &file.name {
			Name::Index => {
				routes.extend(route_file(
					file,
					prefix.to_vec(),
					file_modules,
					layouts,
					diagnostics,
				));
			}
			Name::Segment(segment) => {
				let mut segments = prefix.to_vec();
				segments.push(segment.clone());
				routes.extend(route_file(
					file,
					segments,
					file_modules,
					layouts,
					diagnostics,
				));
			}
			Name::NotFound => {
				routes.extend(not_found_file(
					file,
					prefix,
					file_modules,
					layouts,
					diagnostics,
				));
			}
			Name::Layout => check_layout(file, diagnostics),
			Name::Html => check_html(file, is_root, diagnostics),
			Name::Group(_) | Name::Private(_) => {}
		}
	}
	for child in &directory.directories {
		let mut child_prefix = prefix.to_vec();
		if let Some(Name::Segment(segment)) = &child.name {
			child_prefix.push(segment.clone());
		}
		let mut child_modules = modules.to_vec();
		child_modules.push(child.module.clone());
		collect(
			child,
			&child_prefix,
			&child_modules,
			layouts,
			false,
			routes,
			diagnostics,
		);
	}
}

/// The directory's layout, when its `_layout.rs` exports one that can wrap
/// a page.
fn directory_layout(
	directory: &Directory,
	modules: &[String],
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<Call> {
	let file = directory
		.files
		.iter()
		.find(|file| file.name == Name::Layout)?;
	let export = file.exports.as_ref()?.components.get(&Component::Layout)?;
	let mut problems = Vec::new();
	let mut props = Vec::new();
	for param in &export.params {
		let prop = match param.name.as_str() {
			"children" if param.by_ref => {
				problems.push((
					Some(export.line),
					format!(
						"`{}` takes `children` by reference, but it owns them: declare `children: \
						 Children`",
						export.name
					),
				));
				continue;
			}
			"children" => Prop::Children,
			"route" => {
				Prop::Route {
					by_ref: param.by_ref,
				}
			}
			"navigation" => {
				Prop::Navigation {
					by_ref: param.by_ref,
				}
			}
			name => {
				problems.push((
					Some(export.line),
					format!(
						"`{}` takes `{name}`, but a layout receives only {LAYOUT_PROPS}",
						export.name
					),
				));
				continue;
			}
		};
		props.push(prop);
	}
	if !props.contains(&Prop::Children) && problems.is_empty() {
		problems.push((
			Some(export.line),
			format!(
				"`{}` wraps every route below it, so it takes `children: Children` and places them",
				export.name
			),
		));
	}
	if !problems.is_empty() {
		report(file, problems, diagnostics);
		return None;
	}
	let mut modules = modules.to_vec();
	modules.push(file.module.clone());
	Some(Call {
		modules,
		function: export.name.clone(),
		props,
	})
}

/// The page's call, checking that it asks only for props a page receives.
fn page_call(
	export: &ComponentExport,
	modules: Vec<String>,
	has_route: bool,
	problems: &mut Vec<Problem>,
) -> Call {
	let mut props = Vec::new();
	for param in &export.params {
		match param.name.as_str() {
			"route" if has_route => {
				props.push(Prop::Route {
					by_ref: param.by_ref,
				});
			}
			"route" => {
				problems.push((
					Some(export.line),
					format!(
						"`{}` takes `route`, but this route has no `#[derive(Route)]` struct to \
						 pass; declare one, or drop the parameter",
						export.name
					),
				));
			}
			"navigation" => {
				props.push(Prop::Navigation {
					by_ref: param.by_ref,
				});
			}
			name => {
				problems.push((
					Some(export.line),
					format!(
						"`{}` takes `{name}`, but a page receives only {PAGE_PROPS}",
						export.name
					),
				));
			}
		}
	}
	Call {
		modules,
		function: export.name.clone(),
		props,
	}
}

fn route_file(
	file: &File,
	segments: Vec<Segment>,
	modules: Vec<String>,
	layouts: &[Call],
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<Route> {
	let exports = file.exports.as_ref()?;
	let mut problems = Vec::new();
	let page = page_export(exports, &mut problems);
	for (component, export) in &exports.components {
		let line = &export.line;
		if matches!(component, Component::Layout | Component::Html) {
			problems.push((
				Some(*line),
				format!(
					"`{}` belongs in {}",
					component.function_name(),
					belongs_in(*component)
				),
			));
		}
	}
	for (hook, line) in &exports.hooks {
		if matches!(hook, Hook::Navigator | Hook::Redirect) {
			problems.push((
				Some(*line),
				format!("`{}` belongs in `_layout.rs`", hook.name()),
			));
		}
	}

	let mut seen = Vec::new();
	for param in segments.iter().filter_map(Segment::param) {
		if seen.contains(&param) {
			problems.push((
				None,
				format!("the parameter `{param}` appears twice in this route's path"),
			));
		}
		seen.push(param);
	}
	let optional = segments
		.iter()
		.filter(|segment| {
			matches!(
				segment,
				Segment::OptionalParam(_) | Segment::OptionalRest(_)
			)
		})
		.count();
	if optional > MAX_OPTIONAL_SEGMENTS {
		problems.push((
			None,
			format!(
				"this route has {optional} optional segments; each one doubles the URL shapes the \
				 router tries for it, so a route has at most {MAX_OPTIONAL_SEGMENTS}"
			),
		));
	}
	let target = match &exports.route {
		Some(route) => {
			check_struct(route, &segments, &mut problems);
			Target::Struct {
				name: route.name.clone(),
				unit: route.unit,
			}
		}
		None if seen.is_empty() => Target::Unit,
		None => {
			let fields = seen
				.iter()
				.map(|param| format!("`{param}`"))
				.collect::<Vec<_>>()
				.join(", ");
			problems.push((
				None,
				format!(
					"this route has path parameters, so it needs a `#[derive(Route)] pub struct` \
					 with the fields {fields}"
				),
			));
			Target::Unit
		}
	};
	let variant = match &target {
		Target::Struct { name, .. } => name.clone(),
		_ => derived_name(&segments, ""),
	};
	let has_route = matches!(target, Target::Struct { .. });
	let call = page.map(|page| page_call(page, modules, has_route, &mut problems));
	// A route with problems stays out of the table, so it causes no
	// knock-on errors about names or competing URLs.
	if !problems.is_empty() {
		report(file, problems, diagnostics);
		return None;
	}
	Some(Route {
		variant,
		shapes: shapes(&segments),
		segments,
		file: file.path.clone(),
		kind: RouteKind::Page,
		target,
		page: page?.style,
		page_call: call?,
		layouts: layouts.to_vec(),
	})
}

fn not_found_file(
	file: &File,
	prefix: &[Segment],
	modules: Vec<String>,
	layouts: &[Call],
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<Route> {
	let exports = file.exports.as_ref()?;
	let mut problems = Vec::new();
	let page = page_export(exports, &mut problems);
	let call = page.map(|page| page_call(page, modules, true, &mut problems));
	if prefix.iter().any(|segment| segment.param().is_some()) {
		problems.push((
			None,
			"`+not-found.rs` cannot sit under a dynamic segment yet; move it to a directory whose \
			 path is static"
				.to_owned(),
		));
	}
	if let Some(route) = &exports.route {
		problems.push((
			Some(route.line),
			"`+not-found.rs` receives `leptoskit_router::NotFound` with the unmatched path, so it \
			 cannot declare a route struct"
				.to_owned(),
		));
	}
	for (hook, line) in &exports.hooks {
		if *hook != Hook::Load {
			problems.push((
				Some(*line),
				format!("`{}` has no meaning in `+not-found.rs`", hook.name()),
			));
		}
	}
	if !problems.is_empty() {
		report(file, problems, diagnostics);
		return None;
	}
	let mut segments = prefix.to_vec();
	segments.push(Segment::OptionalRest("path".to_owned()));
	Some(Route {
		variant: derived_name(prefix, "NotFound"),
		// Not-found routes come after every page, so their one shape keeps
		// the catch-all whole.
		shapes: vec![Shape {
			segments: segments.clone(),
			absent: Vec::new(),
		}],
		segments,
		file: file.path.clone(),
		kind: RouteKind::NotFound,
		target: Target::NotFound,
		page: page?.style,
		page_call: call?,
		layouts: layouts.to_vec(),
	})
}

fn page_export<'a>(
	exports: &'a Exports,
	problems: &mut Vec<Problem>,
) -> Option<&'a ComponentExport> {
	let page = exports.components.get(&Component::Page);
	if page.is_none() {
		problems.push(missing_export(
			exports,
			Component::Page,
			"a route file exports `pub fn page`, or `pub fn Page` annotated with `#[component]`",
		));
	}
	page
}

/// The problem for a file without its required export. A private function
/// with the export's name is the likely cause, so the message names it, at
/// its line.
fn missing_export(exports: &Exports, component: Component, message: &str) -> Problem {
	match exports.private.get(&component) {
		Some((name, line)) => {
			(
				Some(*line),
				format!("`{name}` must be `pub` for the router to use it"),
			)
		}
		None => (None, message.to_owned()),
	}
}

fn check_struct(route: &RouteStruct, segments: &[Segment], problems: &mut Vec<Problem>) {
	let line = Some(route.line);
	if route.name == "Route" || route.name == "Navigation" {
		problems.push((
			line,
			format!(
				"a route struct cannot be called `{}`: the generated module defines that name",
				route.name
			),
		));
	}
	for segment in segments {
		let Some(param) = segment.param() else {
			continue;
		};
		let Some(field) = route.fields.iter().find(|field| field.name == param) else {
			problems.push((
				line,
				format!(
					"`{}` needs a field `{param}` for the path segment `{}`",
					route.name,
					written(segment)
				),
			));
			continue;
		};
		let (expected, reason) = match segment {
			Segment::Param(_) => (FieldKind::One, "is always one value, so it is a plain type"),
			Segment::OptionalParam(_) => {
				(FieldKind::Optional, "may be absent, so it is an `Option`")
			}
			Segment::Rest(_) | Segment::OptionalRest(_) => {
				(
					FieldKind::Many,
					"holds every remaining segment, so it is a `Vec`",
				)
			}
			Segment::Static(_) => continue,
		};
		if field.kind != expected {
			problems.push((
				line,
				format!("the field `{param}`: `{}` {reason}", written(segment)),
			));
		}
	}
}

fn check_layout(file: &File, diagnostics: &mut Vec<Diagnostic>) {
	let Some(exports) = &file.exports else {
		return;
	};
	let mut problems = Vec::new();
	if !exports.components.contains_key(&Component::Layout) {
		problems.push(missing_export(
			exports,
			Component::Layout,
			"`_layout.rs` exports `pub fn layout`, or `pub fn Layout` annotated with \
			 `#[component]`",
		));
	}
	for (component, export) in &exports.components {
		let line = &export.line;
		if matches!(component, Component::Page | Component::Html) {
			problems.push((
				Some(*line),
				format!(
					"`{}` belongs in {}",
					component.function_name(),
					belongs_in(*component)
				),
			));
		}
	}
	if let Some(route) = &exports.route {
		problems.push((
			Some(route.line),
			"a route struct belongs in a route file, not `_layout.rs`".to_owned(),
		));
	}
	if let Some(line) = exports.hooks.get(&Hook::Prerender) {
		problems.push((
			Some(*line),
			"`prerender` belongs in a route file".to_owned(),
		));
	}
	report(file, problems, diagnostics);
}

fn check_html(file: &File, is_root: bool, diagnostics: &mut Vec<Diagnostic>) {
	let mut problems = Vec::new();
	if !is_root {
		problems.push((
			None,
			"`+html.rs` is the document shell, so it lives at the top of the routes directory"
				.to_owned(),
		));
	}
	if let Some(exports) = &file.exports
		&& !exports.components.contains_key(&Component::Html)
	{
		problems.push(missing_export(
			exports,
			Component::Html,
			"`+html.rs` exports `pub fn html`, or `pub fn Html` annotated with `#[component]`",
		));
	}
	report(file, problems, diagnostics);
}

fn belongs_in(component: Component) -> &'static str {
	match component {
		Component::Layout => "`_layout.rs`",
		Component::Html => "`+html.rs`",
		_ => "a route file",
	}
}

fn report(file: &File, problems: Vec<Problem>, diagnostics: &mut Vec<Diagnostic>) {
	diagnostics.extend(
		problems
			.into_iter()
			.map(|(line, message)| Diagnostic::new(&file.path, line, message)),
	);
}

/// A segment as it is written in a file name.
fn written(segment: &Segment) -> String {
	match segment {
		Segment::Static(text) => text.clone(),
		Segment::Param(name) => format!("[{name}]"),
		Segment::OptionalParam(name) => format!("[[{name}]]"),
		Segment::Rest(name) => format!("[...{name}]"),
		Segment::OptionalRest(name) => format!("[[...{name}]]"),
	}
}

/// A variant name from a route's static segments: `/settings/privacy` is
/// `SettingsPrivacy` and `/` is `Index`. A name that is not an identifier is
/// escaped the way module names are, so `/404` is `_404`.
fn derived_name(segments: &[Segment], suffix: &str) -> String {
	let name: String = segments
		.iter()
		.filter_map(|segment| {
			match segment {
				Segment::Static(text) => Some(name::pascal_case(text)),
				_ => None,
			}
		})
		.collect();
	if name.is_empty() && suffix.is_empty() {
		return "Index".to_owned();
	}
	name::escape(name + suffix)
}

fn check_names(routes: &[Route], dir: &Path, diagnostics: &mut Vec<Diagnostic>) {
	let mut names: BTreeMap<&str, &Route> = BTreeMap::new();
	for route in routes {
		if let Some(previous) = names.insert(&route.variant, route) {
			diagnostics.push(Diagnostic::new(
				&route.file,
				None,
				format!(
					"this route and `{}` are both named `{}`; give one a `#[derive(Route)] pub \
					 struct` with another name",
					relative(&previous.file, dir),
					route.variant
				),
			));
		}
	}
}

/// Every shape of URL `segments` match, with each optional segment present or
/// absent, dropping repeats: `[[a]]/[[b]]` reads one segment as `a`, the way
/// its pattern does, rather than as `b`.
fn shapes(segments: &[Segment]) -> Vec<Shape> {
	let mut shapes = vec![Shape {
		segments: Vec::new(),
		absent: Vec::new(),
	}];
	for segment in segments {
		let (present, name) = match segment {
			Segment::OptionalParam(name) => (Segment::Param(name.clone()), name),
			Segment::OptionalRest(name) => (Segment::Rest(name.clone()), name),
			required => {
				for shape in &mut shapes {
					shape.segments.push(required.clone());
				}
				continue;
			}
		};
		shapes = shapes
			.into_iter()
			.flat_map(|shape| {
				let mut with = shape.clone();
				with.segments.push(present.clone());
				let mut without = shape;
				without.absent.push(name.clone());
				[with, without]
			})
			.collect();
	}
	let mut seen = BTreeSet::new();
	shapes.retain(|shape| seen.insert(shape_key(&shape.segments)));
	shapes
}

/// What decides which URLs a shape matches: each segment's rank and static
/// text. Parameter names do not count.
fn shape_key(segments: &[Segment]) -> (Vec<u8>, Vec<String>) {
	segments
		.iter()
		.map(|segment| {
			match segment {
				Segment::Static(text) => (segment.rank(), text.clone()),
				_ => (segment.rank(), String::new()),
			}
		})
		.unzip()
}

/// Most specific first: segment by segment, a static segment beats a
/// parameter, which beats a catch-all. Not-found routes come after every
/// page, deepest first.
fn order_key(route: &Route, shape: &Shape) -> (bool, usize, (Vec<u8>, Vec<String>), PathBuf) {
	let not_found = route.kind == RouteKind::NotFound;
	let depth = if not_found {
		usize::MAX - shape.segments.len()
	} else {
		0
	};
	(
		not_found,
		depth,
		shape_key(&shape.segments),
		route.file.clone(),
	)
}

/// Reports every route that shares a shape with another: the two match the
/// same URLs, so one of them could never be chosen. Each pair is reported
/// once, at its most specific shared shape.
fn check_duplicates(
	routes: &[Route],
	matches: &[(usize, usize)],
	dir: &Path,
	diagnostics: &mut Vec<Diagnostic>,
) {
	let key = |&(route, shape): &(usize, usize)| {
		let (not_found, _, shape, _) = order_key(&routes[route], &routes[route].shapes[shape]);
		(not_found, shape)
	};
	let mut reported = BTreeSet::new();
	for run in matches.chunk_by(|left, right| key(left) == key(right)) {
		let (first, shape) = run[0];
		for &(other, _) in &run[1..] {
			if other != first && reported.insert((first, other)) {
				diagnostics.push(Diagnostic::new(
					&routes[other].file,
					None,
					format!(
						"this route and `{}` match the same URLs, `{}`",
						relative(&routes[first].file, dir),
						crate::pattern_display(&routes[first].shapes[shape].segments)
					),
				));
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn writes_segments_the_way_file_names_spell_them() {
		let written: Vec<_> = [
			Segment::Static("notes".to_owned()),
			Segment::Param("note".to_owned()),
			Segment::OptionalParam("lang".to_owned()),
			Segment::Rest("topic".to_owned()),
			Segment::OptionalRest("path".to_owned()),
		]
		.iter()
		.map(written)
		.collect();
		assert_eq!(
			written,
			["notes", "[note]", "[[lang]]", "[...topic]", "[[...path]]"]
		);
	}

	#[test]
	fn names_routes_after_their_static_segments() {
		assert_eq!(derived_name(&[], ""), "Index");
		assert_eq!(derived_name(&[], "NotFound"), "NotFound");
		assert_eq!(
			derived_name(
				&[
					Segment::Static("settings".to_owned()),
					Segment::Param("section".to_owned()),
					Segment::Static("sign-in".to_owned()),
				],
				""
			),
			"SettingsSignIn"
		);
		let statics = |names: &[&str]| -> Vec<Segment> {
			names
				.iter()
				.map(|name| Segment::Static((*name).to_owned()))
				.collect()
		};
		assert_eq!(derived_name(&statics(&["404"]), ""), "_404");
		assert_eq!(derived_name(&statics(&["2fa", "setup"]), ""), "_2faSetup");
		assert_eq!(derived_name(&statics(&["self"]), ""), "Self_");
		assert_eq!(derived_name(&statics(&["404"]), "NotFound"), "_404NotFound");
	}

	/// The table for a routes tree written to a temporary directory, with its
	/// diagnostics.
	fn table(files: &[(&str, &str)]) -> (Table, Vec<String>) {
		let project = tempfile::tempdir().expect("temp dir");
		let dir = project.path().join("routes");
		for (path, source) in files {
			let path = dir.join(path);
			std::fs::create_dir_all(path.parent().expect("a parent")).expect("create dirs");
			std::fs::write(path, source).expect("write route file");
		}
		let mut diagnostics = Vec::new();
		let root = crate::tree::read(&dir, &mut diagnostics);
		let table = build(&root, &dir, &mut diagnostics);
		let messages = diagnostics
			.iter()
			.map(|diagnostic| diagnostic.relative_to(&dir))
			.collect();
		(table, messages)
	}

	/// The order the generated code tries the table's shapes in.
	fn order(table: &Table) -> Vec<String> {
		table
			.matches
			.iter()
			.map(|&(route, shape)| {
				let route = &table.routes[route];
				let shape = crate::pattern_display(&route.shapes[shape].segments);
				format!("{} {shape}", route.variant)
			})
			.collect()
	}

	#[test]
	fn an_optional_segment_takes_its_place_once_present_and_once_absent() {
		let (table, messages) = table(&[
			(
				"[section].rs",
				"#[derive(Route)]\npub struct Section { pub section: String }\npub fn page() {}",
			),
			(
				"[[lang]]/about.rs",
				"#[derive(Route)]\npub struct About { pub lang: Option<String> }\npub fn page() {}",
			),
			(
				"fr/[page].rs",
				"#[derive(Route)]\npub struct French { pub page: String }\npub fn page() {}",
			),
		]);
		assert!(messages.is_empty(), "{messages:?}");
		// `/about` is a static segment, so it beats `/:section`; `/fr/about`
		// starts with a parameter, so `/fr/:page` beats it.
		assert_eq!(
			order(&table),
			[
				"About /about",
				"French /fr/:page",
				"Section /:section",
				"About /:lang/about",
			]
		);
		let names: Vec<_> = table
			.routes
			.iter()
			.map(|route| route.variant.as_str())
			.collect();
		assert_eq!(names, ["About", "French", "Section"]);
		assert_eq!(table.routes[0].shapes[1].absent, ["lang"]);
	}

	#[test]
	fn a_route_drops_shapes_it_repeats() {
		let (table, messages) = table(&[(
			"[[a]]/[[b]].rs",
			"#[derive(Route)]\npub struct Pair { pub a: Option<u8>, pub b: Option<u8> }\npub fn \
			 page() {}",
		)]);
		assert!(messages.is_empty(), "{messages:?}");
		// One segment is `a`, as the pattern reads it; `b` alone is a repeat.
		assert_eq!(order(&table), ["Pair /", "Pair /:a", "Pair /:a/:b"]);
		let absent: Vec<_> = table.routes[0]
			.shapes
			.iter()
			.map(|shape| shape.absent.clone())
			.collect();
		assert_eq!(
			absent,
			[
				Vec::new(),
				vec!["b".to_owned()],
				vec!["a".to_owned(), "b".to_owned()]
			]
		);
	}

	#[test]
	fn not_found_routes_keep_their_catch_all_and_come_last_deepest_first() {
		let (table, messages) = table(&[
			("+not-found.rs", "pub fn page() {}"),
			("photos/+not-found.rs", "pub fn page() {}"),
			(
				"[[...rest]].rs",
				"#[derive(Route)]\npub struct Rest { pub rest: Vec<String> }\npub fn page() {}",
			),
		]);
		assert!(messages.is_empty(), "{messages:?}");
		assert_eq!(
			order(&table),
			[
				"Rest /",
				"Rest /*rest",
				"PhotosNotFound /photos/*path?",
				"NotFound /*path?",
			]
		);
	}
}
