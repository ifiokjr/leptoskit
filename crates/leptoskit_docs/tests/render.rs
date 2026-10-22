//! Markdown to HTML, snapshotted fixture by fixture, and every way a document
//! can be wrong.

use leptos::prelude::*;
use leptoskit_docs::Components;
use leptoskit_docs::Document;
use leptoskit_docs::Heading;
use leptoskit_docs::Link;
use leptoskit_docs::PropError;
use leptoskit_docs::Renderer;
use leptoskit_docs::render_to_html;

/// One block element per line, so snapshot diffs point at what changed.
fn pretty(html: &str) -> String {
	const BLOCKS: &[&str] = &[
		"<aside",
		"<blockquote",
		"<div",
		"<figure",
		"<h1",
		"<h2",
		"<h3",
		"<h4",
		"<hr",
		"<li",
		"<ol",
		"<p",
		"<pre",
		"<section",
		"<table",
		"<tbody",
		"<thead",
		"<tr",
		"<ul",
		"</aside",
		"</blockquote",
		"</div",
		"</figure",
		"</ol",
		"</section",
		"</table",
		"</ul",
		"<details",
		"<summary",
		"<figcaption",
		"<a class=\"lk-card\"",
	];
	let mut out = String::with_capacity(html.len() * 2);
	let mut rest = html;

	while let Some(index) = rest.find('<') {
		let (text, tag) = rest.split_at(index);
		out.push_str(text);
		if BLOCKS.iter().any(|block| tag.starts_with(block))
			&& !out.is_empty()
			&& !out.ends_with('\n')
		{
			out.push('\n');
		}
		out.push('<');
		rest = &tag[1..];
	}
	out.push_str(rest);
	out
}

fn snapshot(source: &str, components: &Components) -> String {
	pretty(
		&render_to_html(source, components).unwrap_or_else(|diagnostics| {
			panic!(
				"{}",
				diagnostics
					.iter()
					.map(ToString::to_string)
					.collect::<Vec<_>>()
					.join("\n")
			)
		}),
	)
}

#[component]
fn Badge(tone: String, count: u32, pill: bool, children: Children) -> impl IntoView {
	view! {
		<span class="badge" data-tone=tone data-pill=pill.then_some("")>
			{children()} <b>{count}</b>
		</span>
	}
}

fn custom_components() -> Components {
	Components::new()
		.with("Badge", |props, children| {
			let tone = props.require("tone")?.to_owned();
			let count = props.parse("count")?.unwrap_or(0);
			let pill = props.parse("pill")?.unwrap_or(false);
			Ok(view! { <Badge tone count pill>{children()}</Badge> }.into_any())
		})
		.with("Feature", |props, _children| {
			let title = props.require("title")?.to_owned();
			let href = props
				.url("href")
				.ok_or_else(|| PropError::new("<Feature> needs an `href`"))?;
			Ok(view! { <a class="feature" href=href>{title}</a> }.into_any())
		})
}

#[test]
fn renders_commonmark() {
	insta::assert_snapshot!(snapshot(
		include_str!("fixtures/render/commonmark.md"),
		&Components::new()
	));
}

#[test]
fn renders_github_flavoured_markdown() {
	insta::assert_snapshot!(snapshot(
		include_str!("fixtures/render/gfm.md"),
		&Components::new()
	));
}

#[test]
fn renders_built_in_components_and_html() {
	insta::assert_snapshot!(snapshot(
		include_str!("fixtures/render/components.md"),
		&Components::new()
	));
}

#[test]
fn renders_registered_leptos_components() {
	insta::assert_snapshot!(snapshot(
		include_str!("fixtures/render/custom.md"),
		&custom_components()
	));
}

