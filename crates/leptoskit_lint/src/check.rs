//! Checking one parsed file against the rules.

use std::collections::BTreeSet;
use std::collections::HashMap;

use leptoskit_config::LintConfig;
use leptoskit_config::LintLevel;
use leptoskit_config::MarkupStyle;
use proc_macro2::Span;
use syn::Attribute;
use syn::Expr;
use syn::ItemFn;
use syn::Macro;
use syn::ReturnType;
use syn::Signature;
use syn::Type;
use syn::TypeParamBound;
use syn::visit::Visit;

use crate::Rule;

/// The components built into leptoskit that `view!` writes as tags: the
/// function each tag stands for when the tree is written with builders.
///
/// Helpers that take elements as arguments, such as `ListTile` or
/// `Expanded`, have no tag and stay builder calls inside markup, so they are
/// not listed. `tests/catalog.rs` keeps this in step with the crates that
/// declare the tags.
pub const BUILT_IN_COMPONENTS: &[&str] = &[
	"ActivityIndicator",
	"Alert",
	"Button",
	"Card",
	"Center",
	"Checkbox",
	"ClipRect",
	"Column",
	"Divider",
	"Heading",
	"Image",
	"LinearProgressIndicator",
	"ListView",
	"PasswordInput",
	"Pressable",
	"QueryClientProvider",
	"Radio",
	"Rive",
	"Row",
	"ScrollView",
	"Slider",
	"Spacer",
	"Status",
	"Switch",
	"Text",
	"TextInput",
	"Vector",
	"VerticalDivider",
	"View",
	"Wrap",
];

/// The attributes that make a function a component: leptoskit's and leptos's
/// `#[component]`, and leptos's `#[island]`.
const COMPONENT_ATTRIBUTES: &[&str] = &["component", "island"];

/// The types a component returns, besides `impl IntoView`.
const VIEW_TYPES: &[&str] = &["Element", "Fragment", "AnyView"];

/// What a rule found in a file, before it is placed in a [`crate::Diagnostic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
	/// The rule that found it.
	pub rule: Rule,
	/// Where it starts.
	pub start: Location,
	/// Where it ends.
	pub end: Location,
	/// What is wrong.
	pub message: String,
	/// The note under the highlighted source.
	pub label: String,
	/// How to fix it.
	pub help: String,
}

/// A position in a file: a 1-based line and a 0-based column in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
	/// The line, from 1.
	pub line: usize,
	/// The column in characters, from 0.
	pub column: usize,
}

impl From<proc_macro2::LineColumn> for Location {
	fn from(position: proc_macro2::LineColumn) -> Self {
		Self {
			line: position.line,
			column: position.column,
		}
	}
}

/// The names `view!` can call as tags in a run: the built-in components and
/// every `#[component]` function the run has seen.
#[derive(Debug, Clone, Default)]
pub struct ComponentNames {
	names: BTreeSet<String>,
}

impl ComponentNames {
	/// The built-in components alone.
	#[must_use]
	pub fn built_in() -> Self {
		Self {
			names: BUILT_IN_COMPONENTS
				.iter()
				.map(|&name| name.to_owned())
				.collect(),
		}
	}

	/// Adds every `#[component]` function declared in `file`.
	pub fn collect(&mut self, file: &syn::File) {
		struct Collector<'a>(&'a mut BTreeSet<String>);

		impl<'ast> Visit<'ast> for Collector<'_> {
			fn visit_item_fn(&mut self, function: &'ast ItemFn) {
				if is_component(&function.attrs) {
					self.0.insert(function.sig.ident.to_string());
				}
				syn::visit::visit_item_fn(self, function);
			}
		}

		Collector(&mut self.names).visit_file(file);
	}

	/// Adds the names another set holds.
	pub fn extend(&mut self, other: &Self) {
		self.names.extend(other.names.iter().cloned());
	}

	fn contains(&self, name: &str) -> bool {
		self.names.contains(name)
	}
}

/// Checks a parsed file against every rule the configuration turns on.
///
/// `source` is the text `file` was parsed from: a `// leptoskit-lint:
/// allow(rule)` comment on the line above a finding silences it.
#[must_use]
pub fn check_file(
	file: &syn::File,
	source: &str,
	config: &LintConfig,
	components: &ComponentNames,
) -> Vec<Finding> {
	let mut checker = Checker {
		config,
		components,
		findings: Vec::new(),
	};
	checker.visit_file(file);
	let allowed = allow_comments(source);
	let mut findings = checker.findings;
	findings.retain(|finding| {
		!allowed
			.get(&finding.start.line)
			.is_some_and(|rules| rules.contains(finding.rule.name()))
	});
	findings.sort_by_key(|finding| finding.start);
	findings
}

struct Checker<'a> {
	config: &'a LintConfig,
	components: &'a ComponentNames,
	findings: Vec<Finding>,
}

