//! Turning a parsed document into elements.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_native::TextSpan;
use leptoskit_native::components::Heading;
use leptoskit_native::components::Image;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::reactive::For;
use markdown::mdast;
use markdown::mdast::AttributeContent;
use markdown::mdast::AttributeValue;
use markdown::mdast::Node;

use crate::Diagnostic;
use crate::Inner;
use crate::Rendered;
use crate::class;
use crate::expr::Expr;
use crate::expr::Scope;
use crate::expr::display;
use crate::parse;
use crate::registry::Component;
use crate::registry::Context;
use crate::registry::PropKind;
use crate::registry::Props;
use crate::state::Declare;
use crate::value::Value;

/// The longest text a formula may show, in characters.
const MAX_SHOWN_TEXT: usize = 4096;

/// Where diagnostics and the node count go while a document first renders.
///
/// Parts of a document that follow variables render again later, inside
/// their region. Those renders find the sink closed: nobody reads their
/// diagnostics, and each one counts its own nodes against the limit, which
/// holds because nothing a variable changes can add more than the document
/// itself and the chart limit allow.
struct Sink {
	open: AtomicBool,
	diagnostics: Mutex<Vec<Diagnostic>>,
	nodes: AtomicUsize,
	exhausted: AtomicBool,
}

impl Sink {
	fn new() -> Arc<Self> {
		Arc::new(Self {
			open: AtomicBool::new(true),
			diagnostics: Mutex::new(Vec::new()),
			nodes: AtomicUsize::new(0),
			exhausted: AtomicBool::new(false),
		})
	}

	fn push(&self, diagnostic: Diagnostic) {
		if self.open.load(Ordering::Relaxed) {
			let mut diagnostics = self
				.diagnostics
				.lock()
				.unwrap_or_else(PoisonError::into_inner);
			if diagnostics.len() < 100 && !diagnostics.contains(&diagnostic) {
				diagnostics.push(diagnostic);
			}
		}
	}
}

/// One render of a document, or of one part of it.
struct Pass {
	ui: Arc<Inner>,
	sink: Arc<Sink>,
	depth: usize,
	/// Nodes this pass may still create, for a part rendering again after
	/// the document's first render.
	local_nodes: usize,
}

/// Renders `source`.
pub(crate) fn document(ui: &Arc<Inner>, source: &str) -> Rendered {
	let sink = Sink::new();
	let mut pass = Pass {
		ui: Arc::clone(ui),
		sink: Arc::clone(&sink),
		depth: 0,
		local_nodes: ui.limits.max_nodes,
	};

	let (children, partial) = if source.len() > ui.limits.max_source_bytes {
		let message = format!(
			"the document is {} bytes; the limit is {}",
			source.len(),
			ui.limits.max_source_bytes
		);
		(vec![pass.failure(None, message)], false)
	} else {
		match parse::parse(source) {
			Ok(parsed) => {
				let nodes = parsed.root.children().cloned().unwrap_or_default();
				pass.declare(&nodes);
				(pass.blocks(&nodes), parsed.partial)
			}
			Err(diagnostic) => {
				let message = diagnostic.message.clone();
				sink.push(diagnostic);
				let shown = pass.error_element(&message);
				(vec![shown], false)
			}
		}
	};

	sink.open.store(false, Ordering::Relaxed);
	let diagnostics = std::mem::take(
		&mut *sink
			.diagnostics
			.lock()
			.unwrap_or_else(PoisonError::into_inner),
	);

	Rendered {
		element: View()
			.class("flex-col gap-3 min-w-0 overflow-hidden")
			.children(children),
		diagnostics,
		partial,
	}
}

fn position(node: Option<&Node>) -> (usize, usize) {
	node.and_then(Node::position).map_or((0, 0), |position| {
		(position.start.line, position.start.column)
	})
}

/// A component tag's parts, whichever of flow or text it was written as.
struct Tag<'a> {
	name: Option<&'a str>,
	attributes: &'a [AttributeContent],
	children: &'a [Node],
}

fn as_tag(node: &Node) -> Option<Tag<'_>> {
	match node {
		Node::MdxJsxFlowElement(element) => {
			Some(Tag {
				name: element.name.as_deref(),
				attributes: &element.attributes,
				children: &element.children,
			})
		}
		Node::MdxJsxTextElement(element) => {
			Some(Tag {
				name: element.name.as_deref(),
				attributes: &element.attributes,
				children: &element.children,
			})
		}
		_ => None,
	}
}

