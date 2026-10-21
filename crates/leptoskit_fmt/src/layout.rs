//! The markup style: how parsed markup becomes a document for the printer.
//!
//! - A macro with one node stays on one line when it fits: `view! { <Counter />
//!   }`. Several nodes go on lines of their own.
//! - An element whose first child is text or a `{block}` keeps its children
//!   inline when they fit, and otherwise packs them onto as few lines as
//!   possible, like words in a paragraph.
//! - Any other element puts each child on a line of its own.
//! - One attribute never wraps. Several share the tag's line when they fit, and
//!   otherwise go one per line with the closing `>` or `/>` on its own.
//! - Comments stay where they were relative to their neighbours, and a run of
//!   blank lines between children becomes one.
//! - Text, string literals, and unquoted text are copied exactly; Rust
//!   expressions are formatted by rustfmt at the column they will sit in.
//!
//! Every Rust expression is formatted for the indentation its line will have
//! and the columns before it, so rustfmt's width decisions hold in place.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use leptoskit_config::AttributeBraces;
use leptoskit_config::ClosingTags;
use leptoskit_config::FormatConfig;

use crate::config::Layout;
use crate::doc::Builder;
use crate::doc::Doc;
use crate::doc::Line;
use crate::doc::broken_group;
use crate::doc::group;
use crate::doc::indent as indented;
use crate::doc::line;
use crate::expr::Formatted;
use crate::expr::Request;
use crate::expr::Resolve;
use crate::expr::Shape;
use crate::expr::body_shape;
use crate::expr::dedent;
use crate::expr::is_literal;
use crate::expr::is_trivial;
use crate::expr::leading_whitespace;
use crate::expr::mask_calls;
use crate::expr::unbraced_is_safe;
use crate::lexer::Comment;
use crate::lexer::Delimiter;
use crate::lexer::MacroCall;
use crate::lexer::find_macros;
use crate::markup;
use crate::markup::Attribute;
use crate::markup::AttributeValue;
use crate::markup::Element;
use crate::markup::Item;
use crate::markup::List;
use crate::markup::Markup;
use crate::markup::MarkupError;
use crate::markup::Node;
use crate::markup::Value;

/// A markup macro laid out inside a Rust expression. Its printed range is
/// tracked so the caller can tell whether its shape settled.
pub(crate) struct Nested {
	pub track: usize,
	/// The shape of the body rustfmt saw.
	pub shape: Shape,
	/// The bytes before the body in the printed call: `view! {`.
	pub head: usize,
}

/// Nested macros parsed so far, by where their call starts in the host:
/// each is parsed once a pass, however many times it is laid out.
pub(crate) type Parsed = HashMap<usize, Result<Rc<Markup>, MarkupError>>;

/// Lays out the markup macros of one source text.
pub(crate) struct Layouter<'a, 'r, R> {
	pub builder: Builder<'a>,
	config: &'a FormatConfig,
	/// The text the markup was parsed from.
	host: &'a str,
	resolver: &'r mut R,
	parsed: &'r mut Parsed,
	/// The width of one indentation level.
	unit: usize,
	pub nested: Vec<Nested>,
	/// Nested macros left as written, and why.
	pub failures: Vec<MarkupError>,
}

impl<'a, 'r, R: Resolve> Layouter<'a, 'r, R> {
	pub(crate) fn new(
		layout: &'a Layout,
		config: &'a FormatConfig,
		host: &'a str,
		resolver: &'r mut R,
		parsed: &'r mut Parsed,
	) -> Self {
		Self {
			builder: Builder::new(layout),
			config,
			host,
			resolver,
			parsed,
			unit: layout.tab_spaces,
			nested: Vec::new(),
			failures: Vec::new(),
		}
	}

	fn text(&self, text: impl Into<Box<str>>) -> Doc {
		self.builder.text(text)
	}

	fn width(&self, text: &str) -> usize {
		self.builder.layout.width(text)
	}

