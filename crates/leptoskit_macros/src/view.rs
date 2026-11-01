//! `view!`: rstml markup expanded into component builder calls.
//!
//! Every tag is a component. `<Name a=x on:press=f>children</Name>` becomes
//! `Name::builder().a(x).on_press(f).children(fragment).build()`, where
//! `Name` is the type that the built-in component or `#[component]` declares
//! beside its function. Each method call carries the span of the markup it
//! came from, so type errors and completions land on the attribute.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::ToTokens;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use rstml::Parser;
use rstml::ParserConfig;
use rstml::node::KVAttributeValue;
use rstml::node::KeyedAttribute;
use rstml::node::KeyedAttributeValue;
use rstml::node::Node;
use rstml::node::NodeAttribute;
use rstml::node::NodeBlock;
use rstml::node::NodeElement;
use rstml::node::NodeName;
use rstml::node::NodeNameFragment;
use syn::Expr;
use syn::Ident;
use syn::Stmt;
use syn::spanned::Spanned;

/// Expands `view!` markup.
///
/// Parsing recovers from errors, so the markup that did parse still expands:
/// rust-analyzer keeps completing method names and expressions inside a
/// half-written view.
pub fn expand(tokens: TokenStream) -> TokenStream {
	let parser = Parser::new(ParserConfig::new().recover_block(true));
	let (nodes, diagnostics) = parser.parse_recoverable(tokens).split();
	let mut expander = Expander::default();
	let view = expander.root(nodes.unwrap_or_default());
	let diagnostics = diagnostics
		.into_iter()
		.map(proc_macro2_diagnostics::Diagnostic::emit_as_expr_tokens);
	let errors = expander
		.errors
		.into_iter()
		.map(|error| error.to_compile_error());

	quote! {
		{
			#(#diagnostics;)*
			#(#errors;)*
			#[allow(unused_imports)]
			use ::leptoskit::__private::BuildView as _;
			#view
		}
	}
}

#[derive(Default)]
struct Expander {
	errors: Vec<syn::Error>,
}

impl Expander {
	/// One element stays an element; anything else becomes a fragment.
	fn root(&mut self, nodes: Vec<Node>) -> TokenStream {
		let nodes = nodes
			.into_iter()
			.filter(|node| !matches!(node, Node::Comment(_)))
			.collect::<Vec<_>>();

		if let [Node::Element(element)] = nodes.as_slice() {
			return self.element(element);
		}

		self.fragment(&nodes)
	}