/// An attribute as written: a fixed value, a formula, or a variable's name.
#[derive(Clone)]
enum Attribute {
	Fixed(Value),
	Formula(Arc<Expr>),
}

impl Attribute {
	fn is_constant(&self) -> bool {
		match self {
			Self::Fixed(_) => true,
			Self::Formula(expr) => expr.is_constant(),
		}
	}
}

/// A component's attributes and data children, ready to evaluate whenever
/// the variables they read change.
#[derive(Clone, Default)]
struct Plan {
	attributes: Vec<(String, Attribute)>,
	data: Vec<(String, Vec<(String, Attribute)>)>,
	/// Variable props whose variable's value the component shows, so it
	/// renders again when the variable changes.
	followed: Vec<String>,
	class: Option<String>,
}

impl Plan {
	fn is_constant(&self) -> bool {
		self.followed.is_empty()
			&& self
				.attributes
				.iter()
				.chain(self.data.iter().flat_map(|(_, attributes)| attributes))
				.all(|(_, attribute)| attribute.is_constant())
	}

	fn evaluate(&self, ui: &Inner) -> Result<Props, String> {
		let evaluate = |attributes: &[(String, Attribute)]| {
			attributes
				.iter()
				.map(|(name, attribute)| {
					let value = match attribute {
						Attribute::Fixed(value) => value.clone(),
						Attribute::Formula(expr) => {
							expr.eval(&ui.store, &mut ui.budget())
								.map_err(|error| format!("`{name}`: {error}"))?
						}
					};
					Ok((name.clone(), value))
				})
				.collect::<Result<_, String>>()
		};

		let mut values: std::collections::BTreeMap<String, Value> = evaluate(&self.attributes)?;
		for name in &self.followed {
			let variable = values.get(name).map(display).unwrap_or_default();
			let current = ui
				.store
				.variable(&variable, &mut ui.budget())
				.ok()
				.flatten()
				.unwrap_or_default();
			values.insert(format!("{name}.value"), current);
		}

		let data = self
			.data
			.iter()
			.map(|(tag, attributes)| {
				Ok((
					tag.clone(),
					Props {
						values: evaluate(attributes)?,
						data: Vec::new(),
					},
				))
			})
			.collect::<Result<_, String>>()?;

		Ok(Props { values, data })
	}
}

impl Pass {
	fn note(&self, node: Option<&Node>, message: impl Into<String>) {
		let (line, column) = position(node);
		self.sink.push(Diagnostic {
			line,
			column,
			message: message.into(),
		});
	}

	/// A note, and the element shown in place of what failed.
	fn failure(&self, node: Option<&Node>, message: impl Into<String>) -> Element {
		let message = message.into();
		self.note(node, message.clone());
		self.error_element(&message)
	}

	fn error_element(&self, message: &str) -> Element {
		if self.ui.show_errors {
			View()
				.class("rounded-md px-3 py-2")
				.class(self.ui.theme.class("bg", "surface"))
				.child(
					Text(format!("⚠ {message}"))
						.class("text-xs")
						.class(self.ui.theme.class("text", "negative")),
				)
		} else {
			View().class("hidden")
		}
	}

	/// Claims one node, failing once the document's limit is used up.
	fn claim(&mut self, node: &Node) -> bool {
		self.claim_many(node, 1)
	}

	fn claim_many(&mut self, node: &Node, count: usize) -> bool {
		let limit = self.ui.limits.max_nodes;
		if self.sink.open.load(Ordering::Relaxed) {
			let used = self.sink.nodes.fetch_add(count, Ordering::Relaxed) + count;
			if used <= limit {
				return true;
			}

			if !self.sink.exhausted.swap(true, Ordering::Relaxed) {
				self.note(
					Some(node),
					format!("the document is limited to {limit} elements; the rest is left out"),
				);
			}
			return false;
		}

		match self.local_nodes.checked_sub(count) {
			Some(left) => {
				self.local_nodes = left;
				true
			}
			None => false,
		}
	}

	fn nodes_left(&self) -> usize {
		if self.sink.open.load(Ordering::Relaxed) {
			self.ui
				.limits
				.max_nodes
				.saturating_sub(self.sink.nodes.load(Ordering::Relaxed))
		} else {
			self.local_nodes
		}
	}

	// --- Declarations -----------------------------------------------------

