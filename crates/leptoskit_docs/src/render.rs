use std::cell::RefCell;
use std::collections::HashMap;

use leptos::html;
use leptos::prelude::*;
use leptos::tachys::html::attribute::any_attribute::AnyAttribute;
use leptos::tachys::html::attribute::any_attribute::IntoAnyAttribute;
use leptos::tachys::html::attribute::custom::custom_attribute;
use leptos::tachys::view::add_attr::AddAnyAttr;
use markdown::mdast;
use markdown::mdast::AlignKind;
use markdown::mdast::AttributeContent;
use markdown::mdast::AttributeValue;
use markdown::mdast::Node;

use crate::Components;
use crate::Diagnostic;
use crate::Document;
use crate::Props;
use crate::components::CalloutKind;
use crate::components::callout;
use crate::highlight;

/// A rendered document: the view, and what a site needs to know about it.
pub struct Rendered {
	/// The document as a leptos view.
	pub view: AnyView,
	/// The text of the first `#` heading, the page's title.
	pub title: Option<String>,
	/// The text of the first paragraph, a summary for search engines and
	/// link previews.
	pub summary: Option<String>,
	/// Every heading, in order, with the `id` its element carries.
	pub headings: Vec<Heading>,
	/// The document's text split at its headings, for a search index.
	pub sections: Vec<Section>,
	/// Every link and image target, as written in the source.
	pub links: Vec<Link>,
	footnotes: Vec<String>,
}

impl Rendered {
	/// Every `id` a link can point at with `#`: headings and footnotes.
	pub fn anchors(&self) -> impl Iterator<Item = &str> {
		self.headings
			.iter()
			.map(|heading| heading.id.as_str())
			.chain(self.footnotes.iter().map(String::as_str))
	}
}

/// A heading in a [`Rendered`] document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
	/// 1 for `#`, up to 6 for `######`.
	pub depth: u8,
	/// The element's `id`, unique in the document.
	pub id: String,
	/// The heading as plain text.
	pub text: String,
}

/// The text under one heading, or before the first heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
	/// The heading's `id`, or `None` for text before the first heading.
	pub id: Option<String>,
	/// The heading as plain text.
	pub heading: Option<String>,
	/// The section's prose, with code blocks left out and whitespace
	/// collapsed.
	pub text: String,
}

/// A link or image target in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
	/// The target as written, before any rewriting.
	pub url: String,
	/// The 1-indexed line of the link.
	pub line: usize,
	/// The 1-indexed column of the link.
	pub column: usize,
}

/// Renders documents with a set of components.
///
/// ```
/// use leptoskit_docs::Components;
/// use leptoskit_docs::Document;
/// use leptoskit_docs::Renderer;
///
/// let components = Components::new();
/// let document = Document::parse("Read [the guide](guide.md#setup).").unwrap();
/// let rendered = Renderer::new(&components)
/// 	.rewrite_urls(|url| url.replace(".md", ".html"))
/// 	.render(&document)
/// 	.unwrap();
///
/// assert_eq!(rendered.links[0].url, "guide.md#setup");
/// ```
pub struct Renderer<'a> {
	components: &'a Components,
	rewrite_url: Box<dyn Fn(&str) -> String + 'a>,
}

impl<'a> Renderer<'a> {
	/// A renderer that leaves URLs as they are written.
	pub fn new(components: &'a Components) -> Self {
		Self {
			components,
			rewrite_url: Box::new(str::to_owned),
		}
	}

	/// Passes every link and image target through `rewrite` before it is
	/// rendered, such as to point links at `.md` files to built pages.
	#[must_use]
	pub fn rewrite_urls(mut self, rewrite: impl Fn(&str) -> String + 'a) -> Self {
		self.rewrite_url = Box::new(rewrite);
		self
	}

