//! Parses a markup macro's body with rstml into a tree that remembers where
//! everything came from.
//!
//! rstml parses tokens, and tokens carry no comments or blank lines, so every
//! gap between two nodes or two attributes is read back from the source text:
//! a comment on the same line as the item before it trails that item, any
//! other comment leads the item after it, and blank lines are remembered so
//! the layout can keep one. Text, literals, and Rust expressions are kept as
//! byte ranges into the source; the layout copies or formats them, never
//! re-tokenizes them.

use std::ops::Range;
use std::str::FromStr;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use rstml::ParserConfig;
use rstml::node::KVAttributeValue;
use rstml::node::KeyedAttributeValue;
use rstml::node::NodeAttribute;
use rstml::node::NodeBlock;
use syn::spanned::Spanned;

use crate::lexer::Comment;
use crate::lexer::Lexer;
use crate::lexer::MacroCall;
use crate::lexer::Token;
use crate::lexer::comments_in;

/// A markup macro's parsed body.
#[derive(Debug)]
pub(crate) struct Markup {
	/// leptos's scoped class, `view! { class = STYLE, <div /> }`.
	pub global_class: Option<Range<usize>>,
	pub nodes: List<Node>,
}

/// Items with the comments and blank lines found between them.
#[derive(Debug)]
pub(crate) struct List<T> {
	pub items: Vec<Item<T>>,
	/// Comments after the last item.
	pub dangling: Vec<Leading>,
}

#[derive(Debug)]
pub(crate) struct Item<T> {
	pub value: T,
	/// Comments on the lines before the item.
	pub leading: Vec<Leading>,
	/// Comments on the same line after the item.
	pub trailing: Vec<Comment>,
	/// A blank line separates the item from what precedes it.
	pub blank_before: bool,
	/// Nothing at all separates the item from the one before it.
	pub joined: bool,
}

/// The comments in one gap between items.
#[derive(Default)]
struct Gap {
	trailing: Vec<Comment>,
	leading: Vec<Leading>,
	/// A blank line separates the last comment, or the item before, from the
	/// item after.
	blank_after: bool,
}

/// A comment on a line of its own.
#[derive(Debug)]
pub(crate) struct Leading {
	pub comment: Comment,
	pub blank_before: bool,
}

#[derive(Debug)]
pub(crate) enum Node {
	Element(Element),
	Fragment(List<Node>),
	/// A quoted string, copied as written.
	Text(Range<usize>),
	/// Unquoted text, copied as written.
	RawText(Range<usize>),
	/// `<!-- … -->`, copied as written.
	Comment(Range<usize>),
	/// `<!DOCTYPE …>`, copied as written.
	Doctype(Range<usize>),
	Block(Value),
}

#[derive(Debug)]
pub(crate) struct Element {
	/// The tag name, without whitespace: `div`, `For`, `svg:rect`.
	pub name: String,
	/// Generic arguments as written: `<T, U>`.
	pub generics: Option<String>,
	pub attributes: List<Attribute>,
	pub children: List<Node>,
	/// The element was written without a closing tag: `<a />`.
	pub self_closed: bool,
}

#[derive(Debug)]
pub(crate) enum Attribute {
	Keyed {
		/// `class`, `on:click`, `prop:value`, …
		key: String,
		value: AttributeValue,
	},
	/// A braced attribute: a spread such as `{..}` or `{..attrs}`.
	Block(Value),
}

#[derive(Debug)]
pub(crate) enum AttributeValue {
	/// A boolean attribute: `disabled`.
	None,
	/// Pattern bindings: `let(item)`, copied as written.
	Binding(Range<usize>),
	Value(Value),
}

/// A Rust value inside markup.
#[derive(Debug)]
pub(crate) enum Value {
	/// A literal written without braces, copied as written.
	Literal(Range<usize>),
	/// A single expression.
	Expr {
		/// The expression itself, without the braces around it.
		range: Range<usize>,
		/// Braces surround it in the source.
		braced: bool,
	},
	/// A braced block that has to stay a block: it has statements, or
	/// comments outside its expression.
	Block(Range<usize>),
	/// A spread, `{..}` or `{..name}`, copied without whitespace.
	Spread(String),
}

/// Why a macro's markup was left as written.
#[derive(Clone, Debug)]
pub(crate) struct MarkupError {
	/// Byte offset in the host text.
	pub offset: usize,
	pub message: String,
}