	/// Declares every `<Var>` and `<Let>` before anything renders, so content
	/// can show a variable declared further down.
	fn declare(&self, nodes: &[Node]) {
		for node in nodes {
			if let Some(tag) = as_tag(node) {
				match tag.name {
					Some("Var") => self.declare_one(node, &tag, false),
					Some("Let") => self.declare_one(node, &tag, true),
					_ => {}
				}
			}

			if let Some(children) = node.children() {
				self.declare(children);
			}
		}
	}

	fn declare_one(&self, node: &Node, tag: &Tag<'_>, formula: bool) {
		let element = if formula { "Let" } else { "Var" };
		let mut name = None;
		let mut value = None;

		for attribute in tag.attributes {
			let AttributeContent::Property(property) = attribute else {
				self.note(
					Some(node),
					format!("<{element}> does not take `{{…}}` spreads"),
				);
				continue;
			};

			match (property.name.as_str(), &property.value) {
				("name", Some(AttributeValue::Literal(text))) => name = Some(text.clone()),
				("name", _) => {
					self.note(
						Some(node),
						format!("<{element}> needs its name as text: name=\"rent\""),
					);
				}
				("value", Some(AttributeValue::Literal(text))) => {
					value = Some(Expr::Literal(Value::Text(text.clone())));
				}
				("value", Some(AttributeValue::Expression(expression))) => {
					match Expr::parse(&expression.value) {
						Ok(expr) => value = Some(expr),
						Err(error) => {
							self.note(Some(node), format!("<{element}> `value`: {error}"));
						}
					}
				}
				("value", None) => value = Some(Expr::Literal(Value::Bool(true))),
				(other, _) => {
					self.note(
						Some(node),
						format!("<{element}> takes only `name` and `value`, not `{other}`"),
					);
				}
			}
		}

		let Some(name) = name else {
			self.note(Some(node), format!("<{element}> needs a `name`"));
			return;
		};

		if !is_identifier(&name) {
			self.note(
				Some(node),
				format!(
					"`{name}` cannot be a variable name: use letters, digits, and underscores, \
					 starting with a letter"
				),
			);
			return;
		}

		let value = value.unwrap_or(Expr::Literal(Value::Null));
		let limit = self.ui.limits.max_variables;
		let outcome = if formula {
			self.ui.store.declare_formula(&name, value, limit)
		} else {
			match value.eval(&self.ui.store, &mut self.ui.budget()) {
				Ok(initial) if initial.weight() > self.ui.limits.max_value_bytes => {
					Declare::Refused(format!(
						"`{name}` starts too large; a value is limited to {} bytes",
						self.ui.limits.max_value_bytes
					))
				}
				Ok(initial) => self.ui.store.declare_input(&name, initial, limit),
				Err(error) => Declare::Refused(format!("<Var name=\"{name}\"> `value`: {error}")),
			}
		};

		if let Declare::Refused(message) = outcome {
			self.note(Some(node), message);
		}
	}

	// --- Blocks -----------------------------------------------------------

	/// Renders block content. Inline content among it, as inside a component
	/// written on one line, becomes a paragraph.
	fn blocks(&mut self, nodes: &[Node]) -> Vec<Element> {
		let mut elements = Vec::new();
		let mut run: Vec<Node> = Vec::new();

		for node in nodes {
			if is_inline(node) {
				run.push(node.clone());
				continue;
			}

			self.flush(&mut run, &mut elements);
			elements.extend(self.block(node));
		}

		self.flush(&mut run, &mut elements);
		elements
	}

	fn flush(&mut self, run: &mut Vec<Node>, elements: &mut Vec<Element>) {
		let blank = run
			.iter()
			.all(|node| matches!(node, Node::Text(text) if text.value.trim().is_empty()));
		let inline = std::mem::take(run);

		if !blank
			&& let Some(first) = inline.first()
			&& self.claim(first)
		{
			elements.push(self.paragraph(&inline));
		}
	}