impl Checker<'_> {
	fn enabled(&self, rule: Rule) -> bool {
		rule.level(&self.config.rules) != LintLevel::Allow
	}

	fn report(&mut self, rule: Rule, span: Span, message: String, label: &str, help: String) {
		self.findings.push(Finding {
			rule,
			start: span.start().into(),
			end: span.end().into(),
			message,
			label: label.to_owned(),
			help,
		});
	}

	fn check_component_attribute(&mut self, attributes: &[Attribute], signature: &Signature) {
		let name = signature.ident.to_string();
		let capitalized = name.chars().next().is_some_and(char::is_uppercase);
		if !capitalized || !returns_view(&signature.output) || is_component(attributes) {
			return;
		}
		self.report(
			Rule::ComponentAttribute,
			signature.ident.span(),
			format!("`{name}` returns a view but is not marked `#[component]`"),
			"a capitalized function that returns a view is a component",
			format!(
				"add `#[component]` above it, so `view!` can call it as `<{name} … />` with named \
				 props"
			),
		);
	}

	/// Reports a tree built with builder calls, once at its root.
	fn check_builder(&mut self, expr: &Expr) -> bool {
		let Some(callee) = builder_root(expr, self.components) else {
			return false;
		};
		let name = callee.to_string();
		self.report(
			Rule::MarkupStyle,
			callee.span(),
			format!("`{name}(…)` builds this tree with builder calls"),
			"this project writes component trees as markup",
			format!(
				"write it with `view!`, as `view! {{ <{name} …>…</{name}> }}`, or set `markup = \
				 \"any\"` under `[lint]` in leptoskit.toml"
			),
		);
		true
	}
}

impl<'ast> Visit<'ast> for Checker<'_> {
	fn visit_item_fn(&mut self, function: &'ast ItemFn) {
		if self.enabled(Rule::ComponentAttribute) {
			self.check_component_attribute(&function.attrs, &function.sig);
		}
		syn::visit::visit_item_fn(self, function);
	}

	fn visit_expr(&mut self, expr: &'ast Expr) {
		let checks_builders =
			self.enabled(Rule::MarkupStyle) && self.config.markup == MarkupStyle::View;
		// One report per tree: the calls nested inside it are part of it.
		if checks_builders && self.check_builder(expr) {
			return;
		}
		syn::visit::visit_expr(self, expr);
	}

	fn visit_macro(&mut self, mac: &'ast Macro) {
		let checks_markup =
			self.enabled(Rule::MarkupStyle) && self.config.markup == MarkupStyle::Builder;
		if checks_markup && is_view_macro(mac) {
			let span = mac
				.path
				.segments
				.last()
				.map_or_else(Span::call_site, |segment| segment.ident.span());
			self.report(
				Rule::MarkupStyle,
				span,
				"`view!` writes this tree as markup".to_owned(),
				"this project writes component trees with builder calls",
				"build it with the component functions, as `View().child(Text(\"…\"))`, or set \
				 `markup = \"any\"` under `[lint]` in leptoskit.toml"
					.to_owned(),
			);
		}
		syn::visit::visit_macro(self, mac);
	}
}

/// Whether an attribute list marks a component.
fn is_component(attributes: &[Attribute]) -> bool {
	attributes.iter().any(|attribute| {
		attribute.path().segments.last().is_some_and(|segment| {
			COMPONENT_ATTRIBUTES
				.iter()
				.any(|name| segment.ident == name)
		})
	})
}

/// Whether a function returns `impl IntoView`, an `Element`, a `Fragment`, or
/// an `AnyView`.
fn returns_view(output: &ReturnType) -> bool {
	let ReturnType::Type(_, ty) = output else {
		return false;
	};
	match &**ty {
		Type::ImplTrait(implementation) => {
			implementation.bounds.iter().any(|bound| {
				matches!(bound, TypeParamBound::Trait(bound)
					if bound.path.segments.last().is_some_and(|segment| segment.ident == "IntoView"))
			})
		}
		Type::Path(path) => {
			path.qself.is_none()
				&& path
					.path
					.segments
					.last()
					.is_some_and(|segment| VIEW_TYPES.iter().any(|name| segment.ident == name))
		}
		_ => false,
	}
}

/// The component a builder expression starts from: `View` in
/// `View().class("p-4").child(…)`, or in `AspectRatio(1.0)?.child(…)`.
fn builder_root<'a>(expr: &'a Expr, components: &ComponentNames) -> Option<&'a syn::Ident> {
	match expr {
		Expr::MethodCall(call) => builder_root(&call.receiver, components),
		Expr::Try(attempt) => builder_root(&attempt.expr, components),
		Expr::Call(call) => {
			let Expr::Path(path) = &*call.func else {
				return None;
			};
			let mut segments = path.path.segments.iter().rev();
			let ident = &segments.next()?.ident;
			// A component is a function, so only a module path such as
			// `leptoskit::prelude` may precede it: `Message::Text(…)` is a
			// variant, and `Self::View(…)` an associated function.
			let in_type = segments
				.next()
				.is_some_and(|segment| segment.ident.to_string().starts_with(char::is_uppercase));
			let is_component =
				path.qself.is_none() && !in_type && components.contains(&ident.to_string());
			is_component.then_some(ident)
		}
		_ => None,
	}
}

fn is_view_macro(mac: &Macro) -> bool {
	mac.path
		.segments
		.last()
		.is_some_and(|segment| segment.ident == "view")
}

/// The rules each line silences with a `// leptoskit-lint: allow(rule, …)`
/// comment on the line above it.
fn allow_comments(source: &str) -> HashMap<usize, BTreeSet<&str>> {
	const DIRECTIVE: &str = "leptoskit-lint: allow(";

	let mut allowed = HashMap::new();
	for (index, line) in source.lines().enumerate() {
		let Some(comment) = line.trim_start().strip_prefix("//") else {
			continue;
		};
		let Some(rest) = comment.trim_start().strip_prefix(DIRECTIVE) else {
			continue;
		};
		let Some((rules, _)) = rest.split_once(')') else {
			continue;
		};
		// Lines count from 1, so the next line's number is this index + 2.
		allowed
			.entry(index + 2)
			.or_insert_with(BTreeSet::new)
			.extend(rules.split(',').map(str::trim));
	}
	allowed
}