	/// Renders `document`, or lists everything wrong with it: unknown
	/// components, invalid or unknown props, and footnotes without a
	/// definition.
	pub fn render(&self, document: &Document) -> Result<Rendered, Vec<Diagnostic>> {
		let mut walker = Walker {
			components: self.components,
			links: LinkLog {
				rewrite: &*self.rewrite_url,
				links: RefCell::new(Vec::new()),
			},
			definitions: HashMap::new(),
			footnote_definitions: HashMap::new(),
			footnotes: Vec::new(),
			slugs: HashMap::new(),
			title: None,
			summary: None,
			headings: Vec::new(),
			sections: Vec::new(),
			quiet: 0,
			diagnostics: Vec::new(),
		};
		walker.collect_definitions(&document.root);

		let body = walker.children(&document.root, false);
		let footnotes = walker.footnotes_section();
		let view = view! {
			{body}
			{footnotes}
		}
		.into_any();

		if !walker.diagnostics.is_empty() {
			return Err(walker.diagnostics);
		}

		for section in &mut walker.sections {
			section.text = section
				.text
				.split_whitespace()
				.collect::<Vec<_>>()
				.join(" ");
		}

		Ok(Rendered {
			view,
			title: walker.title,
			summary: walker.summary,
			headings: walker.headings,
			sections: walker.sections,
			links: walker.links.links.into_inner(),
			footnotes: walker
				.footnotes
				.into_iter()
				.map(|id| footnote_id(&id))
				.collect(),
		})
	}
}

/// Renders `document` with `components`, leaving URLs as written.
pub fn render(document: &Document, components: &Components) -> Result<Rendered, Vec<Diagnostic>> {
	Renderer::new(components).render(document)
}

/// Parses and renders `source` straight to an HTML string.
pub fn render_to_html(source: &str, components: &Components) -> Result<String, Vec<Diagnostic>> {
	let document = Document::parse(source).map_err(|diagnostic| vec![diagnostic])?;
	Ok(to_static_html(render(&document, components)?.view))
}

/// Renders `view` to HTML for a static page, which is never hydrated.
///
/// leptos's server renderer separates adjacent text and marks empty slots
/// with `<!>` comments so the browser can hydrate the page later. A static
/// page has nothing to hydrate, so they are dropped. Text and attribute
/// values are escaped, so the sequence cannot occur anywhere else.
pub(crate) fn to_static_html(view: impl RenderHtml) -> String {
	view.to_html().replace("<!>", "")
}

/// Rewrites and records link targets, shared with the [`Props`] of every
/// component so links in props are checked like links in prose.
pub(crate) struct LinkLog<'a> {
	rewrite: &'a dyn Fn(&str) -> String,
	links: RefCell<Vec<Link>>,
}

impl LinkLog<'_> {
	pub(crate) fn record(&self, url: &str, line: usize, column: usize) -> String {
		self.links.borrow_mut().push(Link {
			url: url.to_owned(),
			line,
			column,
		});
		(self.rewrite)(url)
	}
}

struct Walker<'d, 'r> {
	components: &'r Components,
	links: LinkLog<'r>,
	definitions: HashMap<&'d str, &'d mdast::Definition>,
	footnote_definitions: HashMap<&'d str, &'d mdast::FootnoteDefinition>,
	/// Footnote identifiers in the order they are first referenced.
	footnotes: Vec<String>,
	slugs: HashMap<String, usize>,
	title: Option<String>,
	summary: Option<String>,
	headings: Vec<Heading>,
	sections: Vec<Section>,
	/// Above zero while rendering text that is not section prose, such as a
	/// heading's own text.
	quiet: usize,
	diagnostics: Vec<Diagnostic>,
}

