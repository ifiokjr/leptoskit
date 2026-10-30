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
use crate::tree::Call;
use crate::tree::Child;
use crate::tree::Tree;
use crate::tree::Written;
use crate::tree::is_view_macro;

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

	pub(crate) fn contains(&self, name: &str) -> bool {
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

	/// Reports a tree written in the style the project does not use, once
	/// at its root.
	fn check_markup_style(&mut self, tree: &Tree) {
		let expected = match tree.written {
			Written::Builder => MarkupStyle::Builder,
			Written::Markup => MarkupStyle::View,
		};
		if !self.enabled(Rule::MarkupStyle) || matches!(self.config.markup, MarkupStyle::Any) {
			return;
		}
		if self.config.markup == expected {
			return;
		}
		let name = &tree.name;
		match tree.written {
			Written::Builder => {
				self.report(
					Rule::MarkupStyle,
					tree.span,
					format!("`{name}(…)` builds this tree with builder calls"),
					"this project writes component trees as markup",
					format!(
						"write it with `view!`, as `view! {{ <{name} …>…</{name}> }}`, or set \
						 `markup = \"any\"` under `[lint]` in leptoskit.toml"
					),
				);
			}
			Written::Markup => {
				self.report(
					Rule::MarkupStyle,
					tree.span,
					format!("`<{name}>` writes this tree as markup"),
					"this project writes component trees with builder calls",
					format!(
						"build it with the component functions, as `{name}().class(…).child(…)`, \
						 or set `markup = \"any\"` under `[lint]` in leptoskit.toml"
					),
				);
			}
		}
	}

	/// Runs the tree rules over a tree and every tree inside it.
	fn check_tree(&mut self, tree: &Tree) {
		tree.walk(&mut |node| {
			self.check_redundant_view(node);
			// An empty view only stands in for spacing when it sits among
			// siblings that their parent could space instead; on
			// its own it is a separator.
			if node.children().nth(1).is_some() {
				for child in node.children() {
					if let Child::Tree(child) = child {
						self.check_spacer_view(child);
					}
				}
			}
			self.check_children_last(node);
			self.check_image_label(node);
			self.check_color_literal(node);
		});
	}

	fn check_redundant_view(&mut self, tree: &Tree) {
		let single_child = {
			let mut children = tree.children();
			matches!(
				(children.next(), children.next()),
				(Some(Child::Tree(_) | Child::Text), None)
			)
		};
		if !self.enabled(Rule::RedundantView)
			|| tree.name != "View"
			|| tree.attributes().next().is_some()
			|| !single_child
		{
			return;
		}
		self.report(
			Rule::RedundantView,
			tree.span,
			"this `View` only wraps one child".to_owned(),
			"a view with no classes, props, or handlers",
			"use the child directly, or style the view if it lays the child out".to_owned(),
		);
	}

	fn check_spacer_view(&mut self, tree: &Tree) {
		let classes: Option<Vec<&str>> = tree
			.attributes()
			.map(|attribute| {
				(attribute.name == "class")
					.then_some(attribute.text.as_deref())
					.flatten()
			})
			.collect::<Option<Vec<_>>>()
			.map(|texts| texts.into_iter().flat_map(str::split_whitespace).collect());
		let only_sizes = classes.is_some_and(|classes| {
			!classes.is_empty() && classes.iter().all(|class| is_size_class(class))
		});
		if !self.enabled(Rule::SpacerView)
			|| tree.name != "View"
			|| tree.children().next().is_some()
			|| !only_sizes
		{
			return;
		}
		self.report(
			Rule::SpacerView,
			tree.span,
			"this empty `View` only makes space".to_owned(),
			"an empty view sized to push its siblings apart",
			"space the siblings with `gap-*` on their parent, or use `Spacer()` to fill the \
			 remaining room"
				.to_owned(),
		);
	}

	fn check_children_last(&mut self, tree: &Tree) {
		if !self.enabled(Rule::ChildrenLast) || tree.written != Written::Builder {
			return;
		}
		let first_child = tree
			.calls
			.iter()
			.position(|call| matches!(call, Call::Child(_)));
		let late = first_child.and_then(|first| {
			tree.calls[first..].iter().find_map(|call| {
				match call {
					Call::Attribute(attribute) => Some(attribute),
					Call::Child(_) => None,
				}
			})
		});
		let Some(attribute) = late else {
			return;
		};
		let (name, span) = (attribute.name.clone(), attribute.span);
		self.report(
			Rule::ChildrenLast,
			span,
			format!("`.{name}(…)` comes after this tree's children"),
			"set after `.child(…)` or `.children(…)`",
			"set every property before the children, so the tree reads top-down like markup"
				.to_owned(),
		);
	}

	fn check_image_label(&mut self, tree: &Tree) {
		const NAMING: [&str; 5] = [
			"accessibility_label",
			"aria_label",
			"aria_labelledby",
			"aria_hidden",
			"role",
		];
		if !self.enabled(Rule::ImageLabel)
			|| tree.name != "Image"
			|| NAMING.iter().any(|name| tree.has_attribute(name))
		{
			return;
		}
		self.report(
			Rule::ImageLabel,
			tree.span,
			"this image has no accessible name".to_owned(),
			"screen readers announce it without a description",
			"describe it with `accessibility_label`, or mark decoration with `aria_hidden(true)`"
				.to_owned(),
		);
	}

	fn check_color_literal(&mut self, tree: &Tree) {
		if !self.enabled(Rule::ColorLiteral) {
			return;
		}
		let literals: Vec<(String, Span)> = tree
			.attributes()
			.filter(|attribute| {
				attribute.color_literal
					|| (attribute.name == "class"
						&& attribute.text.as_deref().is_some_and(has_arbitrary_color))
			})
			.map(|attribute| (attribute.name.clone(), attribute.span))
			.collect();
		for (name, span) in literals {
			self.report(
				Rule::ColorLiteral,
				span,
				format!("`{name}` sets a color written out by hand"),
				"a color outside the theme",
				"use a named color class such as `bg-slate-100`, or a shared constant, so the \
				 palette lives in one place"
					.to_owned(),
			);
		}
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
		// A tree is checked whole, so the calls nested inside it are not
		// visited again as trees of their own.
		if let Some(tree) = Tree::from_builder(expr, self.components) {
			self.check_markup_style(&tree);
			self.check_tree(&tree);
			return;
		}
		syn::visit::visit_expr(self, expr);
	}

	fn visit_macro(&mut self, mac: &'ast Macro) {
		if is_view_macro(mac) {
			let trees = Tree::from_markup(mac.tokens.clone());
			if let Some(first) = trees.first() {
				self.check_markup_style(first);
			}
			for tree in &trees {
				self.check_tree(tree);
			}
		}
		syn::visit::visit_macro(self, mac);
	}
}

/// Whether a Tailwind class only sizes or spaces an element: `h-4`,
/// `w-full`, `md:size-8`.
fn is_size_class(class: &str) -> bool {
	const SIZES: [&str; 8] = [
		"h-", "w-", "size-", "min-h-", "min-w-", "max-h-", "max-w-", "basis-",
	];
	let utility = class.rsplit(':').next().unwrap_or(class);
	SIZES.iter().any(|prefix| utility.starts_with(prefix))
}

/// Whether a class list holds an arbitrary color value, such as
/// `bg-[#1e293b]` or `text-[rgb(15_23_42)]`.
fn has_arbitrary_color(classes: &str) -> bool {
	classes.split_whitespace().any(|class| {
		["-[#", "-[rgb", "-[hsl", "-[oklch"]
			.iter()
			.any(|marker| class.contains(marker))
	})
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