/// Parses the body of `call`, a macro call found in `host`.
pub(crate) fn parse(host: &str, call: &MacroCall) -> Result<Markup, MarkupError> {
	let body_text = &host[call.body.clone()];
	let offset = call.body.start;
	let tokens = TokenStream::from_str(body_text).map_err(|error| {
		MarkupError {
			offset,
			message: format!("the macro body is not valid Rust tokens: {error}"),
		}
	})?;
	let mut parser = Parser {
		host,
		offset,
		lines: LineIndex::new(body_text),
		covered: Vec::new(),
	};
	let (global_class, tokens, nodes_start) = parser.split_global_class(tokens);
	let config = ParserConfig::default()
		.recover_block(true)
		.transform_block(unparsed_block);
	let (nodes, diagnostics) = rstml::Parser::new(config)
		.parse_recoverable(tokens)
		.split_vec();
	if let Some(diagnostic) = diagnostics
		.into_iter()
		.find(|diagnostic| diagnostic.level() == proc_macro2_diagnostics::Level::Error)
	{
		let error: syn::Error = diagnostic.into();
		return Err(MarkupError {
			offset: parser.position(error.span()).start,
			message: error.to_string(),
		});
	}
	let nodes = parser.nodes(&nodes, nodes_start..call.body.end)?;
	parser.check_comments(call.body.clone())?;
	Ok(Markup {
		global_class,
		nodes,
	})
}

struct Parser<'a> {
	host: &'a str,
	/// Where the parsed text starts in the host.
	offset: usize,
	lines: LineIndex<'a>,
	/// Every range whose comments are accounted for: the gaps the layout
	/// keeps comments from, and the Rust it hands to rustfmt.
	covered: Vec<Range<usize>>,
}