	fn block(&mut self, node: &Node) -> Option<Element> {
		if let Some(tag) = as_tag(node) {
			return self.component(node, &tag);
		}

		if !self.claim(node) {
			return None;
		}

		match node {
			Node::Paragraph(paragraph) => Some(self.paragraph(&paragraph.children)),
			Node::Heading(heading) => Some(self.heading(heading)),
			Node::List(list) => Some(self.list(list)),
			Node::Blockquote(quote) => Some(self.quote(&quote.children)),
			Node::Code(code) => {
				Some(
					View()
						.class("rounded-md px-3 py-2 min-w-0")
						.class(self.ui.theme.class("bg", "surface"))
						.child(Text(code.value.clone()).class("text-sm")),
				)
			}
			Node::ThematicBreak(_) => {
				Some(
					View()
						.class("h-px w-full")
						.class(self.ui.theme.class("bg", "line")),
				)
			}
			Node::Table(table) => Some(self.table(table)),
			Node::MdxFlowExpression(expression) => {
				let expression = Node::MdxTextExpression(mdast::MdxTextExpression {
					value: expression.value.clone(),
					position: expression.position.clone(),
					stops: Vec::new(),
				});
				Some(self.paragraph(&[expression]))
			}
			Node::Html(_) => {
				self.note(Some(node), "HTML is not supported; use components");
				None
			}
			_ => None,
		}
	}

	fn heading(&mut self, heading: &mdast::Heading) -> Element {
		let class = match heading.depth {
			1 => "text-2xl font-bold",
			2 => "text-xl font-semibold",
			3 => "text-lg font-semibold",
			_ => "text-base font-semibold",
		};
		let depth = heading.depth;
		self.text(&heading.children, move |spans| {
			Heading(depth, String::new()).spans(spans).class(class)
		})
	}

	fn list(&mut self, list: &mdast::List) -> Element {
		let mut number = list.start.unwrap_or(1);
		let items = list
			.children
			.iter()
			.filter_map(|item| {
				let Node::ListItem(item) = item else {
					return None;
				};
				let marker = match (item.checked, list.ordered) {
					(Some(true), _) => "☑".to_owned(),
					(Some(false), _) => "☐".to_owned(),
					(None, true) => {
						number += 1;
						format!("{}.", number - 1)
					}
					(None, false) => "•".to_owned(),
				};
				let content = self.blocks(&item.children);
				Some(
					View()
						.class("flex-row gap-2 items-start min-w-0")
						.child(Text(marker).class("min-w-4"))
						.child(
							View()
								.class("flex-col gap-1 flex-1 min-w-0")
								.children(content),
						),
				)
			})
			.collect::<Vec<_>>();

		View().class("flex-col gap-1 min-w-0").children(items)
	}

	fn quote(&mut self, children: &[Node]) -> Element {
		let content = self.blocks(children);
		View()
			.class("flex-row gap-3 min-w-0")
			.child(
				View()
					.class("w-[3px] rounded-full")
					.class(self.ui.theme.class("bg", "line")),
			)
			.child(
				View()
					.class("flex-col gap-2 flex-1 min-w-0")
					.children(content),
			)
	}

	fn table(&mut self, table: &mdast::Table) -> Element {
		let mut rows = Vec::new();

		for (index, row) in table.children.iter().enumerate() {
			let Node::TableRow(row) = row else {
				continue;
			};

			if index > 0 {
				rows.push(
					View()
						.class("h-px w-full")
						.class(self.ui.theme.class("bg", "line")),
				);
			}

			let header = index == 0;
			let cells = row
				.children
				.iter()
				.enumerate()
				.filter_map(|(column, cell)| {
					let Node::TableCell(cell) = cell else {
						return None;
					};
					let end = matches!(table.align.get(column), Some(mdast::AlignKind::Right));
					let class = match (header, end) {
						(true, true) => "text-sm font-semibold text-end",
						(true, false) => "text-sm font-semibold",
						(false, true) => "text-sm text-end",
						(false, false) => "text-sm",
					};
					let content = self.text(&cell.children, move |spans| {
						Text(String::new()).spans(spans).class(class)
					});
					Some(View().class("flex-1 min-w-0 px-2 py-1.5").child(content))
				})
				.collect::<Vec<_>>();
			rows.push(View().class("flex-row min-w-0").children(cells));
		}

		View().class("flex-col min-w-0").children(rows)
	}

	// --- Inline content -----------------------------------------------------