#[test]
fn describes_the_document() {
	let components = Components::new();
	let document = Document::parse(include_str!("fixtures/render/commonmark.md")).unwrap();
	let rendered = Renderer::new(&components)
		.rewrite_urls(|url| format!("/rewritten/{url}"))
		.render(&document)
		.unwrap();

	assert_eq!(rendered.title.as_deref(), Some("The title of the page"));
	assert!(
		rendered
			.summary
			.as_deref()
			.is_some_and(|summary| summary.starts_with("The first paragraph has strong, emphasis")),
		"{:?}",
		rendered.summary
	);
	assert_eq!(
		rendered.headings,
		[
			Heading {
				depth: 1,
				id: "the-title-of-the-page".to_owned(),
				text: "The title of the page".to_owned()
			},
			Heading {
				depth: 2,
				id: "lists".to_owned(),
				text: "Lists".to_owned()
			},
			Heading {
				depth: 2,
				id: "code".to_owned(),
				text: "Code".to_owned()
			},
			Heading {
				depth: 2,
				id: "references".to_owned(),
				text: "References".to_owned()
			},
		]
	);
	assert_eq!(
		rendered
			.links
			.iter()
			.map(|link| link.url.as_str())
			.collect::<Vec<_>>(),
		[
			"guide/next.md",
			"assets/dot.svg",
			"guide/next.md#setup",
			"assets/dot.svg"
		]
	);
	assert_eq!(
		rendered.links[0],
		Link {
			url: "guide/next.md".to_owned(),
			line: 5,
			column: 13
		}
	);

	let sections: Vec<_> = rendered
		.sections
		.iter()
		.map(|section| section.id.as_deref())
		.collect();
	assert_eq!(
		sections,
		[
			Some("the-title-of-the-page"),
			Some("lists"),
			Some("code"),
			Some("references")
		]
	);
	assert_eq!(
		rendered.sections[1].text,
		"a tight list with two items and a nested ordered list an ordered list that starts at \
		 three a loose list whose items are paragraphs A plain quote, with markup."
	);
	assert_eq!(
		rendered.sections[2].text, "",
		"code blocks stay out of search"
	);

	let html = rendered.view.to_html();
	assert!(html.contains("href=\"/rewritten/guide/next.md\""), "{html}");
	assert!(html.contains("src=\"/rewritten/assets/dot.svg\""), "{html}");
}

#[test]
fn records_links_in_component_props_and_html() {
	let components = custom_components();
	let document = Document::parse(
		"<Feature title=\"A\" href=\"a.md#x\" />\n\n<img src=\"b.png\" alt=\"\" />\n\n<a \
		 href=\"c.md\">C</a>\n",
	)
	.unwrap();
	let rendered = Renderer::new(&components)
		.rewrite_urls(|url| url.replace(".md", ".html"))
		.render(&document)
		.unwrap();
	let urls: Vec<_> = rendered
		.links
		.iter()
		.map(|link| link.url.as_str())
		.collect();

	assert_eq!(urls, ["a.md#x", "b.png", "c.md"]);
	let html = rendered.view.to_html();
	assert!(
		html.contains("href=\"a.html#x\"") && html.contains("href=\"c.html\""),
		"{html}"
	);
}

#[test]
fn lists_footnotes_and_headings_as_anchors() {
	let components = Components::new();
	let document = Document::parse("# Top\n\nText[^a].\n\n[^a]: Note.\n").unwrap();
	let rendered = leptoskit_docs::render(&document, &components).unwrap();

	assert_eq!(
		rendered.anchors().collect::<Vec<_>>(),
		["top", "footnote-a"]
	);
}

#[test]
fn reports_every_problem_in_a_document() {
	let source = "\
<Missing />

<Callout kind=\"danger\">Bad kind.</Callout>

<Callout tone=\"loud\">Unknown prop.</Callout>

<Card>No title.</Card>

<Tab {...props}>Spread.</Tab>

<div class={styles}>Expression on HTML.</div>

<br>Children of a void element.</br>

<span {...props}>Spread on HTML.</span>

<Badge tone=\"mint\" count=\"many\">Unparsable count.</Badge>
";
	let diagnostics = Document::parse(source)
		.map_err(|diagnostic| vec![diagnostic])
		.and_then(|document| leptoskit_docs::render(&document, &custom_components()).map(|_| ()))
		.unwrap_err();

	insta::assert_snapshot!(
		diagnostics
			.iter()
			.map(ToString::to_string)
			.collect::<Vec<_>>()
			.join("\n")
	);
}

#[test]
fn escapes_text_and_attributes() {
	let html = render_to_html(
		"Tags in text: \\<b> & <img src=\"a.png\" alt='<!> &amp; \"quoted\"' />",
		&Components::empty(),
	)
	.unwrap();

	assert_eq!(
		html,
		"<p>Tags in text: &lt;b&gt; &amp; <img src=\"a.png\" alt=\"&lt;!&gt; &amp; \
		 &quot;quoted&quot;\"></p>"
	);
}

#[test]
fn renders_nothing_for_an_empty_document() {
	assert_eq!(render_to_html("", &Components::new()).unwrap(), "");
}

#[test]
fn reports_parse_errors_with_their_position() {
	let diagnostics =
		render_to_html("Fine.\n\n<Callout>\n\nNever closed.\n", &Components::new()).unwrap_err();

	assert_eq!(diagnostics.len(), 1);
	assert_eq!(
		(diagnostics[0].line, diagnostics[0].path.as_deref()),
		(6, None)
	);
}