impl<'d> Walker<'d, '_> {
	fn collect_definitions(&mut self, node: &'d Node) {
		match node {
			Node::Definition(definition) => {
				self.definitions.insert(&definition.identifier, definition);
			}
			Node::FootnoteDefinition(definition) => {
				self.footnote_definitions
					.insert(&definition.identifier, definition);
			}
			_ => {}
		}

		for child in node.children().into_iter().flatten() {
			self.collect_definitions(child);
		}
	}

	fn children(&mut self, node: &'d Node, tight: bool) -> AnyView {
		self.nodes(node.children().map_or(&[][..], Vec::as_slice), tight)
	}

	fn nodes(&mut self, nodes: &'d [Node], tight: bool) -> AnyView {
		nodes
			.iter()
			// Definitions render where they are used, not where they are written.
			.filter(|node| !matches!(node, Node::Definition(_) | Node::FootnoteDefinition(_)))
			.map(|node| self.node(node, tight))
			.collect::<Vec<_>>()
			.into_any()
	}

	/// Renders one node. `tight` is set for the children of a list item in a
	/// tight list, whose paragraphs render without `<p>`.
	#[allow(clippy::too_many_lines)]
	fn node(&mut self, node: &'d Node, tight: bool) -> AnyView {
		let at = position(node);

		match node {
			Node::Root(_) => self.children(node, false),
			Node::Paragraph(_) => {
				if self.summary.is_none() && self.quiet == 0 {
					self.summary = Some(node.to_string());
				}

				let children = self.children(node, false);
				self.prose(" ");
				if tight || only_elements(node) {
					children
				} else {
					html::p().child(children).into_any()
				}
			}
			Node::Heading(heading) => self.heading(heading),
			Node::Blockquote(quote) => self.blockquote(quote),
			Node::List(list) => {
				let tight = !list.spread
					&& list
						.children
						.iter()
						.all(|item| !matches!(item, Node::ListItem(item) if item.spread));
				let items = list
					.children
					.iter()
					.map(|item| self.node(item, tight))
					.collect::<Vec<_>>();

				match (list.ordered, list.start) {
					(true, Some(start)) if start != 1 => {
						html::ol().start(start.to_string()).child(items).into_any()
					}
					(true, _) => html::ol().child(items).into_any(),
					(false, _) => html::ul().child(items).into_any(),
				}
			}
			Node::ListItem(item) => {
				let children = self.children(node, tight);
				self.prose(" ");
				match item.checked {
					Some(checked) => {
						view! {
							<li class="lk-task">
								<input type="checkbox" disabled=true checked=checked />
								{children}
							</li>
						}
						.into_any()
					}
					None => html::li().child(children).into_any(),
				}
			}
			Node::Code(code) => self.code(code, at),
			Node::Table(table) => self.table(table),
			Node::ThematicBreak(_) => html::hr().into_any(),
			Node::Text(text) => {
				self.prose(&text.value);
				text.value.clone().into_any()
			}
			Node::InlineCode(code) => {
				self.prose(&code.value);
				html::code().child(code.value.clone()).into_any()
			}
			Node::Emphasis(_) => html::em().child(self.children(node, false)).into_any(),
			Node::Strong(_) => html::strong().child(self.children(node, false)).into_any(),
			Node::Delete(_) => html::del().child(self.children(node, false)).into_any(),
			Node::Break(_) => html::br().into_any(),
			Node::Link(link) => self.link(&link.url, link.title.as_deref(), node, at),
			Node::LinkReference(reference) => {
				match self.definitions.get(reference.identifier.as_str()) {
					Some(definition) => {
						self.link(&definition.url, definition.title.as_deref(), node, at)
					}
					None => self.children(node, false),
				}
			}
			Node::Image(image) => self.image(&image.url, &image.alt, image.title.as_deref(), at),
			Node::ImageReference(reference) => {
				match self.definitions.get(reference.identifier.as_str()) {
					Some(definition) => {
						self.image(
							&definition.url,
							&reference.alt,
							definition.title.as_deref(),
							at,
						)
					}
					None => reference.alt.clone().into_any(),
				}
			}
			Node::FootnoteReference(reference) => self.footnote_reference(&reference.identifier),
			Node::MdxJsxFlowElement(element) => {
				self.element(
					element.name.as_deref(),
					&element.attributes,
					&element.children,
					at,
				)
			}
			Node::MdxJsxTextElement(element) => {
				self.element(
					element.name.as_deref(),
					&element.attributes,
					&element.children,
					at,
				)
			}
			// The parse options turn every other construct off.
			_ => {
				self.diagnostics
					.push(Diagnostic::new(at.0, at.1, "unsupported Markdown"));
				().into_any()
			}
		}
	}

	fn prose(&mut self, text: &str) {
		if self.quiet > 0 {
			return;
		}

		if self.sections.is_empty() {
			self.sections.push(Section {
				id: None,
				heading: None,
				text: String::new(),
			});
		}

		let section = self.sections.last_mut().expect("a section was just pushed");
		section.text.push_str(text);
	}

	fn heading(&mut self, heading: &'d mdast::Heading) -> AnyView {
		let text = Node::Heading(heading.clone()).to_string();
		let id = self.slug(&text);

		if heading.depth == 1 && self.title.is_none() {
			self.title = Some(text.clone());
		}

		self.headings.push(Heading {
			depth: heading.depth,
			id: id.clone(),
			text: text.clone(),
		});
		self.sections.push(Section {
			id: Some(id.clone()),
			heading: Some(text),
			text: String::new(),
		});

		self.quiet += 1;
		let children = self.nodes(&heading.children, false);
		self.quiet -= 1;

		let anchor = view! {
			<a class="lk-anchor" href=format!("#{id}") aria-label="Link to this section">"#"</a>
		};

		match heading.depth {
			1 => html::h1().id(id).child((children, anchor)).into_any(),
			2 => html::h2().id(id).child((children, anchor)).into_any(),
			3 => html::h3().id(id).child((children, anchor)).into_any(),
			4 => html::h4().id(id).child((children, anchor)).into_any(),
			5 => html::h5().id(id).child((children, anchor)).into_any(),
			_ => html::h6().id(id).child((children, anchor)).into_any(),
		}
	}

	/// A GitHub-style slug, made unique in the document with a numbered
	/// suffix.
	fn slug(&mut self, text: &str) -> String {
		let mut slug: String = text
			.trim()
			.chars()
			.flat_map(char::to_lowercase)
			.filter_map(|character| {
				if character.is_alphanumeric() || matches!(character, '-' | '_') {
					Some(character)
				} else if character.is_whitespace() {
					Some('-')
				} else {
					None
				}
			})
			.collect();

		if slug.is_empty() {
			slug.push_str("section");
		}

		let seen = self.slugs.entry(slug.clone()).or_insert(0);
		*seen += 1;
		if *seen > 1 {
			slug = format!("{slug}-{}", *seen - 1);
		}

		slug
	}

	/// A blockquote, or a callout when it opens with a GitHub alert marker
	/// such as `[!TIP]`.
	fn blockquote(&mut self, quote: &'d mdast::Blockquote) -> AnyView {
		let alert = match quote.children.first() {
			Some(Node::Paragraph(paragraph)) => {
				match paragraph.children.first() {
					Some(Node::Text(text)) => alert(&text.value),
					_ => None,
				}
			}
			_ => None,
		};

		let Some((kind, rest)) = alert else {
			return html::blockquote()
				.child(self.nodes(&quote.children, false))
				.into_any();
		};

		let Some(Node::Paragraph(first)) = quote.children.first() else {
			unreachable!("an alert starts with a paragraph");
		};
		let rest = rest.trim_start().to_owned();
		self.prose(&rest);
		let mut lead = Vec::new();
		if !rest.is_empty() {
			lead.push(rest.into_any());
		}
		lead.extend(
			first.children[1..]
				.iter()
				.map(|child| self.node(child, false)),
		);
		self.prose(" ");

		let lead = (!lead.is_empty()).then(|| html::p().child(lead));
		let body = (lead, self.nodes(&quote.children[1..], false)).into_any();
		callout(kind, None, body)
	}

	fn code(&mut self, code: &mdast::Code, at: (usize, usize)) -> AnyView {
		let language = code.lang.as_deref().and_then(highlight::language);
		let title = code.meta.as_deref().and_then(code_title).map(|title| {
			view! { <figcaption class="lk-code-title">{title.to_owned()}</figcaption> }
		});

		let highlighted = match language.map(|language| highlight::highlight(&code.value, language))
		{
			Some(Ok(html)) => html,
			Some(Err(message)) => {
				self.diagnostics.push(Diagnostic::new(at.0, at.1, message));
				None
			}
			None => None,
		};

		let body = match highlighted {
			Some(html) => html::code().inner_html(html).into_any(),
			None => html::code().child(code.value.clone()).into_any(),
		};

		view! {
			<figure class="lk-code" data-lang=language.unwrap_or("text").to_owned()>
				{title} <pre>{body}</pre>
			</figure>
		}
		.into_any()
	}

	fn table(&mut self, table: &'d mdast::Table) -> AnyView {
		let mut rows = table.children.iter();
		let head = rows.next().map(|row| self.row(row, &table.align, true));
		let body = rows
			.map(|row| self.row(row, &table.align, false))
			.collect::<Vec<_>>();

		view! {
			<div class="lk-table">
				<table>
					<thead>{head}</thead>
					<tbody>{body}</tbody>
				</table>
			</div>
		}
		.into_any()
	}

	fn row(&mut self, row: &'d Node, align: &[AlignKind], header: bool) -> AnyView {
		let cells = row
			.children()
			.into_iter()
			.flatten()
			.enumerate()
			.map(|(column, cell)| {
				let style = match align.get(column) {
					Some(AlignKind::Left) => Some("text-align: left"),
					Some(AlignKind::Right) => Some("text-align: right"),
					Some(AlignKind::Center) => Some("text-align: center"),
					Some(AlignKind::None) | None => None,
				};
				let children = self.children(cell, false);
				self.prose(" ");

				if header {
					html::th().style(style).child(children).into_any()
				} else {
					html::td().style(style).child(children).into_any()
				}
			})
			.collect::<Vec<_>>();

		html::tr().child(cells).into_any()
	}

	fn link(
		&mut self,
		url: &str,
		title: Option<&str>,
		node: &'d Node,
		at: (usize, usize),
	) -> AnyView {
		let href = self.links.record(url, at.0, at.1);
		let children = self.children(node, false);
		html::a()
			.href(href)
			.title(title.map(str::to_owned))
			.child(children)
			.into_any()
	}

	fn image(&mut self, url: &str, alt: &str, title: Option<&str>, at: (usize, usize)) -> AnyView {
		let src = self.links.record(url, at.0, at.1);
		html::img()
			.src(src)
			.alt(alt.to_owned())
			.title(title.map(str::to_owned))
			.loading("lazy")
			.into_any()
	}

	/// GFM only parses a reference to a footnote that is defined, so every
	/// reference has a definition.
	fn footnote_reference(&mut self, identifier: &str) -> AnyView {
		// Only the first reference carries the id the footnote links back to.
		let (number, back) =
			if let Some(index) = self.footnotes.iter().position(|seen| seen == identifier) {
				(index + 1, None)
			} else {
				self.footnotes.push(identifier.to_owned());
				(
					self.footnotes.len(),
					Some(format!("{}-ref", footnote_id(identifier))),
				)
			};

		view! {
			<sup class="lk-footnote-ref">
				<a href=format!("#{}", footnote_id(identifier)) id=back>{number}</a>
			</sup>
		}
		.into_any()
	}

	/// The footnotes, in the order they were first referenced.
	fn footnotes_section(&mut self) -> Option<AnyView> {
		if self.footnotes.is_empty() {
			return None;
		}

		let mut items = Vec::new();
		let mut index = 0;
		// Definitions can reference further footnotes, which extends the list.
		while let Some(identifier) = self.footnotes.get(index).cloned() {
			let definition = self.footnote_definitions[identifier.as_str()];
			let id = footnote_id(&identifier);
			let content = self.nodes(&definition.children, false);
			items.push(view! {
				<li id=id.clone()>
					{content}
					<a
						class="lk-footnote-back"
						href=format!("#{id}-ref")
						aria-label="Back to the text"
					>
						"↩"
					</a>
				</li>
			});
			index += 1;
		}

		Some(
			view! {
				<section class="lk-footnotes">
					<ol>{items}</ol>
				</section>
			}
			.into_any(),
		)
	}

	fn element(
		&mut self,
		name: Option<&str>,
		attributes: &[AttributeContent],
		children: &'d [Node],
		at: (usize, usize),
	) -> AnyView {
		match name {
			None => self.nodes(children, false),
			Some(name) if name.starts_with(|first: char| first.is_ascii_uppercase()) => {
				self.component(name, attributes, children, at)
			}
			Some(name) => self.html_element(name, attributes, children, at),
		}
	}

	fn component(
		&mut self,
		name: &str,
		attributes: &[AttributeContent],
		children: &'d [Node],
		at: (usize, usize),
	) -> AnyView {
		let Some(component) = self.components.get(name) else {
			let known = self.components.names().collect::<Vec<_>>().join(", ");
			self.diagnostics.push(Diagnostic::new(
				at.0,
				at.1,
				format!("unknown component <{name}>; the known components are {known}"),
			));
			return ().into_any();
		};

		let body = self.nodes(children, false);
		let props = match Props::new(name, attributes, &self.links, at) {
			Ok(props) => props,
			Err(error) => {
				self.diagnostics
					.push(Diagnostic::new(at.0, at.1, error.to_string()));
				return ().into_any();
			}
		};

		let view = component(&props, Box::new(move || body));
		for unread in props.unread() {
			self.diagnostics.push(Diagnostic::new(
				at.0,
				at.1,
				format!("<{name}> has no `{unread}` attribute"),
			));
		}

		view.unwrap_or_else(|error| {
			self.diagnostics
				.push(Diagnostic::new(at.0, at.1, error.to_string()));
			().into_any()
		})
	}

	fn html_element(
		&mut self,
		name: &str,
		attributes: &[AttributeContent],
		children: &'d [Node],
		at: (usize, usize),
	) -> AnyView {
		let mut attrs = Vec::new();
		for attribute in attributes {
			let AttributeContent::Property(property) = attribute else {
				self.diagnostics.push(Diagnostic::new(
					at.0,
					at.1,
					format!("<{name}> cannot take spread attributes"),
				));
				continue;
			};

			let value = match &property.value {
				None => String::new(),
				Some(AttributeValue::Literal(value)) => value.clone(),
				Some(AttributeValue::Expression(_)) => {
					self.diagnostics.push(Diagnostic::new(
						at.0,
						at.1,
						format!(
							"HTML attributes take text: write `{}=\"…\"` on <{name}> instead of \
							 braces",
							property.name
						),
					));
					continue;
				}
			};
			let value = if matches!(property.name.as_str(), "href" | "src") {
				self.links.record(&value, at.0, at.1)
			} else {
				value
			};

			attrs.push(custom_attribute(property.name.clone(), value).into_any_attr());
		}

		if let Some(void) = void_element(name, attrs.clone()) {
			if !children.is_empty() {
				self.diagnostics.push(Diagnostic::new(
					at.0,
					at.1,
					format!("<{name}> cannot have children; close it with `<{name} />`"),
				));
			}
			return void;
		}

		let children = self.nodes(children, false);
		// An element built at runtime needs to own its tag name.
		html::custom(String::from(name))
			.add_any_attr(attrs)
			.child(children)
			.into_any()
	}
}