	/// A paragraph: one text, unless components sit inside it, which makes it
	/// a wrapping row of text and components.
	fn paragraph(&mut self, children: &[Node]) -> Element {
		if let [Node::Image(image)] = children {
			return self.image(image);
		}

		if !children.iter().any(|child| as_tag(child).is_some()) {
			return self.text(children, |spans| Text(String::new()).spans(spans));
		}

		let mut parts = Vec::new();
		let mut run: Vec<Node> = Vec::new();
		for child in children {
			if as_tag(child).is_some() {
				if !run.is_empty() {
					let text = std::mem::take(&mut run);
					parts.push(self.text(&text, |spans| Text(String::new()).spans(spans)));
				}
				if let Some(element) = self.block(child) {
					parts.push(element);
				}
			} else {
				run.push(child.clone());
			}
		}
		if !run.is_empty() {
			parts.push(self.text(&run, |spans| Text(String::new()).spans(spans)));
		}

		View()
			.class("flex-row flex-wrap items-center gap-x-1 gap-y-1 min-w-0")
			.children(parts)
	}

	fn image(&mut self, image: &mdast::Image) -> Element {
		let allowed = self
			.ui
			.images
			.as_ref()
			.is_some_and(|allow| allow(&image.url));

		if allowed {
			return Image(image.url.clone())
				.aria_label(image.alt.clone())
				.class("w-full rounded-lg");
		}

		self.note(
			None,
			format!(
				"the image {:?} is not shown: this application does not allow images from that \
				 address",
				image.url
			),
		);
		Text(if image.alt.is_empty() {
			"[image]".to_owned()
		} else {
			format!("[{}]", image.alt)
		})
		.class("text-sm")
		.class(self.ui.theme.class("text", "muted"))
	}

	/// Text with its formulas evaluated. When a formula reads variables, the
	/// text sits in a region that rebuilds it when they change.
	fn text(
		&mut self,
		children: &[Node],
		build: impl Fn(Vec<TextSpan>) -> Element + Send + Sync + 'static,
	) -> Element {
		let mut sources = Vec::new();
		collect_expressions(children, &mut sources);

		let formulas: Vec<Result<Arc<Expr>, String>> = sources
			.iter()
			.map(|(source, node)| {
				Expr::parse(source).map(Arc::new).map_err(|error| {
					self.note(Some(node), format!("`{{{source}}}`: {error}"));
					error.to_string()
				})
			})
			.collect();

		let first = evaluate_all(&self.ui, &formulas);
		for (result, (source, node)) in first.iter().zip(&sources) {
			if let Err(error) = result {
				self.note(Some(node), format!("`{{{source}}}`: {error}"));
			}
		}

		let constant = formulas
			.iter()
			.all(|formula| formula.as_ref().map_or(true, |expr| expr.is_constant()));
		let ui = Arc::clone(&self.ui);
		let nodes: Arc<[Node]> = children.into();

		if constant {
			return build(spans(&ui, &nodes, first));
		}

		let key_ui = Arc::clone(&ui);
		For(
			move || vec![evaluate_all(&key_ui, &formulas)],
			// One key for every value, so the rebuild updates the text widget
			// in place rather than replacing it.
			|_| (),
			move |values| build(spans(&ui, &nodes, values)),
		)
		.class("min-w-0")
	}

	// --- Components ---------------------------------------------------------