impl Parser<'_> {
	/// The host range of a span from the parsed body.
	///
	/// Through line and column rather than `Span::byte_range`, which counts
	/// characters into a shared map on every call.
	fn position(&self, span: Span) -> Range<usize> {
		let start = self.lines.offset(span.start()) + self.offset;
		let end = self.lines.offset(span.end()) + self.offset;
		start..end
	}

	/// From the start of `first` to the end of `last`.
	fn between(&self, first: Span, last: Span) -> Range<usize> {
		self.position(first).start..self.position(last).end
	}

	fn error(&self, span: Span, message: impl Into<String>) -> MarkupError {
		MarkupError {
			offset: self.position(span).start,
			message: message.into(),
		}
	}

	/// Splits off leptos's `class = STYLE,` prefix, returning the class
	/// expression, the remaining tokens, and where the nodes start.
	fn split_global_class(
		&mut self,
		tokens: TokenStream,
	) -> (Option<Range<usize>>, TokenStream, usize) {
		let start = self.offset;
		let mut iter = tokens.clone().into_iter();
		let prefix: Vec<TokenTree> = iter.by_ref().take(4).collect();
		match prefix.as_slice() {
			[
				TokenTree::Ident(class),
				TokenTree::Punct(eq),
				value,
				TokenTree::Punct(comma),
			] if class == "class" && eq.as_char() == '=' && comma.as_char() == ',' => {
				let value = self.position(value.span());
				let after = self.position(comma.span()).end;
				self.covered.push(value.clone());
				(Some(value), iter.collect(), after)
			}
			_ => (None, tokens, start),
		}
	}

	fn nodes(
		&mut self,
		nodes: &[rstml::node::Node],
		region: Range<usize>,
	) -> Result<List<Node>, MarkupError> {
		let mut items = Vec::with_capacity(nodes.len());
		for node in nodes {
			let range = self.node_range(node)?;
			let value = self.node(node, range.clone())?;
			items.push((value, range));
		}
		self.list(items, region)
	}

	/// The source range of a node, from its first token to its last. Taken
	/// from the delimiting tokens: `Spanned::span` would re-serialize the
	/// whole subtree to find them, for every node at every depth.
	fn node_range(&self, node: &rstml::node::Node) -> Result<Range<usize>, MarkupError> {
		use rstml::node::Node as Rstml;
		Ok(match node {
			Rstml::Element(element) => {
				let open = &element.open_tag;
				let last = element
					.close_tag
					.as_ref()
					.map_or(open.end_tag.token_gt.span, |close| close.token_gt.span);
				self.between(open.token_lt.span, last)
			}
			Rstml::Fragment(fragment) => {
				let last = fragment
					.tag_close
					.as_ref()
					.map_or(fragment.tag_open.token_gt.span, |close| close.token_gt.span);
				self.between(fragment.tag_open.token_lt.span, last)
			}
			Rstml::Text(text) => self.position(text.value.span()),
			Rstml::RawText(raw) => {
				let span = raw.join_spans().ok_or_else(|| {
					self.error(Span::call_site(), "unquoted text has no source position")
				})?;
				self.position(span)
			}
			Rstml::Comment(comment) => {
				self.between(
					comment.token_start.token_lt.span,
					comment.token_end.token_gt.span,
				)
			}
			Rstml::Doctype(doctype) => {
				self.between(doctype.token_start.token_lt.span, doctype.token_end.span)
			}
			Rstml::Block(NodeBlock::ValidBlock(block)) => {
				self.position(block.brace_token.span.join())
			}
			Rstml::Block(NodeBlock::Invalid(invalid)) => self.position(invalid.brace.span.join()),
			Rstml::Custom(_) => unreachable!("rstml::Infallible custom nodes cannot be parsed"),
		})
	}

	/// The source range of an attribute, from its key or brace to the end of
	/// its value.
	fn attribute_range(&self, attribute: &NodeAttribute) -> Range<usize> {
		match attribute {
			NodeAttribute::Block(NodeBlock::ValidBlock(block)) => {
				self.position(block.brace_token.span.join())
			}
			NodeAttribute::Block(NodeBlock::Invalid(invalid)) => {
				self.position(invalid.brace.span.join())
			}
			NodeAttribute::Attribute(keyed) => {
				let key = self.position(keyed.key.span());
				let end = match &keyed.possible_value {
					KeyedAttributeValue::None => key.end,
					KeyedAttributeValue::Binding(binding) => {
						self.position(binding.paren.span.join()).end
					}
					KeyedAttributeValue::Value(value) => {
						match &value.value {
							KVAttributeValue::Expr(syn::Expr::Block(block)) => {
								self.position(block.block.brace_token.span.join()).end
							}
							KVAttributeValue::Expr(expr) => self.position(expr.span()).end,
							KVAttributeValue::InvalidBraced(invalid) => {
								self.position(invalid.brace.span.join()).end
							}
						}
					}
				};
				key.start..end
			}
		}
	}

	fn node(&mut self, node: &rstml::node::Node, range: Range<usize>) -> Result<Node, MarkupError> {
		use rstml::node::Node as Rstml;
		Ok(match node {
			Rstml::Element(element) => Node::Element(self.element(element)?),
			Rstml::Fragment(fragment) => {
				let start = self.position(fragment.tag_open.token_gt.span).end;
				let end = fragment.tag_close.as_ref().map_or(range.end, |close| {
					self.position(close.start_tag.token_lt.span).start
				});
				Node::Fragment(self.nodes(&fragment.children, start..end)?)
			}
			Rstml::Text(_) => {
				self.covered.push(range.clone());
				Node::Text(range)
			}
			Rstml::RawText(_) => {
				self.covered.push(range.clone());
				Node::RawText(range)
			}
			Rstml::Comment(_) => {
				self.covered.push(range.clone());
				Node::Comment(range)
			}
			Rstml::Doctype(_) => {
				self.covered.push(range.clone());
				Node::Doctype(range)
			}
			Rstml::Block(block) => Node::Block(self.block(block)?),
			Rstml::Custom(_) => unreachable!("rstml::Infallible custom nodes cannot be parsed"),
		})
	}

	fn element(
		&mut self,
		element: &rstml::node::NodeElement<rstml::Infallible>,
	) -> Result<Element, MarkupError> {
		let open = &element.open_tag;
		let name_range = self.position(open.name.span());
		let name = compact(&self.host[name_range.clone()]);
		let generics_range = match (&open.generics.lt_token, &open.generics.gt_token) {
			(Some(lt), Some(gt)) => Some(self.between(lt.span, gt.span)),
			_ => None,
		};
		let generics = generics_range
			.clone()
			.map(|range| collapse_whitespace(&self.host[range]));
		let attributes_start = generics_range.map_or(name_range.end, |range| range.end);
		let end_tag = &open.end_tag;
		let end_tag_start = end_tag
			.token_solidus
			.as_ref()
			.map_or(end_tag.token_gt.span, |solidus| solidus.span);
		let attributes_end = self.position(end_tag_start).start;
		let mut attributes = Vec::with_capacity(open.attributes.len());
		for attribute in &open.attributes {
			let range = self.attribute_range(attribute);
			attributes.push((self.attribute(attribute)?, range));
		}
		let attributes = self.list(attributes, attributes_start..attributes_end)?;
		let children_start = self.position(end_tag.token_gt.span).end;
		let children_end = element.close_tag.as_ref().map_or(children_start, |close| {
			self.position(close.start_tag.token_lt.span).start
		});
		let children = self.nodes(&element.children, children_start..children_end)?;
		Ok(Element {
			name,
			generics,
			attributes,
			children,
			self_closed: element.close_tag.is_none(),
		})
	}

	fn attribute(&mut self, attribute: &NodeAttribute) -> Result<Attribute, MarkupError> {
		match attribute {
			NodeAttribute::Block(block) => Ok(Attribute::Block(self.block(block)?)),
			NodeAttribute::Attribute(keyed) => {
				let key = compact(&self.host[self.position(keyed.key.span())]);
				let value = match &keyed.possible_value {
					KeyedAttributeValue::None => AttributeValue::None,
					KeyedAttributeValue::Binding(binding) => {
						let range = self.position(binding.paren.span.join());
						self.covered.push(range.clone());
						AttributeValue::Binding(range)
					}
					KeyedAttributeValue::Value(value) => {
						match &value.value {
							KVAttributeValue::Expr(expr) => AttributeValue::Value(self.value(expr)),
							KVAttributeValue::InvalidBraced(invalid) => {
								return Err(self.error(
									invalid.brace.span.join(),
									"the attribute value is not a valid block",
								));
							}
						}
					}
				};
				Ok(Attribute::Keyed { key, value })
			}
		}
	}

	fn block(&mut self, block: &NodeBlock) -> Result<Value, MarkupError> {
		match block {
			NodeBlock::ValidBlock(block) => Ok(self.braced(block)),
			NodeBlock::Invalid(invalid) => {
				Err(self.error(invalid.brace.span.join(), "the block is not valid Rust"))
			}
		}
	}

	fn value(&mut self, expr: &syn::Expr) -> Value {
		match expr {
			syn::Expr::Block(block) if block.attrs.is_empty() && block.label.is_none() => {
				self.braced(&block.block)
			}
			syn::Expr::Lit(_) => {
				let range = self.position(expr.span());
				self.covered.push(range.clone());
				Value::Literal(range)
			}
			_ => {
				let range = self.position(expr.span());
				self.covered.push(range.clone());
				Value::Expr {
					range,
					braced: false,
				}
			}
		}
	}

	/// A braced block: a single expression when that is all it holds, or a
	/// whole block otherwise.
	///
	/// rstml hands blocks over unparsed (see [`unparsed_block`]), so the
	/// block's tokens decide: a statement separator, an item or `let`, or a
	/// comment before or after the expression make it a block.
	fn braced(&mut self, block: &syn::Block) -> Value {
		let outer = self.position(block.brace_token.span.join());
		self.covered.push(outer.clone());
		let inner = outer.start + 1..outer.end - 1;
		let tokens: Vec<(Token, Range<usize>)> = Lexer::new(&self.host[inner.clone()]).collect();
		let (Some((first, first_range)), Some((last, last_range))) =
			(tokens.first(), tokens.last())
		else {
			return Value::Block(outer);
		};
		let mut depth = 0usize;
		let separated = tokens.iter().any(|(token, _)| {
			match token {
				Token::Open(_) => depth += 1,
				Token::Close(_) => depth = depth.saturating_sub(1),
				Token::Punct(b';') => return depth == 0,
				_ => {}
			}
			false
		});
		let first_text = &self.host[inner.start + first_range.start..inner.start + first_range.end];
		let statement = matches!(first, Token::Punct(b'#'))
			|| (*first == Token::Ident && STATEMENT_KEYWORDS.contains(&first_text));
		let commented = [first, last]
			.iter()
			.any(|token| matches!(token, Token::LineComment | Token::BlockComment));
		if separated || statement || commented {
			return Value::Block(outer);
		}
		let range = inner.start + first_range.start..inner.start + last_range.end;
		let text = &self.host[range.clone()];
		if is_spread(text) {
			return Value::Spread(format!("{{{}}}", compact(text)));
		}
		Value::Expr {
			range,
			braced: true,
		}
	}

	/// Attaches the comments and blank lines in `region`'s gaps to `items`.
	fn list<T>(
		&mut self,
		values: Vec<(T, Range<usize>)>,
		region: Range<usize>,
	) -> Result<List<T>, MarkupError> {
		let mut items: Vec<Item<T>> = Vec::with_capacity(values.len());
		let mut previous_end = region.start;
		let mut values = values.into_iter();
		loop {
			let next = values.next();
			let gap_end = next.as_ref().map_or(region.end, |(_, range)| range.start);
			let gap = self.gap(previous_end..gap_end, !items.is_empty())?;
			if let Some(previous) = items.last_mut() {
				previous.trailing = gap.trailing;
			}
			let Some((value, range)) = next else {
				return Ok(List {
					items,
					dangling: gap.leading,
				});
			};
			items.push(Item {
				value,
				leading: gap.leading,
				trailing: Vec::new(),
				blank_before: gap.blank_after,
				joined: previous_end == range.start,
			});
			previous_end = range.end;
		}
	}

	/// Splits the comments in a gap into those trailing the item before it
	/// (on the same line) and those leading the item after it.
	fn gap(&mut self, gap: Range<usize>, after_item: bool) -> Result<Gap, MarkupError> {
		self.check_gap(gap.clone())?;
		self.covered.push(gap.clone());
		let mut result = Gap::default();
		let mut cursor = gap.start;
		let mut same_line = after_item;
		for comment in comments_in(self.host, gap.clone()) {
			let between = &self.host[cursor..comment.range.start];
			same_line &= !between.contains('\n');
			cursor = comment.range.end;
			if same_line {
				result.trailing.push(comment);
			} else {
				result.leading.push(Leading {
					comment,
					blank_before: has_blank_line(between),
				});
			}
		}
		result.blank_after = has_blank_line(&self.host[cursor..gap.end]);
		Ok(result)
	}

	/// A gap between items holds only whitespace and comments; anything else
	/// means a construct this parser does not model.
	fn check_gap(&self, gap: Range<usize>) -> Result<(), MarkupError> {
		if gap.start > gap.end {
			return Err(MarkupError {
				offset: gap.end,
				message: "the markup's source positions overlap".to_owned(),
			});
		}
		match Lexer::new(&self.host[gap.clone()])
			.find(|(token, _)| !matches!(token, Token::LineComment | Token::BlockComment))
		{
			Some((_, range)) => {
				Err(MarkupError {
					offset: gap.start + range.start,
					message: "unexpected tokens between markup nodes".to_owned(),
				})
			}
			None => Ok(()),
		}
	}

	/// Fails when a comment sits somewhere the layout cannot keep it, such as
	/// inside a tag name, rather than dropping it.
	fn check_comments(&mut self, body: Range<usize>) -> Result<(), MarkupError> {
		self.covered.sort_by_key(|range| range.start);
		let lost = comments_in_text(self.host, body)
			.into_iter()
			.find(|comment| {
				!self.covered.iter().any(|range| {
					range.start <= comment.range.start && comment.range.end <= range.end
				})
			});
		match lost {
			Some(comment) => {
				Err(MarkupError {
					offset: comment.range.start,
					message: "a comment inside a tag can only go between attributes".to_owned(),
				})
			}
			None => Ok(()),
		}
	}
}