	/// The whole call, `view! { … }`, on a line indented `indent` columns.
	pub(crate) fn macro_doc(&mut self, call: &MacroCall, markup: &Markup, indent: usize) -> Doc {
		let head = match call.delimiter {
			Delimiter::Brace => format!("{}! {{", call.path),
			Delimiter::Paren => format!("{}!(", call.path),
			Delimiter::Bracket => format!("{}![", call.path),
		};
		let close = call.delimiter.close().to_string();
		let nodes = &markup.nodes;
		if markup.global_class.is_none() && nodes.items.is_empty() && nodes.dangling.is_empty() {
			return self.text(format!("{head}{close}"));
		}
		let separator = if call.delimiter == Delimiter::Brace {
			Line::Space
		} else {
			Line::Soft
		};
		let inner = indent + self.unit;
		let mut body = Vec::new();
		if let Some(class) = &markup.global_class {
			body.push(self.text(format!("class={},", compact(&self.host[class.clone()]))));
			if !nodes.items.is_empty() || !nodes.dangling.is_empty() {
				body.push(line(Line::Hard));
			}
		}
		let single = markup.global_class.is_none()
			&& nodes.items.len() == 1
			&& nodes.dangling.is_empty()
			&& !has_comments(&nodes.items[0]);
		body.push(self.stacked(nodes, inner));
		let body = Doc::Concat(body);
		if single {
			return group(Doc::Concat(vec![
				self.text(head),
				indented(Doc::Concat(vec![line(separator), body])),
				line(separator),
				self.text(close),
			]));
		}
		Doc::Concat(vec![
			self.text(head),
			indented(Doc::Concat(vec![line(Line::Hard), body])),
			line(Line::Hard),
			self.text(close),
		])
	}

	/// Nodes one per line, keeping single blank lines between them.
	fn stacked(&mut self, list: &List<Node>, indent: usize) -> Doc {
		let mut parts = Vec::new();
		for (index, item) in list.items.iter().enumerate() {
			if index > 0 {
				parts.push(line(Line::Hard));
			}
			let node = self.node(&item.value, indent);
			parts.push(self.with_comments(item, node, index == 0, true));
		}
		parts.push(self.dangling(list, false));
		Doc::Concat(parts)
	}

	/// The comments after the last item, each on its own line. The first
	/// starts a new line only when there are items before it.
	fn dangling<T>(&self, list: &List<T>, keep_blank_lines: bool) -> Doc {
		let mut parts = Vec::new();
		for (index, leading) in list.dangling.iter().enumerate() {
			let first = list.items.is_empty() && index == 0;
			if !first {
				parts.push(line(Line::Hard));
				if keep_blank_lines && leading.blank_before {
					parts.push(line(Line::Hard));
				}
			}
			parts.push(self.comment(&leading.comment));
		}
		Doc::Concat(parts)
	}

	/// `doc` with the item's leading comments above it and trailing comments
	/// after it.
	fn with_comments<T>(
		&self,
		item: &Item<T>,
		doc: Doc,
		first: bool,
		keep_blank_lines: bool,
	) -> Doc {
		let mut parts = Vec::new();
		let mut at_start = first;
		for leading in &item.leading {
			if keep_blank_lines && leading.blank_before && !at_start {
				parts.push(line(Line::Hard));
			}
			parts.push(self.comment(&leading.comment));
			parts.push(line(Line::Hard));
			at_start = false;
		}
		if keep_blank_lines && item.blank_before && !at_start {
			parts.push(line(Line::Hard));
		}
		parts.push(doc);
		for comment in &item.trailing {
			parts.push(self.text(" "));
			parts.push(self.comment(comment));
		}
		Doc::Concat(parts)
	}

	fn comment(&self, comment: &Comment) -> Doc {
		self.verbatim(&self.host[comment.range.clone()])
	}

	/// Text copied exactly: lines after the first are not re-indented, since
	/// they may be inside a string.
	fn verbatim(&self, text: &str) -> Doc {
		let mut parts = Vec::new();
		for (index, text_line) in text.split('\n').enumerate() {
			if index > 0 {
				parts.push(line(Line::Literal));
			}
			if !text_line.is_empty() {
				parts.push(self.text(text_line));
			}
		}
		Doc::Concat(parts)
	}