	fn component(&mut self, node: &Node, tag: &Tag<'_>) -> Option<Element> {
		let Some(name) = tag.name else {
			// A fragment, `<>…</>`, is just its children.
			let children = self.blocks(tag.children);
			return Some(View().class("flex-col gap-3 min-w-0").children(children));
		};

		if matches!(name, "Var" | "Let") {
			return None;
		}

		if !self.claim(node) {
			return None;
		}

		if self.depth >= self.ui.limits.max_depth {
			return Some(self.failure(
				Some(node),
				format!(
					"components nest more than {} deep",
					self.ui.limits.max_depth
				),
			));
		}

		let Some(component) = self.ui.components.get(name).cloned() else {
			let known = self
				.ui
				.components
				.keys()
				.map(String::as_str)
				.collect::<Vec<_>>()
				.join(", ");
			self.note(
				Some(node),
				format!("there is no component <{name}>; the components are {known}"),
			);
			let children = self.blocks(tag.children);
			return Some(View().class("flex-col gap-3 min-w-0").children(children));
		};

		let plan = match self.plan(node, tag, &component) {
			Ok(plan) => plan,
			Err(error) => return Some(self.failure(Some(node), error)),
		};
		let content: Vec<Node> = tag
			.children
			.iter()
			.filter(|child| {
				!as_tag(child)
					.and_then(|child| child.name)
					.is_some_and(|child| component.data_children.iter().any(|data| data == child))
			})
			.cloned()
			.collect();

		if component.inert_content
			&& let Some(live) = self.first_live(&content)
		{
			return Some(self.failure(
				Some(node),
				format!(
					"<{name}> cannot hold <{live}>: its content is pressed as one, so it holds \
					 only text, figures, and layout"
				),
			));
		}

		if component.children.is_none()
			&& content
				.iter()
				.any(|child| !matches!(child, Node::Text(text) if text.value.trim().is_empty()))
		{
			self.note(
				Some(node),
				format!("<{name}> takes no content between its tags"),
			);
		}

		let text = content
			.iter()
			.map(ToString::to_string)
			.collect::<Vec<_>>()
			.join(" ");

		if plan.is_constant() {
			let props = match plan.evaluate(&self.ui) {
				Ok(props) => props,
				Err(error) => return Some(self.failure(Some(node), format!("<{name}> {error}"))),
			};
			return Some(self.invoke(
				node,
				&component,
				&props,
				&content,
				&text,
				plan.class.as_deref(),
			));
		}

		// The props read variables: render in a region keyed by their values,
		// so the component renders again when, and only when, they change.
		if let Err(error) = plan.evaluate(&self.ui) {
			self.note(Some(node), format!("<{name}> {error}"));
		}

		let ui = Arc::clone(&self.ui);
		let sink = Arc::clone(&self.sink);
		let depth = self.depth;
		let node = node.clone();
		let key_ui = Arc::clone(&ui);
		let key_plan = plan.clone();
		Some(
			For(
				move || vec![key_plan.evaluate(&key_ui)],
				// The rebuild reuses the widgets in place, so a native slider
				// keeps its drag while its value follows the variable.
				|_| (),
				move |props| {
					let mut pass = Pass {
						ui: Arc::clone(&ui),
						sink: Arc::clone(&sink),
						depth,
						local_nodes: ui.limits.max_nodes,
					};
					match props {
						Ok(props) => {
							pass.invoke(
								&node,
								&component,
								&props,
								&content,
								&text,
								plan.class.as_deref(),
							)
						}
						Err(error) => {
							pass.failure(Some(&node), format!("<{}> {error}", component.name))
						}
					}
				},
			)
			.class("min-w-0"),
		)
	}