/// Every comment token in `range` of `host`, which may hold any Rust.
fn comments_in_text(host: &str, range: Range<usize>) -> Vec<Comment> {
	let offset = range.start;
	Lexer::new(&host[range])
		.filter_map(|(token, range)| {
			let block = match token {
				Token::LineComment => false,
				Token::BlockComment => true,
				_ => return None,
			};
			Some(Comment {
				range: range.start + offset..range.end + offset,
				block,
			})
		})
		.collect()
}

/// Converts proc-macro2's line and column positions to byte offsets.
struct LineIndex<'a> {
	text: &'a str,
	text_is_ascii: bool,
	/// The byte offset each line starts at.
	starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
	fn new(text: &'a str) -> Self {
		let mut starts = vec![0];
		starts.extend(
			text.bytes()
				.enumerate()
				.filter(|(_, byte)| *byte == b'\n')
				.map(|(index, _)| index + 1),
		);
		Self {
			text,
			text_is_ascii: text.is_ascii(),
			starts,
		}
	}

	/// The byte offset of a 1-based line and 0-based character column.
	fn offset(&self, position: proc_macro2::LineColumn) -> usize {
		let Some(&start) = self.starts.get(position.line.wrapping_sub(1)) else {
			return self.text.len();
		};
		if self.text_is_ascii {
			return (start + position.column).min(self.text.len());
		}
		self.text[start..]
			.char_indices()
			.nth(position.column)
			.map_or(self.text.len(), |(index, _)| start + index)
	}
}

