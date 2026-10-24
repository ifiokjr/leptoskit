# Writing docs

[`leptoskit_docs`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_docs) turns a folder of Markdown into a documentation site like this one. It works the way [MDX](https://mdxjs.com) does, with leptos where MDX has React: a capitalised tag in a Markdown file renders a leptos component, and the Markdown between its tags becomes the component's children.

This page is built that way. The callout below is a `<Callout>`, the numbered steps are a `<Steps>`, and the tabs are `<Tabs>`.

<Callout kind="tip" title="Try it on this site">

Every page has an "Edit this page" link at the bottom. The source of this page is [`docs/src/guide/14-writing-docs.md`](https://github.com/ifiokjr/leptoskit/blob/main/docs/src/guide/14-writing-docs.md).

</Callout>

## Set up a docs site

<Steps>

### Write a book

Put your pages in a folder with a `SUMMARY.md` that lists them, in the same format [mdBook](https://rust-lang.github.io/mdBook/format/summary.html) uses:

```markdown title="docs/src/SUMMARY.md"
# Summary

[Introduction](readme.md)

# Guide

- [Installation](guide/installation.md)
- [Configuration](guide/configuration.md)
```

Each chapter builds into a page beside its source: `guide/installation.md` becomes `guide/installation.html`, and a `readme.md` becomes its folder's `index.html`. Every other file in the folder, such as images, is copied over as it is.

### Add a small binary

Depend on `leptoskit_docs` and describe the site:

```rust title="docs/site/src/main.rs"
use leptoskit_docs::Site;

fn main() -> Result<(), leptoskit_docs::Error> {
	let build = Site::new("docs/src")
		.title("My app")
		.repository("https://github.com/me/my_app")
		.edit_url("https://github.com/me/my_app/edit/main/docs/src/{path}")
		.build("docs/dist")?;

	println!("built {} pages", build.pages);
	Ok(())
}
```

### Write and preview

`Site::serve` builds the site, serves it, rebuilds whenever a file in the book changes, and reloads every open page:

```rust
let server = Site::new("docs/src").serve("docs/dist", "127.0.0.1:4400")?;
println!("http://{}/", server.address());
server.wait();
```

### Publish

`docs/dist` is plain static files. Upload it to GitHub Pages or any static host. Give the site its public address with `.url("https://me.github.io/my_app/")` so it can write canonical links, link previews, and a 404 page that works at any depth.

</Steps>

## Use components

The built-in components cover what most docs need:

<Tabs>
<Tab label="Callout">

```markdown
<Callout kind="warning" title="Careful">

Deleting the cache forces a full rebuild.

</Callout>
```

`kind` is `note`, `tip`, `important`, `warning`, or `caution`. GitHub's alert syntax, `> [!WARNING]`, renders the same aside.

</Tab>
<Tab label="Steps">

```markdown
<Steps>

### Install the CLI

Run `cargo install leptoskit_cli --locked`.

### Create a project

Run `lk new my_app`.

</Steps>
```

Every `###` heading inside `<Steps>` becomes a numbered step.

</Tab>
<Tab label="Cards">

```markdown
<Cards>
	<Card title="Get started" href="guide/installation.md">
		Install the CLI and scaffold a project.
	</Card>
</Cards>
```

Card links are checked like any other link.

</Tab>
<Tab label="Tabs">

```markdown
<Tabs>
<Tab label="macOS">Run `brew install …`.</Tab>
<Tab label="Linux">Run `apt install …`.</Tab>
</Tabs>
```

Tabs with the same label switch together across the page, and the choice is remembered.

</Tab>
</Tabs>

Lowercase tags are plain HTML, so `<kbd>⌘K</kbd>` renders <kbd>⌘K</kbd>. As in MDX, every tag must be closed (`<br />`, not `<br>`), and indented code blocks are off. Keep a component's Markdown children unindented anyway: Markdown formatters such as dprint still read indented lines after a blank line as code.

## Write your own components

Your own components are ordinary leptos components, registered under the tag name documents use:

```rust
use leptos::prelude::*;
use leptoskit_docs::Components;
use leptoskit_docs::Site;

#[component]
fn Badge(tone: String, children: Children) -> impl IntoView {
	view! { <span class="badge" data-tone=tone>{children()}</span> }
}

let components = Components::new().with("Badge", |props, children| {
	let tone = props.get("tone").unwrap_or("ember").to_owned();
	Ok(view! { <Badge tone>{children()}</Badge> }.into_any())
});

Site::new("docs/src")
	.components(components)
	.stylesheet("theme/badge.css")
	.build("docs/dist")?;
```

Then write `<Badge tone="mint">New</Badge>` in any page. The component reads its attributes from `Props`:

| Method                  | Returns                                                                                      |
| ----------------------- | -------------------------------------------------------------------------------------------- |
| `props.get("name")`     | The attribute's text, if it is set. An attribute without a value, like `wide`, reads `true`. |
| `props.require("name")` | The text, or an error that names the missing attribute.                                      |
| `props.parse("name")`   | The text parsed with `FromStr`, so `count={3}` can become a `u32`.                           |
| `props.url("name")`     | A link target, rewritten from `.md` to the built page and checked like any other link.       |

The home page's hero, the screens in [examples](../examples.md), and the swatches on the [brand page](../brand.md) are components written this way. Their source is in [`docs/site`](https://github.com/ifiokjr/leptoskit/tree/main/docs/site).

## What a build checks

A build either succeeds or lists every problem in the book, each with its file, line, and column:

```text
the docs have 3 problems:
  guide/install.md:12:1: unknown component <Calout>; the known components are Callout, Card, …
  guide/install.md:20:1: <Callout> has no `knd` attribute
  readme.md:8:15: broken link `guide/setup.md#usage`: `guide/setup.md` has no heading with the id `usage`
```

- Markdown that does not parse, such as a tag that is never closed.
- Components that are not registered, and attributes a component never reads, which is how a misspelt prop shows up.
- Props that do not parse, such as `kind="danger"` on a `<Callout>`.
- Links to chapters that are not in `SUMMARY.md`, to headings that do not exist, to files missing from the book, or outside the book. Absolute paths are rejected too, since they break when the site is served from a subdirectory like `/leptoskit/`.
- Links to local addresses such as `http://localhost:3000` or `http://127.0.0.1:3000`, including bare URLs that GitHub-flavoured Markdown would turn into links. They only work on the reader's own machine, so write them as inline code.

Nothing is written until the build is clean, so a broken link never ships.