	fn node(&mut self, node: &Node, indent: usize) -> Doc {
		match node {
			Node::Element(element) => self.element(element, indent),
			Node::Fragment(children) => {
				let open = self.text("<>");
				self.children(open, children, "</>", indent)
			}
			Node::Text(range)
			| Node::RawText(range)
			| Node::Comment(range)
			| Node::Doctype(range) => self.verbatim(&self.host[range.clone()]),
			Node::Block(value) => self.braced(value, indent, 0),
		}
	}

	fn element(&mut self, element: &Element, indent: usize) -> Doc {
		let name = format!(
			"<{}{}",
			element.name,
			element.generics.as_deref().unwrap_or_default()
		);
		let children = &element.children;
		let childless = children.items.is_empty() && children.dangling.is_empty();
		let self_closing = childless
			&& match self.config.closing_tags {
				ClosingTags::SelfClosing => true,
				ClosingTags::Explicit => is_void(&element.name),
				ClosingTags::Preserve => element.self_closed,
			};
		let open = self.open_tag(&name, &element.attributes, self_closing, indent);
		if self_closing {
			return open;
		}
		self.children(open, children, &format!("</{}>", element.name), indent)
	}

	fn open_tag(
		&mut self,
		name: &str,
		attributes: &List<Attribute>,
		self_closing: bool,
		indent: usize,
	) -> Doc {
		let items = &attributes.items;
		let commented = !attributes.dangling.is_empty() || items.iter().any(has_comments);
		let end = if self_closing { " />" } else { ">" };
		match items.as_slice() {
			[] if !commented => self.text(format!("{name}{end}")),
			[only] if !commented => {
				let prefix = self.width(name) + 1;
				let attribute = self.attribute(&only.value, indent, prefix, false);
				Doc::Concat(vec![
					self.text(format!("{name} ")),
					attribute,
					self.text(end),
				])
			}
			_ => {
				let inner = indent + self.unit;
				let mut parts = Vec::new();
				let mut previous_line_comment = false;
				for (index, item) in items.iter().enumerate() {
					let before_block = matches!(items.get(index + 1), Some(next) if matches!(next.value, Attribute::Block(_)));
					parts.push(line(if previous_line_comment {
						Line::Hard
					} else {
						Line::Space
					}));
					let attribute = self.attribute(&item.value, inner, 0, before_block);
					parts.push(self.with_comments(item, attribute, index == 0, false));
					previous_line_comment = ends_with_line_comment(item);
				}
				if items.is_empty() {
					parts.push(line(Line::Hard));
				}
				parts.push(self.dangling(attributes, false));
				let closing = if self_closing {
					vec![line(Line::Space), self.text("/>")]
				} else {
					vec![line(Line::Soft), self.text(">")]
				};
				let mut doc = vec![self.text(name.to_owned()), indented(Doc::Concat(parts))];
				doc.extend(closing);
				if commented {
					broken_group(Doc::Concat(doc))
				} else {
					group(Doc::Concat(doc))
				}
			}
		}
	}

	/// The children of an element or fragment, after its opening tag.
	fn children(&mut self, open: Doc, list: &List<Node>, close: &str, indent: usize) -> Doc {
		let Some(first) = list.items.first() else {
			if list.dangling.is_empty() {
				return Doc::Concat(vec![open, self.text(close.to_owned())]);
			}
			let dangling = self.dangling(list, false);
			return Doc::Concat(vec![
				open,
				indented(Doc::Concat(vec![line(Line::Hard), dangling])),
				line(Line::Hard),
				self.text(close.to_owned()),
			]);
		};
		let inner = indent + self.unit;
		let textual = matches!(
			first.value,
			Node::Text(_) | Node::RawText(_) | Node::Block(_)
		);
		if !textual {
			let children = self.stacked(list, inner);
			return Doc::Concat(vec![
				open,
				indented(Doc::Concat(vec![line(Line::Hard), children])),
				line(Line::Hard),
				self.text(close.to_owned()),
			]);
		}
		let mut parts = Vec::new();
		for (index, item) in list.items.iter().enumerate() {
			if index > 0 {
				parts.push(fill_separator(&list.items[index - 1], item));
			}
			let node = self.node(&item.value, inner);
			// A group, so a child that must break counts as not fitting and
			// gets lines of its own.
			parts.push(group(self.with_comments(item, node, index == 0, true)));
		}
		let opening_line = if first.leading.is_empty() {
			Line::Soft
		} else {
			Line::Hard
		};
		let ends_in_comment = list.items.last().is_some_and(ends_with_line_comment);
		let closing_line = if ends_in_comment || !list.dangling.is_empty() {
			Line::Hard
		} else {
			Line::Soft
		};
		let inside = vec![
			line(opening_line),
			Doc::Fill(parts),
			self.dangling(list, true),
		];
		group(Doc::Concat(vec![
			open,
			indented(Doc::Concat(inside)),
			line(closing_line),
			self.text(close.to_owned()),
		]))
	}