/// Words that start a statement or item rather than an expression.
const STATEMENT_KEYWORDS: &[&str] = &[
	"let",
	"fn",
	"struct",
	"enum",
	"union",
	"impl",
	"trait",
	"type",
	"use",
	"mod",
	"static",
	"extern",
	"macro_rules",
];

/// `..` or `..name`: a spread of attributes.
fn is_spread(text: &str) -> bool {
	text.strip_prefix("..").is_some_and(|rest| {
		let rest = rest.trim_start();
		!rest.starts_with(['.', '='])
			&& rest
				.chars()
				.all(|character| character.is_alphanumeric() || matches!(character, '_' | ':'))
	})
}

/// Tells rstml to skip parsing block contents as Rust: the formatter only
/// needs the braces, and rustfmt parses the contents anyway. syn parsing
/// every block was most of the cost of reading markup.
fn unparsed_block(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<TokenStream>> {
	// One tree at a time: parsing a whole `TokenStream` leaves the stream on
	// a cursor rstml cannot advance to.
	while !input.is_empty() {
		input.parse::<TokenTree>()?;
	}
	Ok(Some(TokenStream::new()))
}

fn has_blank_line(whitespace: &str) -> bool {
	whitespace.bytes().filter(|byte| *byte == b'\n').count() >= 2
}

/// `text` without any whitespace: names like `on : click` become `on:click`.
fn compact(text: &str) -> String {
	text.split_whitespace().collect()
}

fn collapse_whitespace(text: &str) -> String {
	text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::lexer::find_macros;

	fn parse_view(src: &str) -> Result<Markup, MarkupError> {
		let call = find_macros(src, &["view".to_owned()]).remove(0);
		parse(src, &call)
	}

	#[test]
	fn keeps_comments_and_blank_lines_between_nodes() {
		let src = "view! {\n\t// lead\n\t<a />  // trail\n\n\t<b />\n\t// dangling\n}";
		let markup = parse_view(src).unwrap();
		let items = &markup.nodes.items;
		assert_eq!(items.len(), 2);
		assert_eq!(&src[items[0].leading[0].comment.range.clone()], "// lead");
		assert_eq!(&src[items[0].trailing[0].range.clone()], "// trail");
		assert!(items[1].blank_before);
		assert_eq!(
			&src[markup.nodes.dangling[0].comment.range.clone()],
			"// dangling"
		);
	}

	#[test]
	fn reads_values_and_spreads() {
		let src = r#"view! { <A b="lit" c={x} d={ let y = 1; y } e=f(g) {..} /> }"#;
		let markup = parse_view(src).unwrap();
		let Node::Element(element) = &markup.nodes.items[0].value else {
			panic!("element");
		};
		let values: Vec<_> = element
			.attributes
			.items
			.iter()
			.map(|item| {
				match &item.value {
					Attribute::Keyed {
						value: AttributeValue::Value(Value::Literal(range)),
						..
					} => format!("lit {}", &src[range.clone()]),
					Attribute::Keyed {
						value: AttributeValue::Value(Value::Expr { range, braced }),
						..
					} => format!("expr {} {braced}", &src[range.clone()]),
					Attribute::Keyed {
						value: AttributeValue::Value(Value::Block(range)),
						..
					} => format!("block {}", &src[range.clone()]),
					Attribute::Block(Value::Spread(spread)) => format!("spread {spread}"),
					other => format!("{other:?}"),
				}
			})
			.collect();
		assert_eq!(
			values,
			vec![
				"lit \"lit\"",
				"expr x true",
				"block { let y = 1; y }",
				"expr f(g) false",
				"spread {..}",
			]
		);
	}

	#[test]
	fn reads_the_global_class() {
		let src = "view! { class = STYLE, <div /> }";
		let markup = parse_view(src).unwrap();
		assert_eq!(&src[markup.global_class.unwrap()], "STYLE");
		assert_eq!(markup.nodes.items.len(), 1);
	}

	#[test]
	fn rejects_invalid_markup_and_unplaceable_comments() {
		let error = parse_view("view! { <div> }").unwrap_err();
		assert!(!error.message.is_empty());
		let error = parse_view("view! { <div class /* here */ = \"a\" /> }").unwrap_err();
		assert!(error.message.contains("comment"), "{}", error.message);
	}
}