	fn element(&mut self, element: &NodeElement<rstml::Infallible>) -> TokenStream {
		let name = element.name();
		let span = name.span();
		let Some(path) = self.tag_path(name) else {
			return quote_spanned!(span=> ::leptoskit::Fragment::new());
		};

		if !element.open_tag.generics.args.is_empty() {
			self.error(
				element.open_tag.generics.span(),
				"tags take no generic arguments; give a prop value a type annotation instead",
			);
		}

		let mut builder = quote_spanned!(span=> #path::builder());

		for attribute in element.attributes() {
			match attribute {
				NodeAttribute::Attribute(attribute) => {
					if let Some(call) = self.attribute(attribute) {
						builder = quote!(#builder #call);
					}
				}
				NodeAttribute::Block(block) => {
					self.error(
						block.span(),
						"attributes need a name, like `class=…` or `on:press=…`",
					);
				}
			}
		}

		if !element.children.is_empty() {
			let children = self.fragment(&element.children);
			let method = Ident::new("children", span);
			builder = quote!(#builder.#method(#children));
		}

		let build = Ident::new("build", span);
		quote!(#builder.#build())
	}

	/// The path a tag names: `View`, or `ui::Card`.
	fn tag_path(&mut self, name: &NodeName) -> Option<TokenStream> {
		if let NodeName::Path(path) = name {
			if let Some(ident) = path.path.get_ident()
				&& ident
					.to_string()
					.starts_with(|first: char| first.is_lowercase())
			{
				self.error(
					ident.span(),
					format!(
						"`<{ident}>` is not a component: component names start with a capital \
						 letter, like `<View>`; leptos's `view!` is the one for HTML elements"
					),
				);

				return None;
			}

			return Some(path.to_token_stream());
		}

		self.error(
			name.span(),
			format!(
				"`{}` is not a component; tags name a component, like `<View>` or `<ui::Card>`",
				name.to_token_stream()
			),
		);
		None
	}

	/// The builder method call an attribute makes.
	fn attribute(&mut self, attribute: &KeyedAttribute) -> Option<TokenStream> {
		let method = self.method(&attribute.key)?;

		let arguments = match &attribute.possible_value {
			KeyedAttributeValue::None => quote_spanned!(method.span()=> true),
			KeyedAttributeValue::Value(value) => {
				match &value.value {
					KVAttributeValue::Expr(Expr::Tuple(tuple)) => tuple.elems.to_token_stream(),
					KVAttributeValue::Expr(expr) => unbrace(expr),
					// rstml has reported the error; keep the tokens so
					// rust-analyzer can complete inside them.
					KVAttributeValue::InvalidBraced(block) => block.to_token_stream(),
				}
			}
			KeyedAttributeValue::Binding(binding) => {
				self.error(
					binding.span(),
					format!("write the handler as a value: `{}=|…| …`", attribute.key),
				);
				return None;
			}
		};

		Some(quote!(.#method(#arguments)))
	}

	/// `class` calls `.class`; `on:press` calls `.on_press`.
	fn method(&mut self, key: &NodeName) -> Option<Ident> {
		match key {
			NodeName::Path(path) if path.path.get_ident().is_some() => {
				path.path.get_ident().cloned()
			}

			NodeName::Punctuated(parts) => {
				let pairs = parts.pairs().collect::<Vec<_>>();
				if let [first, second] = pairs.as_slice()
					&& let (NodeNameFragment::Ident(prefix), Some(punct)) =
						(first.value(), first.punct())
					&& prefix == "on"
					&& punct.as_char() == ':'
					&& let NodeNameFragment::Ident(event) = second.value()
				{
					return Some(format_ident!("on_{}", event, span = key.span()));
				}

				self.error(
					key.span(),
					format!(
						"`{key}` is not an attribute: use a prop name like `class`, or `on:event` \
						 for a handler"
					),
				);
				None
			}
			_ => {
				self.error(
					key.span(),
					format!("`{key}` is not an attribute: use a prop name like `class`"),
				);
				None
			}
		}
	}

	/// `Fragment::new().with(a).with(b)` for a list of child nodes.
	fn fragment(&mut self, nodes: &[Node]) -> TokenStream {
		let mut fragment = quote!(::leptoskit::Fragment::new());

		for node in nodes {
			let child = match node {
				Node::Element(element) => self.element(element),
				Node::Fragment(nested) => self.fragment(&nested.children),
				Node::Text(text) => text.value.to_token_stream(),
				Node::RawText(text) => {
					let span = text.join_spans().unwrap_or_else(Span::call_site);
					syn::LitStr::new(&text.to_string_best(), span).to_token_stream()
				}

				Node::Block(NodeBlock::ValidBlock(block)) if block.stmts.is_empty() => continue,
				Node::Block(NodeBlock::ValidBlock(block)) => {
					unbrace(&Expr::Block(syn::ExprBlock {
						attrs: Vec::new(),
						label: None,
						block: block.clone(),
					}))
				}
				Node::Block(invalid @ NodeBlock::Invalid(_)) => invalid.to_token_stream(),
				Node::Comment(_) => continue,
				// The parser is configured without custom nodes, so only a
				// doctype reaches this arm.
				Node::Doctype(_) | Node::Custom(_) => {
					self.error(node.span(), "a view has no doctype");
					continue;
				}
			};

			fragment = quote!(#fragment.with(#child));
		}

		fragment
	}

	fn error(&mut self, span: Span, message: impl std::fmt::Display) {
		self.errors.push(syn::Error::new(span, message));
	}
}

/// `{ value }` becomes `value`, which avoids `unused_braces` warnings on
/// `.with({ value })`; a block with statements stays a block.
fn unbrace(expr: &Expr) -> TokenStream {
	if let Expr::Block(block) = expr
		&& block.attrs.is_empty()
		&& block.label.is_none()
		&& let [Stmt::Expr(inner, None)] = block.block.stmts.as_slice()
	{
		return inner.to_token_stream();
	}

	expr.to_token_stream()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn pretty(tokens: &TokenStream) -> String {
		let file = syn::parse2::<syn::File>(quote!(fn view() { #tokens })).expect("valid Rust");
		prettyplease::unparse(&file)
	}

	fn errors(tokens: TokenStream) -> Vec<String> {
		let output = expand(tokens).to_string();
		output
			.split("compile_error")
			.skip(1)
			.map(|part| {
				let start = part.find('"').expect("a message") + 1;
				let end = part[start..].find('"').expect("a closing quote") + start;
				part[start..end].to_owned()
			})
			.collect()
	}

	#[test]
	fn a_tag_becomes_builder_calls() {
		insta::assert_snapshot!(pretty(&expand(quote! {
			<View class="p-4" on:press=move || tapped() enabled>
				<Text>"Count: " {count}</Text>
				<ui::Card border=(1.0, ink) />
				"tail"
			</View>
		})));
	}

	#[test]
	fn several_roots_and_fragments_become_fragments() {
		insta::assert_snapshot!(pretty(&expand(quote! {
			<Text>"a"</Text>
			<>
				<Text>"b"</Text>
				{items}
			</>
			<!-- "a comment" -->
			{}
		})));
	}

	#[test]
	fn blocks_keep_statements_and_unwrap_single_values() {
		insta::assert_snapshot!(pretty(&expand(quote! {
			<View class={classes}>
				{ let label = name.clone(); label }
				{(a, b)}
			</View>
		})));
	}

	#[test]
	fn unquoted_text_is_a_string() {
		let output = expand(quote! { <Text>Hello world</Text> }).to_string();
		assert!(output.contains("\"Hello world\""), "{output}");
	}

	#[test]
	fn an_empty_view_is_an_empty_fragment() {
		insta::assert_snapshot!(pretty(&expand(quote! {})));
	}

	#[test]
	fn unsupported_markup_reports_errors() {
		assert_eq!(
			errors(quote! {
				<View {spread} data-id="x" aria:label="x" bind:value=x on:press(event)>
					<!DOCTYPE html>
					<{dynamic}/>
				</View>
			}),
			[
				"attributes need a name, like `class=…` or `on:press=…`",
				"`data-id` is not an attribute: use a prop name like `class`, or `on:event` for a \
				 handler",
				"`aria:label` is not an attribute: use a prop name like `class`, or `on:event` \
				 for a handler",
				"`bind:value` is not an attribute: use a prop name like `class`, or `on:event` \
				 for a handler",
				"write the handler as a value: `on:press=|…| …`",
				"a view has no doctype",
				"`{ dynamic }` is not a component; tags name a component, like `<View>` or \
				 `<ui::Card>`",
			]
		);
	}

	#[test]
	fn generic_tags_and_path_keys_report_errors() {
		assert_eq!(
			errors(quote! { <List<u8> a::b=1 /> <div /> }),
			[
				"tags take no generic arguments; give a prop value a type annotation instead",
				"`a::b` is not an attribute: use a prop name like `class`",
				"`<div>` is not a component: component names start with a capital letter, like \
				 `<View>`; leptos's `view!` is the one for HTML elements",
			]
		);
	}

	#[test]
	fn parse_errors_still_expand_what_parsed() {
		let output = expand(quote! { <View class="p-4"><Text>"open" </View> }).to_string();
		assert!(output.contains("compile_error"), "{output}");
		assert!(output.contains("View :: builder ()"), "{output}");

		// Unparsable braces stay in the expansion for rust-analyzer.
		let output = expand(quote! {
			<View class={a b}>
				<!-- "kept out" -->
				{c d}
			</View>
		})
		.to_string();
		assert!(output.contains("{ a b }"), "{output}");
		assert!(output.contains("{ c d }"), "{output}");
		assert!(!output.contains("kept out"), "{output}");
		assert!(output.contains("compile_error"), "{output}");
		assert!(output.contains("View :: builder ()"), "{output}");
	}
}