	fn attribute(
		&mut self,
		attribute: &Attribute,
		indent: usize,
		prefix: usize,
		before_block: bool,
	) -> Doc {
		match attribute {
			Attribute::Block(value) => self.braced(value, indent, prefix),
			Attribute::Keyed { key, value } => {
				match value {
					AttributeValue::None => self.text(key.clone()),
					AttributeValue::Binding(range) => {
						self.text(format!(
							"{key}{}",
							collapse_whitespace(&self.host[range.clone()])
						))
					}
					AttributeValue::Value(value) => {
						let prefix = prefix + self.width(key) + 1;
						let value = self.attribute_value(value, indent, prefix, before_block);
						Doc::Concat(vec![self.text(format!("{key}=")), value])
					}
				}
			}
		}
	}

	fn attribute_value(
		&mut self,
		value: &Value,
		indent: usize,
		prefix: usize,
		before_block: bool,
	) -> Doc {
		let (range, braced) = match value {
			Value::Literal(range) => {
				let literal = self.verbatim(&self.host[range.clone()]);
				if self.config.attribute_braces == AttributeBraces::Always {
					return Doc::Concat(vec![self.text("{"), literal, self.text("}")]);
				}
				return literal;
			}
			Value::Block(_) | Value::Spread(_) => return self.braced(value, indent, prefix),
			Value::Expr { range, braced } => (range.clone(), *braced),
		};
		let source = dedent(&self.host[range.clone()]);
		let keep_braces = match self.config.attribute_braces {
			AttributeBraces::WhenRequired => !unbraced_is_safe(&source, before_block),
			AttributeBraces::Always => true,
			AttributeBraces::AlwaysUnlessLiteral => !is_literal(&source),
			AttributeBraces::Preserve => braced || !unbraced_is_safe(&source, before_block),
		};
		if keep_braces {
			return self.braced(value, indent, prefix);
		}
		self.expression(range, indent, prefix)
	}

	/// A value in braces: `{expr}` with no space inside, or a block.
	fn braced(&mut self, value: &Value, indent: usize, prefix: usize) -> Doc {
		match value {
			Value::Expr { range, .. } | Value::Literal(range) => {
				let expression = self.expression(range.clone(), indent, prefix + 1);
				Doc::Concat(vec![self.text("{"), expression, self.text("}")])
			}
			Value::Block(range) => self.expression(range.clone(), indent, prefix),
			Value::Spread(spread) => self.text(spread.clone()),
		}
	}

