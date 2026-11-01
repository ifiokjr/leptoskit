<h1 align="center">leptoskit_docs</h1>

<p align="center">
  <strong>leptoskit_docs</strong> turns Markdown with leptos components into a static site with navigation, search, highlighting, and checked links.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_docs"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_docs"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_docs.svg"></a>
  <a href="https://docs.rs/leptoskit_docs"><img alt="docs.rs" src="https://docs.rs/leptoskit_docs/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Write documentation in Markdown and drop leptos components straight into it. A capitalised tag is a component; the Markdown between its tags is the component's children.

Lowercase tags (`<figure>`, `<kbd>`) are plain HTML, and GitHub alerts (`> [!WARNING]`) render as callouts too.

## Your own components

Components are leptos components. Register them by tag name, read their attributes from `Props`, and render their children:

```rust
use leptos::prelude::*;
use leptoskit_docs::Components;
use leptoskit_docs::Site;

#[component]
fn Badge(tone: String, children: Children) -> impl IntoView {
	view! {
		<span class="badge" data-tone=tone>
			{children()}
		</span>
	}
}

fn main() -> Result<(), leptoskit_docs::Error> {
	let components = Components::new().with("Badge", |props, children| {
		let tone = props.get("tone").unwrap_or("ember").to_owned();
		Ok(view! { <Badge tone>{children()}</Badge> }.into_any())
	});

	Site::new("docs/src")
		.title("My app")
		.components(components)
		.stylesheet("theme/badge.css")
		.build("docs/dist")?;
	Ok(())
}
```

Mistakes fail the build with the file, line, and column: an unknown component, a misspelt attribute (every attribute a component never reads is reported), a prop that does not parse, a link to a page or heading that does not exist, or a link to a local address such as `http://localhost:3000`, which only works on the reader's own machine and belongs in inline code.

## The site

`Site` reads an mdBook-style `SUMMARY.md` and builds every chapter it lists into a static page beside its source, `guide/install.md` into `guide/install.html`. You get:

- chapter navigation, an on-page table of contents, and previous and next links;
- search over every section, from an index built with the site;
- syntax highlighting at build time with tree-sitter grammars, so pages need no highlighting script;
- light and dark themes, copy buttons, linked headings, and tabs that switch together;
- a 404 page, canonical links, and link previews when you give the site its public address;
- `Site::serve`, a development server that rebuilds on every edit and reloads open pages.

Every page reads in full without JavaScript; the script only adds search, the theme switch, and the other conveniences.

## The dialect

GitHub-flavoured Markdown with MDX's JSX: tables, task lists, strikethrough, footnotes, and autolink literals, plus tags. As in MDX, every tag must be closed (`<br />`, not `<br>`) and indented code blocks are off, so component children can be indented. Unlike MDX there is no JavaScript: braces in prose are plain text, and in an attribute (`count={3}`) they pass their text to the component. HTML comments are allowed and dropped.

## Why markdown-rs

The parser is [`markdown`](https://crates.io/crates/markdown) (markdown-rs), the Rust port of micromark, which MDX itself is built on. It parses MDX's JSX natively, with positions on every node, so components arrive as typed elements with their attributes instead of raw HTML strings to re-parse. `pulldown-cmark` and `comrak` are faster and both excellent CommonMark parsers, but neither understands JSX; on either, a component with Markdown children would mean pairing up raw HTML fragments by hand.

## How it fits

The leptoskit book is built with it, and apps can use the same generator for their own docs. It is MDX-shaped but Rust-native: components are leptos components, and mistakes fail the build with a location.

## Use it

```markdown
# Installing

<Callout kind="tip">

Install the CLI once and every project can use it.

</Callout>

<Steps>

### Install the CLI

Run `cargo install leptoskit_cli --locked`.

### Create a project

Run `lk new my_app`.

</Steps>
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_docs)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Writing docs](https://ifiokjr.github.io/leptoskit/guide/14-writing-docs.html)