	/// Reads a component's attributes against its props.
	fn plan(&self, node: &Node, tag: &Tag<'_>, component: &Component) -> Result<Plan, String> {
		let name = &component.name;
		let mut plan = Plan::default();

		for attribute in tag.attributes {
			let AttributeContent::Property(property) = attribute else {
				self.note(
					Some(node),
					format!("<{name}> does not take `{{…}}` spreads"),
				);
				continue;
			};
			let prop = property.name.as_str();

			if prop == "class" {
				match &property.value {
					Some(AttributeValue::Literal(classes)) => {
						match class::check(classes) {
							Ok(()) => plan.class = Some(self.ui.theme.expand(classes)),
							Err(error) => self.note(Some(node), error),
						}
					}
					// Classes that follow variables, such as
					// `{left < 0 ? "text-negative" : "text-positive"}`.
					Some(AttributeValue::Expression(expression)) => {
						match Expr::parse(&expression.value) {
							Ok(expr) => {
								plan.attributes
									.push(("class".to_owned(), Attribute::Formula(Arc::new(expr))));
							}
							Err(error) => {
								self.note(Some(node), format!("<{name}> `class`: {error}"));
							}
						}
					}
					None => {
						self.note(
							Some(node),
							format!("<{name}> `class` needs classes, such as class=\"p-4\""),
						);
					}
				}
				continue;
			}

			let Some(spec) = component.prop_spec(prop) else {
				let props = component
					.props
					.iter()
					.map(|prop| format!("`{}`", prop.name))
					.collect::<Vec<_>>()
					.join(", ");
				self.note(
					Some(node),
					format!("<{name}> has no prop `{prop}`; its props are {props}"),
				);
				continue;
			};

			let value = match (&property.value, spec.kind) {
				(None, _) => Attribute::Fixed(Value::Bool(true)),
				(Some(AttributeValue::Literal(text)), _) => {
					Attribute::Fixed(Value::Text(text.clone()))
				}
				(Some(AttributeValue::Expression(expression)), kind) => {
					match Expr::parse(&expression.value) {
						Ok(expr) if kind == PropKind::Variable => {
							if let Some(variable) = expr.as_variable() {
								Attribute::Fixed(Value::Text(variable.to_owned()))
							} else {
								self.note(
									Some(node),
									format!("<{name}> `{prop}` names a variable: {prop}=\"rent\""),
								);
								continue;
							}
						}
						Ok(expr) => Attribute::Formula(Arc::new(expr)),
						Err(error) => {
							self.note(Some(node), format!("<{name}> `{prop}`: {error}"));
							continue;
						}
					}
				}
			};

			if spec.kind == PropKind::Variable && spec.follows {
				plan.followed.push(prop.to_owned());
			}
			plan.attributes.push((prop.to_owned(), value));
		}

		if let Some(missing) = component.props.iter().find(|spec| {
			spec.required && !plan.attributes.iter().any(|(name, _)| *name == spec.name)
		}) {
			return Err(format!("<{name}> needs `{}`", missing.name));
		}

		for child in tag.children {
			let Some(child_tag) = as_tag(child) else {
				continue;
			};
			let Some(child_name) = child_tag.name else {
				continue;
			};
			if !component
				.data_children
				.iter()
				.any(|data| data == child_name)
			{
				continue;
			}

			let spec = self.ui.components.get(child_name);
			let mut attributes = Vec::new();
			for attribute in child_tag.attributes {
				let AttributeContent::Property(property) = attribute else {
					continue;
				};
				if spec.is_some_and(|spec| spec.prop_spec(&property.name).is_none()) {
					self.note(
						Some(child),
						format!("<{child_name}> has no prop `{}`", property.name),
					);
					continue;
				}
				let value = match &property.value {
					None => Attribute::Fixed(Value::Bool(true)),
					Some(AttributeValue::Literal(text)) => {
						Attribute::Fixed(Value::Text(text.clone()))
					}
					Some(AttributeValue::Expression(expression)) => {
						match Expr::parse(&expression.value) {
							Ok(expr) => Attribute::Formula(Arc::new(expr)),
							Err(error) => {
								self.note(
									Some(child),
									format!("<{child_name}> `{}`: {error}", property.name),
								);
								continue;
							}
						}
					}
				};
				attributes.push((property.name.clone(), value));
			}
			plan.data.push((child_name.to_owned(), attributes));
		}

		Ok(plan)
	}

	/// The first component in `nodes`, at any depth, that is not inert.
	fn first_live(&self, nodes: &[Node]) -> Option<String> {
		for node in nodes {
			if let Some(tag) = as_tag(node) {
				if let Some(name) = tag.name
					&& !self
						.ui
						.components
						.get(name)
						.is_some_and(|component| component.inert)
				{
					return Some(name.to_owned());
				}
				if let Some(live) = self.first_live(tag.children) {
					return Some(live);
				}
			} else if let Some(children) = node.children()
				&& let Some(live) = self.first_live(children)
			{
				return Some(live);
			}
		}
		None
	}

	fn invoke(
		&mut self,
		node: &Node,
		component: &Component,
		props: &Props,
		content: &[Node],
		text: &str,
		class: Option<&str>,
	) -> Element {
		self.depth += 1;
		let children = if component.children.is_some() {
			self.blocks(content)
		} else {
			Vec::new()
		};
		self.depth -= 1;

		let before = self.nodes_left();
		let mut cx = Context {
			ui: Arc::clone(&self.ui),
			children,
			text: text.trim().to_owned(),
			nodes_left: before,
		};

		let element = match (component.render)(props, &mut cx) {
			Ok(element) => element,
			Err(error) => return self.failure(Some(node), format!("<{}> {error}", component.name)),
		};

		let used = before.saturating_sub(cx.nodes_left);
		if used > 0 && !self.claim_many(node, used) {
			return View().class("hidden");
		}

		// A class formula, checked once it has a value.
		let dynamic;
		let class = match (class, props.values.get("class")) {
			(Some(classes), _) => Some(classes),
			(None, Some(value)) => {
				let classes = display(value);
				if let Err(error) = class::check(&classes) {
					return self.failure(Some(node), format!("<{}> {error}", component.name));
				}
				dynamic = self.ui.theme.expand(&classes);
				Some(dynamic.as_str())
			}
			(None, None) => None,
		};

		match class {
			Some(classes) => {
				match element.try_class(classes) {
					Ok(element) => element,
					Err(error) => {
						self.failure(Some(node), format!("<{}> `class`: {error}", component.name))
					}
				}
			}
			None => element,
		}
	}
}