/// Whether a paragraph holds nothing but tags, such as a component written
/// on the same line as its text, or `<figcaption>` inside `<figure>`. MDX
/// would wrap those in `<p>`, which is invalid around block elements, so they
/// render bare.
fn only_elements(paragraph: &Node) -> bool {
	let children = paragraph.children().map_or(&[][..], Vec::as_slice);
	children
		.iter()
		.any(|child| matches!(child, Node::MdxJsxTextElement(element) if element.name.is_some()))
		&& children.iter().all(|child| {
			match child {
				Node::MdxJsxTextElement(element) => element.name.is_some(),
				Node::Text(text) => text.value.trim().is_empty(),
				_ => false,
			}
		})
}

/// The void elements, which have no closing tag in HTML.
fn void_element(name: &str, attributes: Vec<AnyAttribute>) -> Option<AnyView> {
	macro_rules! void {
		($($tag:ident)*) => {
			match name {
				$(stringify!($tag) => Some(html::$tag().add_any_attr(attributes).into_any()),)*
				_ => None,
			}
		};
	}

	void!(area base br col embed hr img input link meta source track wbr)
}

/// The kind and the rest of the text when `text` opens with a GitHub alert
/// marker such as `[!NOTE]`.
fn alert(text: &str) -> Option<(CalloutKind, &str)> {
	let marker = text.strip_prefix("[!")?;
	let (kind, rest) = marker.split_once(']')?;
	let kind = kind.parse().ok()?;
	(rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some((kind, rest))
}

/// The `title="…"` of a fence's info string, as in
/// ```` ```toml title="leptoskit.toml" ````.
fn code_title(meta: &str) -> Option<&str> {
	let (_, rest) = meta.split_once("title=\"")?;
	rest.split_once('"').map(|(title, _)| title)
}

fn footnote_id(identifier: &str) -> String {
	let slug: String = identifier
		.chars()
		.map(|character| {
			if character.is_alphanumeric() {
				character.to_ascii_lowercase()
			} else {
				'-'
			}
		})
		.collect();
	format!("footnote-{slug}")
}

fn position(node: &Node) -> (usize, usize) {
	node.position().map_or((0, 0), |position| {
		(position.start.line, position.start.column)
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_alert_markers() {
		assert_eq!(alert("[!TIP]"), Some((CalloutKind::Tip, "")));
		assert_eq!(alert("[!note]\nBody"), Some((CalloutKind::Note, "\nBody")));
		assert_eq!(alert("[!NOTE] Body"), Some((CalloutKind::Note, " Body")));
		assert_eq!(alert("[!NOTEWORTHY]"), None);
		assert_eq!(alert("[!NOTE]worthy"), None);
		assert_eq!(alert("[link]"), None);
	}

	#[test]
	fn reads_code_titles() {
		assert_eq!(
			code_title("title=\"leptoskit.toml\""),
			Some("leptoskit.toml")
		);
		assert_eq!(code_title("ignore title=\"a b\" x"), Some("a b"));
		assert_eq!(code_title("title=open"), None);
	}

	#[test]
	fn slugs_like_github() {
		let components = Components::empty();
		let document = Document::parse(
			"# `view!` and `#[component]`\n## Setup\n## Setup\n## Über 2.0!\n## !!!",
		)
		.unwrap();
		let rendered = Renderer::new(&components).render(&document).unwrap();
		let ids: Vec<_> = rendered
			.headings
			.iter()
			.map(|heading| heading.id.as_str())
			.collect();

		assert_eq!(
			ids,
			[
				"view-and-component",
				"setup",
				"setup-1",
				"über-20",
				"section"
			]
		);
	}
}
