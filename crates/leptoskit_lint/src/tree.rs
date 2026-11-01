//! Component trees, read the same way from builder calls and from `view!`
//! markup, so a rule about trees is written once and holds for both styles.
//!
//! `View().class("p-4").child(Text("Hi"))` and
//! `view! { <View class="p-4"><Text>"Hi"</Text></View> }` both read as a
//! `View` tree with a `class` attribute and one `Text` child.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use rstml::node::KeyedAttribute;
use rstml::node::Node;
use rstml::node::NodeAttribute;
use rstml::node::NodeElement;
use rstml::node::NodeName;
use syn::Expr;
use syn::ExprCall;
use syn::ExprMethodCall;
use syn::Lit;
use syn::spanned::Spanned;

use crate::check::ComponentNames;

/// One component in a tree: the tag or the function the tree starts from.
#[derive(Debug)]
pub struct Tree {
	/// The component's name, such as `View`.
	pub name: String,
	/// Where the name is written.
	pub span: Span,
	/// Attributes and children, in the order they are written.
	pub calls: Vec<Call>,
	/// How the tree is written.
	pub written: Written,
}

/// How a tree is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
	/// Builder calls.
	Builder,
	/// `view!` markup.
	Markup,
}

/// One step in building a tree.
#[derive(Debug)]
pub enum Call {
	/// A property: a builder method, or a markup attribute.
	Attribute(Attribute),
	/// A child.
	Child(Child),
}

/// A property set on a tree.
#[derive(Debug)]
pub struct Attribute {
	/// The method or attribute name, such as `class` or `on:press`.
	pub name: String,
	/// Where the name is written.
	pub span: Span,
	/// The value, when it is a string literal.
	pub text: Option<String>,
	/// Whether the value builds a color from literal channels, such as
	/// `Color::rgba(22, 17, 31, 255)`.
	pub color_literal: bool,
}

/// A child of a tree.
#[derive(Debug)]
pub enum Child {
	/// A nested tree.
	Tree(Tree),
	/// Literal text.
	Text,
	/// Anything else: an expression, a list, or a fragment.
	Other,
}

impl Tree {
	/// The tree a builder expression builds, such as
	/// `View().class("p-4").child(…)`, when it starts from a component.
	pub fn from_builder(expr: &Expr, components: &ComponentNames) -> Option<Self> {
		let mut methods: Vec<&ExprMethodCall> = Vec::new();
		let mut current = expr;

		let root = loop {
			match current {
				Expr::MethodCall(call) => {
					methods.push(call);
					current = &call.receiver;
				}
				Expr::Try(attempt) => current = &attempt.expr,
				Expr::Call(call) => break call,
				_ => return None,
			}
		};

		let ident = component_call(root, components)?;
		let mut calls = Vec::new();

		for method in methods.into_iter().rev() {
			let name = method.method.to_string();

			match name.as_str() {
				"child" => {
					let child = method
						.args
						.first()
						.map_or(Child::Other, |argument| child(argument, components));
					calls.push(Call::Child(child));
				}
				"children" => {
					match method.args.first() {
						Some(Expr::Array(array)) => {
							calls.extend(
								array
									.elems
									.iter()
									.map(|element| Call::Child(child(element, components))),
							);
						}
						_ => calls.push(Call::Child(Child::Other)),
					}
				}
				_ => {
					calls.push(Call::Attribute(Attribute {
						name,
						span: method.method.span(),
						text: method.args.first().and_then(string_literal),
						color_literal: method.args.iter().any(is_color_literal),
					}));
				}
			}
		}

		Some(Self {
			name: ident.to_string(),
			span: ident.span(),
			calls,
			written: Written::Builder,
		})
	}

	/// The trees a `view!` macro's markup builds. Markup that does not
	/// parse yields the trees that did; the compiler reports the rest.
	pub fn from_markup(tokens: TokenStream) -> Vec<Self> {
		let parser = rstml::Parser::new(rstml::ParserConfig::new().recover_block(true));
		let (nodes, _diagnostics) = parser.parse_recoverable(tokens).split();
		let mut trees = Vec::new();
		collect_markup(&nodes.unwrap_or_default(), &mut trees);
		trees
	}

	/// The attributes, in the order they are written.
	pub fn attributes(&self) -> impl Iterator<Item = &Attribute> {
		self.calls.iter().filter_map(|call| {
			match call {
				Call::Attribute(attribute) => Some(attribute),
				Call::Child(_) => None,
			}
		})
	}

	/// The children, in the order they are written.
	pub fn children(&self) -> impl Iterator<Item = &Child> {
		self.calls.iter().filter_map(|call| {
			match call {
				Call::Child(child) => Some(child),
				Call::Attribute(_) => None,
			}
		})
	}

	/// Whether the tree sets an attribute named `name`.
	pub fn has_attribute(&self, name: &str) -> bool {
		self.attributes().any(|attribute| attribute.name == name)
	}