	/// A Rust expression, formatted by rustfmt, with the markup macros inside
	/// it laid out in place.
	fn expression(&mut self, range: Range<usize>, indent: usize, prefix: usize) -> Doc {
		let source = &self.host[range.clone()];
		let calls = find_macros(source, &self.config.macros);
		let text = dedent(&mask_calls(source, &calls));
		let formatted = if is_trivial(&text) {
			Arc::new(Formatted::new(text))
		} else {
			let request = Request {
				text,
				indent,
				prefix,
			};
			match self.resolver.resolve(request) {
				Some(formatted) => formatted,
				// Laid out again once rustfmt has answered.
				None => return Doc::Concat(Vec::new()),
			}
		};
		if formatted.unparsed {
			self.failures.push(MarkupError {
				offset: range.start,
				message: "rustfmt could not parse this Rust, so it was left as written".to_owned(),
			});
		}
		let found = find_macros(&formatted.text, &self.config.macros);
		if found.len() != calls.len() {
			self.failures.push(MarkupError {
				offset: range.start,
				message: "rustfmt changed the markup macros inside this expression".to_owned(),
			});
			return self.verbatim(source);
		}
		let mut parts = Vec::new();
		let mut cursor = 0;
		let mut line_index = 0;
		for (placed, original) in found.iter().zip(&calls) {
			self.formatted_lines(
				&mut parts,
				&formatted,
				cursor..placed.range.start,
				&mut line_index,
			);
			let line_start = formatted.text[..placed.range.start]
				.rfind('\n')
				.map_or(0, |index| index + 1);
			let offset = leading_whitespace(&formatted.text[line_start..]);
			let call = shift(original, range.start);
			let nested = self.nested_macro(&call, indent + self.width(offset));
			parts.push(Doc::IndentBy(offset.into(), Box::new(nested)));
			line_index += formatted.text[placed.range.clone()].matches('\n').count();
			cursor = placed.range.end;
		}
		self.formatted_lines(
			&mut parts,
			&formatted,
			cursor..formatted.text.len(),
			&mut line_index,
		);
		Doc::Concat(parts)
	}

	fn nested_macro(&mut self, call: &MacroCall, indent: usize) -> Doc {
		let host = self.host;
		let parsed = self
			.parsed
			.entry(call.range.start)
			.or_insert_with(|| markup::parse(host, call).map(Rc::new))
			.clone();
		match parsed {
			Ok(markup) => {
				let track = self.builder.track_id();
				self.nested.push(Nested {
					track,
					shape: body_shape(&self.host[call.body.clone()]),
					head: call.path.len() + 1 + usize::from(call.delimiter == Delimiter::Brace) + 1,
				});
				let doc = self.macro_doc(call, &markup, indent);
				Doc::Track(track, Box::new(doc))
			}
			Err(error) => {
				self.failures.push(error);
				self.verbatim(&self.host[call.range.clone()])
			}
		}
	}

	/// The lines of `formatted` in `range`: each line after the first starts
	/// at the current indentation, unless it begins inside a string.
	fn formatted_lines(
		&self,
		parts: &mut Vec<Doc>,
		formatted: &Formatted,
		range: Range<usize>,
		line_index: &mut usize,
	) {
		for (index, text_line) in formatted.text[range].split('\n').enumerate() {
			if index > 0 {
				*line_index += 1;
				let verbatim = formatted
					.verbatim
					.get(*line_index)
					.copied()
					.unwrap_or(false);
				parts.push(line(if verbatim { Line::Literal } else { Line::Hard }));
			}
			if !text_line.is_empty() {
				parts.push(self.text(text_line));
			}
		}
	}
}

/// What separates two children packed into a fill.
fn fill_separator(previous: &Item<Node>, item: &Item<Node>) -> Doc {
	if ends_with_line_comment(previous) || !item.leading.is_empty() {
		line(Line::Hard)
	} else if item.blank_before {
		Doc::Concat(vec![line(Line::Hard), line(Line::Hard)])
	} else if item.joined {
		line(Line::Soft)
	} else {
		line(Line::Space)
	}
}

fn shift(call: &MacroCall, offset: usize) -> MacroCall {
	MacroCall {
		range: call.range.start + offset..call.range.end + offset,
		path: call.path.clone(),
		delimiter: call.delimiter,
		body: call.body.start + offset..call.body.end + offset,
	}
}

fn has_comments<T>(item: &Item<T>) -> bool {
	!item.leading.is_empty() || !item.trailing.is_empty()
}

fn ends_with_line_comment<T>(item: &Item<T>) -> bool {
	item.trailing.last().is_some_and(|comment| !comment.block)
}

/// HTML elements that never have children, and so always self-close.
fn is_void(name: &str) -> bool {
	matches!(
		name,
		"area"
			| "base"
			| "br"
			| "col"
			| "embed"
			| "hr"
			| "img"
			| "input"
			| "link"
			| "meta"
			| "param"
			| "source"
			| "track"
			| "wbr"
	)
}

fn compact(text: &str) -> String {
	text.split_whitespace().collect()
}

fn collapse_whitespace(text: &str) -> String {
	text.split_whitespace().collect::<Vec<_>>().join(" ")
}