fn is_inline(node: &Node) -> bool {
	matches!(
		node,
		Node::Text(_)
			| Node::Strong(_)
			| Node::Emphasis(_)
			| Node::Delete(_)
			| Node::InlineCode(_)
			| Node::Link(_)
			| Node::Break(_)
			| Node::Image(_)
			| Node::MdxTextExpression(_)
			| Node::MdxJsxTextElement(_)
	)
}

fn is_identifier(name: &str) -> bool {
	let mut characters = name.chars();
	characters
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
		&& characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
		&& !matches!(name, "true" | "false" | "null")
}

fn collect_expressions(nodes: &[Node], out: &mut Vec<(String, Node)>) {
	for node in nodes {
		match node {
			Node::MdxTextExpression(expression) => {
				out.push((expression.value.clone(), node.clone()));
			}
			other => {
				if as_tag(other).is_none()
					&& let Some(children) = other.children()
				{
					collect_expressions(children, out);
				}
			}
		}
	}
}

fn evaluate_all(ui: &Inner, formulas: &[Result<Arc<Expr>, String>]) -> Vec<Result<Value, String>> {
	formulas
		.iter()
		.map(|formula| {
			match formula {
				Ok(expr) => {
					expr.eval(&ui.store, &mut ui.budget())
						.map_err(|error| error.to_string())
				}
				Err(error) => Err(error.clone()),
			}
		})
		.collect()
}

/// How a run of inline text is styled, from the Markdown around it.
#[derive(Clone, Copy, Default)]
#[allow(
	clippy::struct_excessive_bools,
	reason = "each flag is one independent Markdown emphasis"
)]
struct Style {
	bold: bool,
	italic: bool,
	code: bool,
	link: bool,
}

/// The styled runs of `nodes`, with each formula replaced by its value, in
/// order.
fn spans(ui: &Inner, nodes: &[Node], values: Vec<Result<Value, String>>) -> Vec<TextSpan> {
	let mut out = Vec::new();
	let mut values = values.into_iter();
	push_spans(ui, nodes, Style::default(), &mut values, &mut out);
	out
}

fn push_spans(
	ui: &Inner,
	nodes: &[Node],
	style: Style,
	values: &mut impl Iterator<Item = Result<Value, String>>,
	out: &mut Vec<TextSpan>,
) {
	for node in nodes {
		match node {
			Node::Text(text) => out.push(styled(ui, &text.value, style)),
			Node::InlineCode(code) => {
				out.push(styled(
					ui,
					&code.value,
					Style {
						code: true,
						..style
					},
				));
			}
			Node::Break(_) => out.push(styled(ui, "\n", style)),
			Node::Strong(strong) => {
				push_spans(
					ui,
					&strong.children,
					Style {
						bold: true,
						..style
					},
					values,
					out,
				);
			}
			Node::Emphasis(emphasis) => {
				push_spans(
					ui,
					&emphasis.children,
					Style {
						italic: true,
						..style
					},
					values,
					out,
				);
			}
			Node::Link(link) => {
				push_spans(
					ui,
					&link.children,
					Style {
						link: true,
						..style
					},
					values,
					out,
				);
			}
			Node::Image(image) => out.push(styled(ui, &format!("[{}]", image.alt), style)),
			Node::MdxTextExpression(_) => {
				let shown = match values.next() {
					Some(Ok(value)) => {
						let mut text = display(&value);
						if text.chars().count() > MAX_SHOWN_TEXT {
							text = text.chars().take(MAX_SHOWN_TEXT).collect::<String>() + "…";
						}
						styled(ui, &text, style)
					}
					Some(Err(_)) | None if ui.show_errors => {
						TextSpan::new("⚠").color(ui.theme.resolve("negative"))
					}
					Some(Err(_)) | None => styled(ui, "—", style),
				};
				out.push(shown);
			}
			other => {
				if let Some(children) = other.children() {
					push_spans(ui, children, style, values, out);
				}
			}
		}
	}
}

fn styled(ui: &Inner, text: &str, style: Style) -> TextSpan {
	let mut span = TextSpan::new(text);
	if style.bold {
		span = span.bold();
	}
	if style.italic {
		span = span.italic();
	}
	if style.code {
		span = span.background(ui.theme.resolve("surface"));
	}
	if style.link {
		span = span.color(ui.theme.resolve("accent"));
	}
	span
}