	/// Calls `visit` with this tree and every tree nested inside it.
	pub fn walk(&self, visit: &mut impl FnMut(&Self)) {
		visit(self);

		for child in self.children() {
			if let Child::Tree(tree) = child {
				tree.walk(visit);
			}
		}
	}
}

/// The component a call starts a tree from: `View()` or
/// `leptoskit::prelude::Text(…)`, but not `Message::Text(…)`, a variant, or
/// `Self::View(…)`, an associated function.
pub fn component_call<'a>(
	call: &'a ExprCall,
	components: &ComponentNames,
) -> Option<&'a syn::Ident> {
	let Expr::Path(path) = &*call.func else {
		return None;
	};

	if path.qself.is_some() {
		return None;
	}

	let mut segments = path.path.segments.iter().rev();
	let ident = &segments.next()?.ident;
	let in_type = segments
		.next()
		.is_some_and(|segment| segment.ident.to_string().starts_with(char::is_uppercase));
	(!in_type && components.contains(&ident.to_string())).then_some(ident)
}

fn child(expr: &Expr, components: &ComponentNames) -> Child {
	if let Some(tree) = Tree::from_builder(expr, components) {
		return Child::Tree(tree);
	}

	match expr {
		Expr::Lit(literal) if matches!(literal.lit, Lit::Str(_)) => Child::Text,

		Expr::Macro(mac) if is_view_macro(&mac.mac) => {
			let mut trees = Tree::from_markup(mac.mac.tokens.clone());
			match (trees.pop(), trees.is_empty()) {
				(Some(tree), true) => Child::Tree(tree),
				_ => Child::Other,
			}
		}

		_ => Child::Other,
	}
}

/// Whether a macro is `view!`, by any path.
pub fn is_view_macro(mac: &syn::Macro) -> bool {
	mac.path
		.segments
		.last()
		.is_some_and(|segment| segment.ident == "view")
}

fn string_literal(expr: &Expr) -> Option<String> {
	match expr {
		Expr::Lit(literal) => {
			match &literal.lit {
				Lit::Str(text) => Some(text.value()),
				_ => None,
			}
		}
		Expr::Paren(inner) => string_literal(&inner.expr),
		Expr::Block(block) => {
			match block.block.stmts.as_slice() {
				[syn::Stmt::Expr(inner, None)] => string_literal(inner),
				_ => None,
			}
		}
		_ => None,
	}
}

/// Whether an expression builds a color from literal channels:
/// `Color::rgb(…)`, `Color::rgba(…)`, or `Color::hex(…)` with literal
/// arguments.
fn is_color_literal(expr: &Expr) -> bool {
	let Expr::Call(call) = expr else {
		return false;
	};
	let Expr::Path(path) = &*call.func else {
		return false;
	};
	let mut segments = path.path.segments.iter().rev();
	let constructor = segments.next().map(|segment| segment.ident.to_string());
	let is_color = segments
		.next()
		.is_some_and(|segment| segment.ident == "Color");
	is_color
		&& matches!(constructor.as_deref(), Some("rgb" | "rgba" | "hex"))
		&& call
			.args
			.iter()
			.all(|argument| matches!(argument, Expr::Lit(_)))
}

fn collect_markup(nodes: &[Node], trees: &mut Vec<Tree>) {
	for node in nodes {
		match node {
			Node::Element(element) => trees.push(markup_tree(element)),
			Node::Fragment(fragment) => collect_markup(&fragment.children, trees),
			_ => {}
		}
	}
}

fn markup_tree(element: &NodeElement<rstml::Infallible>) -> Tree {
	let name = element.name();
	let mut calls: Vec<Call> = element
		.attributes()
		.iter()
		.filter_map(|attribute| {
			match attribute {
				NodeAttribute::Attribute(attribute) => {
					Some(Call::Attribute(markup_attribute(attribute)))
				}
				NodeAttribute::Block(_) => None,
			}
		})
		.collect();

	for node in element.children() {
		let child = match node {
			Node::Element(element) => Child::Tree(markup_tree(element)),
			Node::Text(_) | Node::RawText(_) => Child::Text,
			Node::Comment(_) => continue,
			_ => Child::Other,
		};

		calls.push(Call::Child(child));
	}

	Tree {
		name: last_segment(name),
		span: name.span(),
		calls,
		written: Written::Markup,
	}
}

fn markup_attribute(attribute: &KeyedAttribute) -> Attribute {
	let value = attribute.value();
	Attribute {
		name: attribute.key.to_string(),
		span: attribute.key.span(),
		text: value.and_then(string_literal),
		color_literal: value.is_some_and(is_color_literal),
	}
}

fn last_segment(name: &NodeName) -> String {
	match name {
		NodeName::Path(path) => {
			path.path
				.segments
				.last()
				.map_or_else(|| name.to_string(), |segment| segment.ident.to_string())
		}
		_ => name.to_string(),
	}
}
